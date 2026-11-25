//! Model-visible state follows the configured endpoint's origin.

use quickie_application::{Conversation, HttpPort};
use quickie_domain_agents::LlmEndpoint;
use quickie_llm::OpenAiClient;
use url::Origin;

pub(in crate::browser::agent::workspace) struct ModelContext<H> {
    origin: Option<Origin>,
    pub(in crate::browser::agent::workspace) conversation: Conversation,
    pub(in crate::browser::agent::workspace) llm: OpenAiClient<H>,
}

impl<H: HttpPort + Clone> ModelContext<H> {
    pub(in crate::browser::agent::workspace) fn new(
        http: H,
        endpoint: Option<&LlmEndpoint>,
    ) -> Self {
        Self {
            origin: origin_of(endpoint),
            conversation: Conversation::default(),
            llm: OpenAiClient::new(http),
        }
    }

    /// Discard both wire replay and plain history before another origin can use them.
    pub(in crate::browser::agent::workspace) fn bind(
        &mut self,
        endpoint: Option<&LlmEndpoint>,
        http: &H,
    ) -> bool {
        let origin = origin_of(endpoint);
        if self.origin == origin {
            return false;
        }
        *self = Self::new(http.clone(), endpoint);
        true
    }
}

fn origin_of(endpoint: Option<&LlmEndpoint>) -> Option<Origin> {
    endpoint.map(|endpoint| endpoint.base().as_url().origin())
}

#[cfg(test)]
mod tests;
