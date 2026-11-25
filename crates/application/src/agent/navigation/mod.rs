//! Selecting a link for an explicitly requested navigation in the agent profile.

use std::collections::HashSet;

use quickie_domain_agents::{LlmEndpoint, Prompt, Question};
use url::Url;

use super::{Agent, AskError, ask::key_of};
use crate::{AgentRepo, ChatEvent, ChatRequest, Ended, Entropy, LlmPort, SecretStore, StopSignal};

const MAX_LINKS: usize = 24;
const MAX_LABEL_CHARS: usize = 112;
const MAX_REPLY_BYTES: usize = 64;

/// A visible link the engine read without activating it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLink {
    /// The link's resolved destination.
    pub url: Url,
    /// The link text as read from the document.
    pub label: String,
}

impl AgentLink {
    /// The destination has a known video path.
    #[must_use]
    pub fn is_video(&self) -> bool {
        likely_video(&self.url)
    }
}

/// Visible links in document order, read from the same document at the same time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLinks {
    /// The document that contained these links, as captured by the engine.
    pub document: Url,
    /// The links observed in that document.
    pub links: Vec<AgentLink>,
}

/// Why an agent could not select a link.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    /// The user has not configured an endpoint.
    #[error("there is no configured model")]
    NoEndpoint,
    /// The source is not an ordinary web document.
    #[error("link selection requires an HTTP or HTTPS document without credentials")]
    InvalidDocument,
    /// The endpoint did not return exactly one known candidate or abstain.
    #[error("the model returned an invalid link selection")]
    InvalidSelection,
    /// The selection was stopped before completion.
    #[error("link selection was stopped")]
    Stopped,
    /// The key could not be read or the configured model failed.
    #[error(transparent)]
    Ask(#[from] AskError),
}

/// A link selection prepared on the main thread and ready to run on a worker.
#[derive(Debug)]
pub struct SelectingLink {
    endpoint: LlmEndpoint,
    prompt: Prompt,
    candidates: Vec<AgentLink>,
}

impl<R: AgentRepo, S: SecretStore, E: Entropy> Agent<R, S, E> {
    /// Prepares an explicit request to select one link from the current document. The candidate
    /// URLs stay in Quickie. The shell revalidates a selected link before any navigation.
    ///
    /// # Errors
    /// The endpoint is missing or the source is not an ordinary web document.
    pub fn plan_navigation(
        &self,
        question: &Question,
        snapshot: AgentLinks,
    ) -> Result<SelectingLink, SelectionError> {
        let endpoint = self.endpoint.clone().ok_or(SelectionError::NoEndpoint)?;
        if !ordinary_url(&snapshot.document) {
            return Err(SelectionError::InvalidDocument);
        }
        let origin = snapshot.document.origin();
        let mut seen = HashSet::new();
        let candidates: Vec<_> = snapshot
            .links
            .into_iter()
            .filter(|link| ordinary_url(&link.url) && link.url.origin() == origin)
            .filter(|link| seen.insert(link.url.clone()))
            .take(MAX_LINKS)
            .collect();
        let labels = candidates.iter().map(|link| {
            let label = link
                .label
                .char_indices()
                .nth(MAX_LABEL_CHARS)
                .map_or(link.label.as_str(), |(end, _)| &link.label[..end]);
            (label, link.is_video())
        });
        let prompt = Prompt::select_link(question, labels, &self.entropy.fence_marker());
        Ok(SelectingLink {
            endpoint,
            prompt,
            candidates,
        })
    }
}

impl SelectingLink {
    /// Asks the configured model to select one of the prepared links. Blocks on the worker
    /// thread. `None` means the model abstained or there were no eligible links to send.
    ///
    /// # Errors
    /// The request stopped, the model or its credentials failed, or the reply was invalid.
    pub fn run(
        &self,
        llm: &impl LlmPort,
        secrets: &impl SecretStore,
        stop: &StopSignal,
    ) -> Result<Option<AgentLink>, SelectionError> {
        if stop.is_stopped() {
            return Err(SelectionError::Stopped);
        }
        if self.candidates.is_empty() {
            return Ok(None);
        }
        let key = key_of(&self.endpoint, secrets)?;
        let request = ChatRequest {
            endpoint: &self.endpoint,
            key: key.as_ref(),
            prompt: &self.prompt,
        };
        let mut reply = String::new();
        let mut overflowed = false;
        let ended = llm.chat(&request, stop, &mut |event| {
            if let ChatEvent::Delta(text) = event {
                if text.len() > MAX_REPLY_BYTES - reply.len() {
                    overflowed = true;
                } else {
                    reply.push_str(&text);
                }
            }
        });
        if stop.is_stopped() {
            return Err(SelectionError::Stopped);
        }
        if ended.map_err(AskError::from)? == Ended::Stopped {
            return Err(SelectionError::Stopped);
        }
        if overflowed {
            return Err(SelectionError::InvalidSelection);
        }
        let selection = reply.trim();
        let index = selection
            .parse::<usize>()
            .ok()
            .filter(|index| index.to_string() == selection)
            .ok_or(SelectionError::InvalidSelection)?;
        if index == 0 {
            return Ok(None);
        }
        self.candidates
            .get(index - 1)
            .cloned()
            .map(Some)
            .ok_or(SelectionError::InvalidSelection)
    }
}

fn ordinary_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
}

fn likely_video(url: &Url) -> bool {
    if url.path() == "/watch" {
        return url
            .query_pairs()
            .any(|(key, value)| key == "v" && !value.is_empty());
    }
    ["/shorts/", "/video/"].into_iter().any(|prefix| {
        url.path()
            .strip_prefix(prefix)
            .is_some_and(|id| !id.is_empty())
    })
}
