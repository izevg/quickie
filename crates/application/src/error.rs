//! Errors a use case can report.

use quickie_domain_browsing::{BrowsingError, CorruptSession};
use quickie_domain_library::{DownloadError, NotSaved};
use quickie_domain_shields::SiteError;

/// The engine refused or failed a command.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("engine: {0}")]
pub struct EngineError(pub String);

/// The store failed to read or write.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("store: {0}")]
pub struct StoreError(pub String);

/// An http request failed: no connection, a non-2xx status, or a body that never came.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("http: {0}")]
pub struct HttpError(pub String);

/// The engine's content blocker refused the rules it was given.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("content blocker: {0}")]
pub struct BlockerError(pub String);

/// Why a use case failed. On [`AppError::Engine`] the session kept the change and the engine
/// catches up later, and on [`AppError::Blocker`] the shields kept it and the next change
/// installs the rules again; on any other variant the session, the library, or the shields are
/// unchanged.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    /// The command named a tab or window that is gone.
    #[error(transparent)]
    Browsing(#[from] BrowsingError),
    /// The engine failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The saved session is corrupt; the shell starts a fresh one and keeps the file aside.
    #[error("saved session is corrupt: {0}")]
    CorruptSession(#[from] CorruptSession),
    /// The command named a page that is not bookmarked.
    #[error(transparent)]
    NotSaved(#[from] NotSaved),
    /// The command named a download that is gone, or that cannot take it in its state.
    #[error(transparent)]
    Download(#[from] DownloadError),
    /// The command named a page that has no site to set shields for.
    #[error(transparent)]
    NoSite(#[from] SiteError),
    /// The engine's content blocker refused the rules.
    #[error(transparent)]
    Blocker(#[from] BlockerError),
}
