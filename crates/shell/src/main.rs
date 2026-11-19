//! Quickie, the composition root: wires the engine adapter to the UI and runs the app.
//!
//! For now this is the development app on the `engine-webkit` adapter (`docs/architecture.md`
//! §3, P0).
//! Run it from the repository root with:
//!
//! ```sh
//! mise exec -- cargo run -p quickie
//! ```
//!
//! `⌘L` opens the address overlay (`⏎` goes, `⎋` closes), `⌘[` / `⌘]` go back and forward,
//! `⌘R` reloads, `⌘Q` quits. Launching opens no network connection; the start page is inline.

#![expect(
    unsafe_code,
    reason = "AppKit through objc2: window setup, menu items, and the key event monitor"
)]

mod address;
mod input;
mod window;

use std::process::ExitCode;

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSMenu, NSMenuItem};
use objc2_foundation::NSString;
use quickie_engine_webkit::WebKitEngine;

use window::{Browser, install_shortcuts};

fn main() -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        return ExitCode::FAILURE;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.setMainMenu(Some(&main_menu(mtm)));

    let browser = Browser::new(&WebKitEngine::new(mtm), mtm);
    let _monitor = install_shortcuts(&browser);
    browser.show();
    app.activate();
    app.run();
    ExitCode::SUCCESS
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
