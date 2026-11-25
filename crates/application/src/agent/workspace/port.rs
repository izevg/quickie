//! The endpoint contract for the built-in agent's bounded browser tools.

use quickie_domain_agents::{ApiKey, LlmEndpoint};

/// A page the built-in agent may inspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentPage {
    /// The source page attached to the turn.
    Source,
    /// The page in the isolated agent workspace.
    Workspace,
}

/// A browser operation understood by the built-in agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentTool {
    /// Read the visible text of a page.
    ReadPage(AgentPage),
    /// List links with browser-issued numeric identifiers.
    ListLinks(AgentPage),
    /// Open a previously listed link in the agent workspace.
    OpenLink(u32),
}

/// A validated tool request and its endpoint-issued correlation identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentToolCall {
    /// The identifier echoed by the corresponding tool result.
    pub id: String,
    /// The bounded browser operation requested.
    pub tool: AgentTool,
}

/// One message in the ordered tool completion history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolChatMessage {
    /// Instructions issued by the application.
    System(String),
    /// The user's request.
    User(String),
    /// A previous textual answer.
    Assistant(String),
    /// A previous assistant tool request.
    Call(AgentToolCall),
    /// The result of one previous tool request.
    Result {
        /// The corresponding tool call identifier.
        id: String,
        /// The tool's text output, treated as untrusted data.
        content: String,
    },
}

/// One nonstreaming completion with the built-in agent's tools.
#[derive(Clone, Copy, Debug)]
pub struct ToolChatRequest<'a> {
    /// Where to ask, and which model.
    pub endpoint: &'a LlmEndpoint,
    /// The endpoint's key, when configured.
    pub key: Option<&'a ApiKey>,
    /// The complete ordered message history.
    pub messages: &'a [ToolChatMessage],
    /// Whether workspace inspection and link navigation are available.
    pub automation: bool,
}

/// A completed answer or exactly one validated tool request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolChatReply {
    /// The assistant's final text.
    Answer(String),
    /// One bounded browser operation to handle before asking again.
    Call(AgentToolCall),
}

#[cfg(test)]
mod tests {
    use quickie_domain_agents::{BaseUrl, LocalServer, ModelId};

    use super::*;
    use crate::{ChatEvent, ChatRequest, Ended, LlmError, LlmPort, StopSignal};

    struct LegacyLlm;

    impl LlmPort for LegacyLlm {
        fn chat(
            &self,
            _: &ChatRequest<'_>,
            _: &StopSignal,
            _: &mut dyn FnMut(ChatEvent),
        ) -> Result<Ended, LlmError> {
            panic!("a tool request must not fall back to chat")
        }

        fn models(&self, _: &BaseUrl, _: Option<&ApiKey>) -> Result<Vec<ModelId>, LlmError> {
            panic!("a tool request must not list models")
        }

        fn probe(&self, _: LocalServer) -> Result<Vec<ModelId>, LlmError> {
            panic!("a tool request must not probe endpoints")
        }
    }

    #[test]
    fn a_legacy_adapter_refuses_tools_without_using_its_chat_path() {
        // Arrange
        let endpoint = LlmEndpoint::new(
            BaseUrl::try_from("https://api.example.com/v1").unwrap(),
            ModelId::try_from("small").unwrap(),
        );
        // Act
        let reply = LegacyLlm.complete_with_tools(
            &ToolChatRequest {
                endpoint: &endpoint,
                key: None,
                messages: &[],
                automation: true,
            },
            &StopSignal::new(),
        );
        // Assert
        assert!(matches!(reply, Err(LlmError::Api(_))));
    }
}
