//! What the bridge does not carry yet: the browser's own requests over Chromium's network
//! stack, the content blocker of the pages' requests, and web extensions. Each refuses, without
//! calling Chromium, rather than going some other way: the product's traffic goes through the
//! engine's network stack or nowhere.

use objc2::rc::Retained;
use objc2_app_kit::NSView;
use quickie_application::{
    BlockerError, BlockerRules, ContentBlockerPort, Ended, Flow, HttpError, HttpEvent, HttpPort,
    HttpRequest, NoExtensions, RulesStatus, StopSignal,
};
use url::Url;

/// Why the browser's own requests fail.
const NO_NETWORK: &str = "the Chromium bridge cannot send the browser's own requests yet";

/// Chromium's network stack for the browser's own requests, once the bridge carries them.
#[derive(Clone, Debug)]
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

/// Why the content blocker refuses.
pub(super) const NO_BLOCKER: &str = "the Chromium bridge has no content blocker yet";

/// Chromium's content blocker, once the bridge carries one; the engine hands none out yet.
/// Given rules, it refuses them, so it never settles and never has cosmetic filters.
#[derive(Debug)]
pub struct ChromiumContentBlocker;

impl ContentBlockerPort for ChromiumContentBlocker {
    fn install(&mut self, _rules: &BlockerRules) -> Result<(), BlockerError> {
        Err(BlockerError(NO_BLOCKER.to_owned()))
    }

    fn status(&self) -> RulesStatus {
        RulesStatus::Idle
    }

    fn cosmetics_ready(&self) -> bool {
        false
    }

    fn on_settled(&mut self, _settled: Box<dyn Fn()>) {}
}

/// The host of Chromium's web extensions, once the bridge carries them: none exists yet, so
/// none can be called. Their popups will be views in the shell's windows.
pub type ChromiumExtensions = NoExtensions<Retained<NSView>>;
