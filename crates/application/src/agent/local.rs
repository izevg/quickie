//! Finding models: the local servers on this machine, and the models an endpoint offers. Both
//! block on the network, so the shell runs them on a worker thread, and only when the user asks.

use quickie_domain_agents::{BaseUrl, LlmEndpoint, LocalServer, ModelId};

use super::AskError;
use super::answer::key_of;
use crate::{LlmPort, SecretStore};

/// A local server that answered, with what it can run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalFound {
    /// Which server.
    pub server: LocalServer,
    /// Its OpenAI-compatible API, ready to be the endpoint.
    pub base: BaseUrl,
    /// Its models, as it named them.
    pub models: Vec<ModelId>,
}

/// Probes each [`LocalServer`] on loopback and returns those that answered. This is the only
/// place Quickie looks for them, and it runs only when the user clicks "Detect local LLM":
/// a probe is a connection like any other, so nothing probes on its own.
pub fn detect_local(llm: &impl LlmPort) -> Vec<LocalFound> {
    LocalServer::ALL
        .into_iter()
        .filter_map(|server| {
            let models = llm.probe(server).ok()?;
            let base = BaseUrl::try_from(server.base_url()).ok()?;
            Some(LocalFound {
                server,
                base,
                models,
            })
        })
        .collect()
}

/// The models `endpoint` offers, asked with its key from the Keychain, for picking another one
/// in settings.
///
/// # Errors
///
/// The key is missing or unreadable, or the endpoint failed.
pub fn list_models(
    llm: &impl LlmPort,
    secrets: &impl SecretStore,
    endpoint: &LlmEndpoint,
) -> Result<Vec<ModelId>, AskError> {
    let key = key_of(endpoint, secrets)?;
    Ok(llm.models(endpoint.base(), key.as_ref())?)
}
