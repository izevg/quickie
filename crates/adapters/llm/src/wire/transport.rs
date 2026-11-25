//! The HTTP exchange for the agent's bounded native tool calls.

use std::sync::{Arc, Mutex};

use quickie_application::{
    Ended, Flow, HttpEvent, HttpMethod, HttpPort, HttpRequest, LlmError, StopSignal,
    ToolChatMessage, ToolChatReply, ToolChatRequest,
};
use quickie_domain_agents::LlmEndpoint;
use serde_json::Value;

use super::{error_message, headers, responses, tools};
use crate::{MAX_KEPT_BYTES, is_success, keep, no_status};

const MAX_REQUEST_BYTES: usize = 8 << 20;
type Replay = Vec<(usize, Vec<Value>)>;

#[derive(Default)]
pub(crate) struct ToolTransport {
    workspace: Arc<Mutex<Workspace>>,
    turn: Mutex<Turn>,
}

#[derive(Default)]
struct Workspace {
    last_success: Option<(LlmEndpoint, Api)>,
    committed: Option<Arc<History>>,
}

#[derive(Default)]
struct Turn {
    endpoint: Option<LlmEndpoint>,
    pinned: Option<Api>,
    native: NativeHistory,
    pending: Option<History>,
}

#[derive(Clone, Default)]
struct NativeHistory {
    /// Count of old messages omitted after the system message. Item positions use full history.
    omitted: usize,
    items: Replay,
}

struct History {
    endpoint: LlmEndpoint,
    api: Api,
    messages: Vec<ToolChatMessage>,
    native: NativeHistory,
}

struct Completion {
    reply: ToolChatReply,
    items: Vec<Value>,
    omitted: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Api {
    ChatCompletions,
    Responses,
}

enum AttemptError {
    Unsupported(LlmError),
    Failed(LlmError),
}

impl AttemptError {
    fn into_error(self) -> LlmError {
        match self {
            Self::Unsupported(error) | Self::Failed(error) => error,
        }
    }
}

impl ToolTransport {
    pub(crate) fn for_turn(&self) -> Self {
        Self {
            workspace: Arc::clone(&self.workspace),
            turn: Mutex::default(),
        }
    }

    pub(crate) fn finish_turn(&self) {
        if let Ok(mut turn) = self.turn.lock()
            && let Some(history) = turn.pending.take()
            && let Ok(mut workspace) = self.workspace.lock()
        {
            workspace.committed = Some(Arc::new(history));
        }
    }

    pub(crate) fn complete(
        &self,
        http: &impl HttpPort,
        request: &ToolChatRequest<'_>,
        stop: &StopSignal,
    ) -> Result<ToolChatReply, LlmError> {
        // The shell calls this on a worker and waits for each tool before the next step.
        let mut turn = self
            .turn
            .lock()
            .map_err(|_| LlmError::Api("the agent transport is unavailable".to_owned()))?;
        if turn.endpoint.as_ref() != Some(request.endpoint) {
            *turn = Turn {
                endpoint: Some(request.endpoint.clone()),
                ..Turn::default()
            };
        }
        turn.pending = None;
        let (last_success, committed) = self.context(request.endpoint);
        let mut api = turn
            .pinned
            .or(last_success)
            .unwrap_or_else(|| Api::preferred(request.endpoint));
        let mut native = if turn.pinned.is_some() {
            turn.native.clone()
        } else {
            committed
                .filter(|history| history.api == api)
                .and_then(|history| history.replay_for(request.messages))
                .unwrap_or_default()
        };
        let Completion {
            reply,
            items,
            omitted,
        } = match api.complete(http, request, &native, stop) {
            Ok(reply) => reply,
            Err(AttemptError::Unsupported(_)) if turn.pinned.is_none() => {
                api = api.other();
                native = NativeHistory::default();
                api.complete(http, request, &native, stop)
                    .map_err(|error| match error {
                        AttemptError::Unsupported(_) => LlmError::ToolsUnavailable,
                        AttemptError::Failed(error) => error,
                    })?
            }
            Err(error) => return Err(error.into_error()),
        };
        if stop.is_stopped() {
            return Err(stopped());
        }
        if omitted > native.omitted {
            native.items.retain(|(index, _)| *index > omitted);
            native.omitted = omitted;
        }
        if !items.is_empty() {
            native.items.push((request.messages.len(), items));
        }
        match &reply {
            ToolChatReply::Call(_) => {
                turn.pinned = Some(api);
                turn.native = native;
            }
            ToolChatReply::Answer(text) => {
                let mut messages = request.messages.to_vec();
                messages.push(ToolChatMessage::Assistant(text.clone()));
                turn.native = NativeHistory::default();
                turn.pending = Some(History {
                    endpoint: request.endpoint.clone(),
                    api,
                    messages,
                    native,
                });
            }
        }
        if let Ok(mut workspace) = self.workspace.lock() {
            workspace.last_success = Some((request.endpoint.clone(), api));
        }
        Ok(reply)
    }

    fn context(&self, endpoint: &LlmEndpoint) -> (Option<Api>, Option<Arc<History>>) {
        let Ok(workspace) = self.workspace.lock() else {
            return (None, None);
        };
        let last_success = workspace
            .last_success
            .as_ref()
            .and_then(|(previous, api)| (previous == endpoint).then_some(*api));
        let committed = workspace
            .committed
            .as_ref()
            .filter(|history| history.endpoint == *endpoint)
            .cloned();
        (last_success, committed)
    }
}

impl History {
    /// The application may remove whole old turns after the system message.
    fn replay_for(&self, messages: &[ToolChatMessage]) -> Option<NativeHistory> {
        let (ToolChatMessage::User(_), retained) = messages.split_last()? else {
            return None;
        };
        let (system, retained) = retained.split_first()?;
        let (previous_system, previous) = self.messages.split_first()?;
        let removed = previous.len().checked_sub(retained.len())?;
        if system != previous_system || previous.get(removed..)? != retained {
            return None;
        }
        Some(NativeHistory {
            omitted: self.native.omitted.saturating_sub(removed),
            items: self
                .native
                .items
                .iter()
                .filter(|(index, _)| *index > removed)
                .map(|(index, items)| (index - removed, items.clone()))
                .collect(),
        })
    }
}

impl Api {
    fn preferred(endpoint: &LlmEndpoint) -> Self {
        if endpoint.base().as_url().host_str() == Some("api.openai.com") {
            Self::Responses
        } else {
            Self::ChatCompletions
        }
    }

    fn other(self) -> Self {
        match self {
            Self::ChatCompletions => Self::Responses,
            Self::Responses => Self::ChatCompletions,
        }
    }

    fn route(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/chat/completions",
            Self::Responses => "/responses",
        }
    }

    fn complete(
        self,
        http: &impl HttpPort,
        request: &ToolChatRequest<'_>,
        native: &NativeHistory,
        stop: &StopSignal,
    ) -> Result<Completion, AttemptError> {
        let (body, omitted) = self
            .fit_body(request, native, stop)
            .map_err(AttemptError::Failed)?;
        let base = request.endpoint.base();
        let url = match self {
            Self::ChatCompletions => base.chat_completions(),
            Self::Responses => base.responses(),
        };
        let http_request = HttpRequest {
            method: HttpMethod::Post(body),
            url,
            headers: headers("application/json", true, request.key),
        };
        let (status, body) = exchange(http, &http_request, stop).map_err(AttemptError::Failed)?;
        if !is_success(status) {
            let message = error_message(&body);
            let unsupported = protocol_unavailable(
                self,
                status,
                &body,
                &message,
                request.endpoint.model().as_str(),
            );
            let error = LlmError::Status { status, message };
            return if unsupported {
                Err(AttemptError::Unsupported(error))
            } else {
                Err(AttemptError::Failed(error))
            };
        }
        match self {
            Self::ChatCompletions => tools::tool_reply(&body, request.automation),
            Self::Responses => responses::response_reply(&body, request.automation),
        }
        .map(|(reply, items)| Completion {
            reply,
            items,
            omitted,
        })
        .map_err(AttemptError::Failed)
    }

    fn encode(self, request: &ToolChatRequest<'_>, replay: &[(usize, Vec<Value>)]) -> Vec<u8> {
        match self {
            Self::ChatCompletions => tools::tool_body(request, replay),
            Self::Responses => responses::response_body(request, replay),
        }
    }

    fn body_after(
        self,
        request: &ToolChatRequest<'_>,
        native: &NativeHistory,
        omitted: usize,
    ) -> Option<Vec<u8>> {
        if omitted == 0 {
            return Some(self.encode(request, &native.items));
        }
        let (system @ ToolChatMessage::System(_), past) = request.messages.split_first()? else {
            return None;
        };
        let mut messages = vec![system.clone()];
        messages.extend_from_slice(past.get(omitted..)?);
        let replay: Replay = native
            .items
            .iter()
            .filter(|(index, _)| *index > omitted)
            .map(|(index, items)| (index - omitted, items.clone()))
            .collect();
        Some(self.encode(
            &ToolChatRequest {
                messages: &messages,
                ..*request
            },
            &replay,
        ))
    }

    fn fit_body(
        self,
        request: &ToolChatRequest<'_>,
        native: &NativeHistory,
        stop: &StopSignal,
    ) -> Result<(Vec<u8>, usize), LlmError> {
        if stop.is_stopped() {
            return Err(stopped());
        }
        if let Some(body) = self.body_after(request, native, native.omitted)
            && body.len() <= MAX_REQUEST_BYTES
        {
            return Ok((body, native.omitted));
        }
        // Each cut leaves the system message and starts at a user prompt. Completed tool
        // calls, their results, and opaque replay therefore leave together. The last prompt
        // marks the active turn, which stays whole even if that means rejecting the request.
        let cuts: Vec<_> = request
            .messages
            .iter()
            .enumerate()
            .skip(native.omitted.saturating_add(2))
            .filter_map(|(index, message)| {
                matches!(message, ToolChatMessage::User(_)).then_some(index - 1)
            })
            .collect();
        // Removing complete turns only makes the encoding smaller. Find the earliest cut
        // without encoding the full conversation once for every retained turn.
        let (mut low, mut high) = (0, cuts.len());
        let mut fitted = None;
        while low < high {
            if stop.is_stopped() {
                return Err(stopped());
            }
            let middle = usize::midpoint(low, high);
            let omitted = cuts[middle];
            if let Some(body) = self.body_after(request, native, omitted)
                && body.len() <= MAX_REQUEST_BYTES
            {
                fitted = Some((body, omitted));
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        fitted.ok_or_else(|| {
            LlmError::TooLarge(format!("a tool request over {MAX_REQUEST_BYTES} bytes"))
        })
    }
}

/// Only a definite endpoint or capability rejection lets the browser resend the request.
fn protocol_unavailable(api: Api, status: u16, body: &[u8], message: &str, model: &str) -> bool {
    if matches!(status, 405 | 501) {
        return true;
    }
    if !matches!(status, 400 | 404 | 422) {
        return false;
    }
    let message = message.to_ascii_lowercase();
    if message.contains(api.route())
        && message.contains(api.other().route())
        && (message.contains("not supported") || message.contains("unsupported"))
        && (message.contains("please use") || message.contains("instead"))
    {
        return true;
    }
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let error = parsed
        .as_ref()
        .map(|value| value.get("error").unwrap_or(value));
    let code = error
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str);
    let parameter = error
        .and_then(|value| value.get("param"))
        .and_then(Value::as_str);
    if status == 404 {
        return parameter != Some("model")
            && !message.contains("model")
            && !message.contains("deployment")
            && code.is_none_or(|code| {
                matches!(code, "not_found" | "route_not_found" | "endpoint_not_found")
            });
    }
    matches!(
        code,
        Some("unsupported_api" | "unsupported_endpoint" | "tools_not_supported")
    ) || (matches!(code, Some("unsupported_parameter" | "unknown_parameter"))
        && matches!(
            parameter,
            Some("tools" | "tool_choice" | "parallel_tool_calls" | "store" | "include")
        ))
        || (api == Api::ChatCompletions
            && status == 400
            && parsed
                .as_ref()
                .and_then(|value| value.get("error"))
                .is_some_and(|error| rejects_ollama_tools(error, model)))
}

fn rejects_ollama_tools(error: &Value, model: &str) -> bool {
    let kind = error.get("type").and_then(Value::as_str);
    if !matches!(kind, Some("api_error" | "invalid_request_error"))
        || error.get("code") != Some(&Value::Null)
        || error.get("param") != Some(&Value::Null)
    {
        return false;
    }
    let Some(reported) = error
        .get("message")
        .and_then(Value::as_str)
        .and_then(|message| message.strip_suffix(" does not support tools"))
    else {
        return false;
    };

    // Ollama 0.12.11 reports the installed model's fully qualified name.
    let mut parts = model.rsplitn(3, '/');
    let name = parts.next().unwrap_or_default();
    let namespace = parts.next().unwrap_or("library");
    let registry = parts.next().unwrap_or("registry.ollama.ai");
    let registry = registry
        .split_once("://")
        .map_or(registry, |(_, host)| host);
    let (name, tag) = name.rsplit_once(':').unwrap_or((name, "latest"));
    let qualified = format!("{registry}/{namespace}/{name}:{tag}");
    reported.eq_ignore_ascii_case(&qualified)
        || (kind == Some("invalid_request_error")
            && (reported == model || reported == format!("'{model}'")))
}

fn exchange(
    http: &impl HttpPort,
    request: &HttpRequest,
    stop: &StopSignal,
) -> Result<(u16, Vec<u8>), LlmError> {
    if stop.is_stopped() {
        return Err(stopped());
    }
    let mut status = None;
    let mut body = Vec::new();
    let mut whole = true;
    let ended = http.stream(request, stop, &mut |event| {
        if stop.is_stopped() {
            return Flow::Stop;
        }
        match event {
            HttpEvent::Status(code) => status = Some(code),
            HttpEvent::Body(bytes) => whole = keep(&mut body, bytes),
        }
        if whole { Flow::Continue } else { Flow::Stop }
    })?;
    if stop.is_stopped() {
        return Err(stopped());
    }
    let status = status.ok_or_else(no_status)?;
    if !whole {
        return Err(LlmError::TooLarge(format!(
            "a tool completion over {MAX_KEPT_BYTES} bytes"
        )));
    }
    if ended != Ended::Complete {
        return Err(stopped());
    }
    Ok((status, body))
}

fn stopped() -> LlmError {
    LlmError::Api("the tool completion was stopped".to_owned())
}
