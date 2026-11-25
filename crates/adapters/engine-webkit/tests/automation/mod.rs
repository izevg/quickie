use std::cell::Cell;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use objc2::{MainThreadMarker, rc::autoreleasepool};
use objc2_app_kit::NSApplication;
use quickie_engine_webkit::{
    AutomationError, BrowsingContext, InteractiveAutomation, LocalAutomation,
};
use url::Url;

pub(super) const COOKIE: &str = "quickie_automation=isolated";
static REDIRECT_TARGET: Mutex<Option<u16>> = Mutex::new(None);
static CANCELLATION_REQUESTS: AtomicU64 = AtomicU64::new(0);

pub(super) fn check(mtm: MainThreadMarker, base: &Url) -> Result<&'static str, String> {
    checkpoint("starting AppKit");
    let result = autoreleasepool(|_| run(mtm, base));
    checkpoint("outer autorelease pool drained");
    result
}

fn run(mtm: MainThreadMarker, base: &Url) -> Result<&'static str, String> {
    let _app = NSApplication::sharedApplication(mtm);
    let expected = std::env::var("QUICKIE_AUTOMATION_FRAMEWORK").ok();
    checkpoint("opening first standalone session");
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

    scenario("second standalone session", || {
        standalone_isolation(mtm, base, &initial, &check_cookie)
    })?;
    if only_context(&mut first)?.url != cookie_present.as_str() {
        return Err(String::from(
            "closing the second session affected the first session",
        ));
    }

    // Keep the first store alive while the interactive session checks that it has no cookie.
    scenario("interactive cookie and redirect", || {
        interactive_cookie_and_redirect(mtm, base)
    })?;
    drop(first);
    checkpoint("first standalone session released");

    interactive(mtm, base)?;
    Ok(
        "standalone and interactive navigation, private stores, redirect and download policy, failure and timeout",
    )
}

fn standalone_isolation(
    mtm: MainThreadMarker,
    base: &Url,
    initial: &BrowsingContext,
    check_cookie: &Url,
) -> Result<(), String> {
    let route = |path: &str| base.join(path).map_err(|error| error.to_string());
    let cookie_absent = route("automation/cookie-absent")?;
    let mut second = LocalAutomation::open(mtm).map_err(|error| error.to_string())?;
    let second_context = only_context(&mut second)?;
    if !matches!(
        second.navigate(initial, check_cookie),
        Err(AutomationError::CommandFailed)
    ) {
        return Err(String::from("another session accepted a foreign context"));
    }
    second
        .navigate(&second_context, check_cookie)
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
    checkpoint("waiting for standalone navigation timeout");
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
    checkpoint("standalone timeout verified, releasing second session");
    Ok(())
}

fn only_context(session: &mut LocalAutomation) -> Result<BrowsingContext, String> {
    let contexts = session.contexts().map_err(|error| error.to_string())?;
    let [context] = contexts
        .try_into()
        .map_err(|_: Vec<_>| "expected one private browsing context")?;
    Ok(context)
}

fn interactive_cookie_and_redirect(mtm: MainThreadMarker, base: &Url) -> Result<(), String> {
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
    Ok(())
}

fn interactive(mtm: MainThreadMarker, base: &Url) -> Result<(), String> {
    let route = |name| base.join(name).map_err(|error| error.to_string());
    scenario("interactive cancellation preserves session", || {
        cancellation_preserves_session(mtm, base)
    })?;
    scenario("interactive dispatch rechecks provenance", || {
        dispatch_rechecks_provenance(mtm, base)
    })?;
    scenario("interactive attachment policy", || {
        let mut session = interactive_session(mtm)?;
        let result = until_navigation(&mut session, &route("automation/attachment")?)?;
        if !matches!(result, Err(AutomationError::CommandFailed)) {
            return Err(format!(
                "interactive automation did not reject an attachment: {result:?}"
            ));
        }
        Ok(())
    })?;
    scenario("interactive navigation timeout", || {
        let mut session = interactive_session(mtm)?;
        checkpoint("waiting for interactive navigation timeout");
        let result = until_navigation(&mut session, &route("automation/stall")?)?;
        if !matches!(result, Err(AutomationError::Timeout)) {
            return Err(format!(
                "interactive navigation did not time out: {result:?}"
            ));
        }
        checkpoint("interactive timeout verified, releasing session");
        Ok(())
    })?;
    scenario("interactive callback after drop", || {
        no_callback_after_drop(mtm, &route("automation/check-cookie")?)
    })
}

fn cancellation_preserves_session(mtm: MainThreadMarker, base: &Url) -> Result<(), String> {
    let route = |name| base.join(name).map_err(|error| error.to_string());
    let mut session = interactive_session(mtm)?;
    checkpoint("cancellation: setting private cookie");
    until_navigation(&mut session, &route("automation/set-cookie")?)?
        .map_err(|error| format!("setting private cookie: {error}"))?;
    cancel_and_resume(&mut session, base, false)?;
    cancel_and_resume(&mut session, base, true)
}

fn cancel_and_resume(
    session: &mut InteractiveAutomation,
    base: &Url,
    wait_for_request: bool,
) -> Result<(), String> {
    let route = |name| base.join(name).map_err(|error| error.to_string());
    let view = std::ptr::from_ref(session.view());
    let abandoned = Rc::new(Cell::new(false));
    let called = Rc::clone(&abandoned);
    let previous_requests = CANCELLATION_REQUESTS.load(Ordering::Relaxed);
    let stall = if wait_for_request {
        "automation/stall-cancel"
    } else {
        "automation/stall"
    };
    session
        .navigate(&route(stall)?, move |_| called.set(true))
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut loading = false;
    while Instant::now() < deadline {
        super::spin();
        // SAFETY: the live view belongs to this main-thread session.
        if unsafe { session.view().isLoading() }
            && (!wait_for_request
                || CANCELLATION_REQUESTS.load(Ordering::Relaxed) > previous_requests)
        {
            loading = true;
            break;
        }
    }
    if !loading {
        return Err("the cancellation fixture never started loading".to_owned());
    }
    checkpoint(if wait_for_request {
        "cancellation: stopping server-confirmed request"
    } else {
        "cancellation: stopping queued API load"
    });
    session.cancel_navigation();
    let stopped = Instant::now() + Duration::from_secs(2);
    loop {
        // SAFETY: the live view belongs to this main-thread session.
        if !unsafe { session.view().isLoading() } {
            break;
        }
        if Instant::now() >= stopped {
            return Err("cancelled native view is still loading".to_owned());
        }
        super::spin();
    }
    if !session.is_connected() || std::ptr::from_ref(session.view()) != view {
        return Err("cancelling replaced or disconnected the private session".to_owned());
    }
    checkpoint("cancellation: navigating again in retained session");
    until_navigation(session, &route("automation/check-cookie")?)?
        .map_err(|error| format!("navigation after cancellation: {error}"))?;
    // SAFETY: a read-only getter on the live main-thread view.
    let shown = unsafe { session.view().URL() }.and_then(|url| url.absoluteString());
    if abandoned.get()
        || shown.is_none_or(|url| url.to_string() != format!("{base}automation/cookie-present"))
    {
        return Err("cancelling delivered an old callback or lost the private cookie".to_owned());
    }
    Ok(())
}

fn dispatch_rechecks_provenance(mtm: MainThreadMarker, base: &Url) -> Result<(), String> {
    let mut session = interactive_session(mtm)?;
    let eligible = Rc::new(Cell::new(true));
    let check = Rc::clone(&eligible);
    let result = Rc::new(Cell::new(None));
    let completed = Rc::clone(&result);
    let url = base
        .join("automation/set-cookie")
        .map_err(|e| e.to_string())?;
    session
        .navigate_when(
            &url,
            move || check.get(),
            move |answer| completed.set(Some(answer)),
        )
        .map_err(|error| error.to_string())?;
    eligible.set(false);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut answer = None;
    while Instant::now() < deadline && answer.is_none() {
        super::spin();
        answer = result.take();
    }
    // A revoked dispatch must leave the original view connected and its blank page intact.
    // SAFETY: the view is owned by the live main-thread session.
    let shown = unsafe { session.view().URL() }.and_then(|url| url.absoluteString());
    if !matches!(answer, Some(Err(AutomationError::CommandFailed)))
        || !session.is_connected()
        || shown.is_some_and(|shown| shown.to_string() == url.as_str())
    {
        return Err("revoked provenance dispatched a native navigation".to_owned());
    }
    Ok(())
}

fn no_callback_after_drop(mtm: MainThreadMarker, url: &Url) -> Result<(), String> {
    let mut session = interactive_session(mtm)?;
    let abandoned = Rc::new(Cell::new(false));
    let called = Rc::clone(&abandoned);
    session
        .navigate(url, move |_| {
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

fn scenario(name: &str, run: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    checkpoint(&format!("starting {name}"));
    let result = autoreleasepool(|_| run());
    let outcome = if result.is_ok() { "passed" } else { "failed" };
    checkpoint(&format!("{outcome} {name}"));
    result
}

fn checkpoint(message: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "automation checkpoint: {message}");
    let _ = stdout.flush();
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
        "stall" | "stall-cancel" => {
            if route == "stall-cancel" {
                CANCELLATION_REQUESTS.fetch_add(1, Ordering::Relaxed);
            }
            // Wait for the browser to cancel this request. Bound the server side too.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
            let _ = stream.read(&mut [0]);
        }
        // Leave without writing a response to the deliberate `/close` request.
        _ => {}
    }
}
