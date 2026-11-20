//! Web extensions as an engine capability: the [`ExtensionHost`] that runs them in the main
//! profile's pages, and the [`ExtensionPackages`] that check a package and keep it where the
//! host reads it. Both name outcomes: how an engine loads an extension (`WKWebExtension` on
//! `WebKit`, its own extension system on Chromium) is its own business.
//!
//! Like pages, the host never calls back into a use case: everything an extension asks of the
//! browser (a permission, a popup to show, a tab to open, activate, or close) arrives as an
//! [`ExtensionEvent`], drained by the shell after the host's waker ran.

use std::convert::Infallible;
use std::marker::PhantomData;
use std::path::PathBuf;

use quickie_domain_browsing::{TabId, WindowId};
use quickie_domain_settings::{ExtensionId, ExtensionVersion, Grants, Site, SiteAccess};
use url::Url;

use crate::{EngineError, PackageError, ReplyToken};

/// What the engine read from an extension's manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionInfo {
    pub name: String,
    pub version: ExtensionVersion,
    /// What the manifest says it is for; may be empty.
    pub description: String,
    /// The permissions and host patterns it asks for up front.
    pub requested: Grants,
    /// What its toolbar action is called, when it has one: its popup or its click.
    pub action: Option<String>,
    /// Its keyboard commands (`commands` in the manifest), in manifest order.
    pub commands: Vec<ExtensionCommand>,
}

/// A keyboard command an extension declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionCommand {
    /// Its name in the manifest, `_execute_action` for the action's own.
    pub name: String,
    /// What it does, as the manifest describes it.
    pub title: String,
    /// The key the manifest suggests, in `keybindings.json`'s syntax (`cmd+shift+y`), as the
    /// engine resolved it for the platform; `None` without one.
    pub shortcut: Option<String>,
}

/// What an extension may do where, as the host applies it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Access {
    pub granted: Grants,
    /// The sites the user allowed or blocked it on.
    pub sites: Vec<(Site, SiteAccess)>,
}

/// A browser window's tabs as extensions see them, in the window's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowTabs {
    pub window: WindowId,
    pub tabs: Vec<TabId>,
    pub active: Option<TabId>,
}

/// What the host reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionEvent {
    /// The package [`ExtensionHost::read`] was asked for was read, or could not be.
    Read {
        id: ExtensionId,
        info: Result<ExtensionInfo, String>,
    },
    /// A started extension could not run.
    Failed { id: ExtensionId, error: String },
    /// A running extension asks for more than it was granted, while it waits;
    /// [`ExtensionHost::answer`] with `token` answers it.
    Asked {
        token: ReplyToken,
        id: ExtensionId,
        /// The tab it asks in, when it asks from one.
        tab: Option<TabId>,
        request: Grants,
    },
    /// The extension's action popup is ready to show.
    Popup { id: ExtensionId },
    /// The extension opens a page in a new tab; [`ExtensionHost::opened`] with `token` says
    /// which tab, or none.
    OpenTab {
        token: ReplyToken,
        id: ExtensionId,
        url: Url,
        /// In front, or in the background.
        active: bool,
    },
    /// The extension brings a tab forward.
    ActivateTab(TabId),
    /// The extension closes a tab.
    CloseTab(TabId),
}

/// Runs web extensions in the main profile's pages, never in the agent's profile, and without
/// access to throwaway jars' tabs and data.
pub trait ExtensionHost {
    /// A popup's page, as the toolkit shows it; the shell names its type.
    type Popup;

    /// Reads the package at `location` (from [`ExtensionPackages::unpack`]) as `id`, without
    /// running it; [`ExtensionEvent::Read`] says what is in it. A package read earlier for
    /// `id` and not started is dropped.
    ///
    /// # Errors
    ///
    /// The engine cannot run extensions.
    fn read(&mut self, id: &ExtensionId, location: &str) -> Result<(), EngineError>;

    /// Runs `id`'s package with `access`: the one last read, which replaces a running
    /// version, else the running one, which takes the new access.
    ///
    /// # Errors
    ///
    /// Nothing was read for `id`.
    fn start(&mut self, id: &ExtensionId, access: &Access) -> Result<(), EngineError>;

    /// Stops `id`; its package stays, to start again.
    fn stop(&mut self, id: &ExtensionId);

    /// Drops a package read for `id` and not started; a running version keeps running. Its
    /// file goes with the next [`ExtensionPackages::prune`].
    fn discard(&mut self, id: &ExtensionId);

    /// Stops `id` and forgets it, and the storage it kept in the profile; its package goes
    /// with the next [`ExtensionPackages::prune`].
    fn remove(&mut self, id: &ExtensionId);

    /// Runs `id`'s action as a click on its toolbar button would, on `tab`'s page: its popup
    /// opens ([`ExtensionEvent::Popup`]), or it hears the click.
    ///
    /// # Errors
    ///
    /// `id` is not running or has no action.
    fn perform_action(&mut self, id: &ExtensionId, tab: Option<TabId>) -> Result<(), EngineError>;

    /// Runs `id`'s keyboard command `command`.
    ///
    /// # Errors
    ///
    /// `id` is not running or has no such command.
    fn perform_command(&mut self, id: &ExtensionId, command: &str) -> Result<(), EngineError>;

    /// Answers [`ExtensionEvent::Asked`]: `granted` is what the extension may now do of what it
    /// asked, nothing for a refusal.
    fn answer(&mut self, token: ReplyToken, granted: &Grants);

    /// Answers [`ExtensionEvent::OpenTab`] with the tab that opened, `None` when none did.
    fn opened(&mut self, token: ReplyToken, tab: Option<TabId>);

    /// The windows and their tabs as they are now, the focused one first, so extensions see
    /// what the user sees.
    fn arrange(&mut self, windows: &[WindowTabs]);

    /// Moves what extensions did since the last call to the end of `into`, oldest first.
    fn drain(&mut self, into: &mut Vec<ExtensionEvent>);

    /// Sets what runs, later on the main loop and never inside a call to the host, when events
    /// wait: it should [`Self::drain`] them. Set once, at start.
    fn set_waker(&mut self, wake: Box<dyn Fn()>);

    /// The popup [`ExtensionEvent::Popup`] announced, sized as its extension asked; `None` once
    /// it is gone.
    fn popup(&self) -> Option<Self::Popup>;

    /// Closes the popup on screen, if any, as clicking outside it would.
    fn close_popup(&self);
}

/// The host of an engine that runs no web extensions, whose popups would be `P`s: none can
/// exist, so none is ever called.
#[derive(Debug)]
pub struct NoExtensions<P>(Infallible, PhantomData<P>);

impl<P> ExtensionHost for NoExtensions<P> {
    type Popup = P;

    fn read(&mut self, _: &ExtensionId, _: &str) -> Result<(), EngineError> {
        match self.0 {}
    }

    fn start(&mut self, _: &ExtensionId, _: &Access) -> Result<(), EngineError> {
        match self.0 {}
    }

    fn stop(&mut self, _: &ExtensionId) {
        match self.0 {}
    }

    fn discard(&mut self, _: &ExtensionId) {
        match self.0 {}
    }

    fn remove(&mut self, _: &ExtensionId) {
        match self.0 {}
    }

    fn perform_action(&mut self, _: &ExtensionId, _: Option<TabId>) -> Result<(), EngineError> {
        match self.0 {}
    }

    fn perform_command(&mut self, _: &ExtensionId, _: &str) -> Result<(), EngineError> {
        match self.0 {}
    }

    fn answer(&mut self, _: ReplyToken, _: &Grants) {
        match self.0 {}
    }

    fn opened(&mut self, _: ReplyToken, _: Option<TabId>) {
        match self.0 {}
    }

    fn arrange(&mut self, _: &[WindowTabs]) {
        match self.0 {}
    }

    fn drain(&mut self, _: &mut Vec<ExtensionEvent>) {
        match self.0 {}
    }

    fn set_waker(&mut self, _: Box<dyn Fn()>) {
        match self.0 {}
    }

    fn popup(&self) -> Option<P> {
        match self.0 {}
    }

    fn close_popup(&self) {
        match self.0 {}
    }
}

/// A package to install, as it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageSource {
    /// A CRX3 package downloaded from the Chrome Web Store.
    Crx(Vec<u8>),
    /// A `.crx` or `.zip` file the user chose.
    File(PathBuf),
    /// A folder with a `manifest.json` the user chose; it is used where it is.
    Folder(PathBuf),
}

/// What a package must be to be installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    /// The id its signature must prove.
    pub id: ExtensionId,
    /// The SHA-256 of the package as the store announced it, in lowercase hexadecimal.
    pub sha256: Option<String>,
}

/// A package checked and kept where the host reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unpacked {
    /// The id its signature proves, or, for a package without one, the id of where it was
    /// found.
    pub id: ExtensionId,
    /// Where [`ExtensionHost::read`] finds it.
    pub location: String,
}

/// Checks packages and keeps them for the host. Its calls block on the disk: call them on a
/// worker thread, never on the main thread.
pub trait ExtensionPackages {
    /// Checks `source` (a CRX3 package's signatures, and `expected` when given) and keeps it
    /// where [`ExtensionHost::read`] reads it.
    ///
    /// # Errors
    ///
    /// The package is malformed, its signature does not hold, it is not what `expected` says,
    /// or it could not be kept.
    fn unpack(
        &self,
        source: &PackageSource,
        expected: Option<&Expected>,
    ) -> Result<Unpacked, PackageError>;

    /// Deletes every kept package but those at `keep`: removed extensions, replaced versions,
    /// and declined installs. A folder the user chose is never deleted.
    ///
    /// # Errors
    ///
    /// A file could not be deleted.
    fn prune(&self, keep: &[String]) -> Result<(), PackageError>;
}
