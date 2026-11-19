//! The browser: the use cases, one `NSWindow` per session window, the overlays, and the keys
//! that drive them. Every change goes through a use case; this module only routes keys to
//! commands, commands to use cases, and puts the outcome on screen.
//!
//! - [`keys`]: key events → the keymap → a command; the overlays' fixed keys; the `⌃⇥` hold.
//! - [`commands`]: the dispatcher, from a command id to the use case that does it.
//! - [`palette`]: the command palette's query and what choosing a row does.
//! - [`page`]: find in page, the origin chip, and the load progress line.
//! - [`navigation`]: the tab overview and its pinch, and link hints.
//! - [`popovers`]: the downloads and the shields popovers.

mod commands;
mod keys;
mod navigation;
mod page;
mod palette;
mod popovers;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSApplication, NSWindow};
use quickie_application::{AppError, PaletteItem};
use quickie_domain_browsing::{Session, TabId, WindowId};
use quickie_domain_commands::{Catalog, KeyStroke, Keymap, Modifiers};
use quickie_store::PendingWrites;
use quickie_ui_widgets::{AddressOverlay, CommandPalette, TabSwitcher, ToastStack};
use url::Url;

pub use keys::install_shortcuts;

use crate::input::cycle::CycleHold;
use crate::window::{browser_window, set_title};
use crate::{App, Guard, Lib, Palette};
use navigation::NavigationOverlays;

/// What the browser runs on, composed by `main`.
pub struct Services {
    pub browsing: App,
    pub library: Lib,
    pub shields: Guard,
    pub palette: Palette,
    pub catalog: Catalog,
    pub keymap: Keymap,
    /// The stores' writes in flight, polled for failures.
    pub writes: Vec<PendingWrites>,
    /// What went wrong while starting, told once the windows are on screen.
    pub notices: Vec<String>,
    /// Where downloads are saved.
    pub downloads_folder: PathBuf,
}

pub struct Browser {
    browsing: RefCell<App>,
    library: RefCell<Lib>,
    shields: RefCell<Guard>,
    palette: Palette,
    catalog: Catalog,
    keymap: Keymap,
    writes: Vec<PendingWrites>,
    windows: RefCell<BTreeMap<WindowId, Retained<NSWindow>>>,
    address: AddressOverlay,
    command_palette: CommandPalette,
    switcher: TabSwitcher,
    navigation: NavigationOverlays,
    toasts: Rc<ToastStack>,
    page: page::PageWidgets,
    popovers: popovers::Popovers,
    /// The palette's answer on screen, in its order.
    results: RefCell<Vec<PaletteItem>>,
    cycle: RefCell<Option<CycleHold>>,
    /// The modifiers that hold the cycle open; releasing one commits it.
    cycle_modifiers: Cell<Modifiers>,
    /// The first stroke of a two-stroke chord, waiting for the second.
    pending: Cell<Option<KeyStroke>>,
    /// The start page: new tabs open on it, and the address overlay shows it as empty.
    home: Url,
    /// Startup problems, told by [`Self::show`].
    notices: RefCell<Vec<String>>,
    this: Weak<Self>,
    mtm: MainThreadMarker,
}

impl Browser {
    /// Puts a started session on screen: one window per session window, each showing its
    /// active tab. Engine reports reach the session and the history from here on.
    pub fn new(services: Services, home: Url, mtm: MainThreadMarker) -> Rc<Self> {
        let mut notices = services.notices;
        if let Some(corrupt) = services.browsing.corrupt_on_start() {
            notices.push(format!(
                "Saved session was unreadable, started fresh ({corrupt})"
            ));
        }
        notices.extend(
            services
                .library
                .corrupt_on_start()
                .iter()
                .map(|corrupt| format!("Started fresh: {corrupt}")),
        );
        notices.extend(
            services
                .shields
                .corrupt_on_start()
                .iter()
                .map(|corrupt| format!("Started fresh: {corrupt}")),
        );
        let browser = Rc::new_cyclic(|this| Self {
            browsing: RefCell::new(services.browsing),
            library: RefCell::new(services.library),
            shields: RefCell::new(services.shields),
            palette: services.palette,
            catalog: services.catalog,
            keymap: services.keymap,
            writes: services.writes,
            windows: RefCell::new(BTreeMap::new()),
            address: AddressOverlay::new(mtm),
            command_palette: CommandPalette::new(mtm),
            switcher: TabSwitcher::new(mtm),
            navigation: NavigationOverlays::new(mtm),
            toasts: ToastStack::new(mtm),
            page: page::PageWidgets::new(mtm),
            popovers: popovers::Popovers::new(services.downloads_folder, mtm),
            results: RefCell::new(Vec::new()),
            cycle: RefCell::new(None),
            cycle_modifiers: Cell::new(Modifiers::CONTROL),
            pending: Cell::new(None),
            home,
            notices: RefCell::new(notices),
            this: this.clone(),
            mtm,
        });
        browser.open_windows();
        browser.wire_palette();
        browser.wire_find();
        browser.wire_navigation();
        browser.wire_popovers();
        // Only lists the user enabled and that never arrived, or stale ones when the user keeps
        // lists fresh: nothing on a fresh profile.
        let due = browser.shields.borrow().due_fetches();
        browser.fetch_lists(due);
        let weak = Rc::downgrade(&browser);
        browser
            .browsing
            .borrow_mut()
            .engine_mut()
            .on_navigated(move || {
                if let Some(browser) = weak.upgrade() {
                    browser.drain();
                }
            });
        browser
    }

    pub fn show(&self) {
        let windows: Vec<(WindowId, Retained<NSWindow>)> = self
            .windows
            .borrow()
            .iter()
            .map(|(&id, window)| (id, window.clone()))
            .collect();
        for (id, window) in windows {
            window.center();
            window.makeKeyAndOrderFront(None);
            self.settle(id);
        }
        let notices = std::mem::take(&mut *self.notices.borrow_mut());
        for notice in notices {
            self.toast(&notice);
        }
    }

    /// Records what pages committed since the last time, puts the pages' loading, finds, and
    /// origins on screen, records downloads, and tells of a save that failed on a writer
    /// thread. Runs from the engine's waker on the main run loop, and after every command.
    pub fn drain(&self) {
        self.record_navigations();
        self.page_reports();
        self.download_reports();
        for writes in &self.writes {
            if let Some(error) = writes.take_error() {
                self.toast(&format!("Not saved: {error}"));
            }
        }
    }

    /// Pages' reports into the session, and, for a commit, a visit into history; a later title
    /// renames the visit.
    fn record_navigations(&self) {
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let navigations = browsing.engine_mut().take_navigations();
        if navigations.is_empty() {
            return;
        }
        let mut failures = Vec::new();
        let mut loaded = Vec::new();
        for navigated in navigations {
            if navigated.committed {
                loaded.push(navigated.tab);
            }
            let (url, title) = (navigated.url.clone(), navigated.title.clone());
            if let Err(error) = browsing.navigated(navigated.tab, navigated.url, navigated.title) {
                failures.push(error);
                continue;
            }
            let Some(jar) = browsing.session().tab(navigated.tab).map(|tab| tab.jar) else {
                continue;
            };
            let mut library = self.library.borrow_mut();
            let recorded = if navigated.committed {
                library.record_visit(&url, &title, jar).map(drop)
            } else {
                library.retitle_visit(&url, &title, jar).map(drop)
            };
            if let Err(error) = recorded {
                failures.push(error);
            }
        }
        drop(browsing);
        for error in failures {
            self.report(&error);
        }
        for tab in loaded {
            self.applied(tab);
        }
        self.refresh_titles();
    }

    /// Shows `text` as a toast in the current window.
    fn toast(&self, text: &str) {
        if let Some(host) = self.current().and_then(|(_, window)| window.contentView()) {
            self.toasts.show(&host, text);
        }
    }

    fn report(&self, error: &AppError) {
        self.toast(&format!("That didn't work: {error}"));
    }

    /// After a command: the window's active page takes the keyboard (unless an overlay has
    /// it), and window titles follow the session.
    fn settle(&self, window: WindowId) {
        let view = {
            let mut browsing = self.browsing.borrow_mut();
            let active = browsing.session().active(window);
            active.and_then(|tab| browsing.engine_mut().view(tab).map(Message::retain))
        };
        let overlay = self.address.is_open()
            || self.command_palette.is_open()
            || self.navigation.overview_open();
        if let (Some(ns_window), Some(view)) = (self.window(window), view)
            && !overlay
        {
            ns_window.makeFirstResponder(Some(&view));
        }
        self.refresh_titles();
        self.page_changes();
    }

    fn refresh_titles(&self) {
        let browsing = self.browsing.borrow();
        let session = browsing.session();
        for (&id, window) in self.windows.borrow().iter() {
            let title = session
                .active(id)
                .and_then(|tab| session.tab(tab))
                .map_or("", |tab| tab.title.as_str());
            set_title(window, title);
        }
    }

    /// One `NSWindow` per session window, its content view registered with the engine.
    fn open_windows(&self) {
        let ids: Vec<WindowId> = self.browsing.borrow().session().windows().collect();
        for id in ids {
            if self.windows.borrow().contains_key(&id) {
                continue;
            }
            let window = browser_window(self.mtm);
            if let Some(host) = window.contentView() {
                self.browsing
                    .borrow_mut()
                    .engine_mut()
                    .attach_host(id, &host);
            }
            self.windows.borrow_mut().insert(id, window);
        }
    }

    /// The window commands act on: the key window, else the first one.
    fn current(&self) -> Option<(WindowId, Retained<NSWindow>)> {
        let key = NSApplication::sharedApplication(self.mtm).keyWindow();
        let windows = self.windows.borrow();
        key.and_then(|key| windows.iter().find(|(_, window)| ***window == *key))
            .or_else(|| windows.iter().next())
            .map(|(&id, window)| (id, window.clone()))
    }

    fn window(&self, id: WindowId) -> Option<Retained<NSWindow>> {
        self.windows.borrow().get(&id).cloned()
    }

    fn active(&self, window: WindowId) -> Option<TabId> {
        self.browsing.borrow().session().active(window)
    }

    /// Runs a browsing use case; a failure is told in a toast and yields `None`.
    fn run<T>(&self, command: impl FnOnce(&mut App) -> Result<T, AppError>) -> Option<T> {
        let outcome = command(&mut self.browsing.borrow_mut());
        outcome.map_err(|error| self.report(&error)).ok()
    }

    /// Closes the E2 overlays: only one shows at a time, and a command runs with none.
    fn close_overlays(&self) {
        self.address.hide();
        self.command_palette.hide();
        self.hide_popovers();
        self.cancel_cycle();
    }
}

/// What the switcher calls a tab: its title, else its host, else its URL.
fn label(session: &Session, tab: TabId) -> String {
    let Some(tab) = session.tab(tab) else {
        return String::new();
    };
    if tab.title.is_empty() {
        tab.url.host_str().unwrap_or(tab.url.as_str()).to_owned()
    } else {
        tab.title.clone()
    }
}
