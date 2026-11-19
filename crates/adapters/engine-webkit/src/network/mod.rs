//! The development engine's network for the browser's own requests: the application's
//! [`HttpPort`] on `NSURLSession`, the system stack `WebKit` itself loads pages with. The
//! Chromium engine replaces it with Chromium's network service, which the egress test watches.

#[cfg(test)]
mod tests;

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSString, NSURL, NSURLResponse, NSURLSession,
    NSURLSessionConfiguration,
};
use quickie_application::{HttpError, HttpPort};
use url::Url;

/// How long a request may wait for the next bytes before it fails.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a whole request may take, filter lists of several megabytes included.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);

/// GETs over an ephemeral `NSURLSession`: no cookies, no cache, no credentials kept, so a
/// request carries nothing about the user and leaves nothing behind.
#[derive(Debug)]
pub struct UrlSessionHttp {
    session: Retained<NSURLSession>,
}

impl Default for UrlSessionHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl UrlSessionHttp {
    /// A session of its own, with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        let configuration = NSURLSessionConfiguration::ephemeralSessionConfiguration();
        configuration.setTimeoutIntervalForRequest(IDLE_TIMEOUT.as_secs_f64());
        configuration.setTimeoutIntervalForResource(TOTAL_TIMEOUT.as_secs_f64());
        configuration.setHTTPShouldSetCookies(false);
        Self {
            session: NSURLSession::sessionWithConfiguration(&configuration),
        }
    }
}

impl Drop for UrlSessionHttp {
    fn drop(&mut self) {
        self.session.invalidateAndCancel();
    }
}

impl HttpPort for UrlSessionHttp {
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError> {
        if MainThreadMarker::new().is_some() {
            return Err(HttpError(
                "a request never runs on the main thread".to_owned(),
            ));
        }
        let target = NSURL::URLWithString(&NSString::from_str(url.as_str()))
            .ok_or_else(|| HttpError(format!("not a URL the system can load: {url}")))?;
        let (answer, answered) = mpsc::sync_channel(1);
        let block = RcBlock::new(
            move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
                // The caller may have given up already; then nobody needs the answer.
                let _ = answer.send(outcome(data, response, error));
            },
        );
        // SAFETY: the block is called once, on the session's own queue; everything it captures
        // (a channel sender) is `Send`, and it outlives the call because the task retains it.
        let task = unsafe {
            self.session
                .dataTaskWithURL_completionHandler(&target, &block)
        };
        task.resume();
        answered
            .recv_timeout(TOTAL_TIMEOUT + IDLE_TIMEOUT)
            .unwrap_or_else(|_| {
                task.cancel();
                Err(HttpError(format!("no answer from {url}")))
            })
    }
}

/// What a finished task brought: the body of a 2xx response, or why there is none.
fn outcome(
    data: *mut NSData,
    response: *mut NSURLResponse,
    error: *mut NSError,
) -> Result<Vec<u8>, HttpError> {
    // SAFETY: Foundation passes a valid error or nil, alive during the call.
    if let Some(error) = unsafe { error.as_ref() } {
        return Err(HttpError(error.localizedDescription().to_string()));
    }
    // SAFETY: Foundation passes a valid response or nil, alive during the call.
    let status = unsafe { response.as_ref() }
        .and_then(|response| response.downcast_ref::<NSHTTPURLResponse>())
        .map(NSHTTPURLResponse::statusCode)
        .ok_or_else(|| HttpError("the answer is not http".to_owned()))?;
    if !(200..300).contains(&status) {
        return Err(HttpError(format!("status {status}")));
    }
    // SAFETY: Foundation passes valid data or nil, alive during the call.
    Ok(unsafe { data.as_ref() }.map_or_else(Vec::new, NSData::to_vec))
}
