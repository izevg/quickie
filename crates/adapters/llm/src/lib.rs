//! The built-in agent's LLM client: the application's [`LlmPort`] for any OpenAI-compatible
//! endpoint (hosted APIs, Ollama, LM Studio), on top of its [`HttpPort`].
//!
//! It has no network stack of its own. Every request goes through the [`HttpPort`] it is given,
//! which in the product is Chromium's network service, so the egress test sees LLM traffic like
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
    ChatEvent, ChatRequest, Ended, HttpEvent, HttpMethod, HttpPort, HttpRequest, LlmError, LlmPort,
    StopSignal,
};
use quickie_domain_agents::{ApiKey, BaseUrl, LocalServer, ModelId};
use url::Url;

use sse::{Decoder, Event};
use wire::Chunk;

/// The most of a body kept whole: a model list, an error, or an answer that did not stream.
/// A stream's tokens go out as they come; only its start is kept, in case it was no stream.
const MAX_KEPT_BYTES: usize = 1 << 20;

/// An OpenAI-compatible client over an [`HttpPort`].
#[derive(Debug)]
pub struct OpenAiClient<H> {
    http: H,
}

impl<H: HttpPort> OpenAiClient<H> {
    /// A client that sends every request through `http`.
    #[must_use]
    pub fn new(http: H) -> Self {
        Self { http }
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
        self.http
            .stream(&request, &StopSignal::new(), &mut |event| match event {
                HttpEvent::Status(code) => status = Some(code),
                HttpEvent::Body(bytes) => keep(&mut body, bytes),
            })?;
        match status {
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
    /// True once `[DONE]` arrived; anything after it is ignored.
    done: bool,
    /// The first failure; anything after it is ignored.
    failed: Option<LlmError>,
}

impl Reply {
    fn take(&mut self, event: HttpEvent<'_>, on_event: &mut dyn FnMut(ChatEvent)) {
        match event {
            HttpEvent::Status(code) => self.status = Some(code),
            HttpEvent::Body(bytes) => {
                keep(&mut self.kept, bytes);
                if self.status.is_some_and(is_success) {
                    for event in self.decoder.push(bytes) {
                        self.event(&event, on_event);
                    }
                }
            }
        }
    }

    fn event(&mut self, event: &Event, on_event: &mut dyn FnMut(ChatEvent)) {
        self.streamed = true;
        if self.done || self.failed.is_some() {
            return;
        }
        if event.name.as_deref() == Some("error") {
            self.failed = Some(LlmError::Api(wire::error_message(event.data.as_bytes())));
            return;
        }
        match wire::chunk(&event.data) {
            Ok(Chunk::Done) => self.done = true,
            Ok(Chunk::Events(events)) => events.into_iter().for_each(&mut *on_event),
            Err(error) => self.failed = Some(error),
        }
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
        if ended == Ended::Stopped {
            return self.failed.map_or(Ok(Ended::Stopped), Err);
        }
        if let Some(event) = std::mem::take(&mut self.decoder).finish() {
            self.event(&event, on_event);
        }
        if let Some(error) = self.failed {
            return Err(error);
        }
        if !self.streamed {
            wire::whole(&self.kept)?.into_iter().for_each(on_event);
        }
        Ok(Ended::Complete)
    }
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Appends `bytes` to `kept` while it is under [`MAX_KEPT_BYTES`].
fn keep(kept: &mut Vec<u8>, bytes: &[u8]) {
    let room = MAX_KEPT_BYTES.saturating_sub(kept.len());
    kept.extend_from_slice(bytes.get(..room).unwrap_or(bytes));
}

fn no_status() -> LlmError {
    LlmError::Malformed("an answer without a status".to_owned())
}
