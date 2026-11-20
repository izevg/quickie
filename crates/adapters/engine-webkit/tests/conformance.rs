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
    self, Driven, Event, Group, Harness, PageSource, Resource, SITE, WINDOW,
};
use quickie_application::{BlockerRules, ContentBlockerPort, PageRect};
use quickie_domain_browsing::TabId;
use quickie_engine_webkit::{BlockerStatus, WebKitContentBlocker, WebKitEngine};
use tempfile::TempDir;
use url::Url;

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
    let mut harness = WebKit {
        mtm,
        base,
        folders: Vec::new(),
        windows: Vec::new(),
        blockers: Vec::new(),
    };

    let report = conformance::run(&mut harness);

    print(&report.to_string());
    if report.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
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
            | Group::Events
            | Group::Opened
            | Group::Jars
            | Group::Dialogs
            | Group::Find
            | Group::Hints
            | Group::Text
            | Group::Downloads
            | Group::Blocker
            | Group::Crash
            | Group::Zoom => None,
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
        engine.attach_host(WINDOW, &host);
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

    fn set_waker(engine: &mut WebKitEngine, wake: Box<dyn Fn()>) {
        engine.set_waker(wake);
    }

    fn drain(engine: &mut WebKitEngine, into: &mut Vec<Event<Self>>) {
        engine.drain(into);
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
            while blocker.status() == BlockerStatus::Compiling && Instant::now() < give_up {
                spin();
            }
            let status = blocker.status();
            self.blockers.push(blocker);
            match status {
                BlockerStatus::Installed(_) => Ok(()),
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
    while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
        header.clear();
    }
    let path = request.split_whitespace().nth(1).unwrap_or_default();
    let path = path
        .split('?')
        .next()
        .unwrap_or_default()
        .trim_start_matches('/');
    let found = SITE.iter().find(|(served, _)| *served == path);
    let mut stream = reader.into_inner();
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
