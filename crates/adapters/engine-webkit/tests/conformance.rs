//! The engine conformance suite on real `WKWebView`s: the application's checks, driven through
//! [`WebKitEngine`], against the suite's pages served from memory on 127.0.0.1. No request leaves
//! the machine.
//!
//! `WebKit` wants the main thread and its run loop, so this test has its own `main` instead of
//! the test harness, whose tests run on worker threads.

#![expect(
    unsafe_code,
    reason = "the crash check asks a web view for its web content process, which only an \
              Objective-C message can"
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSSize};
use quickie_application::conformance::{
    self, Driven, Group, Harness, PageSource, Resource, SITE, WINDOW,
};
use quickie_application::{
    BlockerRules, ContentBlockerPort, Engine, PageRect, PageSurface, RulesStatus,
};
use quickie_domain_browsing::TabId;
use quickie_engine_webkit::{WebKitContentBlocker, WebKitEngine};
use tempfile::TempDir;
use url::Url;

mod agent_links;
mod automation;

/// How long a check waits for what it expects before it fails.
const PATIENCE: Duration = Duration::from_secs(20);

/// How long the engine works when a check waits for nothing to come.
const IDLE: Duration = Duration::from_millis(600);

/// The size of each engine's window, in points.
const WIDTH: f64 = 1024.0;
const HEIGHT: f64 = 768.0;

fn main() -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        return failed("WebKit needs the main thread");
    };
    let base = match serve() {
        Ok(base) => base,
        Err(why) => return failed(&why),
    };
    if std::env::args().any(|argument| argument == "--automation-only") {
        return match automation::check(mtm, &base) {
            Ok(what) => {
                print(&format!("automation: passed ({what})\n"));
                ExitCode::SUCCESS
            }
            Err(why) => failed(&why),
        };
    }
    let mut harness = WebKit {
        mtm,
        base,
        folders: Vec::new(),
        windows: Vec::new(),
        blockers: Vec::new(),
    };
    if std::env::args().any(|argument| argument == "--agent-links-only") {
        return match agent_links::check(&mut harness) {
            Ok(()) => {
                print("agent links: passed (ordered, read-only, isolated and current)\n");
                ExitCode::SUCCESS
            }
            Err(why) => failed(&why),
        };
    }
    if std::env::args().any(|argument| argument == "--inspector-only") {
        return match inspector::check(&mut harness) {
            Ok(what) => {
                print(&format!("inspector: passed ({what})\n"));
                ExitCode::SUCCESS
            }
            Err(why) => failed(&why),
        };
    }

    let automated = automation::check(mtm, &harness.base);
    match &automated {
        Ok(what) => print(&format!("automation: passed ({what})\n")),
        Err(why) => print(&format!("automation: FAILED: {why}\n")),
    }

    let report = conformance::run(&mut harness);
    print(&report.to_string());
    let links = agent_links::check(&mut harness);
    match &links {
        Ok(()) => print("agent links: passed (ordered, read-only, isolated and current)\n"),
        Err(why) => print(&format!("agent links: FAILED: {why}\n")),
    }
    let inspected = inspector::check(&mut harness);
    match &inspected {
        Ok(what) => print(&format!("inspector: passed ({what})\n")),
        Err(why) => print(&format!("inspector: FAILED: {why}\n")),
    }

    if report.passed() && inspected.is_ok() && automated.is_ok() && links.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Web Inspector, which the suite leaves to the harness: on our fork a developer tools'
/// command opens it over the page on the requested panel, it stays with its tab, and the toggle
/// closes it. The system `WebKit` reports that this command is unavailable.
mod inspector {
    mod stories;

    pub(super) use stories::answer;

    use std::cell::RefCell;
    use std::rc::Rc;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{msg_send, sel};
    use objc2_app_kit::{NSApplication, NSView};
    use objc2_foundation::{NSError, NSPoint, NSRect, NSString};
    use objc2_web_kit::WKWebView;
    use quickie_application::conformance::{Harness, PageSource, WINDOW};
    use quickie_application::{
        DevTools, DevToolsPanel, Engine, EngineEvent, EnginePort, PageSurface,
    };
    use quickie_domain_browsing::{Jar, Profile, TabId};
    use quickie_engine_webkit::WebKitEngine;

    use super::{HEIGHT, WIDTH, WebKit};

    /// What was checked, or why it failed.
    pub(super) fn check(harness: &mut WebKit) -> Result<&'static str, String> {
        let _app = NSApplication::sharedApplication(harness.mtm);
        let mut engine = harness.engine();
        let first = TabId::new(Profile::Main, 1);
        open(harness, &mut engine, first, "plain")?;
        let on_fork = AnyClass::get(c"_WKInspector").is_some_and(|class| {
            class.responds_to(sel!(showTab:)) && class.responds_to(sel!(attachToSide:))
        });
        let network = DevTools::Panel(DevToolsPanel::Network);
        if !on_fork {
            return match engine.inspect(first, network) {
                Err(_) => Ok("the system WebKit says where Web Inspector opens from"),
                Ok(()) => Err("the system WebKit claims to have opened Web Inspector".to_owned()),
            };
        }

        let page_frame = engine.view(first).ok_or("the page has no view")?.frame();
        engine
            .inspect(first, network)
            .map_err(|error| format!("Network: {error}"))?;
        let controller = inspector_for(&engine, first).ok_or("Web Inspector has no controller")?;
        if !harness.pump(&mut engine, &mut |_| frontend(&controller).is_some()) {
            return Err("Web Inspector did not create its frontend".into());
        }
        // SAFETY: `_WKInspectorAttachmentSideBottom` is the first NSInteger enum value.
        // Request it explicitly so a previously saved detach preference cannot move this test.
        let () = unsafe { msg_send![&*controller, attachToSide: 0_isize] };
        let mut dock_result = Ok(());
        let docked = harness.pump(&mut engine, &mut |engine| {
            dock_result = floats_over_page(engine, first, page_frame);
            dock_result.is_ok()
        });
        if !docked {
            dock_result?;
        }
        let shown = Rc::new(RefCell::new(String::new()));
        let selected = harness.pump(&mut engine, &mut |engine| {
            selected_tab(engine, first, &shown);
            *shown.borrow() == "network"
        });
        if !selected {
            return Err(format!(
                "Web Inspector shows the tab {:?}, not network",
                shown.borrow()
            ));
        }

        let second = TabId::new(Profile::Main, 2);
        open(harness, &mut engine, second, "other")?;
        let host = engine
            .view(second)
            .and_then(NSView::window)
            .and_then(|window| window.contentView())
            .ok_or("the second tab's page is not in the window")?;
        if host.subviews().len() != 1 || page_height(&engine, second) < HEIGHT {
            return Err(
                "the first tab's Web Inspector stayed when the window showed the \
                        second tab"
                    .to_owned(),
            );
        }
        engine
            .show(WINDOW, first)
            .map_err(|error| error.to_string())?;
        if std::env::var("QUICKIE_INSPECTOR_WINDOW_STORIES").as_deref() == Ok("1") {
            stories::check(&mut engine, first)?;
        }

        engine
            .inspect(first, DevTools::Toggle)
            .map_err(|error| format!("Toggle: {error}"))?;
        let closed = harness.pump(&mut engine, &mut |engine| {
            page_height(engine, first) >= HEIGHT && siblings(engine, first) == 1
        });
        if !closed {
            return Err("the toggle left Web Inspector open".to_owned());
        }
        Ok(
            "floating dock and tab ownership, detached Console and source links, responsive Network and Requests return, and close",
        )
    }

    /// Makes a page of the suite's `path` in `tab`, shows it, and waits until it committed.
    fn open(
        harness: &mut WebKit,
        engine: &mut WebKitEngine,
        tab: TabId,
        path: &str,
    ) -> Result<(), String> {
        let url = harness
            .base()
            .join(path)
            .map_err(|error| error.to_string())?;
        engine
            .create(tab, &url, Jar::Main)
            .and_then(|()| engine.show(WINDOW, tab))
            .map_err(|error| error.to_string())?;
        let mut events = Vec::new();
        let committed = harness.pump(engine, &mut |engine| {
            engine.drain(&mut events);
            events.iter().any(
                |event| matches!(event, EngineEvent::Committed { tab: about, .. } if *about == tab),
            )
        });
        if committed {
            Ok(())
        } else {
            Err(format!("{url} never committed"))
        }
    }

    /// The height of the tab's page view, `0` without one.
    fn page_height(engine: &WebKitEngine, tab: TabId) -> f64 {
        engine
            .view(tab)
            .map_or(0.0, |view| view.frame().size.height)
    }

    /// How many views share the page view's superview, itself included.
    fn siblings(engine: &WebKitEngine, tab: TabId) -> usize {
        // SAFETY: a getter; the superview is retained while it is counted.
        engine
            .view(tab)
            .and_then(|view| unsafe { view.superview() })
            .map_or(0, |parent: Retained<NSView>| parent.subviews().len())
    }

    fn floats_over_page(engine: &WebKitEngine, tab: TabId, original: NSRect) -> Result<(), String> {
        let page = engine.view(tab).ok_or("the docked page has no view")?;
        // SAFETY: the getter retains the view's live parent during this check.
        let Some(parent) = (unsafe { page.superview() }) else {
            return Err("the docked page has no superview".to_owned());
        };
        // SAFETY: the getter retains the ancestor used for point conversion below.
        let enclosing = unsafe { parent.superview() };
        let views = parent.subviews();
        let Some(dock) = views.lastObject() else {
            return Err("the docked page's superview has no children".to_owned());
        };
        if page.frame() != original
            || views.len() != 2
            || std::ptr::eq(page, Retained::as_ptr(&dock))
        {
            return Err(format!(
                "Web Inspector did not float over the unchanged page: frame {:?} vs {original:?}, {} views",
                page.frame(),
                views.len(),
            ));
        }

        let frame = dock.frame();
        let middle_y = frame.origin.y + frame.size.height / 2.0;
        let gutter = NSPoint::new(frame.origin.x + 6.0, middle_y);
        let center = NSPoint::new(frame.origin.x + frame.size.width / 2.0, middle_y);
        let [hit_page, hit_dock] = [(gutter, page), (center, &dock)].map(|(point, expected)| {
            let translated = parent.convertPoint_toView(point, enclosing.as_deref());
            parent.hitTest(translated).is_some_and(|hit| {
                std::ptr::eq(expected, Retained::as_ptr(&hit)) || hit.isDescendantOf(expected)
            })
        });
        if hit_page && hit_dock {
            Ok(())
        } else {
            Err(format!(
                "wrong hit targets for Inspector dock {frame:?}: page received gutter={hit_page}, Inspector received center={hit_dock}",
            ))
        }
    }

    /// Asks the tab's Web Inspector which tab it shows; the answer lands in `shown`.
    fn selected_tab(engine: &WebKitEngine, tab: TabId, shown: &Rc<RefCell<String>>) {
        let Some(inspector) = inspector_for(engine, tab) else {
            return;
        };
        let Some(frontend) = frontend(&inspector) else {
            return;
        };
        let shown = Rc::clone(shown);
        let reply = RcBlock::new(move |value: *mut AnyObject, _error: *mut NSError| {
            // SAFETY: `WebKit` hands over the script's value, an `NSString` here, or nil.
            if let Some(value) = unsafe { value.as_ref() }
                && let Some(text) = value.downcast_ref::<NSString>()
            {
                *shown.borrow_mut() = text.to_string();
            }
        });
        let script = NSString::from_str("WI.tabBrowser.selectedTabContentView?.type ?? ''");
        // SAFETY: a script for the inspector's own page; the block is copied by `WebKit`.
        unsafe { frontend.evaluateJavaScript_completionHandler(&script, Some(&reply)) };
    }

    fn inspector_for(engine: &WebKitEngine, tab: TabId) -> Option<Retained<AnyObject>> {
        let view = engine.view(tab)?;
        // SAFETY: on the fork this getter returns the tab's live inspector or nil.
        unsafe { msg_send![view, _inspector] }
    }

    fn frontend(inspector: &AnyObject) -> Option<Retained<WKWebView>> {
        // SAFETY: `_WKInspector (WKTesting)` answers this getter with its frontend view or nil.
        unsafe { msg_send![inspector, inspectorWebView] }
    }

    const _: () = assert!(
        WIDTH > 500.0 && HEIGHT > 333.0,
        "WebKit docks only on large pages"
    );
}

#[expect(
    clippy::print_stdout,
    reason = "the test runner's own report, like the harness's"
)]
fn print(report: &str) {
    print!("{report}");
}

fn failed(why: &str) -> ExitCode {
    print(&format!("conformance could not start: {why}\n"));
    ExitCode::FAILURE
}

/// The `WebKit` engine as the suite drives it.
struct WebKit {
    mtm: MainThreadMarker,
    base: Url,
    /// Downloads folders and compiled rule lists, deleted when the run ends.
    folders: Vec<TempDir>,
    /// Each engine's window, off screen, its content view the host its pages fill; kept as
    /// long as the engine may show pages in it.
    windows: Vec<Retained<NSWindow>>,
    /// The content blockers installed, kept so their rules stay.
    blockers: Vec<WebKitContentBlocker>,
}

impl WebKit {
    fn folder(&mut self) -> Result<PathBuf, String> {
        let folder = tempfile::tempdir().map_err(|error| error.to_string())?;
        let path = folder.path().to_owned();
        self.folders.push(folder);
        Ok(path)
    }
}

impl PageSource for WebKit {
    fn base(&self) -> Url {
        self.base.clone()
    }
}

impl Harness for WebKit {
    type Engine = WebKitEngine;

    fn unsupported(&self, group: Group) -> Option<&'static str> {
        match group {
            Group::Permissions => Some(
                "pages in this test binary have no navigator.mediaDevices, so they cannot ask: \
                 WebKit offers capture only to an app whose Info.plist describes camera use \
                 (NSCameraUsageDescription), which the bundled app has and a test binary does not",
            ),
            Group::Core
            | Group::Surface
            | Group::Events
            | Group::Opened
            | Group::Jars
            | Group::Dialogs
            | Group::Fullscreen
            | Group::Find
            | Group::Hints
            | Group::Text
            | Group::Downloads
            | Group::Blocker
            | Group::Crash
            | Group::Zoom
            | Group::Video => None,
        }
    }

    fn engine(&mut self) -> WebKitEngine {
        let mut engine = WebKitEngine::new(&[], self.mtm);
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT));
        // SAFETY: a borderless, buffered window of a plain rect, made on the main thread; it is
        // never closed, so releasing it when closed cannot free it twice.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(self.mtm),
                frame,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        let host = NSView::initWithFrame(NSView::alloc(self.mtm), frame);
        window.setContentView(Some(&host));
        if let Err(error) = engine.attach_host(WINDOW, &host) {
            print(&format!("the window's host was refused: {error}\n"));
        }
        self.windows.push(window);
        engine
    }

    fn pump(
        &mut self,
        engine: &mut WebKitEngine,
        done: &mut dyn FnMut(&mut WebKitEngine) -> bool,
    ) -> bool {
        let give_up = Instant::now() + PATIENCE;
        loop {
            if done(engine) {
                return true;
            }
            if Instant::now() > give_up {
                return false;
            }
            spin();
        }
    }

    fn idle(&mut self, _engine: &mut WebKitEngine) {
        let until = Instant::now() + IDLE;
        while Instant::now() < until {
            spin();
        }
    }

    fn click(
        &mut self,
        engine: &mut WebKitEngine,
        tab: TabId,
        at: PageRect,
        command: bool,
    ) -> Driven<Result<(), String>> {
        let Some(view) = engine.view(tab) else {
            return Ok(Err(format!("{tab} has no page")));
        };
        let Some(window) = view.window() else {
            return Ok(Err(format!("{tab}'s page is not in a window")));
        };
        // The page view fills the window's content, whose origin is at its bottom left.
        let point = NSPoint::new(at.x + at.width / 2.0, HEIGHT - (at.y + at.height / 2.0));
        let flags = if command {
            NSEventModifierFlags::Command
        } else {
            NSEventModifierFlags::empty()
        };
        for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                point,
                flags,
                0.0,
                window.windowNumber(),
                None,
                0,
                1,
                1.0,
            );
            let Some(event) = event else {
                return Ok(Err("AppKit made no mouse event".to_owned()));
            };
            if kind == NSEventType::LeftMouseDown {
                view.mouseDown(&event);
            } else {
                view.mouseUp(&event);
            }
        }
        Ok(Ok(()))
    }

    fn new_folder(&mut self) -> Driven<Result<PathBuf, String>> {
        Ok(self.folder())
    }

    fn downloaded(&self, _engine: &WebKitEngine, file: &Path) -> Driven<Option<Vec<u8>>> {
        Ok(std::fs::read(file).ok())
    }

    fn block(
        &mut self,
        engine: &mut WebKitEngine,
        rules: &BlockerRules,
    ) -> Driven<Result<(), String>> {
        Ok(self.folder().and_then(|folder| {
            let mut blocker = engine
                .content_blocker(&folder)
                .map_err(|error| error.to_string())?;
            blocker.install(rules).map_err(|error| error.to_string())?;
            let give_up = Instant::now() + PATIENCE;
            while blocker.status() == RulesStatus::Compiling && Instant::now() < give_up {
                spin();
            }
            let status = blocker.status();
            self.blockers.push(blocker);
            match status {
                RulesStatus::Active(_) => Ok(()),
                other => Err(format!("the blocker ended {other:?}")),
            }
        }))
    }

    fn crash(&mut self, engine: &mut WebKitEngine, tab: TabId) -> Driven<Result<(), String>> {
        let Some(view) = engine.view(tab) else {
            return Ok(Err(format!("{tab} has no page")));
        };
        if !view.respondsToSelector(sel!(_webProcessIdentifier)) {
            return Ok(Err(
                "this WebKit does not name its web content process".to_owned()
            ));
        }
        // SAFETY: the view is a `WKWebView`, which answers `_webProcessIdentifier` (checked
        // above) with its web content process's `pid_t`, and the message has no arguments.
        let pid: i32 = unsafe { msg_send![view, _webProcessIdentifier] };
        Ok(Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .map_err(|error| error.to_string())
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("kill -9 {pid} failed: {status}"))
                }
            }))
    }
}

/// Runs the main run loop, which also drains the main dispatch queue, for a moment.
fn spin() {
    NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
}

/// A server on 127.0.0.1 for the suite's resources.
fn serve() -> Result<Url, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || answer(stream));
        }
    });
    Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(|error| error.to_string())
}

fn answer(stream: TcpStream) {
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    let mut header = String::new();
    let mut automation_cookie = false;
    while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("cookie")
            && value.trim() == automation::COOKIE
        {
            automation_cookie = true;
        }
        header.clear();
    }
    let path = request.split_whitespace().nth(1).unwrap_or_default();
    let path = path
        .split('?')
        .next()
        .unwrap_or_default()
        .trim_start_matches('/');
    let mut stream = reader.into_inner();
    if let Some(route) = path.strip_prefix("automation/") {
        automation::answer(&mut stream, route, automation_cookie);
        return;
    }
    if path == "inspector/story.js" {
        inspector::answer(&mut stream);
        return;
    }
    if path == agent_links::ROUTE {
        agent_links::answer(&mut stream);
        return;
    }
    let found = SITE.iter().find(|(served, _)| *served == path);
    let Some((path, resource)) = found else {
        let _ = write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return;
    };
    let body = resource.body();
    let (length, disposition) = match resource {
        Resource::Attachment(_) => (body.len(), format!("attachment; filename=\"{path}\"")),
        // Promises far more than it sends, so the download waits for the rest.
        Resource::Stalled(_) => (
            body.len() + 1_000_000,
            format!("attachment; filename=\"{path}\""),
        ),
        Resource::Page(_) | Resource::Script(_) => (body.len(), "inline".to_owned()),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {length}\r\n\
         Content-Disposition: {disposition}\r\nCache-Control: no-store\r\n\
         Connection: close\r\n\r\n{body}",
        resource.content_type()
    );
    let _ = stream.flush();
    if matches!(resource, Resource::Stalled(_)) {
        // Holds the connection until the download is cancelled and the client lets go.
        let _ = stream.read(&mut [0; 1]);
    }
}
