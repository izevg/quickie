//! The built-in agent's LLM client: the application's [`LlmPort`] for any OpenAI-compatible
//! endpoint (hosted APIs, Ollama, LM Studio), on top of its [`HttpPort`].
//!
//! It has no network stack of its own. Every request goes through the [`HttpPort`] it is given,
//! which in the product is `WebKit`'s network service, so the egress test sees LLM traffic like
//! any other (`docs/architecture.md` §1).
//!
//! A chat completion streams as server-sent events: the body arrives in pieces cut anywhere,
//! the [`sse`] decoder turns them into events, and [`wire`] reads each event's JSON. Tokens go
//! to the caller as they are decoded, on the calling thread.

mod sse;
#[cfg(test)]
mod tests;
mod wire;

use quickie_application::{
    ChatEvent, ChatRequest, Ended, Flow, HttpEvent, HttpMethod, HttpPort, HttpRequest, LlmError,
    LlmPort, StopSignal, ToolChatReply, ToolChatRequest,
};
use quickie_domain_agents::{ApiKey, BaseUrl, LocalServer, ModelId};
use url::Url;

use sse::{Decoder, Event, Overflow};
use wire::{Chunk, ToolTransport};

/// The most of a body kept whole: a model list, an error, or an answer that did not stream.
/// A stream's tokens go out as they come; only its start is kept, in case it was no stream.
const MAX_KEPT_BYTES: usize = 1 << 20;

/// The most answer text read, in bytes: a long answer is a few kilobytes, and an endpoint that
/// streams on and on ends here instead of filling the browser's memory.
const MAX_ANSWER_BYTES: usize = 256 << 10;

/// An OpenAI-compatible client over an [`HttpPort`].
pub struct OpenAiClient<H> {
    http: H,
    tools: ToolTransport,
}

impl<H> std::fmt::Debug for OpenAiClient<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiClient").finish_non_exhaustive()
    }
}

impl<H: HttpPort> OpenAiClient<H> {
    /// A client that sends every request through `http`.
    #[must_use]
    pub fn new(http: H) -> Self {
        Self {
            http,
            tools: ToolTransport::default(),
        }
    }

    /// Shares endpoint detection with the workspace, with fresh continuation state for this turn.
    #[must_use]
    pub fn for_turn(&self) -> Self
    where
        H: Clone,
    {
        Self {
            http: self.http.clone(),
            tools: self.tools.for_turn(),
        }
    }

    /// Keeps the final model output after the workspace has accepted the answer.
    pub fn finish_turn(&self) {
        self.tools.finish_turn();
    }

    /// GETs `url` whole, for a model list.
    fn fetch(&self, url: Url, key: Option<&ApiKey>) -> Result<Vec<u8>, LlmError> {
        let request = HttpRequest {
            method: HttpMethod::Get,
            url,
            headers: wire::headers("application/json", false, key),
        };
        let mut status = None;
        let mut body = Vec::new();
        let mut whole = true;
        self.http
            .stream(&request, &StopSignal::new(), &mut |event| match event {
                HttpEvent::Status(code) => {
                    status = Some(code);
                    Flow::Continue
                }
                HttpEvent::Body(bytes) => {
                    whole = keep(&mut body, bytes);
                    if whole { Flow::Continue } else { Flow::Stop }
                }
            })?;
        match status {
            Some(code) if is_success(code) && !whole => Err(LlmError::TooLarge(format!(
                "a list of models over {MAX_KEPT_BYTES} bytes"
            ))),
            Some(code) if is_success(code) => Ok(body),
            Some(code) => Err(LlmError::Status {
                status: code,
                message: wire::error_message(&body),
            }),
            None => Err(no_status()),
        }
    }
}

impl<H: HttpPort> LlmPort for OpenAiClient<H> {
    fn complete_with_tools(
        &self,
        request: &ToolChatRequest<'_>,
        stop: &StopSignal,
    ) -> Result<ToolChatReply, LlmError> {
        self.tools.complete(&self.http, request, stop)
    }

    fn chat(
        &self,
        request: &ChatRequest<'_>,
        stop: &StopSignal,
        on_event: &mut dyn FnMut(ChatEvent),
    ) -> Result<Ended, LlmError> {
        let http = HttpRequest {
            method: HttpMethod::Post(wire::chat_body(request)),
            url: request.endpoint.base().chat_completions(),
            headers: wire::headers("text/event-stream", true, request.key),
        };
        let mut reply = Reply::default();
        let ended = self
            .http
            .stream(&http, stop, &mut |event| reply.take(event, on_event))?;
        reply.end(ended, on_event)
    }

    fn models(&self, base: &BaseUrl, key: Option<&ApiKey>) -> Result<Vec<ModelId>, LlmError> {
        wire::models(&self.fetch(base.models(), key)?)
    }

    fn probe(&self, server: LocalServer) -> Result<Vec<ModelId>, LlmError> {
        let url = Url::parse(server.probe_url())
            .map_err(|error| LlmError::Malformed(error.to_string()))?;
        let body = self.fetch(url, None)?;
        match server {
            LocalServer::Ollama => wire::ollama_tags(&body),
            LocalServer::LmStudio => wire::models(&body),
        }
    }
}

/// A chat answer as it arrives.
#[derive(Debug, Default)]
struct Reply {
    status: Option<u16>,
    decoder: Decoder,
    /// The start of the body, for an error or an answer that did not stream.
    kept: Vec<u8>,
    /// True once an event arrived: the answer is a stream.
    streamed: bool,
    /// The bytes of answer text handed on so far.
    answered: usize,
    /// True once `[DONE]` arrived; the stream is ended there.
    done: bool,
    /// The first failure; the stream is ended there.
    failed: Option<LlmError>,
}

impl Reply {
    /// Reads the next part of the answer; stops the stream once the answer is done, failed,
    /// or, for a refusal, once enough of its message is in.
    fn take(&mut self, event: HttpEvent<'_>, on_event: &mut dyn FnMut(ChatEvent)) -> Flow {
        match event {
            HttpEvent::Status(code) => self.status = Some(code),
            HttpEvent::Body(bytes) => {
                let whole = keep(&mut self.kept, bytes);
                if !self.status.is_some_and(is_success) {
                    return if whole { Flow::Continue } else { Flow::Stop };
                }
                match self.decoder.push(bytes) {
                    Ok(events) => {
                        for event in events {
                            self.event(&event, on_event);
                        }
                    }
                    Err(Overflow) => self.fail(overflowed()),
                }
            }
        }
        if self.done || self.failed.is_some() {
            Flow::Stop
        } else {
            Flow::Continue
        }
    }

    fn event(&mut self, event: &Event, on_event: &mut dyn FnMut(ChatEvent)) {
        self.streamed = true;
        if self.done || self.failed.is_some() {
            return;
        }
        if event.name.as_deref() == Some("error") {
            self.fail(LlmError::Api(wire::error_message(event.data.as_bytes())));
            return;
        }
        match wire::chunk(&event.data) {
            Ok(Chunk::Done) => self.done = true,
            Ok(Chunk::Events(events)) => {
                for event in events {
                    self.hand_on(event, on_event);
                }
            }
            Err(error) => self.fail(error),
        }
    }

    /// Hands `event` on, unless the answer outgrew [`MAX_ANSWER_BYTES`].
    fn hand_on(&mut self, event: ChatEvent, on_event: &mut dyn FnMut(ChatEvent)) {
        if let ChatEvent::Delta(text) = &event {
            self.answered += text.len();
            if self.answered > MAX_ANSWER_BYTES {
                self.fail(LlmError::TooLarge(format!(
                    "an answer over {MAX_ANSWER_BYTES} bytes"
                )));
                return;
            }
        }
        on_event(event);
    }

    fn fail(&mut self, error: LlmError) {
        self.failed.get_or_insert(error);
    }

    /// The stream ended as `ended`; what it came to.
    fn end(mut self, ended: Ended, on_event: &mut dyn FnMut(ChatEvent)) -> Result<Ended, LlmError> {
        let status = self.status.ok_or_else(no_status)?;
        if !is_success(status) {
            return Err(LlmError::Status {
                status,
                message: wire::error_message(&self.kept),
            });
        }
        if let Some(error) = self.failed {
            return Err(error);
        }
        if self.done {
            return Ok(Ended::Complete);
        }
        if ended == Ended::Stopped {
            return Ok(Ended::Stopped);
        }
        match std::mem::take(&mut self.decoder).finish() {
            Ok(Some(event)) => self.event(&event, on_event),
            Ok(None) => {}
            Err(Overflow) => self.fail(overflowed()),
        }
        if let Some(error) = self.failed {
            return Err(error);
        }
        if !self.streamed {
            wire::whole(&self.kept)?
                .into_iter()
                .for_each(|event| self.hand_on(event, on_event));
            if let Some(error) = self.failed {
                return Err(error);
            }
        }
        Ok(Ended::Complete)
    }
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Appends `bytes` to `kept` up to [`MAX_KEPT_BYTES`]; false when some did not fit.
fn keep(kept: &mut Vec<u8>, bytes: &[u8]) -> bool {
    let room = MAX_KEPT_BYTES.saturating_sub(kept.len());
    kept.extend_from_slice(bytes.get(..room).unwrap_or(bytes));
    bytes.len() <= room
}

fn overflowed() -> LlmError {
    LlmError::TooLarge(format!(
        "a line or event over {} bytes",
        sse::MAX_LINE_BYTES.max(sse::MAX_DATA_BYTES)
    ))
}

fn no_status() -> LlmError {
    LlmError::Malformed("an answer without a status".to_owned())
}
