//! The development engine's network for the browser's own requests: the application's
//! [`HttpPort`] on `NSURLSession`, the system stack `WebKit` itself loads pages with. The
//! Chromium engine replaces it with Chromium's network service, which the egress test watches.
//!
//! Every request, whole or streamed, runs as a data task of an ephemeral session of its own
//! whose delegate forwards what arrives ([`stream`]). The delegate refuses redirects: a 3xx is
//! the answer, so a request never goes on to a host nobody validated.

mod stream;
#[cfg(test)]
mod tests;

use std::time::Duration;

use objc2::MainThreadMarker;
use quickie_application::{
    Ended, Flow, HttpError, HttpEvent, HttpMethod, HttpPort, HttpRequest, StopSignal,
};
use url::Url;

use stream::Timeouts;

/// Whole requests, such as filter lists of several megabytes: how long one may wait for its
/// next bytes, and how long it may take.
const WHOLE: Timeouts = Timeouts {
    idle: Duration::from_secs(30),
    total: Duration::from_secs(120),
};

/// Streamed requests. Long, as a local LLM server may first load a model of several
/// gigabytes before its first token.
const STREAMED: Timeouts = Timeouts {
    idle: Duration::from_secs(300),
    total: Duration::from_mins(30),
};

/// Requests over ephemeral `NSURLSession`s: no cookies, no cache, no credentials kept, so a
/// request carries nothing about the user and leaves nothing behind.
#[derive(Clone, Debug, Default)]
pub struct UrlSessionHttp;

impl UrlSessionHttp {
    /// The network for the browser's own requests.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl HttpPort for UrlSessionHttp {
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError> {
        off_main_thread()?;
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: url.clone(),
            headers: Vec::new(),
        };
        let mut status = None;
        let mut body = Vec::new();
        stream::stream(&request, WHOLE, &StopSignal::new(), &mut |event| {
            match event {
                HttpEvent::Status(code) => status = Some(code),
                HttpEvent::Body(bytes) => body.extend_from_slice(bytes),
            }
            Flow::Continue
        })?;
        match status {
            Some(code) if (200..300).contains(&code) => Ok(body),
            Some(code) => Err(HttpError(format!("status {code}"))),
            None => Err(HttpError(format!("no answer from {url}"))),
        }
    }

    fn stream(
        &self,
        request: &HttpRequest,
        stop: &StopSignal,
        on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    ) -> Result<Ended, HttpError> {
        off_main_thread()?;
        stream::stream(request, STREAMED, stop, on_event)
    }
}

/// Requests block until they are answered, so they never run on the main thread.
fn off_main_thread() -> Result<(), HttpError> {
    if MainThreadMarker::new().is_some() {
        return Err(HttpError(
            "a request never runs on the main thread".to_owned(),
        ));
    }
    Ok(())
}
