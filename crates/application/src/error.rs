//! Errors a use case can report.

use quickie_domain_browsing::{BrowsingError, CorruptSession};
use quickie_domain_library::NotSaved;

/// The engine refused or failed a command.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("engine: {0}")]
pub struct EngineError(pub String);

/// The store failed to read or write.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("store: {0}")]
pub struct StoreError(pub String);

/// Why a use case failed. On [`AppError::Engine`] the session kept the change and the engine
/// catches up later; on any other variant the session or the library is unchanged.
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
}
