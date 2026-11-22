//! Exercises the live Inspector against a script loaded from the loopback fixture.

use std::cell::RefCell;
use std::io::Write;
use std::net::TcpStream;
use std::rc::Rc;
use std::time::Instant;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObjectProtocol};
use objc2::{msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSTitlebarSeparatorStyle, NSWindowButton,
    NSWindowTitleVisibility,
};
use objc2_foundation::{NSError, NSSize, NSString};
use objc2_web_kit::WKWebView;
use quickie_application::{DevTools, DevToolsPanel, PageSurface};
use quickie_domain_browsing::TabId;
use quickie_engine_webkit::WebKitEngine;

use super::super::{PATIENCE, print, spin};
use super::{frontend, inspector_for};

const STORIES: &str = include_str!("stories.js");
const FIXTURE: &str = include_str!("story.js");

pub(in super::super) fn answer(stream: &mut TcpStream) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{FIXTURE}",
        FIXTURE.len(),
    );
}

pub(super) fn check(
    app: &NSApplication,
    engine: &mut WebKitEngine,
    tab: TabId,
) -> Result<(), String> {
    let previous = app.activationPolicy();
    if !app.setActivationPolicy(NSApplicationActivationPolicy::Accessory) {
        return Err("AppKit refused to make the Inspector story process activatable".to_owned());
    }
    app.activate();
    let story = check_window(engine, tab);
    if !app.setActivationPolicy(previous) && story.is_ok() {
        return Err("AppKit did not restore the test process activation policy".to_owned());
    }
    story
}

fn check_window(engine: &mut WebKitEngine, tab: TabId) -> Result<(), String> {
    let inspector = inspector_for(engine, tab).ok_or("Inspector has no native controller")?;
    let frontend = frontend(&inspector).ok_or("Inspector has no frontend view")?;
    let tab_window = engine.view(tab).and_then(objc2_app_kit::NSView::window);
    let tab_window = tab_window.ok_or("the inspected page has no window")?;

    if !frontend.respondsToSelector(sel!(_setWindowOcclusionDetectionEnabled:)) {
        return Err("WebKit does not expose the test view occlusion setting".to_owned());
    }
    // SAFETY: the checked WKWebView selector takes a BOOL. Set it before the view moves to
    // the new window so WebKit recomputes visibility without runner desktop occlusion.
    let () = unsafe { msg_send![&*frontend, _setWindowOcclusionDetectionEnabled: Bool::NO] };

    engine
        .inspect(tab, DevTools::Panel(DevToolsPanel::Console))
        .map_err(|error| error.to_string())?;
    // SAFETY: the fork Inspector is live and its detach method runs on the main thread.
    let () = unsafe { msg_send![&*inspector, detach] };
    let deadline = Instant::now() + PATIENCE;
    let window = loop {
        if let Some(window) = frontend.window()
            && Retained::as_ptr(&window) != Retained::as_ptr(&tab_window)
        {
            break window;
        }
        if Instant::now() >= deadline {
            return Err("Inspector did not open a separate window".to_owned());
        }
        spin();
    };

    // SAFETY: NSWindow accepts an NSWindowButton and returns its live button or nil.
    let close: Option<Retained<AnyObject>> =
        unsafe { msg_send![&*window, standardWindowButton: NSWindowButton::CloseButton] };
    if window.titleVisibility() != NSWindowTitleVisibility::Hidden
        || !window.titlebarAppearsTransparent()
        || window.titlebarSeparatorStyle() != NSTitlebarSeparatorStyle::None
        || close.is_none()
    {
        return Err(
            "the detached Inspector did not compose its titlebar with window controls".to_owned(),
        );
    }
    window.setContentSize(NSSize::new(1_000.0, 660.0));
    if !window.isVisible() {
        window.orderFrontRegardless();
    }
    window.makeKeyWindow();
    if !window.makeFirstResponder(Some(&frontend)) {
        return Err("the detached Inspector refused keyboard focus".to_owned());
    }
    wait(&frontend, "consoleSelected")?;
    action(&frontend, "observeFrontend")?;
    wait(&frontend, "frontendReady").map_err(|error| {
        format!(
            "{error}; native visible={}, key={}, occlusion={:?}",
            window.isVisible(),
            window.isKeyWindow(),
            window.occlusionState(),
        )
    })?;

    // The browser loads and runs this script normally. Its Console entries and request must
    // reach the actual inspected-target agents through Inspector's protocol.
    let page = engine
        .view(tab)
        .and_then(|view| view.downcast_ref::<WKWebView>())
        .ok_or("the inspected page has no WKWebView")?;
    evaluate(
        page,
        "{ const script = document.createElement('script'); script.src = '/inspector/story.js'; document.head.append(script); 'true'; }",
    )?;
    wait(&frontend, "consoleEntries")?;
    print("inspector story: detached Console rendered the live warning and error\n");
    wait(&frontend, "consoleLayout")?;
    wait(&frontend, "consoleSourceVisible")?;
    action(&frontend, "openSource")?;
    wait(&frontend, "sourceLanded")?;

    wait(&frontend, "networkTabAvailable")?;
    action(&frontend, "selectNetwork")?;
    wait(&frontend, "networkRequestVisible")?;
    action(&frontend, "openRequest")?;
    wait(&frontend, "wideNetwork")?;

    window.setContentSize(NSSize::new(580.0, 660.0));
    wait(&frontend, "stackedNetwork")?;
    window.setContentSize(NSSize::new(580.0, 400.0));
    wait(&frontend, "drawerNetwork")?;
    action(&frontend, "backToRequests")?;
    wait(&frontend, "requestsReturned")?;
    Ok(())
}

fn action(view: &WKWebView, name: &str) -> Result<(), String> {
    let result = run(view, name)?;
    if result == "true" {
        Ok(())
    } else {
        Err(format!("Inspector could not perform {name}: {result}"))
    }
}

fn wait(view: &WKWebView, name: &str) -> Result<(), String> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let result = run(view, name)?;
        if result == "true" {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("Inspector did not show {name}: {result}"));
        }
        spin();
    }
}

fn run(view: &WKWebView, name: &str) -> Result<String, String> {
    evaluate(view, &format!("String(({STORIES})[{name:?}]())"))
}

fn evaluate(view: &WKWebView, script: &str) -> Result<String, String> {
    let result = Rc::new(RefCell::new(None));
    let received = Rc::clone(&result);
    let reply = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        // SAFETY: WebKit supplies a live Objective-C result or nil for this callback.
        let value = unsafe { value.as_ref() };
        // SAFETY: an NSError, when supplied by WebKit, remains live through this callback.
        let error = unsafe { error.as_ref() };
        *received.borrow_mut() = Some(match error {
            Some(error) => Err(error.localizedDescription().to_string()),
            None => value
                .and_then(AnyObject::downcast_ref::<NSString>)
                .map_or_else(
                    || Err("Inspector script returned no text".to_owned()),
                    |value| Ok(value.to_string()),
                ),
        });
    });
    // SAFETY: a test-only script in the live main-thread view; WebKit copies the block.
    unsafe { view.evaluateJavaScript_completionHandler(&NSString::from_str(script), Some(&reply)) };
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if let Some(result) = result.borrow_mut().take() {
            return result;
        }
        spin();
    }
    Err("Inspector script did not reply".to_owned())
}
