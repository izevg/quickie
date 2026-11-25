//! Typed fake [`BrowserHost`] backed by real [`DiagnosticState`] for MCP protocol tests.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use quickie_application::mcp::{
    AgentContextId, BoundedSnapshot, ClientId, DiagnosticCapabilities, DiagnosticEvent,
    DiagnosticEventBatch, DiagnosticEventData, DiagnosticState, DocumentBinding,
    LifecycleEventKind, ObservationTarget, SnapshotKind,
};
use quickie_domain_browsing::{Profile, TabId};
use quickie_mcp::browser::{
    AgentActResult, AgentActStatus, AgentAction, AgentContextCloseResult, AgentContextListResult,
    AgentContextOpenResult, AgentContextSummary, AgentElementRef, AgentLocateResult, AgentLocator,
    AgentNavigateResult, AgentNavigateStatus, AgentObservedState, AgentOperation,
    AgentScreenshotResult, AgentScreenshotTarget, AgentViewport, AgentWaitCondition,
    AgentWaitResult, AgentWaitStatus, ElementBounds, ElementProvenance, FrameScope, LocatedElement,
};
use quickie_mcp::{
    AccessibilitySource, BrowserHost, BrowserOperation, BrowserStatus, DiagnosticDetachResult,
    DiagnosticGrant, ForkCapabilities, HostDispatchError, HostPlatform, HostReply,
    PageEventsResult, PageSnapshotResult, RuntimeMode, ToolError, ToolErrorCode,
};
use url::Url;

/// Approved localhost-only origin used by the fake isolated Agent fixture (zero personal data).
pub const FAKE_AGENT_APPROVED_ORIGIN: &str = "http://127.0.0.1:4173";

/// Minimal valid 1x1 transparent PNG payload (67 bytes, real PNG signature + IHDR/IDAT/IEND).
pub const MINIMAL_VALID_PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D', b'R',
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, b'I', b'D', b'A', b'T', 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae,
    0x42, 0x60, 0x82,
];

/// Scripted behavior for the next `diagnostic_request`.
#[derive(Clone, Debug)]
pub enum ConsentDecision {
    /// Approve diagnostics on the given `DocumentBinding` with these capabilities.
    Approve {
        binding: DocumentBinding,
        capabilities: DiagnosticCapabilities,
    },
    /// Deny diagnostic consent with a neutral error.
    Deny,
}

/// A parked status host reply held until a test triggers cancellation, timeout, or disconnect.
pub struct ParkedReply(HostReply<BrowserStatus>);

impl ParkedReply {
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    pub fn resolve_late(self) {
        self.0.resolve(default_status());
    }
}

/// A parked `agent_wait` reply held until a test triggers cancellation, timeout, or disconnect.
pub struct ParkedAgentWait(HostReply<AgentWaitResult>, AgentContextId);

impl ParkedAgentWait {
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    pub fn resolve_late(self) {
        self.0.resolve(AgentWaitResult {
            context_id: self.1,
            epoch: 2,
            status: AgentWaitStatus::Matched,
            elapsed_ms: 10,
        });
    }
}

#[derive(Clone, Debug)]
struct FakeAgentContext {
    owner: ClientId,
    viewport: AgentViewport,
    epoch: u64,
    origin: Option<String>,
    approved_origin: String,
    scroll_x: i32,
    scroll_y: i32,
}

#[derive(Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "test fixture toggles independent scripted host behaviors"
)]
struct FakeHostState {
    diagnostic: DiagnosticState,
    capabilities: DiagnosticCapabilities,
    agent_automation: bool,
    unsupported_select_option: bool,
    screenshot_override: Option<Vec<u8>>,
    consent_decision: Option<ConsentDecision>,
    dispatch_override: Option<HostDispatchError>,
    park_next_status: bool,
    park_next_agent_wait: bool,
    drop_next_status: bool,
    snapshot_payload: String,
    accessibility_source: Option<AccessibilitySource>,
    native_overflow: bool,
    next_agent_context_id: u64,
    agent_contexts: BTreeMap<AgentContextId, FakeAgentContext>,
    dispatched_clients: Vec<ClientId>,
    closed_clients: Vec<ClientId>,
    parked: VecDeque<ParkedReply>,
    parked_agent_waits: VecDeque<ParkedAgentWait>,
}

/// Thread-safe fake host implementing [`BrowserHost`] for protocol integration tests.
pub struct FakeBrowserHost {
    state: Mutex<FakeHostState>,
}

impl FakeBrowserHost {
    pub fn new() -> Self {
        let capabilities = DiagnosticCapabilities {
            dom: true,
            accessibility: true,
            console: true,
            runtime_errors: true,
            pending_network: true,
            completed_network: true,
            lifecycle: true,
        };
        let state = FakeHostState {
            capabilities,
            snapshot_payload: "<main>synthetic-control</main>".to_owned(),
            accessibility_source: Some(AccessibilitySource::Native),
            next_agent_context_id: 1,
            ..FakeHostState::default()
        };
        Self {
            state: Mutex::new(state),
        }
    }

    pub fn set_agent_automation(&self, enabled: bool) {
        self.state.lock().expect("lock host").agent_automation = enabled;
    }

    pub fn set_unsupported_select_option(&self, unsupported: bool) {
        self.state
            .lock()
            .expect("lock host")
            .unsupported_select_option = unsupported;
    }

    pub fn set_screenshot_override(&self, bytes: Option<Vec<u8>>) {
        self.state.lock().expect("lock host").screenshot_override = bytes;
    }

    pub fn set_consent(&self, decision: ConsentDecision) {
        self.state.lock().expect("lock host").consent_decision = Some(decision);
    }

    pub fn set_dispatch_error(&self, error: Option<HostDispatchError>) {
        self.state.lock().expect("lock host").dispatch_override = error;
    }

    pub fn set_park_next_status(&self, park: bool) {
        self.state.lock().expect("lock host").park_next_status = park;
    }

    pub fn set_park_next_agent_wait(&self, park: bool) {
        self.state.lock().expect("lock host").park_next_agent_wait = park;
    }

    pub fn set_drop_next_status(&self, drop_reply: bool) {
        self.state.lock().expect("lock host").drop_next_status = drop_reply;
    }

    pub fn set_native_overflow(&self, overflow: bool) {
        self.state.lock().expect("lock host").native_overflow = overflow;
    }

    pub fn record_event(
        &self,
        attachment: quickie_application::mcp::AttachmentId,
        binding: &DocumentBinding,
        data: DiagnosticEventData,
    ) {
        let mut guard = self.state.lock().expect("lock host");
        guard
            .diagnostic
            .record_event(attachment, binding, data)
            .expect("record diagnostic event");
    }

    pub fn dispatched_clients(&self) -> Vec<ClientId> {
        self.state
            .lock()
            .expect("lock host")
            .dispatched_clients
            .clone()
    }

    pub fn closed_clients(&self) -> Vec<ClientId> {
        self.state.lock().expect("lock host").closed_clients.clone()
    }

    pub fn active_attachments(&self) -> usize {
        self.state
            .lock()
            .expect("lock host")
            .diagnostic
            .active_count()
    }

    pub fn active_agent_contexts(&self) -> usize {
        self.state.lock().expect("lock host").agent_contexts.len()
    }

    pub fn take_parked(&self) -> Option<ParkedReply> {
        self.state.lock().expect("lock host").parked.pop_front()
    }

    pub fn wait_for_parked(&self, timeout: std::time::Duration) -> ParkedReply {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(parked) = self.take_parked() {
                return parked;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for parked host reply"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    pub fn wait_for_parked_agent_wait(&self, timeout: std::time::Duration) -> ParkedAgentWait {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(parked) = self
                .state
                .lock()
                .expect("lock host")
                .parked_agent_waits
                .pop_front()
            {
                return parked;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for parked agent_wait reply"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    pub fn wait_for_client_closed(&self, client: ClientId, timeout: std::time::Duration) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.closed_clients().contains(&client) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for client_closed({client})"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

impl FakeHostState {
    fn handle_status(&mut self, reply: HostReply<BrowserStatus>) {
        if self.drop_next_status {
            self.drop_next_status = false;
            drop(reply);
            return;
        }
        if self.park_next_status {
            self.park_next_status = false;
            self.parked.push_back(ParkedReply(reply));
            return;
        }
        let mut status = default_status();
        status.fork_capabilities.diagnostic = self.capabilities;
        status.fork_capabilities.agent_automation = self.agent_automation;
        reply.resolve(status);
    }

    fn handle_diagnostic_request(
        &mut self,
        client: ClientId,
        scope: quickie_application::mcp::GrantScope,
        reply: HostReply<DiagnosticGrant>,
    ) {
        let Some(decision) = self.consent_decision.clone() else {
            reply.reject(ToolError::new(
                ToolErrorCode::ApprovalDenied,
                "diagnostic consent denied by user",
            ));
            return;
        };
        match decision {
            ConsentDecision::Deny => {
                reply.reject(ToolError::new(
                    ToolErrorCode::ApprovalDenied,
                    "diagnostic consent denied by user",
                ));
            }
            ConsentDecision::Approve {
                binding,
                capabilities,
            } => {
                let live = binding.clone();
                match self
                    .diagnostic
                    .enable(client, binding.clone(), Some(&live), scope)
                {
                    Ok(enabled) => {
                        self.capabilities = capabilities;
                        reply.resolve(DiagnosticGrant {
                            attachment_id: enabled.attachment,
                            scope,
                            epoch: binding.epoch,
                            origin: binding.origin.ascii_serialization(),
                            capabilities,
                        });
                    }
                    Err(err) => reply.reject(ToolError::from(err)),
                }
            }
        }
    }

    fn handle_page_snapshot(
        &self,
        client: ClientId,
        target: ObservationTarget,
        request: &quickie_application::mcp::SnapshotRequest,
        reply: HostReply<PageSnapshotResult>,
    ) {
        match target {
            ObservationTarget::Diagnostic(attachment) => {
                match self.diagnostic.binding_for(client, attachment).cloned() {
                    Ok(binding) => match self.diagnostic.complete_snapshot(
                        attachment,
                        &binding,
                        &self.snapshot_payload,
                        false,
                    ) {
                        Ok(snapshot) => {
                            let accessibility_source =
                                if request.kind() == SnapshotKind::Accessibility {
                                    self.accessibility_source
                                } else {
                                    None
                                };
                            reply.resolve(PageSnapshotResult {
                                kind: request.kind(),
                                snapshot,
                                accessibility_source,
                            });
                        }
                        Err(err) => reply.reject(ToolError::from(err)),
                    },
                    Err(err) => reply.reject(ToolError::from(err)),
                }
            }
            ObservationTarget::Agent(context_id) => {
                let Some(ctx) = self
                    .agent_contexts
                    .get(&context_id)
                    .filter(|c| c.owner == client)
                else {
                    reply.reject(ToolError::new(
                        ToolErrorCode::AttachmentUnavailable,
                        "agent context is not available",
                    ));
                    return;
                };
                let accessibility_source = if request.kind() == SnapshotKind::Accessibility {
                    Some(AccessibilitySource::SemanticDomFallback)
                } else {
                    None
                };
                reply.resolve(PageSnapshotResult {
                    kind: request.kind(),
                    snapshot: BoundedSnapshot::new(ctx.epoch, "agent-context-text", false),
                    accessibility_source,
                });
            }
        }
    }

    fn handle_agent(&mut self, client: ClientId, operation: AgentOperation) {
        if !self.agent_automation {
            operation.reject(ToolError::new(
                ToolErrorCode::ToolNotAvailable,
                "isolated agent automation is not enabled in this host",
            ));
            return;
        }
        match operation {
            AgentOperation::ContextOpen { viewport, reply } => {
                let id = AgentContextId::new(self.next_agent_context_id);
                self.next_agent_context_id += 1;
                self.agent_contexts.insert(
                    id,
                    FakeAgentContext {
                        owner: client,
                        viewport,
                        epoch: 1,
                        origin: None,
                        approved_origin: FAKE_AGENT_APPROVED_ORIGIN.to_owned(),
                        scroll_x: 0,
                        scroll_y: 0,
                    },
                );
                reply.resolve(AgentContextOpenResult {
                    context_id: id,
                    epoch: 1,
                    viewport,
                });
            }
            AgentOperation::ContextList { reply } => {
                let contexts = self
                    .agent_contexts
                    .iter()
                    .filter(|(_, ctx)| ctx.owner == client)
                    .map(|(&context_id, ctx)| AgentContextSummary {
                        context_id,
                        epoch: ctx.epoch,
                        origin: ctx.origin.clone(),
                        viewport: ctx.viewport,
                    })
                    .collect();
                reply.resolve(AgentContextListResult { contexts });
            }
            AgentOperation::ContextClose { context_id, reply } => {
                let owned = self
                    .agent_contexts
                    .get(&context_id)
                    .is_some_and(|ctx| ctx.owner == client);
                if !owned {
                    reply.reject(ToolError::new(
                        ToolErrorCode::AttachmentUnavailable,
                        "agent context is not available",
                    ));
                    return;
                }
                self.agent_contexts.remove(&context_id);
                reply.resolve(AgentContextCloseResult {
                    context_id,
                    closed: true,
                });
            }
            AgentOperation::Navigate {
                context_id,
                url,
                reply,
                ..
            } => self.handle_agent_navigate(client, context_id, &url, reply),
            AgentOperation::Locate {
                context_id,
                locator,
                max_results,
                reply,
            } => self.handle_agent_locate(client, context_id, &locator, max_results, reply),
            AgentOperation::Act {
                context_id,
                action,
                reply,
            } => self.handle_agent_act(client, context_id, &action, reply),
            AgentOperation::Wait {
                context_id,
                condition,
                reply,
                ..
            } => self.handle_agent_wait(client, context_id, &condition, reply),
            AgentOperation::Screenshot {
                context_id,
                target,
                reply,
                ..
            } => self.handle_agent_screenshot(client, context_id, &target, reply),
        }
    }

    fn handle_agent_navigate(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        url: &Url,
        reply: HostReply<AgentNavigateResult>,
    ) {
        let Some(ctx) = self
            .agent_contexts
            .get_mut(&context_id)
            .filter(|c| c.owner == client)
        else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };
        let requested_origin = url.origin().ascii_serialization();
        if requested_origin != ctx.approved_origin {
            reply.resolve(AgentNavigateResult {
                context_id,
                status: AgentNavigateStatus::ApprovalRequired,
                epoch: ctx.epoch,
                origin: ctx.origin.clone(),
                reason: Some("cross-origin navigation requires explicit user approval".to_owned()),
            });
            return;
        }
        ctx.epoch += 1;
        ctx.origin = Some(requested_origin.clone());
        reply.resolve(AgentNavigateResult {
            context_id,
            status: AgentNavigateStatus::Committed,
            epoch: ctx.epoch,
            origin: Some(requested_origin),
            reason: None,
        });
    }

    fn handle_agent_locate(
        &self,
        client: ClientId,
        context_id: AgentContextId,
        locator: &AgentLocator,
        max_results: usize,
        reply: HostReply<AgentLocateResult>,
    ) {
        let Some(ctx) = self
            .agent_contexts
            .get(&context_id)
            .filter(|c| c.owner == client)
        else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };
        let origin = ctx
            .origin
            .clone()
            .unwrap_or_else(|| ctx.approved_origin.clone());
        let provenance = ElementProvenance {
            origin,
            frame: FrameScope::MainFrame,
            locator_kind: locator.kind(),
        };
        let all_matches = vec![
            LocatedElement {
                element_ref: AgentElementRef {
                    context_id,
                    epoch: ctx.epoch,
                    element_id: "el-input".to_owned(),
                },
                role: "textbox".to_owned(),
                name: Some("Fixture query".to_owned()),
                tag: Some("input".to_owned()),
                bounds: Some(ElementBounds {
                    x: 24.0,
                    y: 48.0,
                    width: 240.0,
                    height: 32.0,
                }),
                provenance: provenance.clone(),
            },
            LocatedElement {
                element_ref: AgentElementRef {
                    context_id,
                    epoch: ctx.epoch,
                    element_id: "el-submit".to_owned(),
                },
                role: "button".to_owned(),
                name: Some("Submit".to_owned()),
                tag: Some("button".to_owned()),
                bounds: Some(ElementBounds {
                    x: 280.0,
                    y: 48.0,
                    width: 96.0,
                    height: 32.0,
                }),
                provenance,
            },
        ];
        let truncated = all_matches.len() > max_results;
        let elements = all_matches.into_iter().take(max_results).collect();
        reply.resolve(AgentLocateResult {
            context_id,
            epoch: ctx.epoch,
            elements,
            truncated,
        });
    }

    fn handle_agent_act(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        action: &AgentAction,
        reply: HostReply<AgentActResult>,
    ) {
        let Some(ctx) = self
            .agent_contexts
            .get_mut(&context_id)
            .filter(|c| c.owner == client)
        else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };

        if self.unsupported_select_option && matches!(action, AgentAction::SelectOption { .. }) {
            reply.reject(ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "select_option is not supported by active backend",
            ));
            return;
        }

        let elem_ref = match action {
            AgentAction::Click { element, .. }
            | AgentAction::DoubleClick { element, .. }
            | AgentAction::Hover { element }
            | AgentAction::Fill { element, .. }
            | AgentAction::SelectOption { element, .. } => Some(element),
            AgentAction::Type { element, .. }
            | AgentAction::PressKey { element, .. }
            | AgentAction::Scroll { element, .. } => element.as_ref(),
        };

        if let Some(el) = elem_ref
            && (el.context_id != context_id
                || el.epoch != ctx.epoch
                || !matches!(el.element_id.as_str(), "el-input" | "el-submit"))
        {
            reply.reject(ToolError::new(
                ToolErrorCode::StaleDocument,
                "element reference is stale for current document epoch",
            ));
            return;
        }

        if let AgentAction::Click { element, .. } = action
            && element.element_id == "el-submit"
        {
            reply.resolve(AgentActResult {
                context_id,
                status: AgentActStatus::ApprovalRequired,
                epoch: ctx.epoch,
                origin: ctx.origin.clone(),
                reason: Some("form submission requires explicit user approval".to_owned()),
                observed_state: None,
            });
            return;
        }

        let observed = match action {
            AgentAction::Fill { text, .. } | AgentAction::Type { text, .. } => AgentObservedState {
                navigated: false,
                focused_role: Some("textbox".to_owned()),
                value_length: Some(text.chars().count()),
                scroll_x: None,
                scroll_y: None,
            },
            AgentAction::Scroll {
                delta_x, delta_y, ..
            } => {
                ctx.scroll_x += *delta_x;
                ctx.scroll_y += *delta_y;
                AgentObservedState {
                    navigated: false,
                    focused_role: None,
                    value_length: None,
                    scroll_x: Some(ctx.scroll_x),
                    scroll_y: Some(ctx.scroll_y),
                }
            }
            _ => AgentObservedState {
                navigated: false,
                focused_role: Some("button".to_owned()),
                value_length: None,
                scroll_x: None,
                scroll_y: None,
            },
        };

        reply.resolve(AgentActResult {
            context_id,
            status: AgentActStatus::Performed,
            epoch: ctx.epoch,
            origin: ctx.origin.clone(),
            reason: None,
            observed_state: Some(observed),
        });
    }

    fn handle_agent_wait(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        condition: &AgentWaitCondition,
        reply: HostReply<AgentWaitResult>,
    ) {
        let Some(ctx) = self
            .agent_contexts
            .get(&context_id)
            .filter(|c| c.owner == client)
        else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };
        if self.park_next_agent_wait {
            self.park_next_agent_wait = false;
            self.parked_agent_waits
                .push_back(ParkedAgentWait(reply, context_id));
            return;
        }
        let status = match condition {
            AgentWaitCondition::TextPresent { text, .. } if text == "never-appears" => {
                AgentWaitStatus::TimedOut
            }
            _ => AgentWaitStatus::Matched,
        };
        reply.resolve(AgentWaitResult {
            context_id,
            epoch: ctx.epoch,
            status,
            elapsed_ms: 12,
        });
    }

    fn handle_agent_screenshot(
        &self,
        client: ClientId,
        context_id: AgentContextId,
        target: &AgentScreenshotTarget,
        reply: HostReply<AgentScreenshotResult>,
    ) {
        let Some(ctx) = self
            .agent_contexts
            .get(&context_id)
            .filter(|c| c.owner == client)
        else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is not available",
            ));
            return;
        };
        let (width, height) = match target {
            AgentScreenshotTarget::Viewport => (ctx.viewport.width, ctx.viewport.height),
            AgentScreenshotTarget::Element(el) => {
                if el.context_id != context_id
                    || el.epoch != ctx.epoch
                    || !matches!(el.element_id.as_str(), "el-input" | "el-submit")
                {
                    reply.reject(ToolError::new(
                        ToolErrorCode::StaleDocument,
                        "element reference is stale for current document epoch",
                    ));
                    return;
                }
                (240, 32)
            }
        };
        let png_bytes = self
            .screenshot_override
            .clone()
            .unwrap_or_else(|| MINIMAL_VALID_PNG.to_vec());
        reply.resolve(AgentScreenshotResult {
            context_id,
            epoch: ctx.epoch,
            width,
            height,
            png_bytes,
        });
    }
}

impl BrowserHost for FakeBrowserHost {
    fn dispatch(
        &self,
        client: ClientId,
        operation: BrowserOperation,
    ) -> Result<(), HostDispatchError> {
        let mut guard = self.state.lock().expect("lock host");
        if let Some(err) = guard.dispatch_override {
            return Err(err);
        }
        guard.dispatched_clients.push(client);

        match operation {
            BrowserOperation::BrowserStatus { reply } => guard.handle_status(reply),
            BrowserOperation::DiagnosticRequest { scope, reply } => {
                guard.handle_diagnostic_request(client, scope, reply);
            }
            BrowserOperation::DiagnosticDetach { attachment, reply } => {
                match guard.diagnostic.detach(client, attachment) {
                    Ok(revoked) => reply.resolve(DiagnosticDetachResult {
                        attachment_id: revoked.attachment,
                        detached: true,
                        reason: revoked.reason,
                    }),
                    Err(err) => reply.reject(ToolError::from(err)),
                }
            }
            BrowserOperation::PageSnapshot {
                target,
                request,
                reply,
            } => guard.handle_page_snapshot(client, target, &request, reply),
            BrowserOperation::PageEvents {
                target,
                cursor,
                filter,
                limit,
                reply,
            } => match target {
                ObservationTarget::Diagnostic(attachment) => {
                    match guard
                        .diagnostic
                        .read_events(client, attachment, cursor, &filter, limit)
                    {
                        Ok(batch) => reply.resolve(PageEventsResult {
                            batch,
                            capabilities: guard.capabilities,
                            native_overflow: guard.native_overflow,
                            closed: false,
                        }),
                        Err(err) => reply.reject(ToolError::from(err)),
                    }
                }
                ObservationTarget::Agent(context_id) => {
                    let Some(ctx) = guard
                        .agent_contexts
                        .get(&context_id)
                        .filter(|c| c.owner == client)
                    else {
                        reply.reject(ToolError::new(
                            ToolErrorCode::AttachmentUnavailable,
                            "agent context is not available",
                        ));
                        return Ok(());
                    };
                    let event = DiagnosticEvent {
                        seq: 1,
                        epoch: ctx.epoch,
                        data: DiagnosticEventData::lifecycle(LifecycleEventKind::Load),
                    };
                    let events = if cursor == 0 && limit > 0 {
                        vec![event]
                    } else {
                        Vec::new()
                    };
                    reply.resolve(PageEventsResult {
                        batch: DiagnosticEventBatch {
                            events,
                            next_cursor: 1,
                            has_more: false,
                            dropped_before: None,
                        },
                        capabilities: guard.capabilities,
                        native_overflow: false,
                        closed: false,
                    });
                }
            },
            BrowserOperation::Agent(agent_op) => guard.handle_agent(client, agent_op),
        }
        Ok(())
    }

    fn client_closed(&self, client: ClientId) {
        let mut guard = self.state.lock().expect("lock host");
        guard.closed_clients.push(client);
        let _ = guard.diagnostic.on_client_disconnected(client);
        guard.agent_contexts.retain(|_, ctx| ctx.owner != client);
    }
}

pub fn default_status() -> BrowserStatus {
    BrowserStatus {
        platform: HostPlatform::Macos,
        runtime_mode: RuntimeMode::Gui,
        product_version: "0.1.0-test".to_owned(),
        fork_capabilities: ForkCapabilities {
            diagnostic: DiagnosticCapabilities {
                dom: true,
                accessibility: true,
                console: true,
                runtime_errors: true,
                pending_network: true,
                completed_network: true,
                lifecycle: true,
            },
            native_accessibility: true,
            visual_inspector_priority: true,
            agent_automation: false,
        },
    }
}

pub fn sample_binding(raw_tab: u64, url: &str, epoch: u64) -> DocumentBinding {
    let parsed = Url::parse(url).expect("parse sample url");
    DocumentBinding {
        tab: TabId::new(Profile::Main, raw_tab),
        origin: parsed.origin(),
        epoch,
    }
}

pub fn default_scope_test_capabilities() -> DiagnosticCapabilities {
    DiagnosticCapabilities {
        dom: true,
        accessibility: true,
        console: true,
        runtime_errors: true,
        pending_network: true,
        completed_network: true,
        lifecycle: true,
    }
}
