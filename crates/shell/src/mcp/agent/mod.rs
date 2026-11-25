//! Pure main-thread coordinator for isolated `Profile::Agent` MCP contexts.
//!
//! Parameterized over [`AgentBackend`] and [`AgentSession`] so both the real macOS `WebKit`
//! [`quickie_engine_webkit::AgentAutomation`] runtime and synthetic unit/integration tests drive
//! the exact same isolation, epoch, approval, and observation state machine.

pub(crate) mod native;
pub(crate) mod policy;
pub(crate) mod standalone;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use quickie_application::mcp::{
    AgentContextId, BoundedSnapshot, ClientId, DiagnosticCapabilities, DiagnosticEventFilter,
    SnapshotKind, SnapshotRequest,
};
use quickie_application::{
    DiagnosticGapReason, DiagnosticLease, DiagnosticSignal, DiagnosticSnapshot, EngineError,
};
use quickie_domain_browsing::WindowId;
use quickie_mcp::browser::{
    AgentActResult, AgentActStatus, AgentAction, AgentContextCloseResult, AgentContextListResult,
    AgentContextOpenResult, AgentContextSummary, AgentElementRef, AgentKey, AgentKeyModifier,
    AgentLocateResult, AgentLocator, AgentMouseButton, AgentNavigateResult, AgentNavigateStatus,
    AgentObservedState, AgentOperation, AgentScreenshotResult, AgentScreenshotTarget,
    AgentViewport, AgentWaitCondition, AgentWaitResult, AgentWaitStage, AgentWaitStatus,
    ElementProvenance, FrameScope, LocatedElement, SelectOptionBy,
};
use quickie_mcp::{HostReply, PageEventsResult, PageSnapshotResult, ToolError, ToolErrorCode};
use url::{Origin, Url};

use self::policy::{
    AgentCapabilities, AgentEngineError, AgentEventBuffer, AgentLocateOutput, AgentObservedOutput,
    AgentSignals, ApprovalDecision, decode_png_base64, evaluate_action_policy,
    evaluate_navigation_policy,
};

/// Hard ceiling on isolated Agent contexts owned by a single MCP client.
pub(crate) const MAX_AGENT_CONTEXTS_PER_CLIENT: usize = 4;

/// Hard ceiling on total concurrent isolated Agent contexts across all clients.
pub(crate) const MAX_AGENT_CONTEXTS_TOTAL: usize = 16;

/// Hard ceiling on queued Agent approval prompts waiting for native UI confirmation.
pub(crate) const MAX_PENDING_AGENT_APPROVALS: usize = 16;

const DEFAULT_INPUT_TIMEOUT: Duration = Duration::from_secs(10);

/// Event delivered asynchronously from a native [`AgentSession`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "main-webkit"),
    expect(
        dead_code,
        reason = "constructed by the WebKit backend when main-webkit is enabled"
    )
)]
pub(crate) enum AgentSessionEvent {
    /// Context navigated or cleared; caller bumps document epoch if still connected, or destroys
    /// context if disconnected.
    ContextCleared,
}

/// Main-thread handle to one isolated `Profile::Agent` browsing context session.
pub(crate) trait AgentSession {
    /// Whether the underlying isolated transport is still connected.
    fn is_connected(&self) -> bool;

    /// Resizes the hidden viewport to `viewport` CSS pixels.
    fn set_viewport(&self, viewport: AgentViewport);

    /// Returns the currently committed URL of the isolated page, if any.
    fn current_url(&self) -> Option<Url>;

    /// Installs an event observer for unsolicited session lifecycle events.
    fn set_event_observer(&self, observer: Option<Box<dyn Fn(AgentSessionEvent) + 'static>>);

    /// Connects a read-only diagnostic observer to this isolated context.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the diagnostic SPI is unavailable or already attached.
    fn observe_diagnostics(
        &self,
        on_signal: Box<dyn Fn(DiagnosticSignal) + 'static>,
    ) -> Result<DiagnosticLease, EngineError>;

    /// Captures a bounded read-only DOM or visible-text snapshot in Quickie's isolated world.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the session is closed or the snapshot kind is unsupported.
    fn diagnostic_snapshot(
        &self,
        request: &SnapshotRequest,
        done: Box<dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static>,
    ) -> Result<(), EngineError>;

    /// Navigates the isolated context to a provenance-approved same-origin HTTP(S) URL.
    fn navigate(
        &self,
        url: Url,
        approved_origin: Origin,
        wait_stage: AgentWaitStage,
        timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    );

    /// Locates bounded ordered elements in Quickie's isolated content world.
    fn locate_elements(
        &self,
        locator: &AgentLocator,
        max_results: usize,
        done: Box<dyn FnOnce(Result<AgentLocateOutput, AgentEngineError>) + 'static>,
    );

    /// Resolves an opaque isolated-world element ID and returns typed containment signals.
    fn resolve_element(
        &self,
        element_id: &str,
        scroll_into_view: bool,
        done: Box<dyn FnOnce(Result<AgentSignals, AgentEngineError>) + 'static>,
    );

    /// Inspects the currently focused element in Quickie's isolated content world.
    fn inspect_focused(
        &self,
        done: Box<dyn FnOnce(Result<AgentSignals, AgentEngineError>) + 'static>,
    );

    /// Performs a trusted pointer click, double-click, or move.
    fn mouse_action(
        &self,
        request: MouseActionRequest,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    );

    /// Performs trusted keyboard text insertion.
    fn type_text(
        &self,
        text: String,
        timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    );

    /// Replaces focused editable content using trusted keyboard select-all + backspace + insert.
    fn fill_text(
        &self,
        text: String,
        timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    );

    /// Presses an allowlisted key with optional modifiers via trusted keyboard input.
    fn press_key(
        &self,
        key: AgentKey,
        modifiers: Vec<AgentKeyModifier>,
        timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    );

    /// Performs a trusted wheel scroll at viewport coordinates `(x, y)`.
    fn scroll_wheel(
        &self,
        x: f64,
        y: f64,
        delta_x: i32,
        delta_y: i32,
        timeout: Duration,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    );

    /// Selects an option in a `<select>` element.
    fn select_option(
        &self,
        element_id: &str,
        option: &SelectOptionBy,
        done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    );

    /// Observes bounded post-action page state in Quickie's isolated content world.
    fn observe_state(
        &self,
        done: Box<dyn FnOnce(Result<AgentObservedOutput, AgentEngineError>) + 'static>,
    );

    /// Waits for a typed condition in Quickie's isolated content world (event-driven, no polling).
    fn wait_for(
        &self,
        condition: &AgentWaitCondition,
        timeout: Duration,
        done: Box<dyn FnOnce(Result<AgentWaitStatus, AgentEngineError>) + 'static>,
    );

    /// Captures a bounded viewport screenshot as base64-encoded PNG.
    fn screenshot_viewport(
        &self,
        max_bytes: usize,
        timeout: Duration,
        done: Box<dyn FnOnce(Result<String, AgentEngineError>) + 'static>,
    );

    /// Closes the isolated session and tears down native state.
    fn close(&self);
}

/// Pointer interaction kind passed to [`AgentSession::mouse_action`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MouseActionKind {
    Click,
    DoubleClick,
    Hover,
}

/// Parameters for [`AgentSession::mouse_action`].
#[derive(Clone, Debug)]
#[cfg_attr(
    not(feature = "main-webkit"),
    expect(
        dead_code,
        reason = "read by the WebKit backend when main-webkit is enabled"
    )
)]
pub(crate) struct MouseActionRequest {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) kind: MouseActionKind,
    pub(crate) button: AgentMouseButton,
    pub(crate) modifiers: Vec<AgentKeyModifier>,
    pub(crate) timeout: Duration,
}

/// Factory backend that reports capabilities and opens new isolated [`AgentSession`]s.
pub(crate) trait AgentBackend {
    /// Honest capabilities of the isolated Agent runtime.
    fn agent_capabilities(&self) -> AgentCapabilities;

    /// Opens a fresh hidden, nonpersistent isolated Agent session.
    ///
    /// # Errors
    /// Returns [`AgentEngineError`] if the fork SPI is unavailable or transport startup fails.
    fn open_session(&self) -> Result<Rc<dyn AgentSession>, AgentEngineError>;
}

/// Public snapshot of the front pending Agent approval prompt for native UI presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentApprovalPrompt {
    pub(crate) id: u64,
    pub(crate) window: WindowId,
    pub(crate) origin: Origin,
    pub(crate) message: String,
}

/// Outcome of dispatching an Agent operation in [`AgentCoordinator`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentDispatchEffect {
    None,
    ApprovalChanged,
}

struct PendingNavigateApproval {
    context_id: AgentContextId,
    url: Url,
    origin: Origin,
    wait_stage: AgentWaitStage,
    timeout: Duration,
    reply: HostReply<AgentNavigateResult>,
}

struct PendingAgentApproval {
    id: u64,
    client: ClientId,
    window: WindowId,
    origin: Origin,
    message: String,
    navigate: PendingNavigateApproval,
}

struct AgentContextRecord {
    owner: ClientId,
    epoch: Cell<u64>,
    viewport: AgentViewport,
    approved_origin: RefCell<Option<Origin>>,
    current_origin: RefCell<Option<String>>,
    capabilities: DiagnosticCapabilities,
    events: RefCell<AgentEventBuffer>,
    native_overflow: Cell<bool>,
    closed_stream: Cell<bool>,
    session: Rc<dyn AgentSession>,
    diagnostic_lease: Option<DiagnosticLease>,
}

impl AgentContextRecord {
    fn current_epoch(&self) -> u64 {
        self.epoch.get()
    }

    fn bump_epoch(&self) -> u64 {
        let next = self.epoch.get().saturating_add(1);
        self.epoch.set(next);
        next
    }
}

/// Main-thread state machine managing isolated `Profile::Agent` contexts for all MCP clients.
pub(crate) struct AgentCoordinator {
    next_context_id: u64,
    next_approval_id: u64,
    contexts: HashMap<AgentContextId, Rc<AgentContextRecord>>,
    pending_approvals: VecDeque<PendingAgentApproval>,
}

impl Default for AgentCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentCoordinator {
    /// Creates an empty Agent coordinator.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            next_context_id: 0,
            next_approval_id: 0,
            contexts: HashMap::new(),
            pending_approvals: VecDeque::new(),
        }
    }

    /// Returns the front pending Agent approval prompt, pruning closed replies first.
    pub(crate) fn front_approval(&mut self) -> Option<AgentApprovalPrompt> {
        self.prune_closed_approvals();
        self.pending_approvals
            .front()
            .map(|item| AgentApprovalPrompt {
                id: item.id,
                window: item.window,
                origin: item.origin.clone(),
                message: item.message.clone(),
            })
    }

    /// Handles a typed [`AgentOperation`] on the main thread.
    ///
    /// Pass `approval_window: Some(window)` in GUI mode when a local window is available to host
    /// the native `DialogSheet`; pass `None` in standalone mode (or when no GUI window exists) so
    /// unapproved operations deterministically surface `approval_required`.
    pub(crate) fn handle_operation(
        &mut self,
        client: ClientId,
        operation: AgentOperation,
        backend: &dyn AgentBackend,
        approval_window: Option<WindowId>,
    ) -> AgentDispatchEffect {
        let caps = backend.agent_capabilities();
        if !caps.isolated_automation {
            operation.reject(ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "isolated agent automation requires the Quickie WebKit fork",
            ));
            return AgentDispatchEffect::None;
        }

        match operation {
            AgentOperation::ContextOpen { viewport, reply } => {
                self.handle_context_open(client, viewport, reply, backend, caps);
                AgentDispatchEffect::None
            }
            AgentOperation::ContextList { reply } => {
                self.handle_context_list(client, reply);
                AgentDispatchEffect::None
            }
            AgentOperation::ContextClose { context_id, reply } => {
                self.handle_context_close(client, context_id, reply)
            }
            AgentOperation::Navigate {
                context_id,
                url,
                wait_stage,
                timeout_ms,
                reply,
            } => self.handle_navigate(
                client,
                context_id,
                url,
                wait_stage,
                Duration::from_millis(timeout_ms),
                approval_window,
                reply,
            ),
            AgentOperation::Locate {
                context_id,
                locator,
                max_results,
                reply,
            } => {
                self.handle_locate(client, context_id, &locator, max_results, reply);
                AgentDispatchEffect::None
            }
            AgentOperation::Act {
                context_id,
                action,
                reply,
            } => self.handle_act(client, context_id, action, approval_window, reply),
            AgentOperation::Wait {
                context_id,
                condition,
                timeout_ms,
                reply,
            } => {
                self.handle_wait(
                    client,
                    context_id,
                    &condition,
                    Duration::from_millis(timeout_ms),
                    reply,
                );
                AgentDispatchEffect::None
            }
            AgentOperation::Screenshot {
                context_id,
                target,
                max_bytes,
                reply,
            } => {
                self.handle_screenshot(client, context_id, target, max_bytes, reply);
                AgentDispatchEffect::None
            }
        }
    }

    /// Captures a bounded read-only snapshot from an isolated Agent context owned by `client`.
    pub(crate) fn handle_page_snapshot(
        &self,
        client: ClientId,
        context_id: AgentContextId,
        request: &SnapshotRequest,
        reply: HostReply<PageSnapshotResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let Some(record) = self.owned_context(client, context_id) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };

        if request.kind() == SnapshotKind::Accessibility && !record.capabilities.accessibility {
            reply.reject(ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "native accessibility snapshots are unavailable",
            ));
            return;
        }
        if request.kind() == SnapshotKind::Dom && !record.capabilities.dom {
            reply.reject(ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "DOM snapshots are unavailable on this engine",
            ));
            return;
        }

        let start_epoch = record.current_epoch();
        let kind = request.kind();
        let weak_record = Rc::downgrade(&record);
        let done = Box::new(move |result: Result<DiagnosticSnapshot, EngineError>| {
            if reply.is_closed() {
                return;
            }
            let Some(record) = weak_record.upgrade() else {
                reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "agent context closed before snapshot completed",
                ));
                return;
            };
            if !record.session.is_connected() || record.current_epoch() != start_epoch {
                reply.reject(ToolError::new(
                    ToolErrorCode::StaleDocument,
                    "agent document changed before snapshot completed",
                ));
                return;
            }
            match result {
                Ok(snap) => reply.resolve(PageSnapshotResult {
                    kind,
                    snapshot: BoundedSnapshot::new(start_epoch, &snap.data, snap.truncated),
                    accessibility_source: None,
                }),
                Err(error) => reply.reject(classify_engine_snapshot_error(&error)),
            }
        });

        if let Err(error) = record.session.diagnostic_snapshot(request, done) {
            let _ = error;
        }
    }

    /// Replays ordered diagnostic events from an isolated Agent context owned by `client`.
    pub(crate) fn handle_page_events(
        &self,
        client: ClientId,
        context_id: AgentContextId,
        cursor: u64,
        filter: &DiagnosticEventFilter,
        limit: usize,
        reply: HostReply<PageEventsResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let Some(record) = self.owned_context(client, context_id) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };

        match record.events.borrow().read(cursor, filter, limit) {
            Ok(batch) => reply.resolve(PageEventsResult {
                batch,
                capabilities: record.capabilities,
                native_overflow: record.native_overflow.get(),
                closed: record.closed_stream.get() || !record.session.is_connected(),
            }),
            Err(error) => reply.reject(error),
        }
    }

    /// Resolves a pending native approval prompt after the user clicked Allow or Cancel.
    pub(crate) fn resolve_approval(
        &mut self,
        approval_id: u64,
        approved_by_user: bool,
    ) -> AgentDispatchEffect {
        let Some(pos) = self
            .pending_approvals
            .iter()
            .position(|item| item.id == approval_id)
        else {
            return AgentDispatchEffect::None;
        };
        let Some(pending) = self.pending_approvals.remove(pos) else {
            return AgentDispatchEffect::None;
        };
        if pending.navigate.reply.is_closed() {
            return AgentDispatchEffect::ApprovalChanged;
        }

        let PendingNavigateApproval {
            context_id,
            url,
            origin,
            wait_stage,
            timeout,
            reply,
        } = pending.navigate;
        let Some(record) = self.owned_context(pending.client, context_id) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context closed before approval was confirmed",
            ));
            return AgentDispatchEffect::ApprovalChanged;
        };
        if !approved_by_user {
            reply.resolve(AgentNavigateResult {
                context_id,
                status: AgentNavigateStatus::Rejected,
                epoch: record.current_epoch(),
                origin: record.current_origin.borrow().clone(),
                reason: Some("origin navigation denied by user".to_owned()),
            });
            return AgentDispatchEffect::ApprovalChanged;
        }
        *record.approved_origin.borrow_mut() = Some(origin.clone());
        execute_navigate(context_id, &record, url, origin, wait_stage, timeout, reply);
        AgentDispatchEffect::ApprovalChanged
    }

    /// Destroys all isolated Agent contexts and pending approvals owned by a disconnected `client`.
    pub(crate) fn on_client_closed(&mut self, client: ClientId) -> bool {
        let owned_ids: Vec<AgentContextId> = self
            .contexts
            .iter()
            .filter(|(_, rec)| rec.owner == client)
            .map(|(&id, _)| id)
            .collect();
        let mut changed = !owned_ids.is_empty();
        for id in owned_ids {
            if let Some(rec) = self.contexts.remove(&id) {
                rec.session.set_event_observer(None);
                rec.session.close();
            }
        }
        let before = self.pending_approvals.len();
        self.pending_approvals.retain(|item| item.client != client);
        if self.pending_approvals.len() != before {
            changed = true;
        }
        changed
    }

    /// Destroys all isolated Agent contexts and rejects all pending approvals on shutdown.
    pub(crate) fn shutdown(&mut self) {
        for (_, rec) in self.contexts.drain() {
            rec.session.set_event_observer(None);
            rec.session.close();
        }
        for pending in self.pending_approvals.drain(..) {
            pending.navigate.reply.reject(ToolError::new(
                ToolErrorCode::Unavailable,
                "browser is shutting down",
            ));
        }
    }

    fn owned_context(
        &self,
        client: ClientId,
        context_id: AgentContextId,
    ) -> Option<Rc<AgentContextRecord>> {
        let rec = self.contexts.get(&context_id)?;
        if rec.owner != client || !rec.session.is_connected() {
            return None;
        }
        Some(Rc::clone(rec))
    }

    fn handle_context_open(
        &mut self,
        client: ClientId,
        viewport: AgentViewport,
        reply: HostReply<AgentContextOpenResult>,
        backend: &dyn AgentBackend,
        caps: AgentCapabilities,
    ) {
        if reply.is_closed() {
            return;
        }
        self.prune_disconnected_contexts();

        let client_count = self
            .contexts
            .values()
            .filter(|rec| rec.owner == client)
            .count();
        if client_count >= MAX_AGENT_CONTEXTS_PER_CLIENT
            || self.contexts.len() >= MAX_AGENT_CONTEXTS_TOTAL
        {
            reply.reject(ToolError::new(
                ToolErrorCode::Busy,
                "maximum isolated agent contexts reached",
            ));
            return;
        }

        let Some(next_id) = self.next_context_id.checked_add(1) else {
            reply.reject(ToolError::new(
                ToolErrorCode::Unavailable,
                "agent context identifier space exhausted",
            ));
            return;
        };
        let session = match backend.open_session() {
            Ok(session) => session,
            Err(error) => {
                reply.reject(error.into_tool_error());
                return;
            }
        };
        session.set_viewport(viewport);

        self.next_context_id = next_id;
        let context_id = AgentContextId::new(next_id);

        let record = Rc::new(AgentContextRecord {
            owner: client,
            epoch: Cell::new(1),
            viewport,
            approved_origin: RefCell::new(None),
            current_origin: RefCell::new(None),
            capabilities: caps.observations,
            events: RefCell::new(AgentEventBuffer::new()),
            native_overflow: Cell::new(false),
            closed_stream: Cell::new(false),
            session: Rc::clone(&session),
            diagnostic_lease: None,
        });

        let weak_for_events = Rc::downgrade(&record);
        session.set_event_observer(Some(Box::new(move |event| {
            let Some(rec) = weak_for_events.upgrade() else {
                return;
            };
            match event {
                AgentSessionEvent::ContextCleared => {
                    let _ = rec.bump_epoch();
                }
            }
        })));

        // Connect read-only diagnostic observer when the fork SPI is present on this runtime.
        let lease = if has_event_spi(caps.observations) {
            let weak_for_diag = Rc::downgrade(&record);
            session
                .observe_diagnostics(Box::new(move |signal| {
                    let Some(rec) = weak_for_diag.upgrade() else {
                        return;
                    };
                    match signal {
                        DiagnosticSignal::Event(data) => {
                            let epoch = rec.current_epoch();
                            rec.events.borrow_mut().push(epoch, data);
                        }
                        DiagnosticSignal::Gap(
                            DiagnosticGapReason::NativeQueueOverflow { .. }
                            | DiagnosticGapReason::RequestMapOverflow { .. },
                        ) => {
                            rec.native_overflow.set(true);
                        }
                        DiagnosticSignal::Gap(DiagnosticGapReason::ChannelClosed) => {
                            rec.closed_stream.set(true);
                        }
                    }
                }))
                .ok()
        } else {
            None
        };

        // Store the lease inside a final immutable record (or swap via Rc::get_mut before insert).
        let mut record = record;
        if let Some(mut_rec) = Rc::get_mut(&mut record) {
            mut_rec.diagnostic_lease = lease;
        } else {
            drop(lease);
        }

        self.contexts.insert(context_id, record);
        reply.resolve(AgentContextOpenResult {
            context_id,
            epoch: 1,
            viewport,
        });
    }

    fn handle_context_list(&mut self, client: ClientId, reply: HostReply<AgentContextListResult>) {
        if reply.is_closed() {
            return;
        }
        self.prune_disconnected_contexts();
        let mut contexts: Vec<AgentContextSummary> = self
            .contexts
            .iter()
            .filter(|(_, rec)| rec.owner == client)
            .map(|(&context_id, rec)| AgentContextSummary {
                context_id,
                epoch: rec.current_epoch(),
                origin: rec.current_origin.borrow().clone(),
                viewport: rec.viewport,
            })
            .collect();
        contexts.sort_by_key(|summary| summary.context_id);
        reply.resolve(AgentContextListResult { contexts });
    }

    fn handle_context_close(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        reply: HostReply<AgentContextCloseResult>,
    ) -> AgentDispatchEffect {
        if reply.is_closed() {
            return AgentDispatchEffect::None;
        }
        let is_owner = self
            .contexts
            .get(&context_id)
            .is_some_and(|rec| rec.owner == client);
        if !is_owner {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return AgentDispatchEffect::None;
        }
        if let Some(rec) = self.contexts.remove(&context_id) {
            rec.session.set_event_observer(None);
            rec.session.close();
        }
        let approval_changed = self.reject_approvals_for_context(
            context_id,
            ToolErrorCode::AttachmentUnavailable,
            "agent context was closed",
        );
        reply.resolve(AgentContextCloseResult {
            context_id,
            closed: true,
        });
        if approval_changed {
            AgentDispatchEffect::ApprovalChanged
        } else {
            AgentDispatchEffect::None
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "navigation dispatch takes typed wire fields plus host approval window"
    )]
    fn handle_navigate(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        url: Url,
        wait_stage: AgentWaitStage,
        timeout: Duration,
        approval_window: Option<WindowId>,
        reply: HostReply<AgentNavigateResult>,
    ) -> AgentDispatchEffect {
        if reply.is_closed() {
            return AgentDispatchEffect::None;
        }
        let Some(record) = self.owned_context(client, context_id) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return AgentDispatchEffect::None;
        };

        let approved = record.approved_origin.borrow().clone();
        match evaluate_navigation_policy(approved.as_ref(), &url) {
            ApprovalDecision::Allowed => {
                let origin = url.origin();
                execute_navigate(context_id, &record, url, origin, wait_stage, timeout, reply);
                AgentDispatchEffect::None
            }
            ApprovalDecision::Rejected { reason } => {
                reply.resolve(AgentNavigateResult {
                    context_id,
                    status: AgentNavigateStatus::Rejected,
                    epoch: record.current_epoch(),
                    origin: record.current_origin.borrow().clone(),
                    reason: Some(reason),
                });
                AgentDispatchEffect::None
            }
            ApprovalDecision::RequiresApproval { origin, message } => {
                let Some(window) = approval_window else {
                    reply.resolve(AgentNavigateResult {
                        context_id,
                        status: AgentNavigateStatus::ApprovalRequired,
                        epoch: record.current_epoch(),
                        origin: record.current_origin.borrow().clone(),
                        reason: Some(
                            "explicit user approval is required before navigating to this origin"
                                .to_owned(),
                        ),
                    });
                    return AgentDispatchEffect::None;
                };
                self.enqueue_approval(
                    client,
                    window,
                    origin.clone(),
                    message,
                    PendingNavigateApproval {
                        context_id,
                        url,
                        origin,
                        wait_stage,
                        timeout,
                        reply,
                    },
                )
            }
        }
    }

    fn handle_locate(
        &self,
        client: ClientId,
        context_id: AgentContextId,
        locator: &AgentLocator,
        max_results: usize,
        reply: HostReply<AgentLocateResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let Some(record) = self.owned_context(client, context_id) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };

        let start_epoch = record.current_epoch();
        let locator_kind = locator.kind();
        let weak_record = Rc::downgrade(&record);
        record.session.locate_elements(
            locator,
            max_results,
            Box::new(move |result| {
                if reply.is_closed() {
                    return;
                }
                let Some(rec) = weak_record.upgrade() else {
                    reply.reject(ToolError::new(
                        ToolErrorCode::AttachmentUnavailable,
                        "agent context closed before locate completed",
                    ));
                    return;
                };
                if !rec.session.is_connected() || rec.current_epoch() != start_epoch {
                    reply.reject(ToolError::new(
                        ToolErrorCode::StaleDocument,
                        "agent document changed before locate completed",
                    ));
                    return;
                }
                match result {
                    Ok(output) => {
                        let origin = rec
                            .current_origin
                            .borrow()
                            .clone()
                            .unwrap_or_else(|| "null".to_owned());
                        let elements = output
                            .elements
                            .into_iter()
                            .map(|item| LocatedElement {
                                element_ref: AgentElementRef {
                                    context_id,
                                    epoch: start_epoch,
                                    element_id: item.element_id,
                                },
                                role: item.role,
                                name: item.name,
                                tag: item.tag,
                                bounds: item.bounds.filter(|b| b.is_valid()),
                                provenance: ElementProvenance {
                                    origin: origin.clone(),
                                    frame: FrameScope::MainFrame,
                                    locator_kind,
                                },
                            })
                            .collect();
                        reply.resolve(AgentLocateResult {
                            context_id,
                            epoch: start_epoch,
                            elements,
                            truncated: output.truncated,
                        });
                    }
                    Err(error) => reply.reject(error.into_tool_error()),
                }
            }),
        );
    }

    fn handle_act(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        action: AgentAction,
        approval_window: Option<WindowId>,
        reply: HostReply<AgentActResult>,
    ) -> AgentDispatchEffect {
        if reply.is_closed() {
            return AgentDispatchEffect::None;
        }
        let Some(record) = self.owned_context(client, context_id) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return AgentDispatchEffect::None;
        };

        let current_epoch = record.current_epoch();
        if let Some(element_ref) = action_element_ref(&action) {
            if element_ref.context_id != context_id {
                reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "element reference belongs to a different agent context",
                ));
                return AgentDispatchEffect::None;
            }
            if element_ref.epoch != current_epoch {
                reply.reject(ToolError::new(
                    ToolErrorCode::StaleDocument,
                    "element reference is from a stale document epoch",
                ));
                return AgentDispatchEffect::None;
            }
        }

        // Targeted element actions resolve live typed containment signals first; untargeted
        // `Type` / `PressKey` inspect the currently focused element; untargeted `Scroll` proceeds
        // directly to viewport center.
        let target_element_id = action_element_ref(&action).map(|r| r.element_id.clone());
        let needs_focused = target_element_id.is_none()
            && matches!(
                action,
                AgentAction::Type { .. } | AgentAction::PressKey { .. }
            );

        if target_element_id.is_none() && !needs_focused {
            let viewport_center = AgentSignals {
                connected: true,
                visible: true,
                action_point: Some((
                    f64::from(record.viewport.width) / 2.0,
                    f64::from(record.viewport.height) / 2.0,
                )),
                ..AgentSignals::default()
            };
            execute_act(
                context_id,
                &record,
                current_epoch,
                action,
                &viewport_center,
                reply,
            );
            return AgentDispatchEffect::None;
        }

        // Resolve typed containment signals asynchronously before performing trusted input.
        // Irreversible or cross-origin actions deterministically surface `ApprovalRequired`.
        let has_target_element = target_element_id.is_some();
        let weak_record = Rc::downgrade(&record);
        let _ = approval_window;
        let on_signals = Box::new(move |signals_res| {
            handle_act_signals(
                context_id,
                current_epoch,
                has_target_element,
                action,
                &weak_record,
                signals_res,
                reply,
            );
        });

        if let Some(element_id) = target_element_id.as_deref() {
            record.session.resolve_element(element_id, true, on_signals);
        } else {
            record.session.inspect_focused(on_signals);
        }
        AgentDispatchEffect::None
    }

    fn handle_wait(
        &self,
        client: ClientId,
        context_id: AgentContextId,
        condition: &AgentWaitCondition,
        timeout: Duration,
        reply: HostReply<AgentWaitResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let Some(record) = self.owned_context(client, context_id) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };
        if matches!(
            condition,
            AgentWaitCondition::LoadStage {
                stage: AgentWaitStage::None
            }
        ) {
            reply.reject(ToolError::new(
                ToolErrorCode::InvalidParams,
                "wait load_stage must be dom_content_loaded or load",
            ));
            return;
        }

        let started = Instant::now();
        let weak_record = Rc::downgrade(&record);
        record.session.wait_for(
            condition,
            timeout,
            Box::new(move |result| {
                if reply.is_closed() {
                    return;
                }
                let Some(rec) = weak_record.upgrade() else {
                    reply.reject(ToolError::new(
                        ToolErrorCode::AttachmentUnavailable,
                        "agent context closed during wait",
                    ));
                    return;
                };
                if !rec.session.is_connected() {
                    reply.reject(ToolError::new(
                        ToolErrorCode::AttachmentUnavailable,
                        "agent context disconnected during wait",
                    ));
                    return;
                }
                let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                match result {
                    Ok(status) => reply.resolve(AgentWaitResult {
                        context_id,
                        epoch: rec.current_epoch(),
                        status,
                        elapsed_ms,
                    }),
                    Err(error) => reply.reject(error.into_tool_error()),
                }
            }),
        );
    }

    fn handle_screenshot(
        &self,
        client: ClientId,
        context_id: AgentContextId,
        target: AgentScreenshotTarget,
        max_bytes: usize,
        reply: HostReply<AgentScreenshotResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let Some(record) = self.owned_context(client, context_id) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };

        let start_epoch = record.current_epoch();
        if let AgentScreenshotTarget::Element(element_ref) = target {
            if element_ref.context_id != context_id {
                reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "element reference belongs to a different agent context",
                ));
                return;
            }
            if element_ref.epoch != start_epoch {
                reply.reject(ToolError::new(
                    ToolErrorCode::StaleDocument,
                    "element reference is from a stale document epoch",
                ));
                return;
            }
            reply.reject(ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "element-scoped screenshots are unavailable on this engine; use viewport target",
            ));
            return;
        }

        let weak_record = Rc::downgrade(&record);
        record.session.screenshot_viewport(
            max_bytes,
            DEFAULT_INPUT_TIMEOUT,
            Box::new(move |result| {
                if reply.is_closed() {
                    return;
                }
                let Some(rec) = weak_record.upgrade() else {
                    reply.reject(ToolError::new(
                        ToolErrorCode::AttachmentUnavailable,
                        "agent context closed before screenshot completed",
                    ));
                    return;
                };
                if !rec.session.is_connected() || rec.current_epoch() != start_epoch {
                    reply.reject(ToolError::new(
                        ToolErrorCode::StaleDocument,
                        "agent document changed before screenshot completed",
                    ));
                    return;
                }
                match result {
                    Ok(base64) => match decode_png_base64(&base64, max_bytes) {
                        Ok((png_bytes, width, height)) => reply.resolve(AgentScreenshotResult {
                            context_id,
                            epoch: start_epoch,
                            width,
                            height,
                            png_bytes,
                        }),
                        Err(err) => reply.reject(err),
                    },
                    Err(err) => reply.reject(err.into_tool_error()),
                }
            }),
        );
    }

    fn enqueue_approval(
        &mut self,
        client: ClientId,
        window: WindowId,
        origin: Origin,
        message: String,
        navigate: PendingNavigateApproval,
    ) -> AgentDispatchEffect {
        self.prune_closed_approvals();
        if self.pending_approvals.len() >= MAX_PENDING_AGENT_APPROVALS {
            navigate.reply.reject(ToolError::new(
                ToolErrorCode::Busy,
                "too many pending agent approval requests",
            ));
            return AgentDispatchEffect::None;
        }
        self.next_approval_id = self.next_approval_id.saturating_add(1);
        self.pending_approvals.push_back(PendingAgentApproval {
            id: self.next_approval_id,
            client,
            window,
            origin,
            message,
            navigate,
        });
        AgentDispatchEffect::ApprovalChanged
    }

    fn prune_closed_approvals(&mut self) {
        self.pending_approvals
            .retain(|item| !item.navigate.reply.is_closed());
    }

    fn prune_disconnected_contexts(&mut self) {
        self.contexts.retain(|_, rec| rec.session.is_connected());
    }

    fn reject_approvals_for_context(
        &mut self,
        context_id: AgentContextId,
        code: ToolErrorCode,
        message: &str,
    ) -> bool {
        let mut kept = VecDeque::with_capacity(self.pending_approvals.len());
        let mut removed = false;
        for pending in self.pending_approvals.drain(..) {
            if pending.navigate.context_id == context_id {
                pending.navigate.reply.reject(ToolError::new(code, message));
                removed = true;
            } else {
                kept.push_back(pending);
            }
        }
        self.pending_approvals = kept;
        removed
    }
}

fn execute_navigate(
    context_id: AgentContextId,
    record: &Rc<AgentContextRecord>,
    url: Url,
    origin: Origin,
    wait_stage: AgentWaitStage,
    timeout: Duration,
    reply: HostReply<AgentNavigateResult>,
) {
    let weak_record = Rc::downgrade(record);
    let expected_origin_str = origin.ascii_serialization();
    record.session.navigate(
        url,
        origin,
        wait_stage,
        timeout,
        Box::new(move |result| {
            if reply.is_closed() {
                return;
            }
            let Some(rec) = weak_record.upgrade() else {
                reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "agent context closed during navigation",
                ));
                return;
            };
            if !rec.session.is_connected() {
                reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "agent context disconnected during navigation",
                ));
                return;
            }
            match result {
                Ok(()) => {
                    let epoch = rec.bump_epoch();
                    let committed_origin = rec
                        .session
                        .current_url()
                        .filter(policy::is_valid_http_url)
                        .map_or(expected_origin_str, |u| u.origin().ascii_serialization());
                    *rec.current_origin.borrow_mut() = Some(committed_origin.clone());
                    reply.resolve(AgentNavigateResult {
                        context_id,
                        status: AgentNavigateStatus::Committed,
                        epoch,
                        origin: Some(committed_origin),
                        reason: None,
                    });
                }
                Err(AgentEngineError::OriginMismatch) => {
                    reply.resolve(AgentNavigateResult {
                        context_id,
                        status: AgentNavigateStatus::Rejected,
                        epoch: rec.current_epoch(),
                        origin: rec.current_origin.borrow().clone(),
                        reason: Some(
                            "navigation redirected outside the approved origin".to_owned(),
                        ),
                    });
                }
                Err(AgentEngineError::Failed(reason)) => {
                    reply.resolve(AgentNavigateResult {
                        context_id,
                        status: AgentNavigateStatus::Rejected,
                        epoch: rec.current_epoch(),
                        origin: rec.current_origin.borrow().clone(),
                        reason: Some(reason),
                    });
                }
                Err(error) => reply.reject(error.into_tool_error()),
            }
        }),
    );
}

fn handle_act_signals(
    context_id: AgentContextId,
    current_epoch: u64,
    has_target_element: bool,
    action: AgentAction,
    weak_record: &std::rc::Weak<AgentContextRecord>,
    signals_res: Result<AgentSignals, AgentEngineError>,
    reply: HostReply<AgentActResult>,
) {
    if reply.is_closed() {
        return;
    }
    let Some(rec) = weak_record.upgrade() else {
        reply.reject(ToolError::new(
            ToolErrorCode::AttachmentUnavailable,
            "agent context closed before action started",
        ));
        return;
    };
    if !rec.session.is_connected() || rec.current_epoch() != current_epoch {
        reply.reject(ToolError::new(
            ToolErrorCode::StaleDocument,
            "agent document changed before action started",
        ));
        return;
    }
    let signals = match signals_res {
        Ok(sig) => sig,
        Err(err) => {
            reply.reject(err.into_tool_error());
            return;
        }
    };
    if !signals.connected {
        if has_target_element {
            reply.reject(ToolError::new(
                ToolErrorCode::StaleDocument,
                "element reference is no longer connected to the document",
            ));
        } else {
            reply.resolve(AgentActResult {
                context_id,
                status: AgentActStatus::Rejected,
                epoch: rec.current_epoch(),
                origin: rec.current_origin.borrow().clone(),
                reason: Some("no focused element for untargeted keyboard input".to_owned()),
                observed_state: None,
            });
        }
        return;
    }

    let approved_origin = rec.approved_origin.borrow().clone();
    match evaluate_action_policy(&action, &signals, approved_origin.as_ref()) {
        ApprovalDecision::Allowed => {
            execute_act(context_id, &rec, current_epoch, action, &signals, reply);
        }
        ApprovalDecision::Rejected { reason } => {
            reply.resolve(AgentActResult {
                context_id,
                status: AgentActStatus::Rejected,
                epoch: rec.current_epoch(),
                origin: rec.current_origin.borrow().clone(),
                reason: Some(reason),
                observed_state: None,
            });
        }
        ApprovalDecision::RequiresApproval { message, .. } => {
            reply.resolve(AgentActResult {
                context_id,
                status: AgentActStatus::ApprovalRequired,
                epoch: rec.current_epoch(),
                origin: rec.current_origin.borrow().clone(),
                reason: Some(message),
                observed_state: None,
            });
        }
    }
}

fn finish_act(
    context_id: AgentContextId,
    start_epoch: u64,
    weak_record: std::rc::Weak<AgentContextRecord>,
    reply: HostReply<AgentActResult>,
) -> Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static> {
    Box::new(move |outcome| {
        if reply.is_closed() {
            return;
        }
        let Some(rec) = weak_record.upgrade() else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context closed during action",
            ));
            return;
        };
        if !rec.session.is_connected() {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context disconnected during action",
            ));
            return;
        }
        if let Err(err) = outcome {
            reply.reject(err.into_tool_error());
            return;
        }
        let weak_obs = Rc::downgrade(&rec);
        rec.session.observe_state(Box::new(move |obs| {
            if reply.is_closed() {
                return;
            }
            let Some(rec) = weak_obs.upgrade() else {
                reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "agent context closed after action",
                ));
                return;
            };
            let current_epoch = rec.current_epoch();
            let observed_state = obs.ok().map(|o| AgentObservedState {
                navigated: current_epoch != start_epoch,
                focused_role: o.focused_role,
                value_length: o.value_length,
                scroll_x: o.scroll_x,
                scroll_y: o.scroll_y,
            });
            reply.resolve(AgentActResult {
                context_id,
                status: AgentActStatus::Performed,
                epoch: current_epoch,
                origin: rec.current_origin.borrow().clone(),
                reason: None,
                observed_state,
            });
        }));
    })
}

fn focus_then(
    session: &Rc<dyn AgentSession>,
    point: Option<(f64, f64)>,
    finish: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
    next: impl FnOnce(Rc<dyn AgentSession>, Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>)
    + 'static,
) {
    let Some((x, y)) = point else {
        finish(Err(AgentEngineError::Failed(
            "target element has no visible viewport geometry".to_owned(),
        )));
        return;
    };
    let session_clone = Rc::clone(session);
    session.mouse_action(
        MouseActionRequest {
            x,
            y,
            kind: MouseActionKind::Click,
            button: AgentMouseButton::Left,
            modifiers: Vec::new(),
            timeout: DEFAULT_INPUT_TIMEOUT,
        },
        Box::new(move |click_res| match click_res {
            Ok(()) => next(session_clone, finish),
            Err(err) => finish(Err(err)),
        }),
    );
}

fn run_mouse(
    session: &Rc<dyn AgentSession>,
    point: Option<(f64, f64)>,
    kind: MouseActionKind,
    button: AgentMouseButton,
    modifiers: Vec<AgentKeyModifier>,
    finish: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
) {
    let Some((x, y)) = point else {
        return finish(Err(AgentEngineError::Failed(
            "target element has no visible viewport geometry".to_owned(),
        )));
    };
    session.mouse_action(
        MouseActionRequest {
            x,
            y,
            kind,
            button,
            modifiers,
            timeout: DEFAULT_INPUT_TIMEOUT,
        },
        finish,
    );
}

fn execute_act(
    context_id: AgentContextId,
    record: &Rc<AgentContextRecord>,
    start_epoch: u64,
    action: AgentAction,
    signals: &AgentSignals,
    reply: HostReply<AgentActResult>,
) {
    let finish = finish_act(context_id, start_epoch, Rc::downgrade(record), reply);
    match action {
        AgentAction::Click {
            button, modifiers, ..
        } => run_mouse(
            &record.session,
            signals.action_point,
            MouseActionKind::Click,
            button,
            modifiers,
            finish,
        ),
        AgentAction::DoubleClick {
            button, modifiers, ..
        } => run_mouse(
            &record.session,
            signals.action_point,
            MouseActionKind::DoubleClick,
            button,
            modifiers,
            finish,
        ),
        AgentAction::Hover { .. } => run_mouse(
            &record.session,
            signals.action_point,
            MouseActionKind::Hover,
            AgentMouseButton::Left,
            Vec::new(),
            finish,
        ),
        AgentAction::Fill { text, .. } => {
            focus_then(
                &record.session,
                signals.action_point,
                finish,
                move |s, f| {
                    s.fill_text(text, DEFAULT_INPUT_TIMEOUT, f);
                },
            );
        }
        AgentAction::Type { element, text } => {
            if element.is_some() {
                focus_then(
                    &record.session,
                    signals.action_point,
                    finish,
                    move |s, f| {
                        s.type_text(text, DEFAULT_INPUT_TIMEOUT, f);
                    },
                );
            } else {
                record
                    .session
                    .type_text(text, DEFAULT_INPUT_TIMEOUT, finish);
            }
        }
        AgentAction::PressKey {
            element,
            key,
            modifiers,
        } => {
            if element.is_some() {
                focus_then(
                    &record.session,
                    signals.action_point,
                    finish,
                    move |s, f| {
                        s.press_key(key, modifiers, DEFAULT_INPUT_TIMEOUT, f);
                    },
                );
            } else {
                record
                    .session
                    .press_key(key, modifiers, DEFAULT_INPUT_TIMEOUT, finish);
            }
        }
        AgentAction::Scroll {
            delta_x, delta_y, ..
        } => {
            let (x, y) = signals.action_point.unwrap_or((
                f64::from(record.viewport.width) / 2.0,
                f64::from(record.viewport.height) / 2.0,
            ));
            record
                .session
                .scroll_wheel(x, y, delta_x, delta_y, DEFAULT_INPUT_TIMEOUT, finish);
        }
        AgentAction::SelectOption { element, option } => {
            record
                .session
                .select_option(&element.element_id, &option, finish);
        }
    }
}

fn action_element_ref(action: &AgentAction) -> Option<&AgentElementRef> {
    match action {
        AgentAction::Click { element, .. }
        | AgentAction::DoubleClick { element, .. }
        | AgentAction::Hover { element }
        | AgentAction::Fill { element, .. }
        | AgentAction::SelectOption { element, .. } => Some(element),
        AgentAction::Type { element, .. }
        | AgentAction::PressKey { element, .. }
        | AgentAction::Scroll { element, .. } => element.as_ref(),
    }
}

fn has_event_spi(caps: DiagnosticCapabilities) -> bool {
    caps.console
        || caps.runtime_errors
        || caps.pending_network
        || caps.completed_network
        || caps.lifecycle
}

fn classify_engine_snapshot_error(error: &EngineError) -> ToolError {
    let lower = error.0.to_ascii_lowercase();
    if lower.contains("selector") {
        return ToolError::new(ToolErrorCode::InvalidParams, &error.0);
    }
    if lower.contains("unavailable") {
        return ToolError::new(ToolErrorCode::UnsupportedCapability, &error.0);
    }
    ToolError::new(ToolErrorCode::Unavailable, &error.0)
}
