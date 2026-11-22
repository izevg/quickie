use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use quickie_engine_webkit::{AutomationError, BrowsingContext, LocalAutomation};
use url::Url;

pub(super) const COOKIE: &str = "quickie_automation=isolated";

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
    Ok("navigation, fresh cookie store, foreign context, failure and timeout")
}

fn only_context(session: &mut LocalAutomation) -> Result<BrowsingContext, String> {
    let contexts = session.contexts().map_err(|error| error.to_string())?;
    let [context] = contexts
        .try_into()
        .map_err(|_: Vec<_>| "expected one private browsing context")?;
    Ok(context)
}

pub(super) fn answer(stream: &mut TcpStream, route: &str, has_cookie: bool) {
    match route {
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
