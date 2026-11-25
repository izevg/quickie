//! Event-driven isolated macOS `WebKit` Automation runtime for MCP Agent contexts.
//!
//! Uses `_WKAutomationSession`'s in-process local client callbacks on the main thread with a
//! dedicated `WKProcessPool`, non-persistent `WKWebsiteDataStore`, hidden `WKWebView`, and
//! same-origin GET navigation policy. Never touches `Main` browsing tabs, visible windows, or
//! raw Inspector or `BiDi` passthrough messages.

mod dispatcher;
mod dom;
mod protocol;
#[cfg(test)]
mod tests;

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchTime, MainThreadBound};
use objc2::MainThreadMarker;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_application::mcp::{DiagnosticCapabilities, SnapshotRequest};
use quickie_application::{DiagnosticLease, DiagnosticSignal, DiagnosticSnapshot, EngineError};
use url::Url;

use super::AutomationError;
use super::BrowsingContext;
use super::native::{Session, supports_isolated_automation};
use crate::observation;

pub use dispatcher::{AutomationRequestId, MAX_IN_FLIGHT_REQUESTS};
pub use dom::{
    DomBounds, ElementSignals, LocatedDomElement, LocatorQuery, LocatorSnapshot, ObservedPageState,
    WaitOutcome, WaitQuery,
};
pub use protocol::{
    ApprovedOrigin, AutomationEvent, CancelInteractionSequenceCommand, InputSource,
    InputSourceState, InputSourceType, InteractionDelta, InteractionSequenceCommand,
    InteractionStep, KeyModifier, KeyboardCommand, KeyboardStep, MAX_BROWSING_CONTEXTS,
    MAX_HANDLE_BYTES, MAX_INBOUND_MESSAGE_BYTES, MAX_INPUT_SOURCES, MAX_INTERACTION_STEPS,
    MAX_KEY_MODIFIERS, MAX_KEYBOARD_INTERACTIONS, MAX_KEYBOARD_TEXT_BYTES,
    MAX_PAGE_LOAD_TIMEOUT_MS, MAX_SCREENSHOT_BASE64_BYTES, MAX_SEQUENCE_DURATION_MS,
    MAX_STEP_DURATION_MS, MAX_VIEWPORT_COORD, MouseButton, MouseCommand, MouseInteraction,
    MouseMoveOrigin, NavigationCommand, PageLoadStrategy, ScreenshotCommand, ScreenshotData,
    ViewportPoint, VirtualKey,
};

use dispatcher::{Dispatcher, InboundDispatch, PendingCompletion};
use protocol::{
    CommandResponse, EncodedCommand, encode_cancel_interaction_sequence,
    encode_get_browsing_contexts, encode_interaction_sequence, encode_keyboard_interactions,
    encode_mouse_interaction, encode_navigate, encode_take_screenshot,
};

type EventObserver = Rc<dyn Fn(AutomationEvent) + 'static>;

/// Honest runtime capabilities of [`AgentAutomation`] on the loaded `WebKit` framework.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentAutomationCapabilities {
    /// Whether `_WKAutomationSession`'s in-process local client and interactive restrictions exist.
    pub isolated_automation: bool,
    /// Read-only diagnostic observation capabilities available on the isolated view.
    pub observations: DiagnosticCapabilities,
}

/// Event-driven isolated `WebKit` Automation session for MCP Agent contexts.
///
/// Owns a dedicated process pool, a non-persistent data store, and an offscreen `WKWebView` that
/// stays at `about:blank` with zero automatic network requests until navigated to a
/// provenance-approved same-origin HTTP(S) URL.
///
/// Dropping or closing the session completes all pending replies with
/// [`AutomationError::Disconnected`], invalidates any attached diagnostic observer, and
/// disconnects the native transport safely.
pub struct AgentAutomation(Rc<AgentState>);

struct AgentState {
    mtm: MainThreadMarker,
    session: OnceCell<Session>,
    dispatcher: RefCell<Dispatcher>,
    active_navigation: Cell<Option<AutomationRequestId>>,
    event_observer: RefCell<Option<EventObserver>>,
    diagnostic_observer: RefCell<Option<Weak<observation::cocoa::ObserverState>>>,
}

impl std::fmt::Debug for AgentAutomation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentAutomation")
            .field("connected", &self.is_connected())
            .field("in_flight", &self.in_flight_count())
            .finish_non_exhaustive()
    }
}

impl AgentAutomation {
    /// Probes Objective-C runtime metadata to report what the loaded `WebKit` supports.
    ///
    /// Calls no fork selectors on stock `WebKit` and never touches any `Main` browsing tab.
    #[must_use]
    pub fn capabilities() -> AgentAutomationCapabilities {
        AgentAutomationCapabilities {
            isolated_automation: supports_isolated_automation(),
            observations: observation::cocoa::capabilities(),
        }
    }

    /// Opens an isolated offscreen Agent automation session with [`MAX_IN_FLIGHT_REQUESTS`].
    ///
    /// # Errors
    /// Returns [`AutomationError::Unsupported`] on stock `WebKit`, or
    /// [`AutomationError::Transport`] if the local client connection fails.
    pub fn open(mtm: MainThreadMarker) -> Result<Self, AutomationError> {
        Self::open_with_limit(mtm, MAX_IN_FLIGHT_REQUESTS)
    }

    /// Opens an isolated offscreen Agent automation session with a bounded in-flight limit
    /// (`1..=MAX_IN_FLIGHT_REQUESTS`).
    ///
    /// # Errors
    /// Returns [`AutomationError::InvalidCommand`] if `max_in_flight` is outside
    /// `1..=MAX_IN_FLIGHT_REQUESTS`, [`AutomationError::Unsupported`] on stock `WebKit`, or
    /// [`AutomationError::Transport`] if the local client connection fails.
    pub fn open_with_limit(
        mtm: MainThreadMarker,
        max_in_flight: usize,
    ) -> Result<Self, AutomationError> {
        if max_in_flight == 0 || max_in_flight > MAX_IN_FLIGHT_REQUESTS {
            return Err(AutomationError::InvalidCommand);
        }
        let state = Rc::new(AgentState {
            mtm,
            session: OnceCell::new(),
            dispatcher: RefCell::new(Dispatcher::new(max_in_flight)),
            active_navigation: Cell::new(None),
            event_observer: RefCell::new(None),
            diagnostic_observer: RefCell::new(None),
        });
        let weak = Rc::downgrade(&state);
        let session = Session::with_handler(mtm, true, move |raw| {
            if let Some(state) = weak.upgrade() {
                state.on_message(raw);
            }
        })?;
        let _ = state.session.set(session);
        Ok(Self(state))
    }

    /// Whether the underlying isolated transport is still open.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        !self.0.dispatcher.borrow().is_closed()
            && self.0.session.get().is_some_and(Session::is_connected)
    }

    /// Number of currently in-flight Automation commands.
    #[must_use]
    pub fn in_flight_count(&self) -> usize {
        self.0.dispatcher.borrow().in_flight_count()
    }

    /// Installs or clears the observer receiving unsolicited classic [`AutomationEvent`]s.
    pub fn set_event_observer(&self, observer: Option<Box<dyn Fn(AutomationEvent) + 'static>>) {
        *self.0.event_observer.borrow_mut() = observer.map(Rc::from);
    }

    /// Issues `Automation.getBrowsingContexts` and invokes `done` asynchronously with the result.
    ///
    /// # Errors
    /// Returns an error if the session is closed, in-flight capacity is full, `timeout` is zero,
    /// or native dispatch fails.
    pub fn list_contexts(
        &self,
        timeout: Duration,
        done: impl FnOnce(Result<Vec<BrowsingContext>, AutomationError>) + 'static,
    ) -> Result<AutomationRequestId, AutomationError> {
        let command = encode_get_browsing_contexts();
        self.0.send_command(&command, false, timeout, move |outcome| {
            done(outcome.and_then(|response| match response {
                CommandResponse::Contexts(contexts) => Ok(contexts),
                _ => Err(AutomationError::InvalidReply),
            }));
        })
    }

    /// Issues `Automation.navigateBrowsingContext` on a provenance-approved same-origin HTTP(S)
    /// URL, enforcing the main-thread same-origin GET navigation policy.
    ///
    /// # Errors
    /// Returns an error if `command` fails URL, origin, or timeout validation, another navigation
    /// is already in flight, in-flight capacity is full, the session is closed, or dispatch fails.
    pub fn navigate(
        &self,
        command: &NavigationCommand,
        timeout: Duration,
        done: impl FnOnce(Result<(), AutomationError>) + 'static,
    ) -> Result<AutomationRequestId, AutomationError> {
        let encoded = encode_navigate(command)?;
        if self.0.active_navigation.get().is_some() {
            return Err(AutomationError::Busy);
        }
        let session = self.0.session()?;
        session.restrict_to(&command.url);
        session.reset_navigation_failure();
        let id = self.0.send_command(&encoded, true, timeout, move |outcome| {
            done(outcome.and_then(|response| match response {
                CommandResponse::Unit => Ok(()),
                _ => Err(AutomationError::InvalidReply),
            }));
        })?;
        self.0.active_navigation.set(Some(id));
        Ok(id)
    }

    /// Issues `Automation.performMouseInteraction` and invokes `done` with the resulting pointer
    /// position.
    ///
    /// # Errors
    /// Returns an error if `command` violates bounds, the session is closed, capacity is full, or
    /// dispatch fails.
    pub fn perform_mouse_interaction(
        &self,
        command: &MouseCommand,
        timeout: Duration,
        done: impl FnOnce(Result<ViewportPoint, AutomationError>) + 'static,
    ) -> Result<AutomationRequestId, AutomationError> {
        let encoded = encode_mouse_interaction(command)?;
        self.0.send_command(&encoded, false, timeout, move |outcome| {
            done(outcome.and_then(|response| match response {
                CommandResponse::Position(point) => Ok(point),
                _ => Err(AutomationError::InvalidReply),
            }));
        })
    }

    /// Issues `Automation.performKeyboardInteractions` for a bounded sequence of trusted key steps.
    ///
    /// # Errors
    /// Returns an error if `command` violates bounds, the session is closed, capacity is full, or
    /// dispatch fails.
    pub fn perform_keyboard_interactions(
        &self,
        command: &KeyboardCommand,
        timeout: Duration,
        done: impl FnOnce(Result<(), AutomationError>) + 'static,
    ) -> Result<AutomationRequestId, AutomationError> {
        let encoded = encode_keyboard_interactions(command)?;
        self.0.send_command(&encoded, false, timeout, move |outcome| {
            done(outcome.and_then(|response| match response {
                CommandResponse::Unit => Ok(()),
                _ => Err(AutomationError::InvalidReply),
            }));
        })
    }

    /// Issues `Automation.performInteractionSequence` for a bounded multi-source input sequence.
    ///
    /// # Errors
    /// Returns an error if `command` violates bounds, the session is closed, capacity is full, or
    /// dispatch fails.
    pub fn perform_interaction_sequence(
        &self,
        command: &InteractionSequenceCommand,
        timeout: Duration,
        done: impl FnOnce(Result<(), AutomationError>) + 'static,
    ) -> Result<AutomationRequestId, AutomationError> {
        let encoded = encode_interaction_sequence(command)?;
        self.0.send_command(&encoded, false, timeout, move |outcome| {
            done(outcome.and_then(|response| match response {
                CommandResponse::Unit => Ok(()),
                _ => Err(AutomationError::InvalidReply),
            }));
        })
    }

    /// Issues `Automation.cancelInteractionSequence` to stop an active input sequence.
    ///
    /// # Errors
    /// Returns an error if `command` violates bounds, the session is closed, capacity is full, or
    /// dispatch fails.
    pub fn cancel_interaction_sequence(
        &self,
        command: &CancelInteractionSequenceCommand,
        timeout: Duration,
        done: impl FnOnce(Result<(), AutomationError>) + 'static,
    ) -> Result<AutomationRequestId, AutomationError> {
        let encoded = encode_cancel_interaction_sequence(command)?;
        self.0.send_command(&encoded, false, timeout, move |outcome| {
            done(outcome.and_then(|response| match response {
                CommandResponse::Unit => Ok(()),
                _ => Err(AutomationError::InvalidReply),
            }));
        })
    }

    /// Issues `Automation.takeScreenshot` with a bounded base64 PNG size limit.
    ///
    /// # Errors
    /// Returns an error if `command` violates bounds, the session is closed, capacity is full, or
    /// dispatch fails.
    pub fn take_screenshot(
        &self,
        command: &ScreenshotCommand,
        timeout: Duration,
        done: impl FnOnce(Result<ScreenshotData, AutomationError>) + 'static,
    ) -> Result<AutomationRequestId, AutomationError> {
        let encoded = encode_take_screenshot(command)?;
        self.0.send_command(&encoded, false, timeout, move |outcome| {
            done(outcome.and_then(|response| match response {
                CommandResponse::Screenshot(data) => Ok(data),
                _ => Err(AutomationError::InvalidReply),
            }));
        })
    }

    /// Cancels one in-flight operation by ID, completing it with [`AutomationError::Cancelled`]
    /// and ignoring any late reply. Other in-flight operations continue normally.
    ///
    /// Returns `true` if `id` was pending and cancelled, or `false` if it was already settled or
    /// unknown.
    #[must_use]
    pub fn cancel(&self, id: AutomationRequestId) -> bool {
        let completion = self.0.dispatcher.borrow_mut().cancel(id);
        let Some(completion) = completion else {
            return false;
        };
        self.0.finish_navigation_if_needed(&completion, true);
        completion.finish(Err(AutomationError::Cancelled));
        true
    }

    /// Attaches a scoped read-only diagnostic observer to this isolated session's `WKWebView`.
    ///
    /// Dropping the returned [`DiagnosticLease`] disconnects the observer. Closing or dropping
    /// this [`AgentAutomation`] invalidates the observer with an explicit channel-closed gap.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the session is closed, the running `WebKit` lacks the local
    /// diagnostic SPI, or another observer is already attached.
    pub fn observe_diagnostics(
        &self,
        on_signal: Box<dyn Fn(DiagnosticSignal) + 'static>,
    ) -> Result<DiagnosticLease, EngineError> {
        if !self.is_connected() {
            return Err(EngineError(
                "isolated automation session is closed".to_owned(),
            ));
        }
        let session = self
            .0
            .session
            .get()
            .ok_or_else(|| EngineError("isolated automation session is closed".to_owned()))?;
        let (lease, handle) = observation::cocoa::ObserverState::connect(session.view(), on_signal)?;
        *self.0.diagnostic_observer.borrow_mut() = Some(handle);
        Ok(lease)
    }

    /// Captures a bounded read-only DOM or visible-text snapshot from this isolated session's
    /// `WKWebView` in Quickie's isolated content world.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the session is closed or the requested snapshot kind is
    /// unavailable.
    pub fn diagnostic_snapshot(
        &self,
        request: &SnapshotRequest,
        done: Box<dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static>,
    ) -> Result<(), EngineError> {
        if !self.is_connected() {
            return Err(EngineError(
                "isolated automation session is closed".to_owned(),
            ));
        }
        let session = self
            .0
            .session
            .get()
            .ok_or_else(|| EngineError("isolated automation session is closed".to_owned()))?;
        observation::cocoa::snapshot::capture(session.view(), request, self.0.mtm, done)
    }

    /// Closes the isolated session, disconnects `WebKit`, invalidates any diagnostic observer, and
    /// completes every pending operation with [`AutomationError::Disconnected`].
    pub fn close(&self) {
        self.0.close();
    }

    /// Resizes the isolated offscreen `WKWebView` to `(width, height)` CSS pixels.
    pub fn set_viewport(&self, width: u32, height: u32) {
        if let Some(session) = self.0.session.get() {
            let frame = NSRect::new(NSPoint::ZERO, NSSize::new(f64::from(width), f64::from(height)));
            session.view().setFrame(frame);
        }
    }

    /// Returns the isolated `WKWebView`'s currently committed URL, if any.
    #[must_use]
    pub fn current_url(&self) -> Option<Url> {
        let session = self.0.session.get()?;
        // SAFETY: `URL` is a main-thread getter on this session's live `WKWebView`.
        let ns_url = unsafe { session.view().URL() }?;
        let raw = ns_url.absoluteString()?.to_string();
        Url::parse(&raw).ok()
    }

    /// Locates bounded, ordered elements in Quickie's isolated content world.
    ///
    /// # Errors
    /// Returns [`AutomationError::Disconnected`] if the session is closed or
    /// [`AutomationError::InvalidCommand`] if the query cannot be encoded.
    pub fn locate_elements(
        &self,
        query: &LocatorQuery,
        max_results: usize,
        done: impl FnOnce(Result<LocatorSnapshot, AutomationError>) + 'static,
    ) -> Result<(), AutomationError> {
        let session = self.0.session()?;
        dom::locate(session.view(), self.0.mtm, query, max_results, done)
    }

    /// Resolves an opaque isolated-world element handle and returns typed containment signals.
    ///
    /// # Errors
    /// Returns [`AutomationError::Disconnected`] if the session is closed.
    pub fn resolve_element(
        &self,
        element_id: &str,
        scroll_into_view: bool,
        done: impl FnOnce(Result<ElementSignals, AutomationError>) + 'static,
    ) -> Result<(), AutomationError> {
        let session = self.0.session()?;
        dom::resolve_element(
            session.view(),
            self.0.mtm,
            element_id,
            scroll_into_view,
            done,
        )
    }

    /// Inspects the currently focused element in Quickie's isolated content world.
    ///
    /// # Errors
    /// Returns [`AutomationError::Disconnected`] if the session is closed.
    pub fn inspect_focused(
        &self,
        done: impl FnOnce(Result<ElementSignals, AutomationError>) + 'static,
    ) -> Result<(), AutomationError> {
        let session = self.0.session()?;
        dom::inspect_focused(session.view(), self.0.mtm, done);
        Ok(())
    }

    /// Selects an option in a `<select>` element in Quickie's isolated content world.
    ///
    /// # Errors
    /// Returns [`AutomationError::Disconnected`] if the session is closed.
    pub fn select_option(
        &self,
        element_id: &str,
        by_value: Option<&str>,
        by_label: Option<&str>,
        done: impl FnOnce(Result<(), AutomationError>) + 'static,
    ) -> Result<(), AutomationError> {
        let session = self.0.session()?;
        dom::select_option(session.view(), self.0.mtm, element_id, by_value, by_label, done)
    }

    /// Observes bounded post-action page state in Quickie's isolated content world.
    ///
    /// # Errors
    /// Returns [`AutomationError::Disconnected`] if the session is closed.
    pub fn observe_state(
        &self,
        done: impl FnOnce(Result<ObservedPageState, AutomationError>) + 'static,
    ) -> Result<(), AutomationError> {
        let session = self.0.session()?;
        dom::observe_state(session.view(), self.0.mtm, done);
        Ok(())
    }

    /// Event-driven wait for a typed condition in Quickie's isolated content world.
    ///
    /// # Errors
    /// Returns [`AutomationError::Disconnected`] if the session is closed or
    /// [`AutomationError::InvalidCommand`] if `timeout` is zero.
    pub fn wait_for_condition(
        &self,
        query: &WaitQuery,
        timeout: Duration,
        done: impl FnOnce(Result<WaitOutcome, AutomationError>) + 'static,
    ) -> Result<(), AutomationError> {
        let session = self.0.session()?;
        dom::wait_for_condition(session.view(), self.0.mtm, query, timeout, done)
    }
}

impl Drop for AgentAutomation {
    fn drop(&mut self) {
        self.0.close();
    }
}

impl AgentState {
    fn session(&self) -> Result<&Session, AutomationError> {
        let session = self.session.get().ok_or(AutomationError::Disconnected)?;
        if !session.is_connected() {
            return Err(AutomationError::Disconnected);
        }
        Ok(session)
    }

    fn send_command(
        self: &Rc<Self>,
        encoded: &EncodedCommand,
        is_navigation: bool,
        timeout: Duration,
        done: impl FnOnce(Result<CommandResponse, AutomationError>) + 'static,
    ) -> Result<AutomationRequestId, AutomationError> {
        if timeout.is_zero() {
            return Err(AutomationError::InvalidCommand);
        }
        let session = self.session()?;
        let when = DispatchTime::try_from(timeout).map_err(|()| AutomationError::InvalidCommand)?;

        let (id, wire) = self
            .dispatcher
            .borrow_mut()
            .begin(encoded, is_navigation, Box::new(done))?;

        if let Err(error) = session.dispatch(&wire) {
            self.dispatcher.borrow_mut().rollback_dispatch(id);
            return Err(error);
        }

        let weak = MainThreadBound::new(Rc::downgrade(self), self.mtm);
        if DispatchQueue::main()
            .after(when, move || {
                if let Some(state) =
                    MainThreadMarker::new().and_then(|mtm| weak.into_inner(mtm).upgrade())
                {
                    state.on_timeout(id);
                }
            })
            .is_err()
        {
            self.dispatcher.borrow_mut().rollback_dispatch(id);
            return Err(AutomationError::Transport);
        }

        Ok(id)
    }

    fn on_message(&self, raw: &str) {
        let dispatch = self.dispatcher.borrow_mut().receive(raw);
        match dispatch {
            InboundDispatch::Reply {
                completion,
                mut outcome,
            } => {
                let nav_failed = completion.is_navigation
                    && self
                        .session
                        .get()
                        .is_some_and(Session::navigation_failed);
                self.finish_navigation_if_needed(&completion, outcome.is_err() || nav_failed);
                if nav_failed && outcome.is_ok() {
                    outcome = Err(AutomationError::CommandFailed);
                }
                completion.finish(outcome);
            }
            InboundDispatch::Event(event) => {
                let observer = self.event_observer.borrow().clone();
                if let Some(observer) = observer {
                    observer(event);
                }
            }
            InboundDispatch::Ignored => {}
        }
    }

    fn on_timeout(&self, id: AutomationRequestId) {
        let completion = self.dispatcher.borrow_mut().timeout(id);
        let Some(completion) = completion else {
            return;
        };
        self.finish_navigation_if_needed(&completion, true);
        completion.finish(Err(AutomationError::Timeout));
    }

    fn finish_navigation_if_needed(&self, completion: &PendingCompletion, stop_loading: bool) {
        if completion.is_navigation && self.active_navigation.get() == Some(completion.id) {
            self.active_navigation.set(None);
            if stop_loading && let Some(session) = self.session.get() {
                session.stop_loading();
            }
        }
    }

    fn close(&self) {
        let observer = self
            .diagnostic_observer
            .borrow_mut()
            .take()
            .and_then(|observer| observer.upgrade());
        if let Some(observer) = observer {
            observer.invalidate();
        }
        let _ = self.event_observer.borrow_mut().take();
        self.active_navigation.set(None);
        let pending = self.dispatcher.borrow_mut().close();
        if let Some(session) = self.session.get() {
            session.close();
        }
        for completion in pending {
            completion.finish(Err(AutomationError::Disconnected));
        }
    }
}
