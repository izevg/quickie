//! The tab overview's pictures on real `WKWebView`s: a page taken off screen by a tab switch
//! is pictured and [`EngineEvent::Pictured`] says so, the page on screen is pictured when
//! asked, each picture is the overview card's shape and size, and a closed tab keeps none, not
//! even one still being drawn when it closed. The pages are served from memory.
//!
//! `WebKit` wants the main thread and its run loop, so this test has its own `main` instead of
//! the test harness, whose tests run on worker threads.

#![expect(
    unsafe_code,
    reason = "a window for the pages, which AppKit makes only unsafely"
)]

use std::collections::HashSet;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBitmapImageRep, NSImage, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
use quickie_application::{Engine, EngineEvent, EnginePort, PageSurface};
use quickie_domain_browsing::{Jar, Profile, TabId, WindowId};
use quickie_engine_webkit::{OpenedPage, THUMBNAIL_SIZE, WebKitEngine};
use url::Url;

type Outcome = Result<(), String>;

/// One test: runs on the main thread, says why it failed.
type Case = fn(MainThreadMarker) -> Outcome;

/// How long a check waits for what it expects before it fails.
const PATIENCE: Duration = Duration::from_secs(20);

/// How long the engine works when a check waits for nothing to come.
const IDLE: Duration = Duration::from_millis(600);

const WINDOW: WindowId = WindowId::new(Profile::Main, 1);

fn main() -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        return report("not on the main thread", Err("WebKit needs it".to_owned()));
    };
    let cases: [(&str, Case); 2] = [
        (
            "a_page_hidden_by_a_tab_switch_and_the_page_on_screen_are_pictured",
            hidden_and_shown_pages_are_pictured,
        ),
        ("a_closed_tab_keeps_no_picture", closed_tab_keeps_none),
    ];
    let mut code = ExitCode::SUCCESS;
    for (name, case) in cases {
        if report(name, case(mtm)).ne(&ExitCode::SUCCESS) {
            code = ExitCode::FAILURE;
        }
    }
    code
}

#[expect(
    clippy::print_stdout,
    reason = "the test runner's own report, like the harness's"
)]
fn report(name: &str, outcome: Outcome) -> ExitCode {
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

fn hidden_and_shown_pages_are_pictured(mtm: MainThreadMarker) -> Outcome {
    let (mut engine, _window) = engine(mtm)?;
    let (red, green) = (tab(1), tab(2));
    open(&mut engine, red, "red")?;
    open(&mut engine, green, "green")?;
    engine.show(WINDOW, red).map_err(|error| error.0)?;

    engine.show(WINDOW, green).map_err(|error| error.0)?;
    wait_for_picture(&mut engine, red)?;
    engine.capture(green);
    wait_for_picture(&mut engine, green)?;

    for (tab, name) in [(red, "red"), (green, "green")] {
        let picture = engine
            .picture(tab)
            .ok_or_else(|| format!("the {name} page has no picture"))?;
        check(
            picture.size() == THUMBNAIL_SIZE,
            &format!("the {name} picture is {:?}", picture.size()),
        )?;
        let colors = colors(&picture)?;
        check(
            colors.len() > 1,
            &format!("the {name} picture is one flat color"),
        )?;
    }
    Ok(())
}

fn closed_tab_keeps_none(mtm: MainThreadMarker) -> Outcome {
    let (mut engine, _window) = engine(mtm)?;
    let (shown, drawing) = (tab(1), tab(2));
    open(&mut engine, shown, "red")?;
    open(&mut engine, drawing, "green")?;
    engine.show(WINDOW, shown).map_err(|error| error.0)?;
    engine.capture(shown);
    wait_for_picture(&mut engine, shown)?;

    engine.close(shown).map_err(|error| error.0)?;
    engine.capture(drawing);
    engine.close(drawing).map_err(|error| error.0)?;
    let late = pictured_within(&mut engine, drawing, IDLE);

    check(
        engine.picture(shown).is_none(),
        "a closed tab kept its picture",
    )?;
    check(
        !late && engine.picture(drawing).is_none(),
        "a picture drawn while its tab closed came back",
    )
}

fn tab(serial: u64) -> TabId {
    TabId::new(Profile::Main, serial)
}

fn page(name: &str) -> Result<Url, String> {
    Url::parse(&format!("quickie://{name}/")).map_err(|error| error.to_string())
}

/// A page of two bands: `name` above, white with text below.
fn html(name: &str) -> String {
    format!(
        "<!doctype html><title>{name}</title><body style=\"margin:0;font:40px sans-serif\">\
         <div style=\"background:{name};height:50vh\"></div><p>The {name} page</p></body>"
    )
}

/// An engine whose window has a host, off screen, serving the red and green pages.
fn engine(mtm: MainThreadMarker) -> Result<(WebKitEngine, Retained<NSWindow>), String> {
    let pages = ["red", "green"]
        .into_iter()
        .map(|name| Ok((page(name)?, html(name))))
        .collect::<Result<Vec<_>, String>>()?;
    let mut engine = WebKitEngine::new(&pages, mtm);
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1024.0, 768.0));
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
    engine.attach_host(WINDOW, &host).map_err(|error| error.0)?;
    Ok((engine, window))
}

/// Opens the tab on page `name` and waits until it has loaded.
fn open(engine: &mut WebKitEngine, tab: TabId, name: &str) -> Outcome {
    engine
        .create(tab, &page(name)?, Jar::Main)
        .map_err(|error| error.0)?;
    let loaded = pump(engine, PATIENCE, |event| is_loaded(event, tab));
    check(loaded, &format!("the {name} page did not load"))
}

fn wait_for_picture(engine: &mut WebKitEngine, tab: TabId) -> Outcome {
    check(
        pictured_within(engine, tab, PATIENCE),
        &format!("{tab} was never pictured"),
    )
}

fn pictured_within(engine: &mut WebKitEngine, tab: TabId, within: Duration) -> bool {
    pump(engine, within, |event| is_pictured(event, tab))
}

fn is_loaded(event: &EngineEvent<OpenedPage>, tab: TabId) -> bool {
    matches!(event, EngineEvent::Loading { tab: about, progress: None } if *about == tab)
}

fn is_pictured(event: &EngineEvent<OpenedPage>, tab: TabId) -> bool {
    matches!(event, EngineEvent::Pictured { tab: about } if *about == tab)
}

/// Runs the main run loop until a drained event matches, or `within` passes.
fn pump(
    engine: &mut WebKitEngine,
    within: Duration,
    wanted: impl Fn(&EngineEvent<OpenedPage>) -> bool,
) -> bool {
    let give_up = Instant::now() + within;
    let mut events = Vec::new();
    loop {
        engine.drain(&mut events);
        if events.iter().any(&wanted) {
            return true;
        }
        events.clear();
        if Instant::now() > give_up {
            return false;
        }
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
}

/// The distinct colors on an 8 × 8 grid over the picture.
fn colors(picture: &NSImage) -> Result<HashSet<String>, String> {
    let data = picture
        .TIFFRepresentation()
        .ok_or("the picture has no bitmap")?;
    let bitmap = NSBitmapImageRep::imageRepWithData(&data).ok_or("the picture is no bitmap")?;
    let (wide, high) = (bitmap.pixelsWide(), bitmap.pixelsHigh());
    let mut colors = HashSet::new();
    for y in (0..high).step_by(usize::try_from(high / 8).unwrap_or(1).max(1)) {
        for x in (0..wide).step_by(usize::try_from(wide / 8).unwrap_or(1).max(1)) {
            if let Some(color) = bitmap.colorAtX_y(x, y) {
                colors.insert(format!("{color:?}"));
            }
        }
    }
    Ok(colors)
}

fn check(condition: bool, otherwise: &str) -> Outcome {
    if condition {
        Ok(())
    } else {
        Err(otherwise.to_owned())
    }
}
