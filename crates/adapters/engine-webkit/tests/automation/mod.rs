use std::cell::Cell;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use quickie_engine_webkit::{
    AutomationError, BrowsingContext, InteractiveAutomation, LocalAutomation,
};
use url::Url;

pub(super) const COOKIE: &str = "quickie_automation=isolated";
static REDIRECT_TARGET: Mutex<Option<u16>> = Mutex::new(None);

pub(super) fn check(mtm: MainThreadMarker, base: &Url) -> Result<&'static str, String> {
    objc2::rc::autoreleasepool(|_| run(mtm, base))
}

fn run(mtm: MainThreadMarker, base: &Url) -> Result<&'static str, String> {
    let _app = NSApplication::sharedApplication(mtm);
    let expected = std::env::var("QUICKIE_AUTOMATION_FRAMEWORK").ok();
    let mut first = match LocalAutomation::open(mtm) {
        Ok(_) if expected.as_deref() == Some("system") => {
            return Err(String::from(
                "expected system WebKit without local automation",
            ));
        }
        Ok(session) => session,
        Err(AutomationError::Unsupported) if expected.as_deref() != Some("fork") => {
            return Ok("unsupported on system WebKit");
        }
        Err(error) => return Err(error.to_string()),
    };

    let initial = only_context(&mut first)?;
    let file =
        Url::parse("file:///tmp/quickie-automation-test").map_err(|error| error.to_string())?;
    if !matches!(
        first.navigate(&initial, &file),
        Err(AutomationError::InvalidUrl)
    ) {
        return Err(String::from("automation accepted a file URL"));
    }

    let route = |path: &str| base.join(path).map_err(|error| error.to_string());
    let set_cookie = route("automation/set-cookie")?;
    let check_cookie = route("automation/check-cookie")?;
    let cookie_present = route("automation/cookie-present")?;
    let cookie_absent = route("automation/cookie-absent")?;

    first
        .navigate(&initial, &set_cookie)
        .map_err(|error| error.to_string())?;
    if only_context(&mut first)?.url != set_cookie.as_str() {
        return Err(String::from(
            "fixture navigation did not update the browsing context",
        ));
    }
    first
        .navigate(&initial, &check_cookie)
        .map_err(|error| error.to_string())?;
    if only_context(&mut first)?.url != cookie_present.as_str() {
        return Err(String::from(
            "fixture did not receive the cookie from the first session",
        ));
    }

    let mut second = LocalAutomation::open(mtm).map_err(|error| error.to_string())?;
    let second_context = only_context(&mut second)?;
    if !matches!(
        second.navigate(&initial, &check_cookie),
        Err(AutomationError::CommandFailed)
    ) {
        return Err(String::from("another session accepted a foreign context"));
    }
    second
        .navigate(&second_context, &check_cookie)
        .map_err(|error| error.to_string())?;
    if only_context(&mut second)?.url != cookie_absent.as_str() {
        return Err(String::from(
            "a fresh automation session saw a cookie from another session",
        ));
    }

    if !matches!(
        second.navigate(&second_context, &route("automation/close")?),
        Err(AutomationError::CommandFailed)
    ) {
        return Err(String::from(
            "automation did not report a navigation failure",
        ));
    }
    if !matches!(
        second.navigate(&second_context, &route("automation/stall")?),
        Err(AutomationError::Timeout)
    ) {
        return Err(String::from(
            "automation did not time out a stalled navigation",
        ));
    }
    if !matches!(second.contexts(), Err(AutomationError::Transport)) {
        return Err(String::from(
            "automation did not close the session on timeout",
        ));
    }
    if only_context(&mut first)?.url != cookie_present.as_str() {
        return Err(String::from(
            "closing the second session affected the first session",
        ));
    }

    interactive(mtm, base)?;
    Ok(
        "standalone and interactive navigation, private stores, redirect and download policy, failure and timeout",
    )
}

fn only_context(session: &mut LocalAutomation) -> Result<BrowsingContext, String> {
    let contexts = session.contexts().map_err(|error| error.to_string())?;
    let [context] = contexts
        .try_into()
        .map_err(|_: Vec<_>| "expected one private browsing context")?;
    Ok(context)
}

fn interactive(mtm: MainThreadMarker, base: &Url) -> Result<(), String> {
    let route = |name| base.join(name).map_err(|error| error.to_string());
    let mut session = interactive_session(mtm)?;
    // The standalone session already set a cookie for this origin. The interactive session
    // must finish a permitted same-origin redirect in its own fresh store.
    let result = until_navigation(&mut session, &route("automation/check-cookie")?)?;
    result.map_err(|error| error.to_string())?;
    // SAFETY: a read-only getter on the live main-thread session view.
    let shown = unsafe { session.view().URL() }.and_then(|url| url.absoluteString());
    if shown.is_none_or(|url| url.to_string() != format!("{base}automation/cookie-absent")) {
        return Err(String::from(
            "interactive automation shared another session's cookie store",
        ));
    }

    let destination = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    destination
        .set_nonblocking(true)
        .map_err(|e| e.to_string())?;
    *REDIRECT_TARGET.lock().map_err(|e| e.to_string())? =
        Some(destination.local_addr().map_err(|e| e.to_string())?.port());
    let result = until_navigation(&mut session, &route("automation/redirect-cross-origin")?)?;

    if !matches!(destination.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    {
        return Err(String::from(
            "automation contacted the cross-origin redirect target",
        ));
    }
    if !matches!(result, Err(AutomationError::CommandFailed)) {
        return Err(format!(
            "automation did not cancel the cross-origin redirect: {result:?}"
        ));
    }

    let mut session = interactive_session(mtm)?;
    let result = until_navigation(&mut session, &route("automation/attachment")?)?;
    if !matches!(result, Err(AutomationError::CommandFailed)) {
        return Err(format!(
            "interactive automation did not reject an attachment: {result:?}"
        ));
    }

    let mut session = interactive_session(mtm)?;
    let result = until_navigation(&mut session, &route("automation/stall")?)?;
    if !matches!(result, Err(AutomationError::Timeout)) {
        return Err(format!(
            "interactive navigation did not time out: {result:?}"
        ));
    }

    let mut session = interactive_session(mtm)?;
    let abandoned = Rc::new(Cell::new(false));
    let called = Rc::clone(&abandoned);
    session
        .navigate(&route("automation/check-cookie")?, move |_| {
            called.set(true);
        })
        .map_err(|error| error.to_string())?;
    drop(session);
    super::spin();
    super::spin();
    if abandoned.get() {
        return Err(String::from(
            "interactive automation called the completion after drop",
        ));
    }
    Ok(())
}

fn interactive_session(mtm: MainThreadMarker) -> Result<InteractiveAutomation, String> {
    InteractiveAutomation::open(mtm).map_err(|error| error.to_string())
}

fn until_navigation(
    session: &mut InteractiveAutomation,
    request: &Url,
) -> Result<Result<(), AutomationError>, String> {
    let result = Rc::new(Cell::new(None));
    let completed = Rc::clone(&result);
    session
        .navigate(request, move |answer| completed.set(Some(answer)))
        .map_err(|error| error.to_string())?;
    if result.take().is_some() {
        return Err(String::from(
            "interactive automation completed inside the call",
        ));
    }
    if !matches!(
        session.navigate(request, |_| {}),
        Err(AutomationError::Busy)
    ) {
        return Err(String::from(
            "interactive automation accepted concurrent navigation",
        ));
    }

    let deadline = Instant::now() + Duration::from_secs(11);
    while Instant::now() < deadline {
        super::spin();
        if let Some(result) = result.take() {
            return Ok(result);
        }
    }
    Err(String::from(
        "interactive navigation did not report a result",
    ))
}

pub(super) fn answer(stream: &mut TcpStream, route: &str, has_cookie: bool) {
    match route {
        "redirect-cross-origin" => {
            if let Some(port) = REDIRECT_TARGET.lock().ok().and_then(|port| *port) {
                let _ = write!(
                    stream,
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
        }
        "attachment" => {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Disposition: attachment; filename=probe.html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
        "set-cookie" => {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nSet-Cookie: {COOKIE}; Path=/\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
        "check-cookie" => {
            let destination = if has_cookie {
                "cookie-present"
            } else {
                "cookie-absent"
            };
            let _ = write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: /automation/{destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
        "cookie-present" | "cookie-absent" => {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
        "stall" => {
            // Wait for the browser to cancel this request. Bound the server side too.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
            let _ = stream.read(&mut [0]);
        }
        // Leave without writing a response to the deliberate `/close` request.
        _ => {}
    }
}
