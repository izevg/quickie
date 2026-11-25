//! Typed in-process automation in an isolated `WebKit` session.

mod agent;
mod interactive;
mod native;
mod reply;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use objc2::{MainThreadMarker, rc::autoreleasepool};
use objc2_foundation::{NSDate, NSRunLoop};
use serde_json::{Value, json};
use thiserror::Error;
use url::Url;

use native::Session;
use reply::Reply;

pub use agent::{
    AgentAutomation, AgentAutomationCapabilities, ApprovedOrigin, AutomationEvent,
    AutomationRequestId, CancelInteractionSequenceCommand, DomBounds, ElementSignals, InputSource,
    InputSourceState, InputSourceType, InteractionDelta, InteractionSequenceCommand,
    InteractionStep, KeyModifier, KeyboardCommand, KeyboardStep, LocatedDomElement, LocatorQuery,
    LocatorSnapshot, MAX_BROWSING_CONTEXTS, MAX_HANDLE_BYTES, MAX_IN_FLIGHT_REQUESTS,
    MAX_INBOUND_MESSAGE_BYTES, MAX_INPUT_SOURCES, MAX_INTERACTION_STEPS, MAX_KEY_MODIFIERS,
    MAX_KEYBOARD_INTERACTIONS, MAX_KEYBOARD_TEXT_BYTES, MAX_PAGE_LOAD_TIMEOUT_MS,
    MAX_SCREENSHOT_BASE64_BYTES, MAX_SEQUENCE_DURATION_MS, MAX_STEP_DURATION_MS,
    MAX_VIEWPORT_COORD, MouseButton, MouseCommand, MouseInteraction, MouseMoveOrigin,
    NavigationCommand, ObservedPageState, PageLoadStrategy, ScreenshotCommand, ScreenshotData,
    ViewportPoint, VirtualKey, WaitOutcome, WaitQuery,
};
pub use interactive::InteractiveAutomation;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const PAGE_TIMEOUT_MS: u32 = 9_000;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A page controlled by this automation session.
#[derive(Debug, PartialEq, Eq)]
pub struct BrowsingContext {
    handle: String,
    /// The page's current URL as reported by `WebKit`.
    pub url: String,
}

impl BrowsingContext {
    /// The opaque `WebKit` browsing context handle.
    #[must_use]
    pub fn handle(&self) -> &str {
        &self.handle
    }
}

/// Failure to open or operate a local automation session.
#[derive(Debug, Error)]
pub enum AutomationError {
    /// The loaded framework has no in-process automation API.
    #[error("local automation requires the Quickie WebKit fork")]
    Unsupported,
    /// `WebKit` could not create or communicate with the session.
    #[error("local WebKit automation transport failed")]
    Transport,
    /// An initial context or a reply did not arrive within the deadline.
    #[error("local WebKit automation timed out")]
    Timeout,
    /// `WebKit` returned an unexpected response.
    #[error("invalid local WebKit automation reply")]
    InvalidReply,
    /// `WebKit` rejected the command or failed the navigation.
    #[error("local WebKit automation command failed")]
    CommandFailed,
    /// The explicitly requested navigation is not an ordinary web URL.
    #[error("automation requires an HTTP or HTTPS URL without credentials")]
    InvalidUrl,
    /// The session cannot accept another concurrent command right now.
    #[error("local WebKit automation is busy")]
    Busy,
    /// The individual automation operation was cancelled before completion.
    #[error("local WebKit automation command was cancelled")]
    Cancelled,
    /// The isolated automation session was closed or disconnected.
    #[error("local WebKit automation session is disconnected")]
    Disconnected,
    /// The command parameters violated protocol or safety bounds.
    #[error("invalid local WebKit automation command parameters")]
    InvalidCommand,
    /// The navigation URL did not match the provenance-approved origin.
    #[error("automation navigation URL does not match the approved origin")]
    OriginMismatch,
    /// The reply exceeded the configured response size bound.
    #[error("local WebKit automation reply exceeded the size bound")]
    ResponseTooLarge,
}

/// Owns a separate process pool, a nonpersistent data store, and an offscreen view.
/// Commands pump the main run loop, so use this from a standalone harness before the browser
/// starts. Drop closes the connection and cancels outstanding page loads.
pub struct LocalAutomation {
    session: Session,
    reply: Rc<RefCell<Reply>>,
    last_id: u32,
}

impl LocalAutomation {
    /// Open an isolated session and wait for its initial blank context.
    ///
    /// # Errors
    /// Returns an error if the loaded framework lacks the transport or the context cannot start.
    pub fn open(mtm: MainThreadMarker) -> Result<Self, AutomationError> {
        let reply = Rc::new(RefCell::new(Reply::default()));
        let session = Session::new(mtm, Rc::clone(&reply), false)?;
        let mut automation = Self {
            session,
            reply,
            last_id: 0,
        };
        until(Instant::now() + REQUEST_TIMEOUT, |deadline| {
            if automation.session.is_loading() {
                return Ok(None);
            }
            let contexts = automation.contexts_before(deadline)?;
            Ok((!contexts.is_empty()).then_some(()))
        })?;
        Ok(automation)
    }

    /// List the pages owned by this session.
    ///
    /// # Errors
    /// Returns an error if the framework fails to answer the request.
    pub fn contexts(&mut self) -> Result<Vec<BrowsingContext>, AutomationError> {
        self.contexts_before(Instant::now() + REQUEST_TIMEOUT)
    }

    /// Navigate the given context to an explicitly supplied web URL.
    ///
    /// # Errors
    /// Returns an error if the URL is invalid, the request times out, or `WebKit` fails the load.
    pub fn navigate(
        &mut self,
        context: &BrowsingContext,
        url: &Url,
    ) -> Result<(), AutomationError> {
        validate_url(url)?;
        self.session.reset_navigation_failure();
        let response = self.request(
            "Automation.navigateBrowsingContext",
            &json!({
                "handle": context.handle,
                "url": url.as_str(),
                "pageLoadStrategy": "Normal",
                "pageLoadTimeout": PAGE_TIMEOUT_MS,
            }),
            Instant::now() + REQUEST_TIMEOUT,
        );
        if response.is_err() {
            self.session.stop_loading();
        }
        response?;
        if self.session.navigation_failed() {
            return Err(AutomationError::CommandFailed);
        }
        Ok(())
    }

    fn contexts_before(
        &mut self,
        deadline: Instant,
    ) -> Result<Vec<BrowsingContext>, AutomationError> {
        let result = self.request("Automation.getBrowsingContexts", &json!({}), deadline)?;
        reply::contexts(&result)
    }

    fn request(
        &mut self,
        method: &str,
        params: &Value,
        deadline: Instant,
    ) -> Result<Value, AutomationError> {
        self.last_id = self
            .last_id
            .checked_add(1)
            .ok_or(AutomationError::Transport)?;
        self.reply.borrow_mut().expect_id(self.last_id);
        let message = json!({"id": self.last_id, "method": method, "params": params});
        let result = self
            .session
            .dispatch(&message.to_string())
            .and_then(|()| until(deadline, |_| self.reply.borrow_mut().take().transpose()));
        self.reply.borrow_mut().clear();
        if matches!(result, Err(AutomationError::Timeout)) {
            self.session.close();
        }
        result
    }
}

fn validate_url(url: &Url) -> Result<(), AutomationError> {
    if matches!(url.scheme(), "http" | "https")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
    {
        Ok(())
    } else {
        Err(AutomationError::InvalidUrl)
    }
}

fn until<T>(
    deadline: Instant,
    mut poll: impl FnMut(Instant) -> Result<Option<T>, AutomationError>,
) -> Result<T, AutomationError> {
    loop {
        if Instant::now() >= deadline {
            return Err(AutomationError::Timeout);
        }
        if let Some(output) = poll(deadline)? {
            return Ok(output);
        }
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(POLL_INTERVAL);
        autoreleasepool(|_| {
            let date = NSDate::dateWithTimeIntervalSinceNow(wait.as_secs_f64());
            NSRunLoop::currentRunLoop().runUntilDate(&date);
        });
    }
}
