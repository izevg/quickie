//! Answering: one request streamed from the endpoint into [`AnswerEvent`]s, on a worker thread.

use quickie_domain_agents::{
    AgentSession, AnswerPart, ApiKey, Citation, KeyRef, LlmEndpoint, PageText, Prompt, Speed,
};

use crate::{
    ChatEvent, ChatRequest, Clock, Ended, LlmError, LlmPort, SecretError, SecretStore, StopSignal,
};

/// What the answer panel receives, in order: tokens and citations as they come, then exactly
/// one of [`Self::Done`], [`Self::Stopped`], or [`Self::Error`].
#[derive(Clone, Debug, PartialEq)]
pub enum AnswerEvent {
    /// The next piece of answer text.
    Token(String),
    /// A quote the page contains, in its place in the answer.
    Citation(Citation),
    /// The answer is complete; how fast it came.
    Done(Speed),
    /// The user stopped the answer; what came so far stays.
    Stopped,
    /// The answer failed; what came so far stays.
    Error(AskError),
}

impl From<AnswerPart> for AnswerEvent {
    fn from(part: AnswerPart) -> Self {
        match part {
            AnswerPart::Text(text) => Self::Token(text),
            AnswerPart::Citation(citation) => Self::Citation(citation),
        }
    }
}

/// Why an answer failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AskError {
    /// The endpoint has a key, but the Keychain no longer holds it; set it again.
    #[error("the Keychain has no key for {0}; set it again in settings")]
    KeyMissing(KeyRef),
    /// The Keychain could not be read.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// The endpoint failed.
    #[error(transparent)]
    Llm(#[from] LlmError),
}

/// A request ready to be answered: the endpoint, the prompt, and the pages the answer may cite.
/// It owns all of it, so the shell can move it to a worker thread.
#[derive(Clone, Debug)]
pub struct Asking {
    endpoint: LlmEndpoint,
    prompt: Prompt,
    pages: Vec<PageText>,
}

impl Asking {
    pub(super) fn new(endpoint: LlmEndpoint, prompt: Prompt, pages: Vec<PageText>) -> Self {
        Self {
            endpoint,
            prompt,
            pages,
        }
    }

    /// The endpoint asked.
    #[must_use]
    pub fn endpoint(&self) -> &LlmEndpoint {
        &self.endpoint
    }

    /// What the endpoint is sent.
    #[must_use]
    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    /// Asks the endpoint and hands every [`AnswerEvent`] to `on_event` as it comes, on this
    /// thread, ending with exactly one terminal event. Blocks until the answer ends or `stop`
    /// is signalled: run it on a worker thread.
    pub fn run(
        self,
        llm: &impl LlmPort,
        secrets: &impl SecretStore,
        clock: &impl Clock,
        stop: &StopSignal,
        on_event: &mut dyn FnMut(AnswerEvent),
    ) {
        let key = match key_of(&self.endpoint, secrets) {
            Ok(key) => key,
            Err(error) => return on_event(AnswerEvent::Error(error)),
        };
        let request = ChatRequest {
            endpoint: &self.endpoint,
            key: key.as_ref(),
            prompt: &self.prompt,
        };
        let mut session = AgentSession::new(&self.pages, clock.now());
        let mut reported = None;
        let ended = llm.chat(&request, stop, &mut |event| match event {
            ChatEvent::Delta(token) => {
                for part in session.received(&token, clock.now()) {
                    on_event(part.into());
                }
            }
            ChatEvent::Usage { completion_tokens } => reported = Some(completion_tokens),
        });
        let finished = session.finish(clock.now(), reported);
        for part in finished.parts {
            on_event(part.into());
        }
        on_event(match ended {
            Ok(Ended::Complete) => AnswerEvent::Done(finished.speed),
            Ok(Ended::Stopped) => AnswerEvent::Stopped,
            Err(error) => AnswerEvent::Error(error.into()),
        });
    }
}

/// The endpoint's key from the Keychain, `None` for an endpoint without one.
pub(super) fn key_of(
    endpoint: &LlmEndpoint,
    secrets: &impl SecretStore,
) -> Result<Option<ApiKey>, AskError> {
    let Some(reference) = endpoint.key() else {
        return Ok(None);
    };
    match secrets.read(reference)? {
        Some(key) => Ok(Some(key)),
        None => Err(AskError::KeyMissing(reference.clone())),
    }
}
