//! The browser: the use cases, one `NSWindow` per session window, the overlays, and the keys
//! that drive them. Every change goes through a use case; this module only routes keys to
//! commands, commands to use cases, and puts the outcome on screen.
//!
//! - [`events`]: what the engine reports, drained in one `match` and routed to the use cases
//!   and the widgets; a page whose process crashed.
//! - [`keys`]: key events → the keymap → a command; the overlays' fixed keys; the `⌃⇥` hold;
//!   what resets when focus moves; the key debug overlay.
//! - [`commands`]: the dispatcher, from a command id to the use case that does it; page zoom,
//!   printing, and saving a page as a PDF.
//! - [`palette`]: the command palette's query and what choosing a row does.
//! - [`address`]: the address overlay's suggestions and where choosing one goes.
//! - [`page`]: find in page, the origin chip, the load glow, the sheets of what pages
//!   ask, and the full-screen notice.
//! - [`navigation`]: the tab overview and its pinch, and link hints.
//! - [`popovers`]: the downloads and the shields popovers.
//! - [`windows`]: new and closed windows, and the pages pages open or close.
//! - [`system`]: links other apps open, the default browser, the profile in Finder, the
//!   Settings window, and web extensions.
//! - [`agent`]: asking the agent about the page, its answer panel, badge, and settings.

mod address;
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

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSApplication, NSView, NSWindow};
use quickie_application::{AppError, Engine, PageSurface, PaletteIndex, PaletteItem};
use quickie_domain_browsing::{Session, TabId, WindowId};
use quickie_domain_commands::{Catalog, CommandId, Keymap, Modifiers};
use quickie_store::{KeybindingsFile, PendingWrites};
use quickie_system::{ExtensionFiles, KeychainSecrets};
use quickie_ui_screens::SettingsScreen;
use quickie_ui_widgets::{
    AddressOverlay, CommandPalette, KeyLogPanel, PageFrame, TabSwitcher, Toast, ToastStack,
};
use url::Url;

pub use keys::install_shortcuts;
pub use system::{KeymapProblems, keymap_from, launch_urls};

use crate::input::cycle::CycleHold;
use crate::input::keystroke::Passed;
use crate::input::keystroke::chord::PendingChord;
use crate::input::keystroke::log::KeyLog;
use crate::window::{browser_window, set_title};
use crate::{Addons, Agents, App, Guard, Lib, Looks, MacEngine, Network, Palette, Zooms};
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
    pub appearance: Looks,
    pub palette: Palette,
    pub catalog: Catalog,
    pub keymap: Keymap,
    /// The user's `keybindings.json`, and the entries the keymap left out of it.
    pub keybindings: KeybindingsFile,
    pub keymap_problems: KeymapProblems,
    /// The stores' writes in flight, polled for failures.
    pub writes: Vec<PendingWrites>,
    /// What went wrong while starting, told once the windows are on screen.
    pub notices: Vec<String>,
    /// Where downloads are saved.
    pub downloads_folder: PathBuf,
    /// The profile folder: the stores and the content rules.
    pub profile: PathBuf,
    /// The main profile's web extensions; `None` on a macOS without them (before 15.4).
    pub extensions: Option<Addons>,
    /// Where extension packages are checked and kept.
    pub extension_files: ExtensionFiles,
}

pub struct Browser {
    /// The engine's network, for the browser's own requests (filter lists, the agent's endpoint,
    /// extension packages): every request of the product goes through the engine's stack.
    http: Network,
    browsing: RefCell<App>,
    library: RefCell<Lib>,
    shields: RefCell<Guard>,
    agent: RefCell<Agents>,
    agent_ui: agent::AgentUi,
    zoom: RefCell<Zooms>,
    extensions: RefCell<Option<Addons>>,
    extension_ui: system::ExtensionUi,
    palette: Palette,
    catalog: Catalog,
    /// Replaced when the user reloads `keybindings.json`.
    keymap: RefCell<Keymap>,
    keybindings: KeybindingsFile,
    /// What the keymap left out of `keybindings.json`, for the Settings window.
    keymap_problems: RefCell<KeymapProblems>,
    /// The Settings window, made on first use.
    settings: OnceCell<SettingsScreen>,
    /// How the last filter list fetch went, for the Settings window.
    lists_status: RefCell<String>,
    /// The user changed the lists: once their rules are compiled, open pages are offered a
    /// reload.
    rules_wanted: Cell<bool>,
    downloads_folder: PathBuf,
    writes: Vec<PendingWrites>,
    windows: RefCell<BTreeMap<WindowId, Retained<NSWindow>>>,
    /// Where each window's page sits: edge to edge, or inset as a card.
    frames: RefCell<BTreeMap<WindowId, PageFrame>>,
    /// The appearance choices, *Inset page* among them.
    appearance: RefCell<Looks>,
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
    /// What the palette offers while it shows; `None` until the next edit gathers it afresh.
    palette_index: RefCell<Option<PaletteIndex>>,
    cycle: RefCell<Option<CycleHold>>,
    /// The modifiers that hold the cycle open; releasing one commits it.
    cycle_modifiers: Cell<Modifiers>,
    /// The first stroke of a two-stroke chord, waiting for the second.
    pending: Cell<PendingChord>,
    /// The last presses the keys passed on: one the page does not take comes back through the
    /// monitor and must go on untouched.
    passed: Cell<Passed>,
    /// The last key presses, while the key debug overlay shows.
    key_log: RefCell<Option<KeyLog>>,
    key_log_panel: KeyLogPanel,
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
        notices.extend(
            services
                .extensions
                .as_ref()
                .and_then(Addons::corrupt_on_start)
                .map(|corrupt| format!("Extensions started fresh: {corrupt}")),
        );
        let http = services.browsing.engine().http();
        let browser = Rc::new_cyclic(|this| Self {
            http,
            browsing: RefCell::new(services.browsing),
            library: RefCell::new(services.library),
            shields: RefCell::new(services.shields),
            agent: RefCell::new(services.agent),
            agent_ui: agent::AgentUi::new(services.secrets, mtm),
            zoom: RefCell::new(services.zoom),
            extensions: RefCell::new(services.extensions),
            extension_ui: system::ExtensionUi::new(services.extension_files, mtm),
            palette: services.palette,
            catalog: services.catalog,
            keymap: RefCell::new(services.keymap),
            keybindings: services.keybindings,
            keymap_problems: RefCell::new(services.keymap_problems),
            settings: OnceCell::new(),
            lists_status: RefCell::new(String::new()),
            rules_wanted: Cell::new(false),
            downloads_folder: services.downloads_folder.clone(),
            writes: services.writes,
            windows: RefCell::new(BTreeMap::new()),
            frames: RefCell::new(BTreeMap::new()),
            appearance: RefCell::new(services.appearance),
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
            palette_index: RefCell::new(None),
            cycle: RefCell::new(None),
            cycle_modifiers: Cell::new(Modifiers::CONTROL),
            pending: Cell::new(PendingChord::default()),
            passed: Cell::new(Passed::default()),
            key_log: RefCell::new(None),
            key_log_panel: KeyLogPanel::new(mtm),
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
        browser.wire_shields();
        browser.wire_engine();
        browser.wire_extensions();
        browser
    }

    /// What pages report reaches the browser: the engine's waker drains it.
    fn wire_engine(&self) {
        let this = self.this.clone();
        self.browsing
            .borrow_mut()
            .engine_mut()
            .set_waker(Box::new(move || {
                if let Some(browser) = this.upgrade() {
                    browser.drain();
                }
            }));
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

    /// Every command the browser runs, built once at start, for the menu bar.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Runs the command `id` as a key would: a menu item asked. An id the catalog does not
    /// know does nothing.
    pub fn run_command(&self, id: &str) {
        if let Ok(id) = CommandId::try_from(id) {
            self.dispatch(&id, None);
            self.drain();
        }
    }

    /// Shows `text` as an info toast in the current window.
    fn toast(&self, text: &str) {
        self.notify(Toast::info(text));
    }

    /// Shows `toast` in the current window; nothing while a page fills it, full screen.
    fn notify(&self, toast: Toast) {
        let Some((id, window)) = self.current() else {
            return;
        };
        if self.page_fullscreen(id) {
            return;
        }
        if let Some(host) = window.contentView() {
            self.toasts.show(&host, toast);
        }
    }

    fn report(&self, error: &AppError) {
        self.notify(Toast::problem(format!("That didn't work: {error}")));
    }

    /// After a command: the active tab's sheet comes up if it asked something, the window's
    /// active page takes the keyboard (unless an overlay or a sheet has it), and window titles
    /// follow the session.
    fn settle(&self, window: WindowId) {
        self.present_asks();
        let view = {
            let browsing = self.browsing.borrow();
            let active = browsing.session().active(window);
            active.and_then(|tab| page_view(browsing.engine(), tab))
        };
        let overlay = self.address.is_open()
            || self.command_palette.is_open()
            || self.navigation.overview_open()
            || self.extension_ui.popup_open()
            || self.sheet_open(window)
            || self.extension_sheet_open(window);
        if let (Some(ns_window), Some(view)) = (self.window(window), view)
            && !overlay
        {
            ns_window.makeFirstResponder(Some(&view));
        }
        self.refresh_titles();
        self.page_changes();
        self.arrange_extensions();
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

    /// One `NSWindow` per session window, its page frame's host registered with the engine.
    fn open_windows(&self) {
        let ids: Vec<WindowId> = self.browsing.borrow().session().windows().collect();
        for id in ids {
            if self.windows.borrow().contains_key(&id) {
                continue;
            }
            self.make_window(id);
        }
    }

    /// A new, hidden `NSWindow` for session window `window`: its page frame laid out as the
    /// appearance says, and its host registered with the engine as the view the window's pages
    /// fill.
    fn make_window(&self, window: WindowId) -> Retained<NSWindow> {
        let (ns_window, frame) = browser_window(self.mtm);
        frame.set_inset(self.inset_page());
        let attached = self
            .browsing
            .borrow_mut()
            .engine_mut()
            .attach_host(window, frame.host());
        if let Err(error) = attached {
            self.report(&error.into());
        }
        self.frames.borrow_mut().insert(window, frame);
        self.windows.borrow_mut().insert(window, ns_window.clone());
        ns_window
    }

    /// The view a window's page fills: inside its page frame, under the overlays.
    fn page_host(&self, window: WindowId) -> Option<Retained<NSView>> {
        self.frames
            .borrow()
            .get(&window)
            .map(|frame| frame.host().retain())
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
        self.palette_sources_changed();
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

/// The view of a tab's page, kept, to hand the keyboard to or to find its window; `None` for a
/// tab without a page.
fn page_view<E: MacEngine>(engine: &E, tab: TabId) -> Option<Retained<NSView>> {
    engine.view(tab).map(Message::retain)
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
