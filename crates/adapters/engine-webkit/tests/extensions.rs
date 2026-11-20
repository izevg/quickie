//! Web extensions on real `WKWebView`s, with the MV3 test extension in `tests/extension/`: its
//! manifest is read as the application's values (name, version, what it asks for, its action,
//! its command and key); once started with its grants, its content script runs on a page of
//! the main jar and its command reaches the page through its background worker and storage;
//! once stopped it runs on no new page. The page comes from a server on `127.0.0.1` in this
//! test; nothing leaves the machine.
//!
//! `WebKit` wants the main thread and its run loop, so this test has its own `main` instead of
//! the test harness, whose tests run on worker threads. On a macOS without `WKWebExtension`
//! (before 15.4) it says so and passes.

#![expect(
    unsafe_code,
    reason = "a window for the pages, which AppKit makes only unsafely"
)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
use quickie_application::{
    Access, EngineEvent, EnginePort, ExtensionEvent, ExtensionHost, ExtensionInfo, PageContent,
    WindowTabs,
};
use quickie_domain_browsing::{Jar, Profile, TabId, WindowId};
use quickie_domain_settings::{ExtensionId, ExtensionVersion};
use quickie_engine_webkit::{OpenedPage, WebKitEngine, WebKitExtensions};
use url::Url;

type Outcome = Result<(), String>;

/// How long a check waits for what it expects before it fails.
const PATIENCE: Duration = Duration::from_secs(20);

const WINDOW: WindowId = WindowId::new(Profile::Main, 1);

/// What the content script writes on the page.
const RAN: &str = "content script ran";

/// What it writes once the command reached the page, with how often it ran.
const MARKED: &str = "command ran (";

fn main() -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        return report(Err("not on the main thread".to_owned()));
    };
    report(run(mtm))
}

#[expect(
    clippy::print_stdout,
    reason = "the test runner's own report, like the harness's"
)]
fn report(outcome: Outcome) -> ExitCode {
    let name = "the_test_extension_reads_runs_and_takes_its_command";
    match outcome {
        Ok(()) => {
            println!("test {name} ... ok");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            println!("test {name} ... FAILED: {reason}");
            ExitCode::FAILURE
        }
    }
}

fn run(mtm: MainThreadMarker) -> Outcome {
    let port = serve()?;
    let mut engine = WebKitEngine::new(&[], mtm);
    let Some(mut extensions) = engine.web_extensions() else {
        #[expect(clippy::print_stdout, reason = "the test runner's own report")]
        {
            println!("WKWebExtension is not on this macOS: nothing to test");
        }
        return Ok(());
    };
    let _window = host(&mut engine, mtm);
    let id = ExtensionId::from_digest(&[7; 16]);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/extension");
    extensions
        .read(
            &id,
            fixture.to_str().ok_or("the fixture path is not UTF-8")?,
        )
        .map_err(|error| error.0)?;
    let info = wait_for_read(&mut extensions, &id)?;
    check_manifest(&info)?;
    extensions
        .start(
            &id,
            &Access {
                granted: info.requested.clone(),
                sites: Vec::new(),
            },
        )
        .map_err(|error| error.0)?;

    let page = Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(|e| e.to_string())?;
    let main = TabId::new(Profile::Main, 1);
    open(&mut engine, main, &page, Jar::Main)?;
    extensions.arrange(&[WindowTabs {
        window: WINDOW,
        tabs: vec![main],
        active: Some(main),
    }]);
    wait_for_text(&mut engine, main, RAN, "the content script never ran")?;

    // The background worker starts with the first event, and an event that comes before its
    // listener is lost: the command is given again until the page shows it.
    let give_up = Instant::now() + PATIENCE;
    loop {
        extensions
            .perform_command(&id, "mark-page")
            .map_err(|error| error.0)?;
        let quiet = Instant::now() + Duration::from_secs(2);
        while Instant::now() < quiet && !text(&mut engine, main)?.contains(MARKED) {
            spin();
        }
        if text(&mut engine, main)?.contains(MARKED) {
            break;
        }
        check(
            Instant::now() < give_up,
            "the command never reached the page",
        )?;
    }

    extensions.stop(&id);
    let later = TabId::new(Profile::Main, 3);
    open(&mut engine, later, &page, Jar::Main)?;
    absent(&mut engine, later, "it ran after it stopped")
}

fn check_manifest(info: &ExtensionInfo) -> Outcome {
    check(
        info.name == "Quickie Test Extension",
        &format!("named {}", info.name),
    )?;
    check(
        ExtensionVersion::try_from("1.0.0").is_ok_and(|version| version == info.version),
        &format!("at {}", info.version),
    )?;
    check(
        info.action.as_deref() == Some("Show Test Popup"),
        &format!("its action is {:?}", info.action),
    )?;
    check(
        info.requested.permissions.contains("storage")
            && info.requested.patterns.contains("http://127.0.0.1/*"),
        &format!("it asks for {:?}", info.requested),
    )?;
    let command = info
        .commands
        .iter()
        .find(|command| command.name == "mark-page")
        .ok_or("its command is missing")?;
    check(
        command.title == "Mark the Page" && command.shortcut.as_deref() == Some("shift+cmd+y"),
        &format!("its command is {command:?}"),
    )
}

/// Serves one small page on `127.0.0.1`, to every request, from a thread; returns the port.
fn serve() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    thread::spawn(move || {
        let body = "<!doctype html><title>Local</title><p>A local page</p>";
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    Ok(port)
}

/// A window, off screen, whose host the engine's pages fill.
fn host(engine: &mut WebKitEngine, mtm: MainThreadMarker) -> Retained<NSWindow> {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
    // SAFETY: a borderless, buffered window of a plain rect, made on the main thread; it is
    // never closed, so releasing it when closed cannot free it twice.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    let host = NSView::initWithFrame(NSView::alloc(mtm), frame);
    window.setContentView(Some(&host));
    engine.attach_host(WINDOW, &host);
    window
}

fn wait_for_read(
    extensions: &mut WebKitExtensions,
    id: &ExtensionId,
) -> Result<ExtensionInfo, String> {
    let give_up = Instant::now() + PATIENCE;
    let mut events = Vec::new();
    while Instant::now() < give_up {
        extensions.drain(&mut events);
        for event in events.drain(..) {
            if let ExtensionEvent::Read { id: read, info } = event
                && read == *id
            {
                return info;
            }
        }
        spin();
    }
    Err("the package was never read".to_owned())
}

/// Opens `tab` on `url` in `jar`, shows it, and waits until it has loaded.
fn open(engine: &mut WebKitEngine, tab: TabId, url: &Url, jar: Jar) -> Outcome {
    engine.create(tab, url, jar).map_err(|error| error.0)?;
    engine.show(WINDOW, tab).map_err(|error| error.0)?;
    let loaded = pump(
        engine,
        |event| matches!(event, EngineEvent::Loading { tab: about, progress: None } if *about == tab),
    );
    check(loaded.is_some(), &format!("{url} did not load in {tab}"))
}

/// Reads the tab's text until it holds `wanted`, or fails with `otherwise`.
fn wait_for_text(engine: &mut WebKitEngine, tab: TabId, wanted: &str, otherwise: &str) -> Outcome {
    let give_up = Instant::now() + PATIENCE;
    while Instant::now() < give_up {
        if text(engine, tab)?.contains(wanted) {
            return Ok(());
        }
        spin();
    }
    Err(otherwise.to_owned())
}

/// Fails with `otherwise` when the tab's page shows the content script's banner within a
/// second.
fn absent(engine: &mut WebKitEngine, tab: TabId, otherwise: &str) -> Outcome {
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        check(!text(engine, tab)?.contains(RAN), otherwise)?;
        spin();
    }
    Ok(())
}

fn text(engine: &mut WebKitEngine, tab: TabId) -> Result<String, String> {
    let ticket = engine.read_text(tab, 10_000).map_err(|error| error.0)?;
    let read = pump(
        engine,
        |event| matches!(event, EngineEvent::Text { ticket: about, .. } if *about == ticket),
    );
    match read {
        Some(EngineEvent::Text { text, .. }) => Ok(text),
        _ => Err(format!("{tab}'s text never came")),
    }
}

/// Runs the main run loop until a drained event matches, and returns it.
fn pump(
    engine: &mut WebKitEngine,
    wanted: impl Fn(&EngineEvent<OpenedPage>) -> bool,
) -> Option<EngineEvent<OpenedPage>> {
    let give_up = Instant::now() + PATIENCE;
    let mut events = Vec::new();
    loop {
        engine.drain(&mut events);
        if let Some(at) = events.iter().position(&wanted) {
            return Some(events.swap_remove(at));
        }
        events.clear();
        if Instant::now() > give_up {
            return None;
        }
        spin();
    }
}

fn spin() {
    NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02));
}

fn check(condition: bool, otherwise: &str) -> Outcome {
    if condition {
        Ok(())
    } else {
        Err(otherwise.to_owned())
    }
}
