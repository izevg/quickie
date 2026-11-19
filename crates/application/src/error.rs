//! Errors a use case can report.

use quickie_domain_agents::PromptError;
use quickie_domain_browsing::{BrowsingError, CorruptSession};
use quickie_domain_library::NotSaved;
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

/// The secret store (the Keychain) failed to read, write, or delete a key.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("keychain: {0}")]
pub struct SecretError(pub String);

/// An LLM endpoint did not answer as the OpenAI-compatible API does.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LlmError {
    /// The request did not get through: no connection, a timeout.
    #[error(transparent)]
    Http(#[from] HttpError),
    /// The endpoint refused the request, with a status outside 2xx and the message it gave,
    /// such as a wrong key or an unknown model.
    #[error("the endpoint answered {status}: {message}")]
    Status {
        /// The HTTP status.
        status: u16,
        /// The endpoint's own message, or the start of its body.
        message: String,
    },
    /// The endpoint reported an error in the middle of the answer.
    #[error("the endpoint failed: {0}")]
    Api(String),
    /// The answer is not shaped as the API's.
    #[error("the endpoint's answer is not OpenAI-compatible: {0}")]
    Malformed(String),
}

/// Why a use case failed. On [`AppError::Engine`] the session kept the change and the engine
/// catches up later, and on [`AppError::Blocker`] the shields kept it and the next change
/// installs the rules again; on any other variant the session, the library, the shields, or the
/// agent's endpoint are unchanged.
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
    /// The command named a page that has no site to set shields for.
    #[error(transparent)]
    NoSite(#[from] SiteError),
    /// The engine's content blocker refused the rules.
    #[error(transparent)]
    Blocker(#[from] BlockerError),
    /// The Keychain failed.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// The agent was asked something before an LLM endpoint was set up.
    #[error("no LLM endpoint is set up")]
    NoEndpoint,
    /// The pages given cannot make a prompt.
    #[error(transparent)]
    Prompt(#[from] PromptError),
}
