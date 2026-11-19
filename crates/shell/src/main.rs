//! Quickie, the composition root: wires the `WebKit` engine, the `SQLite` store, and the clock
//! to the browsing use cases, puts them on screen, and runs the app.
//!
//! For now this is the development app on the `engine-webkit` adapter (`docs/architecture.md`
//! §3, P0). Run it from the repository root with:
//!
//! ```sh
//! mise exec -- cargo run -p quickie
//! ```
//!
//! The session lives in `~/Library/Application Support/Quickie/shell.sqlite` and is restored
//! at launch. `⌘T` opens a tab, `⌘W` closes it into the trash, `⌘⇧T` reopens it, `⌃⇥` /
//! `⌃⇧⇥` switch in most-recently-used order, `⌘1` … `⌘9` go to a pinned slot and `⌥⌘1` …
//! `⌥⌘9` pin the tab there. `⌘L` opens the address overlay (`⏎` goes, `⎋` closes), `⌘[` / `⌘]`
//! go back and forward, `⌘R` reloads, `⌘Q` quits. Launching opens no network connection; the
//! start page is served from memory.

#![expect(
    unsafe_code,
    reason = "AppKit through objc2: windows, menus, timers, the key monitor, the quit notification"
)]

mod browser;
mod input;
mod overlay;
mod window;

use std::path::PathBuf;
use std::process::ExitCode;
use std::ptr::NonNull;
use std::time::{SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationWillTerminateNotification, NSMenu,
    NSMenuItem,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSString};
use quickie_application::{Browsing, Clock};
use quickie_domain_browsing::Timestamp;
use quickie_engine_webkit::WebKitEngine;
use quickie_store::{PendingWrites, WriteBehindStore};
use url::Url;

use browser::{Browser, install_shortcuts};

/// The browsing use cases as the app composes them.
pub type App = Browsing<WebKitEngine, WriteBehindStore, SystemClock>;

/// Where new tabs open: the start page, served by the engine from memory.
const HOME: &str = "quickie://start/";

/// The start page: no external resources, so launching opens no connection.
const START_PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Quickie</title>
<meta name="color-scheme" content="light dark">
<style>
  html, body { height: 100%; margin: 0; }
  body { display: grid; place-items: center; font: 15px -apple-system, system-ui, sans-serif;
         color: CanvasText; background: Canvas; }
  main { text-align: center; }
  h1 { font-size: 34px; font-weight: 600; letter-spacing: -0.5px; margin: 0 0 12px; }
  p { margin: 0; opacity: 0.6; }
  kbd { font: 13px ui-monospace, monospace; padding: 2px 6px; border-radius: 6px;
        border: 1px solid color-mix(in srgb, CanvasText 20%, transparent); }
</style></head>
<body><main><h1>Quickie</h1><p>Press <kbd>⌘L</kbd> to go somewhere.</p></main></body></html>
"#;

/// The wall clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp::from_millis(u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX))
    }
}

fn main() -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        return ExitCode::FAILURE;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.setMainMenu(Some(&main_menu(mtm)));

    let (browser, pending) = match start(mtm) {
        Ok(started) => started,
        Err(reason) => return fail(&reason),
    };
    let _flush_on_quit = flush_on_quit(pending);
    let _monitor = install_shortcuts(&browser);
    browser.show();
    app.activate();
    app.run();
    ExitCode::SUCCESS
}

/// Opens the store, restores the session, and puts it on screen. The store writes on its own
/// thread; the shell keeps a handle on its pending writes.
fn start(mtm: MainThreadMarker) -> Result<(std::rc::Rc<Browser>, PendingWrites), String> {
    let home = Url::parse(HOME).map_err(|error| error.to_string())?;
    let store = WriteBehindStore::open(&store_path()?).map_err(|error| error.to_string())?;
    let pending = store.pending();
    let engine = WebKitEngine::new(&[(home.clone(), START_PAGE.to_owned())], mtm);
    let browsing =
        Browsing::start(engine, store, SystemClock, &home).map_err(|error| error.to_string())?;
    Ok((Browser::new(browsing, home, mtm), pending))
}

/// Writes the pending saves before the app exits: `terminate:` ends the process without
/// dropping the store. The observer lives as long as the returned token.
fn flush_on_quit(pending: PendingWrites) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let block = RcBlock::new(move |_note: NonNull<NSNotification>| {
        if let Err(error) = pending.flush() {
            unsaved(&error.to_string());
        }
    });
    // SAFETY: an immutable string constant AppKit defines and never changes.
    let name = unsafe { NSApplicationWillTerminateNotification };
    // SAFETY: no object filter and no queue, so the block runs on the main thread that posts
    // the notification, and it captures only a `Send` handle.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            None,
            &block,
        )
    }
}

#[expect(
    clippy::print_stderr,
    reason = "the app's last word as it quits: no window is left to show it"
)]
fn unsaved(reason: &str) {
    eprintln!("quickie: the last changes were not saved: {reason}");
}

/// `~/Library/Application Support/Quickie/shell.sqlite`, its directory created.
fn store_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let directory = PathBuf::from(home).join("Library/Application Support/Quickie");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory.join("shell.sqlite"))
}

#[expect(
    clippy::print_stderr,
    reason = "the composition root's last word when the app cannot start: no window exists yet"
)]
fn fail(reason: &str) -> ExitCode {
    eprintln!("quickie: cannot start: {reason}");
    ExitCode::FAILURE
}

/// The menu bar: the app menu, and the Edit menu that gives text fields and pages their
/// standard editing keys.
fn main_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let bar = NSMenu::new(mtm);
    let app = submenu(mtm, "Quickie", &[("Quit Quickie", sel!(terminate:), "q")]);
    let edit = submenu(
        mtm,
        "Edit",
        &[
            ("Undo", sel!(undo:), "z"),
            ("Redo", sel!(redo:), "Z"),
            ("Cut", sel!(cut:), "x"),
            ("Copy", sel!(copy:), "c"),
            ("Paste", sel!(paste:), "v"),
            ("Select All", sel!(selectAll:), "a"),
        ],
    );
    bar.addItem(&app);
    bar.addItem(&edit);
    bar
}

fn submenu(
    mtm: MainThreadMarker,
    title: &str,
    items: &[(&str, Sel, &str)],
) -> Retained<NSMenuItem> {
    let title = NSString::from_str(title);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
    for &(name, action, key) in items {
        menu.addItem(&item(mtm, name, Some(action), key));
    }
    let holder = item(mtm, "", None, "");
    holder.setSubmenu(Some(&menu));
    holder
}

fn item(
    mtm: MainThreadMarker,
    title: &str,
    action: Option<Sel>,
    key: &str,
) -> Retained<NSMenuItem> {
    // SAFETY: the actions are standard responder-chain selectors that take a sender.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    }
}
