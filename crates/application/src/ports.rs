//! Driven ports: what the use cases need from the outside world.

use quickie_domain_browsing::{Jar, JarId, SessionSnapshot, TabId, Timestamp, WindowId};
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
