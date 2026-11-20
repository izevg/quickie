//! The browser: the use cases, one `NSWindow` per session window, the overlays, and the keys
//! that drive them. Every change goes through a use case; this module only routes keys to
//! commands, commands to use cases, and puts the outcome on screen.
//!
//! - [`events`]: what the engine reports, drained in one `match` and routed to the use cases
//!   and the widgets; a page whose process crashed.
//! - [`keys`]: key events → the keymap → a command; the overlays' fixed keys; the `⌃⇥` hold.
//! - [`commands`]: the dispatcher, from a command id to the use case that does it; page zoom,
//!   printing, and saving a page as a PDF.
//! - [`palette`]: the command palette's query and what choosing a row does.
//! - [`page`]: find in page, the origin chip, the load progress line, the sheets of what pages
//!   ask, and the full-screen notice.
//! - [`navigation`]: the tab overview and its pinch, and link hints.
//! - [`popovers`]: the downloads and the shields popovers.
//! - [`windows`]: new and closed windows, and the pages pages open or close.
//! - [`system`]: links other apps open, the default browser, the profile in Finder.
//! - [`agent`]: asking the agent about the page, its answer panel, badge, and settings.

mod agent;
mod commands;
mod events;
mod keys;
mod navigation;
mod page;
mod palette;
mod popovers;
mod system;
mod windows;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSApplication, NSWindow};
use quickie_application::{AppError, PaletteItem};
use quickie_domain_browsing::{Session, TabId, WindowId};
use quickie_domain_commands::{Catalog, CommandId, KeyStroke, Keymap, Modifiers};
use quickie_store::PendingWrites;
use quickie_system::KeychainSecrets;
use quickie_ui_widgets::{AddressOverlay, CommandPalette, TabSwitcher, ToastStack};
use url::Url;

pub use keys::install_shortcuts;
pub use system::launch_urls;

use crate::input::cycle::CycleHold;
use crate::window::{browser_window, set_title};
use crate::{Agents, App, Guard, Lib, Palette, Zooms};
use navigation::NavigationOverlays;

/// What the browser runs on, composed by `main`.
pub struct Services {
    pub browsing: App,
    pub library: Lib,
    pub shields: Guard,
    pub agent: Agents,
    /// The Keychain, for the worker threads that ask the endpoint.
    pub secrets: KeychainSecrets,
    pub zoom: Zooms,
    pub palette: Palette,
    pub catalog: Catalog,
    pub keymap: Keymap,
    /// The stores' writes in flight, polled for failures.
    pub writes: Vec<PendingWrites>,
    /// What went wrong while starting, told once the windows are on screen.
    pub notices: Vec<String>,
    /// Where downloads are saved.
    pub downloads_folder: PathBuf,
    /// The profile folder: the stores and the content rules.
    pub profile: PathBuf,
}

pub struct Browser {
    browsing: RefCell<App>,
    library: RefCell<Lib>,
    shields: RefCell<Guard>,
    agent: RefCell<Agents>,
    agent_ui: agent::AgentUi,
    zoom: RefCell<Zooms>,
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
    /// The engine's events while they are routed, kept between drains for its allocation.
    events: RefCell<Vec<events::Event>>,
    /// Tabs whose page crashed while in the background: told when they come forward.
    crashed: RefCell<BTreeSet<TabId>>,
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
    /// The profile folder, shown in Finder on request.
    profile: PathBuf,
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
        if let Some(corrupt) = services.agent.corrupt_on_start() {
            notices.push(format!(
                "The agent's saved endpoint was unreadable and is set up anew ({corrupt})"
            ));
        }
        notices.extend(
            services
                .zoom
                .corrupt_on_start()
                .map(|corrupt| format!("Site zooms started fresh: {corrupt}")),
        );
        let browser = Rc::new_cyclic(|this| Self {
            browsing: RefCell::new(services.browsing),
            library: RefCell::new(services.library),
            shields: RefCell::new(services.shields),
            agent: RefCell::new(services.agent),
            agent_ui: agent::AgentUi::new(services.secrets, mtm),
            zoom: RefCell::new(services.zoom),
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
            events: RefCell::new(Vec::new()),
            crashed: RefCell::new(BTreeSet::new()),
            results: RefCell::new(Vec::new()),
            cycle: RefCell::new(None),
            cycle_modifiers: Cell::new(Modifiers::CONTROL),
            pending: Cell::new(None),
            home,
            notices: RefCell::new(notices),
            profile: services.profile,
            this: this.clone(),
            mtm,
        });
        browser.open_windows();
        browser.wire_palette();
        browser.wire_find();
        browser.wire_navigation();
        browser.wire_popovers();
        browser.wire_agent();
        // Only lists the user enabled and that never arrived, or stale ones when the user keeps
        // lists fresh: nothing on a fresh profile.
        let due = browser.shields.borrow().due_fetches();
        browser.fetch_lists(due);
        let weak = Rc::downgrade(&browser);
        browser
            .browsing
            .borrow_mut()
            .engine_mut()
            .set_waker(move || {
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

    /// Runs the command `id` as a key would: a menu item asked. An id the catalog does not
    /// know does nothing.
    pub fn run_command(&self, id: &str) {
        if let Ok(id) = CommandId::try_from(id) {
            self.dispatch(&id, None);
            self.drain();
        }
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

    /// After a command: the active tab's sheet comes up if it asked something, the window's
    /// active page takes the keyboard (unless an overlay or a sheet has it), and window titles
    /// follow the session.
    fn settle(&self, window: WindowId) {
        self.present_asks();
        let view = {
            let mut browsing = self.browsing.borrow_mut();
            let active = browsing.session().active(window);
            active.and_then(|tab| browsing.engine_mut().view(tab).map(Message::retain))
        };
        let overlay = self.address.is_open()
            || self.command_palette.is_open()
            || self.navigation.overview_open()
            || self.sheet_open(window)
            || self.agent_ui.settings_open();
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
