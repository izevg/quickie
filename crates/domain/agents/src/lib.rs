//! Agents context: the built-in agent in Ask mode, which reads a page and answers about it.
//!
//! - [`LlmEndpoint`]: where the agent asks, any OpenAI-compatible API. Its [`BaseUrl`] is https,
//!   or plain http only on loopback (Ollama, LM Studio), so a request never crosses the network
//!   in the clear. The API key itself never sits here or in the store: the endpoint keeps a
//!   [`KeyRef`], bound to the endpoint's origin, that names the key in the Keychain.
//! - [`Skill`]: a named recipe, shaped like A2A's `AgentSkill`; [`Skill::builtin`] is the set
//!   Quickie ships.
//! - [`Prompt`]: what the model is sent. Page content is untrusted data: it travels fenced, in
//!   the user's message, never in the system prompt, and the fence cannot be forged from inside.
//! - [`AgentSession`]: one answer as it streams in, with the [`Citation`]s it quotes from the
//!   page and the [`Speed`] it came at.
//!
//! Ask mode only reads, and only on the user's explicit request; acting (Act mode, approvals,
//! provenance) arrives later. Pure: no I/O, no clock (the caller passes [`Timestamp`]s), no
//! async.

mod endpoint;
mod prompt;
mod session;

pub use endpoint::{
    ApiKey, ApiKeyError, BaseUrl, BaseUrlError, CorruptEndpoint, EndpointSnapshot, KeyRef,
    LlmEndpoint, LocalServer, ModelId, ModelIdError,
};
pub use prompt::{
    Message, OutputForm, PageScope, PageText, Prompt, PromptError, Question, QuestionError,
    Request, Role, Skill, SkillId, SkillIdError, TokenBudget,
};
pub use quickie_domain_kernel::Timestamp;
pub use session::{AgentSession, AnswerPart, Citation, Finished, Mode, Speed};
