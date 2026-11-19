//! Driven ports: what the use cases need from the outside world.

use std::sync::Arc;

use quickie_domain_browsing::{Jar, JarId, SessionSnapshot, TabId, Timestamp, WindowId};
use quickie_domain_library::{
    BookmarksSnapshot, DownloadsSnapshot, HistorySnapshot, SearchShortcutsSnapshot,
};
use quickie_domain_shields::{
    ContentHash, FilterListId, FilterListSetSnapshot, Site, SitePoliciesSnapshot,
};
use url::Url;

use crate::{BlockerError, EngineError, HttpError, StoreError};

/// The wall clock.
pub trait Clock {
    /// Now.
    fn now(&self) -> Timestamp;
}

/// Where the session persists (`shell.sqlite` in the product).
pub trait SessionRepo {
    /// The last saved session, `None` on first launch.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn load(&self) -> Result<Option<SessionSnapshot>, StoreError>;

    /// Replaces the saved session, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save(&mut self, snapshot: &SessionSnapshot) -> Result<(), StoreError>;
}

/// The library as last saved. Each part is `None` until it is first saved, so a profile that
/// deleted every search shortcut does not get the defaults back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibrarySnapshot {
    /// The visited pages.
    pub history: Option<HistorySnapshot>,
    /// The saved pages and their tags.
    pub bookmarks: Option<BookmarksSnapshot>,
    /// The `!` keyword searches.
    pub shortcuts: Option<SearchShortcutsSnapshot>,
    /// The downloads of the main jar.
    pub downloads: Option<DownloadsSnapshot>,
}

/// Where the library persists (`shell.sqlite` in the product). Each aggregate saves on its own,
/// so a visit never rewrites the bookmarks.
pub trait LibraryRepo {
    /// Every part of the library that was ever saved.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn load_library(&self) -> Result<LibrarySnapshot, StoreError>;

    /// Replaces the saved history, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_history(&mut self, history: &HistorySnapshot) -> Result<(), StoreError>;

    /// Replaces the saved bookmarks, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_bookmarks(&mut self, bookmarks: &BookmarksSnapshot) -> Result<(), StoreError>;

    /// Replaces the saved search shortcuts, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_shortcuts(&mut self, shortcuts: &SearchShortcutsSnapshot) -> Result<(), StoreError>;

    /// Replaces the saved downloads, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_downloads(&mut self, downloads: &DownloadsSnapshot) -> Result<(), StoreError>;
}

/// One entry of the user's `keybindings.json`, as text: the store does not judge it, the
/// keymap use case does, so a bad entry is reported instead of losing the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingText {
    /// The chord, such as `cmd+k cmd+s`.
    pub key: String,
    /// The command id, or `-<id>` to remove a default binding.
    pub command: String,
    /// The `when` clause, if the entry has one.
    pub when: Option<String>,
}

/// Where the user's key bindings persist. Saving arrives with the keymap editor.
pub trait KeymapRepo {
    /// The user's entries, in the order they were written.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn load_keybindings(&self) -> Result<Vec<KeybindingText>, StoreError>;
}

/// A navigation inside a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Navigation {
    /// Load a URL.
    Load(Url),
    /// History back.
    Back,
    /// History forward.
    Forward,
    /// Reload.
    Reload,
    /// Stop loading.
    Stop,
}

/// The browser engine: Chromium in the product, `WebKit` in development. It owns live pages;
/// it learns about tabs only through these calls, keyed by the domain's [`TabId`].
pub trait EnginePort {
    /// The engine's handle on a page it opened by itself (a popup, a ⌘-click), before the
    /// session gives it a [`TabId`].
    type Page;

    /// Binds a page the engine opened to the tab the session made for it. Called before the
    /// tab is shown, so the engine knows which page to show.
    ///
    /// # Errors
    ///
    /// The page is gone, or the tab already has a page.
    fn adopt(&mut self, page: Self::Page, tab: TabId) -> Result<(), EngineError>;

    /// Creates the page for a tab, in its jar. Called lazily: a restored tab gets a page only
    /// when it is first shown.
    ///
    /// # Errors
    ///
    /// The engine could not create the page.
    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError>;

    /// Shows a tab's page in a window, in place of the one shown before.
    ///
    /// # Errors
    ///
    /// The engine has no page for the tab or no such window.
    fn show(&mut self, window: WindowId, tab: TabId) -> Result<(), EngineError>;

    /// Navigates a tab's page.
    ///
    /// # Errors
    ///
    /// The engine has no page for the tab.
    fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), EngineError>;

    /// Destroys a tab's page.
    ///
    /// # Errors
    ///
    /// The engine failed to close the page.
    fn close(&mut self, tab: TabId) -> Result<(), EngineError>;

    /// Destroys a throwaway jar's cookies and storage; no live tab uses it any more.
    ///
    /// # Errors
    ///
    /// The engine failed to clear the partition.
    fn destroy_jar(&mut self, jar: JarId) -> Result<(), EngineError>;
}

/// The shields as last saved. A profile that never saved them has none of each: no exceptions,
/// every list off and never fetched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShieldsSnapshot {
    /// The per-site exceptions.
    pub sites: SitePoliciesSnapshot,
    /// The filter lists' states.
    pub lists: FilterListSetSnapshot,
    /// The content last fetched of each list, as it arrived.
    pub bodies: Vec<(FilterListId, Vec<u8>)>,
}

/// Where the shields persist (`shell.sqlite` in the product). Each part saves on its own, so
/// toggling a site never rewrites a list's content.
pub trait ShieldsRepo {
    /// Everything ever saved of the shields.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    fn load_shields(&self) -> Result<ShieldsSnapshot, StoreError>;

    /// Replaces the saved per-site exceptions, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_sites(&mut self, sites: &SitePoliciesSnapshot) -> Result<(), StoreError>;

    /// Replaces the saved filter list states, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_lists(&mut self, lists: &FilterListSetSnapshot) -> Result<(), StoreError>;

    /// Replaces the saved content of one list, atomically.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    fn save_list_body(&mut self, list: FilterListId, body: &[u8]) -> Result<(), StoreError>;
}

/// The network, for the browser's own requests, such as fetching a filter list. Chromium's
/// network stack in the product, so the egress test sees every request; `NSURLSession` in
/// development.
pub trait HttpPort {
    /// GETs `url` and returns the body of a successful (2xx) response. Blocks until the whole
    /// body arrived: call it on a worker thread, never on the main thread.
    ///
    /// # Errors
    ///
    /// No connection, a non-2xx status, a timeout, or a call on the main thread.
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError>;
}

/// One filter list's content, as the content blocker compiles it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRules {
    /// Which list.
    pub list: FilterListId,
    /// The hash of `body`, which names the compiled rules.
    pub hash: ContentHash,
    /// The list as fetched: filter rules in Adblock Plus syntax. Shared, not copied, with the
    /// thread that compiles it.
    pub body: Arc<[u8]>,
}

/// Everything the content blocker needs: the enabled lists' content and the sites whose
/// shields are off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockerRules {
    /// The enabled lists that have content, in [`FilterListId::ALL`] order.
    pub lists: Vec<ListRules>,
    /// The sites whose pages load everything, in the order their shields went off.
    pub unshielded: Vec<Site>,
}

/// The engine's request blocker: `adblock` in Chromium's network service in the product, a
/// `WKContentRuleList` in development.
pub trait ContentBlockerPort {
    /// Makes `rules` the ones every page loads with from now on, and returns at once: compiling
    /// takes seconds, so it happens off the main thread, and pages keep the previous rules until
    /// it is done. Rules equal to the installed ones cost nothing.
    ///
    /// # Errors
    ///
    /// The blocker could not start; a failure while compiling is the adapter's to report.
    fn install(&mut self, rules: &BlockerRules) -> Result<(), BlockerError>;
}
