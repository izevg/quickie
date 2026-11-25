//! Drives the macOS video presentation API in a normal tab of the offscreen harness.

use std::io::Write;
use std::net::TcpStream;

use objc2::{msg_send, sel};
use objc2_app_kit::NSView;
use objc2_foundation::{NSObjectProtocol, NSString};
use objc2_web_kit::{WKContentWorld, WKWebView};
use quickie_application::conformance::{Harness, WINDOW};
use quickie_application::{Engine, EngineEvent, EnginePort, Fullscreen, PageContent, PageSurface};
use quickie_domain_browsing::{Jar, JarId, Profile, TabId};

use super::WebKit;

pub(super) const ROUTE: &str = "video-fullscreen/fixture";

pub(super) fn answer(stream: &mut TcpStream) {
    let body = include_str!("fixture.html");
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

pub(super) fn check(harness: &mut WebKit) -> Result<(), String> {
    check_route(harness, ROUTE, false)
}

pub(super) fn native(harness: &mut WebKit) -> Result<(), String> {
    check_route(harness, "video-fullscreen/fixture?native=1", true)
}

fn check_route(harness: &mut WebKit, route: &str, native: bool) -> Result<(), String> {
    let document = harness
        .base
        .join(route)
        .map_err(|error| error.to_string())?;
    let tab = TabId::new(Profile::Main, 1);
    let mut engine = harness.engine();
    engine
        .create(tab, &document, Jar::Throwaway(JarId::new(1)))
        .and_then(|()| engine.show(WINDOW, tab))
        .map_err(|error| error.to_string())?;

    let mut events = Vec::new();
    let mut committed = false;
    if !harness.pump(&mut engine, &mut |engine| {
        engine.drain(&mut events);
        committed |= events.drain(..).any(|event| {
            matches!(event, EngineEvent::Committed { tab: owner, url, .. } if owner == tab && url == document)
        });
        committed && engine.loading(tab).is_none()
    }) {
        return Err("the video fullscreen fixture did not load".to_owned());
    }
    if native {
        install_native_request(harness, &mut engine, tab)?;
    }

    let original_window = engine
        .view(tab)
        .and_then(NSView::window)
        .ok_or("the page has no window")?;
    let ticket = engine
        .link_targets(tab)
        .map_err(|error| error.to_string())?;
    let mut button = None;
    harness.pump(&mut engine, &mut |engine| {
        engine.drain(&mut events);
        button = events.drain(..).find_map(|event| match event {
            EngineEvent::Targets {
                ticket: ready,
                targets,
                ..
            } if ready == ticket => targets.first().map(|target| target.frame),
            _ => None,
        });
        button.is_some()
    });
    harness.click(
        &mut engine,
        tab,
        button.ok_or("the fixture has no fullscreen button")?,
        false,
    )??;

    let mut states = Vec::new();
    let mut title = String::new();
    let mut changed_window = false;
    let done = harness.pump(&mut engine, &mut |engine| {
        changed_window |=
            engine.view(tab).and_then(NSView::window).as_deref() != Some(&*original_window);
        engine.drain(&mut events);
        for event in events.drain(..) {
            match event {
                EngineEvent::Fullscreen { state, .. } => states.push(state),
                EngineEvent::Titled {
                    title: new_title, ..
                } => title = new_title,
                _ => {}
            }
        }
        (title == "restored" && states.contains(&Fullscreen::Left))
            || matches!(
                title.as_str(),
                "not in page" | "video did not fill the page"
            )
            || title.starts_with("failed:")
            || changed_window
            || states.contains(&Fullscreen::OwnWindow)
    });
    if !done
        || changed_window
        || title != "restored"
        || states != [Fullscreen::InWindow, Fullscreen::Left]
    {
        return Err(format!(
            "video presentation did not enter and leave the same page: {title:?}, {states:?}, changed window: {changed_window}"
        ));
    }
    Ok(())
}

fn install_native_request(
    harness: &mut WebKit,
    engine: &mut quickie_engine_webkit::WebKitEngine,
    tab: TabId,
) -> Result<(), String> {
    let view: &WKWebView = engine
        .view(tab)
        .and_then(|view| view.downcast_ref())
        .ok_or("the page has no WebKit view")?;
    let window = engine
        .view(tab)
        .and_then(NSView::window)
        .ok_or("the page has no window")?;
    if view.respondsToSelector(sel!(_setWindowOcclusionDetectionEnabled:)) {
        // SAFETY: this is WebKit's BOOL setter for window occlusion detection.
        let () = unsafe { msg_send![view, _setWindowOcclusionDetectionEnabled: false] };
    }
    // SAFETY: NSWindow's alphaValue setter takes a CGFloat, an f64 on macOS.
    let () = unsafe { msg_send![&*window, setAlphaValue: 0.0_f64] };
    // Order the transparent test window without making it key.
    window.orderBack(None);
    let script = NSString::from_str(
        r#"
        (() => {
            const html = document.documentElement;
            const request = html.requestFullscreen || html.webkitRequestFullscreen;
            if (!request || !String(request).includes("[native code]")) {
                document.title = "failed: native request unavailable";
                return;
            }
            document.getElementById("expand").addEventListener("click", async () => {
                try { await request.call(html); }
                catch (error) { document.title = "failed: " + error.message; }
            }, { once: true });
            const ready = () => {
                if (document.visibilityState === "visible") document.title = "native ready";
            };
            document.addEventListener("visibilitychange", ready);
            ready();
        })()
    "#,
    );
    // SAFETY: the named content world is created on the main thread.
    let world = unsafe {
        WKContentWorld::worldWithName(&NSString::from_str("native-fullscreen-check"), harness.mtm)
    };
    // SAFETY: the view is live on the main thread. The separate world uses WebKit's own API.
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &script, None, &world, None,
        );
    }
    let mut events = Vec::new();
    let mut title = String::new();
    harness.pump(engine, &mut |engine| {
        engine.drain(&mut events);
        for event in events.drain(..) {
            if let EngineEvent::Titled { title: next, .. } = event {
                title = next;
            }
        }
        title == "native ready" || title.starts_with("failed:")
    });
    if title != "native ready" {
        return Err(format!(
            "could not arm the native fullscreen gesture: {title:?}"
        ));
    }
    Ok(())
}
