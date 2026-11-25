//! The JSON of the OpenAI-compatible API, and of Ollama's model list: what goes out and how
//! what comes back is read. Everything that comes back is read defensively: a field that is
//! missing or of another type is absent, never a panic.

mod responses;
#[cfg(test)]
mod tests;
mod tools;
mod transport;

pub(crate) use transport::ToolTransport;

use quickie_application::{ChatEvent, ChatRequest, LlmError};
use quickie_domain_agents::{ApiKey, ModelId};
use serde_json::{Value, json};

/// The longest stretch of an unreadable answer quoted in an error, in characters.
const QUOTED_CHARS: usize = 300;

/// The body of a streaming chat completion request. `include_usage` asks for the server's
/// token count at the end, for the speed readout; servers that do not know it ignore it.
pub(crate) fn chat_body(request: &ChatRequest<'_>) -> Vec<u8> {
    let messages: Vec<Value> = request
        .prompt
        .messages()
        .iter()
        .map(|message| json!({ "role": message.role.as_str(), "content": message.content }))
        .collect();
    json!({
        "model": request.endpoint.model().as_str(),
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    })
    .to_string()
    .into_bytes()
}

/// The headers of a request that accepts `accept`, with a JSON body when `json_body`, and the
/// key as a bearer token only when there is one.
pub(crate) fn headers(
    accept: &str,
    json_body: bool,
    key: Option<&ApiKey>,
) -> Vec<(String, String)> {
    let mut headers = vec![("Accept".to_owned(), accept.to_owned())];
    if json_body {
        headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
    }
    if let Some(key) = key {
        headers.push((
            "Authorization".to_owned(),
            format!("Bearer {}", key.reveal()),
        ));
    }
    headers
}

/// What one event of a chat stream says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Chunk {
    /// `[DONE]`: the answer is complete.
    Done,
    /// Answer text and token counts; often one token, sometimes nothing.
    Events(Vec<ChatEvent>),
}

/// Reads one event's data from a chat stream.
pub(crate) fn chunk(data: &str) -> Result<Chunk, LlmError> {
    if data.trim() == "[DONE]" {
        return Ok(Chunk::Done);
    }
    let value: Value = serde_json::from_str(data).map_err(|_| LlmError::Malformed(quoted(data)))?;
    if let Some(error) = value.get("error") {
        return Err(LlmError::Api(message_of(error)));
    }
    let mut events = Vec::new();
    let content = value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !content.is_empty() {
        events.push(ChatEvent::Delta(content.to_owned()));
    }
    if let Some(tokens) = value
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_u64)
    {
        events.push(ChatEvent::Usage {
            completion_tokens: tokens,
        });
    }
    Ok(Chunk::Events(events))
}

/// What a body that is not a stream says, when a server answered a streaming request with one
/// JSON object: an error, or the whole answer at once.
pub(crate) fn whole(body: &[u8]) -> Result<Vec<ChatEvent>, LlmError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_| LlmError::Malformed(quoted_bytes(body)))?;
    if let Some(error) = value.get("error") {
        return Err(LlmError::Api(message_of(error)));
    }
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| LlmError::Malformed(quoted_bytes(body)))?;
    Ok(vec![ChatEvent::Delta(content.to_owned())])
}

/// The message of an error body: `{"error": {"message": …}}` as hosted APIs send it,
/// `{"error": "…"}` as Ollama does, a bare `{"message": …}`, or else the start of the body as
/// text.
pub(crate) fn error_message(body: &[u8]) -> String {
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let message = parsed.as_ref().and_then(|value| {
        value.get("error").map(message_of).or_else(|| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    });
    message.unwrap_or_else(|| quoted_bytes(body))
}

/// The models of an OpenAI-compatible `/models` answer: `{"data": [{"id": …}, …]}`. An id
/// that is no model id is left out.
pub(crate) fn models(body: &[u8]) -> Result<Vec<ModelId>, LlmError> {
    listed(body, "/data", "id")
}

/// The models of Ollama's `/api/tags` answer: `{"models": [{"name": …}, …]}`.
pub(crate) fn ollama_tags(body: &[u8]) -> Result<Vec<ModelId>, LlmError> {
    listed(body, "/models", "name")
}

fn listed(body: &[u8], list: &str, field: &str) -> Result<Vec<ModelId>, LlmError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_| LlmError::Malformed(quoted_bytes(body)))?;
    let entries = value
        .pointer(list)
        .and_then(Value::as_array)
        .ok_or_else(|| LlmError::Malformed(quoted_bytes(body)))?;
    Ok(entries
        .iter()
        .filter_map(|entry| entry.get(field).and_then(Value::as_str))
        .filter_map(|id| ModelId::try_from(id).ok())
        .collect())
}

/// An `error` field's message: its `message` when it is an object, itself when a string.
fn message_of(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map_or_else(|| quoted(&error.to_string()), str::to_owned)
}

fn quoted_bytes(body: &[u8]) -> String {
    quoted(&String::from_utf8_lossy(body))
}

/// The start of a text, trimmed, to show in an error.
fn quoted(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return "an empty answer".to_owned();
    }
    text.chars().take(QUOTED_CHARS).collect()
}
