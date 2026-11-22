//! Agent use cases: endpoint settings, asking about pages, and selecting links for user-requested
//! navigation.
//!
//! The settings follow the transaction rule of the other use cases: a change is saved first
//! and only then replaces the endpoint, so a failed save changes nothing. The key goes to the
//! [`SecretStore`], never to the store; the endpoint keeps only its [`KeyRef`].
//!
//! Asking starts by reading the pages: a [`PageReader`] picks the tabs the request reads and
//! asks the engine for their text, which comes back as engine events; the shell hands each to
//! the reader until every page is in. Then two halves, like fetching filter lists.
//! [`Agent::ask`] runs on the main thread: it checks there is an endpoint and assembles the
//! prompt, which is cheap. The [`Asking`] it returns owns everything the answer needs, so the
//! shell moves it to a worker thread and calls [`Asking::run`] there, which blocks on the
//! network and hands each [`AnswerEvent`] to a callback; the shell forwards them to the main
//! thread. The endpoint is contacted only then, on the user's explicit request.

mod ask;
mod local;
mod navigation;
#[cfg(test)]
mod tests;

use quickie_domain_agents::{
    ApiKey, BaseUrl, CorruptEndpoint, LlmEndpoint, ModelId, PageText, Prompt, Request, TokenBudget,
};

pub use ask::{AnswerEvent, AskError, Asking, PageReader};
pub use local::{LocalFound, detect_local, list_models};
pub use navigation::{AgentLink, AgentLinks, SelectingLink, SelectionError};

use crate::{AgentRepo, AppError, Entropy, SecretStore};

/// The agent use cases over the profile's endpoint settings and the Keychain.
#[derive(Debug)]
pub struct Agent<R, S, E> {
    endpoint: Option<LlmEndpoint>,
    corrupt_on_start: Option<CorruptEndpoint>,
    repo: R,
    secrets: S,
    entropy: E,
}

impl<R: AgentRepo, S: SecretStore, E: Entropy> Agent<R, S, E> {
    /// Loads the saved endpoint. One that breaks an invariant is dropped, as if none was set
    /// up, and reported by [`Self::corrupt_on_start`]; the next change saves over it.
    /// `entropy` draws each request's fence marker.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    pub fn start(repo: R, secrets: S, entropy: E) -> Result<Self, AppError> {
        let (endpoint, corrupt_on_start) = match repo.load_endpoint()? {
            None => (None, None),
            Some(saved) => match LlmEndpoint::restore(&saved) {
                Ok(endpoint) => (Some(endpoint), None),
                Err(corrupt) => (None, Some(corrupt)),
            },
        };
        Ok(Self {
            endpoint,
            corrupt_on_start,
            repo,
            secrets,
            entropy,
        })
    }

    /// The endpoint the agent asks, if one is set up.
    #[must_use]
    pub fn endpoint(&self) -> Option<&LlmEndpoint> {
        self.endpoint.as_ref()
    }

    /// The saved endpoint that was dropped at start, if it was corrupt.
    #[must_use]
    pub fn corrupt_on_start(&self) -> Option<&CorruptEndpoint> {
        self.corrupt_on_start.as_ref()
    }

    /// Sets up the endpoint at `base`, asking `model`. A key stays with the endpoint only when
    /// the origin stays the same; a key of another origin stays in the Keychain, unused.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn set_endpoint(&mut self, base: BaseUrl, model: ModelId) -> Result<(), AppError> {
        let next = match &self.endpoint {
            Some(current) => current.moved(base, model),
            None => LlmEndpoint::new(base, model),
        };
        self.commit(Some(next))
    }

    /// Gives the endpoint `key`: into the Keychain under the endpoint's origin, then into the
    /// settings. A key already there is replaced.
    ///
    /// # Errors
    ///
    /// No endpoint is set up, the Keychain failed (nothing changed), or the store failed (the
    /// Keychain holds the key, unused until the next try).
    pub fn set_key(&mut self, key: &ApiKey) -> Result<(), AppError> {
        let next = self
            .endpoint
            .clone()
            .ok_or(AppError::NoEndpoint)?
            .with_key();
        if let Some(reference) = next.key() {
            self.secrets.write(reference, key)?;
        }
        self.commit(Some(next))
    }

    /// Takes the key away from the endpoint and out of the Keychain. Without a key, nothing
    /// changes.
    ///
    /// # Errors
    ///
    /// The store failed (nothing changed), or the Keychain failed after the settings dropped the
    /// key, which then stays in the Keychain unused.
    pub fn remove_key(&mut self) -> Result<(), AppError> {
        let Some(current) = self.endpoint.clone() else {
            return Ok(());
        };
        let Some(reference) = current.key().cloned() else {
            return Ok(());
        };
        self.commit(Some(current.without_key()))?;
        self.secrets.delete(&reference)?;
        Ok(())
    }

    /// Forgets the endpoint and deletes its key.
    ///
    /// # Errors
    ///
    /// The store failed (nothing changed), or the Keychain failed after the endpoint was
    /// forgotten, so the key stays in the Keychain unused.
    pub fn forget_endpoint(&mut self) -> Result<(), AppError> {
        let reference = self.endpoint.as_ref().and_then(|e| e.key().cloned());
        self.commit(None)?;
        if let Some(reference) = reference {
            self.secrets.delete(&reference)?;
        }
        Ok(())
    }

    /// Prepares asking `request` about `pages`, the current page first: the user asked, with
    /// `?` in the palette or a skill. Each request is fenced with a fresh marker. Returns at
    /// once; run the result on a worker thread.
    ///
    /// # Errors
    ///
    /// No endpoint is set up, no page was given, or too few for the request.
    pub fn ask(&self, request: &Request, pages: &[PageText]) -> Result<Asking, AppError> {
        let endpoint = self.endpoint.clone().ok_or(AppError::NoEndpoint)?;
        let marker = self.entropy.fence_marker();
        let prompt = Prompt::assemble(request, pages, TokenBudget::DEFAULT, &marker)?;
        Ok(Asking::new(endpoint, prompt))
    }

    fn commit(&mut self, next: Option<LlmEndpoint>) -> Result<(), AppError> {
        let snapshot = next.as_ref().map(LlmEndpoint::snapshot);
        self.repo.save_endpoint(snapshot)?;
        self.endpoint = next;
        Ok(())
    }
}
