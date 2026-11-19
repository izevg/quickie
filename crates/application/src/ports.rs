//! Driven ports: what the use cases need from the outside world.

use quickie_domain_browsing::{Jar, JarId, SessionSnapshot, TabId, Timestamp, WindowId};
use quickie_domain_library::{BookmarksSnapshot, HistorySnapshot, SearchShortcutsSnapshot};
use url::Url;

use crate::{EngineError, StoreError};

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
}

/// Where the library persists (`shell.sqlite` in the product). Each aggregate saves on its own,
/// so a visit never rewrites the bookmarks. Downloads are not persisted yet.
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
