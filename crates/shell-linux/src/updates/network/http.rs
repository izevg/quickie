//! The application HTTP port over the isolated native transfer.

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, Builder};
use std::time::Duration;

use quickie_application::{
    Ended, Flow, HttpError, HttpEvent, HttpMethod, HttpPort, HttpRequest, StopSignal,
};
use url::Url;

use super::{Cancellation, transfer};

const MAX_GET_BYTES: usize = 32 << 20;
const STOP_POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WebKitHttp;

impl HttpPort for WebKitHttp {
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError> {
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: url.clone(),
            headers: Vec::new(),
        };
        let mut status = None;
        let mut bytes = Vec::new();
        let ended = self.stream(&request, &StopSignal::new(), &mut |event| match event {
            HttpEvent::Status(value) => {
                status = Some(value);
                if (200..300).contains(&value) {
                    Flow::Continue
                } else {
                    Flow::Stop
                }
            }
            HttpEvent::Body(chunk) if bytes.len() + chunk.len() <= MAX_GET_BYTES => {
                bytes.extend_from_slice(chunk);
                Flow::Continue
            }
            HttpEvent::Body(_) => Flow::Stop,
        })?;
        match status {
            Some(code) if !(200..300).contains(&code) => Err(HttpError(format!("HTTP {code}"))),
            Some(_) if ended == Ended::Complete => Ok(bytes),
            Some(_) => Err(HttpError("the response exceeded the download limit".into())),
            None => Err(HttpError("the request ended without a response".into())),
        }
    }

    fn stream(
        &self,
        request: &HttpRequest,
        stop: &StopSignal,
        on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    ) -> Result<Ended, HttpError> {
        if stop.is_stopped() {
            return Ok(Ended::Stopped);
        }
        let cancel = Cancellation::new().map_err(HttpError)?;
        let done = AtomicBool::new(false);
        thread::scope(|scope| {
            let watcher = Builder::new()
                .name("quickie-http-stop".into())
                .spawn_scoped(scope, || {
                    while !done.load(Ordering::Acquire) {
                        if stop.is_stopped() {
                            cancel.cancel();
                            break;
                        }
                        thread::park_timeout(STOP_POLL);
                    }
                })
                .map_err(|_| HttpError("could not monitor the request cancellation".into()))?;
            let _monitor = Monitor {
                done: &done,
                thread: watcher.thread(),
            };
            let result = receive(request, &cancel, stop, on_event);
            if stop.is_stopped() {
                Ok(Ended::Stopped)
            } else {
                result
            }
        })
    }
}

struct Monitor<'a> {
    done: &'a AtomicBool,
    thread: &'a thread::Thread,
}

impl Drop for Monitor<'_> {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

fn receive(
    request: &HttpRequest,
    cancel: &Cancellation,
    stop: &StopSignal,
    on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
) -> Result<Ended, HttpError> {
    let response =
        transfer(&request.url, &request.method, &request.headers, cancel).map_err(HttpError)?;
    if stop.is_stopped() || on_event(HttpEvent::Status(response.status)) == Flow::Stop {
        return Ok(Ended::Stopped);
    }
    response
        .read_body(cancel, &mut |bytes| {
            Ok(if stop.is_stopped() {
                Flow::Stop
            } else {
                on_event(HttpEvent::Body(bytes))
            })
        })
        .map_err(HttpError)
}
