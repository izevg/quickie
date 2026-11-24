//! The shared Ask use case and stored endpoint, used only on background workers.

mod secrets;

use std::collections::hash_map::RandomState;
use std::fs::DirBuilder;
use std::hash::BuildHasher;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{SystemTime, UNIX_EPOCH};

use quickie_application::{
    Agent, AnswerEvent, AskError, Clock, Entropy, LlmError, StopSignal, ToolChatMessage,
    ToolChatReply, complete_workspace_step,
};
use quickie_domain_agents::{
    AgentSession, ApiKey, BaseUrl, Citation, FenceMarker, LlmEndpoint, ModelId, PageText, Request,
};
use quickie_domain_browsing::Timestamp;
use quickie_llm::OpenAiClient;
use quickie_store::SqliteStore;

use crate::updates::network::WebKitHttp;
use secrets::SecretService;

type Ask = Agent<SqliteStore, SecretService, LinuxEntropy>;

pub(super) enum Settings {
    Load,
    Save(BaseUrl, ModelId, Option<ApiKey>),
    RemoveKey,
}

pub(super) struct Saved {
    pub(super) endpoint: Option<LlmEndpoint>,
    pub(super) status: String,
}

pub(super) enum ProviderStep {
    Reply(ToolChatReply),
    NeedsStreaming,
    Failed(String),
}

pub(super) fn configure(intent: Settings) -> Result<Saved, String> {
    let mut ask = start()?;
    let status = match intent {
        Settings::Load if ask.corrupt_on_start().is_some() => {
            "The saved endpoint is invalid. Save a new endpoint to use the agent.".to_owned()
        }
        Settings::Load => String::new(),
        Settings::Save(base, model, key) => {
            let saved = ask
                .set_endpoint(base, model)
                .and_then(|()| key.as_ref().map_or(Ok(()), |key| ask.set_key(key)));
            saved.map_or_else(
                |error| error.to_string(),
                |()| "Agent settings saved".into(),
            )
        }
        Settings::RemoveKey => ask.remove_key().map_or_else(
            |error| error.to_string(),
            |()| "Removed the saved API key".into(),
        ),
    };
    Ok(Saved {
        endpoint: ask.endpoint().cloned(),
        status,
    })
}

pub(super) fn answer(
    request: &Request,
    page: PageText,
    endpoint: &LlmEndpoint,
    stop: &StopSignal,
    events: &Sender<Result<AnswerEvent, String>>,
) -> Result<(), String> {
    let ask = configured(endpoint)?;
    if stop.is_stopped() {
        return Ok(());
    }
    let asking = ask
        .ask(request, &[page])
        .map_err(|error| error.to_string())?;
    let client = OpenAiClient::new(WebKitHttp);
    asking.run(&client, &SecretService, &SystemClock, stop, &mut |event| {
        let _ = events.send(Ok(event));
    });
    Ok(())
}

pub(super) fn provider_step(
    client: &OpenAiClient<WebKitHttp>,
    endpoint: &LlmEndpoint,
    messages: &[ToolChatMessage],
    stop: &StopSignal,
) -> ProviderStep {
    if let Err(error) = configured(endpoint) {
        return ProviderStep::Failed(error);
    }
    match complete_workspace_step(endpoint, &SecretService, client, messages, false, stop) {
        Ok(reply) => ProviderStep::Reply(reply),
        Err(AskError::Llm(LlmError::ToolsUnavailable)) => ProviderStep::NeedsStreaming,
        Err(error) => ProviderStep::Failed(error.to_string()),
    }
}

pub(super) fn verified_answer(text: &str, excerpts: &[String]) -> (String, Vec<Citation>) {
    let now = SystemClock.now();
    let mut session = AgentSession::for_texts(excerpts, now);
    session.received(text, now);
    let finished = session.finish(now, None);
    (finished.answer, finished.citations)
}

fn configured(endpoint: &LlmEndpoint) -> Result<Ask, String> {
    let ask = start()?;
    if ask.endpoint() != Some(endpoint) {
        return Err(
            "The agent endpoint changed. Reopen the workspace to use its new settings.".into(),
        );
    }
    Ok(ask)
}

fn start() -> Result<Ask, String> {
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".local/share"))
        })
        .ok_or("Cannot locate the Quickie data directory")?;
    let directory = root.join("quickie");
    DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&directory)
        .map_err(|_| "Cannot create the Quickie data directory")?;
    let repository =
        SqliteStore::open(&directory.join("shell.sqlite")).map_err(|error| error.to_string())?;
    Agent::start(repository, SecretService, LinuxEntropy).map_err(|error| error.to_string())
}

struct LinuxEntropy;

impl Entropy for LinuxEntropy {
    fn fence_marker(&self) -> FenceMarker {
        let mut bytes = [0_u8; 6];
        // SAFETY: The six writable bytes remain valid for the duration of getrandom.
        if unsafe { getrandom(bytes.as_mut_ptr(), bytes.len(), 0) } != 6 {
            let fallback = RandomState::new().hash_one(0_u8).to_le_bytes();
            bytes.copy_from_slice(&fallback[..6]);
        }
        FenceMarker::from_random(bytes)
    }
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        Timestamp::from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
    }
}

unsafe extern "C" {
    fn getrandom(buffer: *mut u8, length: usize, flags: u32) -> isize;
}
