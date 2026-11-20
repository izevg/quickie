//! The menu bar: the standard menus macOS users expect, so the system's shortcuts and the
//! Services, Window, and Help menus behave. Items that are Quickie commands run them through
//! the same dispatcher as the keymap and the palette (`runCommand:` on the app delegate); they
//! carry no key equivalent, because the keymap owns the keys and the user may rebind them.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;
use quickie_domain_commands::{Catalog, CommandId};

use crate::window::APP_NAME;

/// One line of a menu.
enum Entry<'a> {
    /// A submenu `AppKit` fills, like Services.
    Submenu(&'static str, &'a NSMenu),
    /// A standard action sent down the responder chain, with its key equivalent.
    Standard {
        title: &'static str,
        action: Sel,
        key: &'static str,
        modifiers: NSEventModifierFlags,
    },
    /// A Quickie command, titled as the catalog titles it.
    Command(&'static str),
    Separator,
}

const fn standard(title: &'static str, action: Sel, key: &'static str) -> Entry<'static> {
    Entry::Standard {
        title,
        action,
        key,
        modifiers: NSEventModifierFlags::Command,
    }
}

/// Installs the menu bar. `target` receives `runCommand:` for the command items.
pub fn install(app: &NSApplication, target: &AnyObject, catalog: &Catalog, mtm: MainThreadMarker) {
    let bar = NSMenu::new(mtm);
    let services = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Services"));
    let app_menu = menu(
        mtm,
        APP_NAME,
        &[
            standard("About Quickie", sel!(orderFrontStandardAboutPanel:), ""),
            Entry::Separator,
            Entry::Command("quickie.app.setDefaultBrowser"),
            Entry::Command("quickie.app.revealProfile"),
            Entry::Separator,
            Entry::Submenu("Services", &services),
            Entry::Separator,
            standard("Hide Quickie", sel!(hide:), "h"),
            Entry::Standard {
                title: "Hide Others",
                action: sel!(hideOtherApplications:),
                key: "h",
                modifiers: NSEventModifierFlags::Command | NSEventModifierFlags::Option,
            },
            standard("Show All", sel!(unhideAllApplications:), ""),
            Entry::Separator,
            standard("Quit Quickie", sel!(terminate:), "q"),
        ],
        target,
        catalog,
    );
    let file = menu(
        mtm,
        "File",
        &[
            Entry::Command("quickie.tab.new"),
            Entry::Command("quickie.tab.newThrowaway"),
            Entry::Command("quickie.window.new"),
            Entry::Command("quickie.address.open"),
            Entry::Separator,
            Entry::Command("quickie.tab.close"),
            Entry::Command("quickie.window.close"),
            Entry::Command("quickie.tab.reopenClosed"),
            Entry::Separator,
            Entry::Command("quickie.page.savePdf"),
            Entry::Command("quickie.page.print"),
        ],
        target,
        catalog,
    );
    let edit = menu(
        mtm,
        "Edit",
        &[
            standard("Undo", sel!(undo:), "z"),
            standard("Redo", sel!(redo:), "Z"),
            Entry::Separator,
            standard("Cut", sel!(cut:), "x"),
            standard("Copy", sel!(copy:), "c"),
            standard("Paste", sel!(paste:), "v"),
            standard("Select All", sel!(selectAll:), "a"),
        ],
        target,
        catalog,
    );
    let window = menu(
        mtm,
        "Window",
        &[
            standard("Minimize", sel!(performMiniaturize:), "m"),
            standard("Zoom", sel!(performZoom:), ""),
            Entry::Separator,
            standard("Bring All to Front", sel!(arrangeInFront:), ""),
        ],
        target,
        catalog,
    );
    let help = menu(
        mtm,
        "Help",
        &[Entry::Command("quickie.palette.openCommands")],
        target,
        catalog,
    );
    for (title, submenu) in [
        (APP_NAME, &app_menu),
        ("File", &file),
        ("Edit", &edit),
        ("Window", &window),
        ("Help", &help),
    ] {
        let holder = item(mtm, title, None, "");
        holder.setSubmenu(Some(submenu));
        bar.addItem(&holder);
    }
    app.setMainMenu(Some(&bar));
    app.setServicesMenu(Some(&services));
    app.setWindowsMenu(Some(&window));
    app.setHelpMenu(Some(&help));
}

fn menu(
    mtm: MainThreadMarker,
    title: &str,
    entries: &[Entry],
    target: &AnyObject,
    catalog: &Catalog,
) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    add(&menu, entries, target, catalog, mtm);
    menu
}

fn add(
    menu: &NSMenu,
    entries: &[Entry],
    target: &AnyObject,
    catalog: &Catalog,
    mtm: MainThreadMarker,
) {
    for entry in entries {
        match *entry {
            Entry::Standard {
                title,
                action,
                key,
                modifiers,
            } => {
                let line = item(mtm, title, Some(action), key);
                line.setKeyEquivalentModifierMask(modifiers);
                menu.addItem(&line);
            }
            Entry::Command(id) => {
                let Some(command) = CommandId::try_from(id).ok().and_then(|id| catalog.get(&id))
                else {
                    continue;
                };
                let line = item(mtm, command.title, Some(sel!(runCommand:)), "");
                let id = NSString::from_str(id);
                // SAFETY: the target is the app delegate, which `main` keeps alive for the
                // app's lifetime and which implements `runCommand:`; the represented object is
                // an immutable string the item retains.
                unsafe {
                    line.setTarget(Some(target));
                }
                // SAFETY: an immutable string, retained by the item.
                unsafe {
                    line.setRepresentedObject(Some(&id));
                }
                menu.addItem(&line);
            }
            Entry::Submenu(title, submenu) => {
                let line = item(mtm, title, None, "");
                line.setSubmenu(Some(submenu));
                menu.addItem(&line);
            }
            Entry::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
        }
    }
}

fn item(
    mtm: MainThreadMarker,
    title: &str,
    action: Option<Sel>,
    key: &str,
) -> Retained<NSMenuItem> {
    // SAFETY: the actions are standard responder-chain selectors, or `runCommand:` on the app
    // delegate, and all take a sender.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    }
}
