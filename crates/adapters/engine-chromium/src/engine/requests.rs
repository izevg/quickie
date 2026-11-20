//! What the bridge does not carry yet: the browser's own requests over Chromium's network
//! stack, and the content blocker of the pages' requests. Each refuses, without calling
//! Chromium, rather than going some other way: the product's traffic goes through the engine's
//! network stack or nowhere.

use quickie_application::{
    BlockerError, BlockerRules, ContentBlockerPort, Ended, Flow, HttpError, HttpEvent, HttpPort,
    HttpRequest, StopSignal,
};
use url::Url;

/// Why the browser's own requests fail.
const NO_NETWORK: &str = "the Chromium bridge cannot send the browser's own requests yet";

/// Chromium's network stack for the browser's own requests, once the bridge carries them.
#[derive(Clone, Copy, Debug)]
pub struct ChromiumHttp;

impl HttpPort for ChromiumHttp {
    fn get(&self, _url: &Url) -> Result<Vec<u8>, HttpError> {
        Err(HttpError(NO_NETWORK.to_owned()))
    }

    fn stream(
        &self,
        _request: &HttpRequest,
        _stop: &StopSignal,
        _on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    ) -> Result<Ended, HttpError> {
        Err(HttpError(NO_NETWORK.to_owned()))
    }
}

/// Chromium's content blocker, once the bridge carries one; the engine hands none out yet.
#[derive(Debug)]
pub struct ChromiumContentBlocker;

impl ContentBlockerPort for ChromiumContentBlocker {
    fn install(&mut self, _rules: &BlockerRules) -> Result<(), BlockerError> {
        Err(BlockerError(
            "the Chromium bridge has no content blocker yet".to_owned(),
        ))
    }
}
