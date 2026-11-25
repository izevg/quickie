//! Isolated `Profile::Agent` browsing context manager for Linux Quickie MCP.
//!
//! Owns per-client isolated `WebKitGTK` Automation contexts, non-reused `AgentContextId`s,
//! event-driven Automation command correlation, shared `ObservationDecoder` event rings,
//! fixed isolated-world locator/snapshot/wait scripts, and explicit origin/action approval.

pub(crate) mod protocol;
pub(crate) mod scripts;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use quickie_application::mcp::{
    AgentContextId, BoundedSnapshot, ClientId, DiagnosticCapabilities, DiagnosticEvent,
    DiagnosticEventBatch, DiagnosticEventData, DiagnosticEventFilter, MAX_EVENTS_PER_ATTACHMENT,
    SnapshotKind, SnapshotRequest,
};
use quickie_application::{DiagnosticGapReason, DiagnosticSignal};
use quickie_engine_webkit::observation::ObservationDecoder;
use quickie_mcp::browser::{
    AccessibilitySource, AgentActResult, AgentActStatus, AgentAction, AgentContextCloseResult,
    AgentContextListResult, AgentContextOpenResult, AgentContextSummary, AgentElementRef, AgentKey,
    AgentLocateResult, AgentLocator, AgentLocatorKind, AgentMouseButton, AgentNavigateResult,
    AgentNavigateStatus, AgentObservedState, AgentOperation, AgentScreenshotResult,
    AgentScreenshotTarget, AgentViewport, AgentWaitCondition, AgentWaitResult, AgentWaitStage,
    AgentWaitStatus, HostReply, PageEventsResult, PageSnapshotResult, ToolError, ToolErrorCode,
};
use serde_json::Value;
use url::{Origin, Url};

use crate::mcp::runtime::protocol::parse_allowlisted_command;
use crate::mcp::snapshot;

use self::protocol::{
    AutomationWireCommand, InboundFrame, WireKeyStep, WireMouseInteraction, build_fill_steps,
    build_press_key_steps, decode_browsing_context_handle, decode_screenshot_reply,
    parse_inbound_frame,
};
use self::scripts::{
    ResolvedElement, build_accessibility_fallback_script, build_locate_script,
    build_observed_state_script, build_resolve_element_script, build_wait_check_script,
    parse_locate_reply, parse_observed_state_reply, parse_resolve_element_reply,
    parse_wait_check_reply,
};

/// Hard ceiling on isolated Agent contexts per connected MCP client.
pub(crate) const MAX_CONTEXTS_PER_CLIENT: usize = 8;

/// Hard ceiling on total concurrent isolated Agent contexts across all clients.
pub(crate) const MAX_TOTAL_CONTEXTS: usize = 16;

/// Hard ceiling on concurrent in-flight `WebKit` Automation commands per context.
pub(crate) const MAX_IN_FLIGHT_COMMANDS: usize = 16;

/// Default deadline for non-navigation Automation commands (mouse/keyboard/screenshot).
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 10_000;

/// Interval between wait condition re-evaluations when waiting on DOM state.
const WAIT_RECHECK_INTERVAL_MS: u64 = 50;

/// Explicit approval prompt presented in GUI mode before navigating to a new origin or performing
/// an irreversible/ambiguous action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentApprovalPrompt {
    pub(crate) context_id: AgentContextId,
    pub(crate) summary: String,
    pub(crate) detail: String,
}

/// Platform backend implemented by live `WebKitGTK` on Linux and deterministic fakes in tests.
pub(crate) trait AgentBackend {
    /// Owned native isolated session handle (e.g. `GtkAgentSession`).
    type Session;
    /// Retained cancellable isolated-world script evaluation handle.
    type EvalHandle;
    /// Retained cancellable one-shot timer handle (`GLib` `g_timeout_add` on Linux).
    type TimerHandle;

    /// Whether isolated `Profile::Agent` local Automation is supported in this runtime.
    fn is_available(&self) -> bool;

    /// Opens an isolated offscreen controlled `WebKitWebView` + local `AutomationSession`.
    fn open_session(
        &mut self,
        context_id: AgentContextId,
        session_id: &str,
        viewport: AgentViewport,
    ) -> Result<Self::Session, ToolError>;

    /// Explicitly closes and destroys `session`.
    fn close_session(&mut self, session: Self::Session);

    /// Installs an explicit caller-approved origin grant on the controlled view's policy.
    fn grant_navigation(&mut self, session: &mut Self::Session, url: &Url)
    -> Result<(), ToolError>;

    /// Dispatches one strictly allowlisted Automation protocol command with unique `request_id`.
    fn dispatch_command(
        &mut self,
        session: &mut Self::Session,
        request_id: u64,
        command: &AutomationWireCommand,
    ) -> Result<(), ToolError>;

    /// Starts a fixed isolated-world script evaluation on `session`'s controlled view.
    fn start_script(
        &mut self,
        eval_id: u64,
        session: &Self::Session,
        script: &str,
    ) -> Result<Self::EvalHandle, ToolError>;

    /// Schedules a one-shot timer on the owning main context; invokes `on_timer_fired(timer_id)`.
    fn schedule_timer(&mut self, timer_id: u64, delay_ms: u64) -> Self::TimerHandle;

    /// Presents a native approval prompt if running in interactive GUI mode.
    ///
    /// Returns `true` when an interactive prompt is visible and will complete via
    /// [`AgentManager::resolve_approval`], or `false` in non-interactive/standalone mode.
    fn show_approval(&mut self, prompt: AgentApprovalPrompt) -> bool;

    /// Hides any visible native approval prompt.
    fn hide_approval(&mut self);
}

/// No-op backend for hosts without isolated Agent automation support.
#[derive(Debug, Default)]
pub(crate) struct UnavailableAgentBackend;

impl AgentBackend for UnavailableAgentBackend {
    type Session = ();
    type EvalHandle = ();
    type TimerHandle = ();

    fn is_available(&self) -> bool {
        false
    }

    fn open_session(
        &mut self,
        _context_id: AgentContextId,
        _session_id: &str,
        _viewport: AgentViewport,
    ) -> Result<Self::Session, ToolError> {
        Err(unavailable_error())
    }

    fn close_session(&mut self, _session: Self::Session) {}

    fn grant_navigation(
        &mut self,
        _session: &mut Self::Session,
        _url: &Url,
    ) -> Result<(), ToolError> {
        Err(unavailable_error())
    }

    fn dispatch_command(
        &mut self,
        _session: &mut Self::Session,
        _request_id: u64,
        _command: &AutomationWireCommand,
    ) -> Result<(), ToolError> {
        Err(unavailable_error())
    }

    fn start_script(
        &mut self,
        _eval_id: u64,
        _session: &Self::Session,
        _script: &str,
    ) -> Result<Self::EvalHandle, ToolError> {
        Err(unavailable_error())
    }

    fn schedule_timer(&mut self, _timer_id: u64, _delay_ms: u64) -> Self::TimerHandle {}

    fn show_approval(&mut self, _prompt: AgentApprovalPrompt) -> bool {
        false
    }

    fn hide_approval(&mut self) {}
}

struct AgentEventRing {
    events: VecDeque<DiagnosticEvent>,
    next_cursor: u64,
    earliest_cursor: u64,
    native_overflow: bool,
}

impl AgentEventRing {
    const fn new() -> Self {
        Self {
            events: VecDeque::new(),
            next_cursor: 0,
            earliest_cursor: 0,
            native_overflow: false,
        }
    }

    fn push(&mut self, epoch: u64, data: DiagnosticEventData) {
        let seq = self.next_cursor;
        self.next_cursor = self.next_cursor.saturating_add(1);
        if self.events.len() >= MAX_EVENTS_PER_ATTACHMENT {
            self.events.pop_front();
            self.earliest_cursor = self.events.front().map_or(self.next_cursor, |ev| ev.seq);
        }
        self.events.push_back(DiagnosticEvent { seq, epoch, data });
    }

    fn read(
        &self,
        cursor: u64,
        filter: &DiagnosticEventFilter,
        limit: usize,
    ) -> Result<DiagnosticEventBatch, ToolError> {
        if cursor > self.next_cursor {
            return Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "diagnostic event cursor is invalid",
            ));
        }
        let gap_before_cursor = cursor < self.earliest_cursor;
        let effective_start = cursor.max(self.earliest_cursor);
        let max_items = limit.clamp(1, MAX_EVENTS_PER_ATTACHMENT);

        let mut events = Vec::new();
        let mut next_cursor = effective_start;
        let mut has_more = false;
        for event in &self.events {
            if event.seq < effective_start {
                continue;
            }
            if !filter.matches(event) {
                if events.len() < max_items {
                    next_cursor = event.seq.saturating_add(1);
                }
                continue;
            }
            if events.len() >= max_items {
                has_more = true;
                break;
            }
            next_cursor = event.seq.saturating_add(1);
            events.push(event.clone());
        }
        if !has_more {
            next_cursor = self.next_cursor;
        }
        Ok(DiagnosticEventBatch {
            events,
            next_cursor,
            has_more,
            dropped_before: gap_before_cursor.then_some(self.earliest_cursor),
        })
    }
}

enum PendingCommandKind {
    InitHandle {
        deferred: Option<DeferredAfterHandle>,
    },
    Navigate {
        url: Url,
        reply: HostReply<AgentNavigateResult>,
    },
    ActFocusThenKeyboard {
        action_element: Option<String>,
        steps: Vec<WireKeyStep>,
        start_epoch: u64,
        reply: HostReply<AgentActResult>,
    },
    ActFinish {
        action_element: Option<String>,
        start_epoch: u64,
        reply: HostReply<AgentActResult>,
    },
    Screenshot {
        captured_epoch: u64,
        max_bytes: usize,
        reply: HostReply<AgentScreenshotResult>,
    },
    InspectorSub,
}

enum DeferredAfterHandle {
    Navigate {
        url: Url,
        wait_stage: AgentWaitStage,
        timeout_ms: u64,
        reply: HostReply<AgentNavigateResult>,
    },
    Act {
        action: AgentAction,
        resolved: Option<ResolvedElement>,
        reply: HostReply<AgentActResult>,
    },
    Screenshot {
        max_bytes: usize,
        reply: HostReply<AgentScreenshotResult>,
    },
}

struct PendingCommand<T> {
    kind: PendingCommandKind,
    _deadline: Option<T>,
}

struct ContextRuntime<B: AgentBackend> {
    owner: ClientId,
    context_id: AgentContextId,
    viewport: AgentViewport,
    epoch: u64,
    origin: Option<String>,
    approved_origin: Option<Origin>,
    browsing_context_handle: Option<String>,
    handle_in_flight: bool,
    session: B::Session,
    next_cmd_id: u64,
    pending_cmds: BTreeMap<u64, PendingCommand<B::TimerHandle>>,
    decoder: ObservationDecoder,
    inspector_bootstrapped: bool,
    events: AgentEventRing,
}

impl<B: AgentBackend> ContextRuntime<B> {
    fn doc_token(&self) -> String {
        format!("c{}-e{}", self.context_id.get(), self.epoch)
    }

    fn next_command_id(&mut self) -> Result<u64, ToolError> {
        let next = self.next_cmd_id.checked_add(1).ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Unavailable,
                "automation request IDs exhausted",
            )
        })?;
        self.next_cmd_id = next;
        Ok(next)
    }
}

enum PendingEvalKind {
    Locate {
        locator_kind: AgentLocatorKind,
        max_results: usize,
        reply: HostReply<AgentLocateResult>,
    },
    ActResolve {
        action: AgentAction,
        reply: HostReply<AgentActResult>,
    },
    ActObserve {
        start_epoch: u64,
        reply: HostReply<AgentActResult>,
    },
    Snapshot {
        kind: SnapshotKind,
        max_bytes: usize,
        reply: HostReply<PageSnapshotResult>,
    },
    WaitProbe {
        wait_id: u64,
    },
}

struct PendingEval<E> {
    client: ClientId,
    context_id: AgentContextId,
    captured_epoch: u64,
    kind: PendingEvalKind,
    _handle: E,
}

struct PendingWait<T> {
    client: ClientId,
    context_id: AgentContextId,
    condition: AgentWaitCondition,
    started_at: Instant,
    deadline: Instant,
    reply: HostReply<AgentWaitResult>,
    _timer: Option<T>,
}

enum TimerAction {
    CommandDeadline {
        context_id: AgentContextId,
        cmd_id: u64,
    },
    WaitRecheck {
        wait_id: u64,
    },
}

enum PendingApprovalKind {
    Navigate {
        url: Url,
        wait_stage: AgentWaitStage,
        timeout_ms: u64,
        reply: HostReply<AgentNavigateResult>,
    },
    Act {
        action: AgentAction,
        resolved: Option<ResolvedElement>,
        captured_epoch: u64,
        reply: HostReply<AgentActResult>,
    },
}

struct PendingApproval {
    client: ClientId,
    context_id: AgentContextId,
    kind: PendingApprovalKind,
}

/// Main-thread isolated Agent context manager shared by GUI and Standalone Linux MCP hosts.
pub(crate) struct AgentManager<B: AgentBackend> {
    backend: B,
    next_context_id: u64,
    next_eval_id: u64,
    next_wait_id: u64,
    next_timer_id: u64,
    contexts: BTreeMap<AgentContextId, ContextRuntime<B>>,
    pending_evals: HashMap<u64, PendingEval<B::EvalHandle>>,
    pending_waits: HashMap<u64, PendingWait<B::TimerHandle>>,
    timers: HashMap<u64, TimerAction>,
    pending_approval: Option<PendingApproval>,
}

impl<B: AgentBackend> AgentManager<B> {
    #[must_use]
    pub(crate) fn new(backend: B) -> Self {
        Self {
            backend,
            next_context_id: 0,
            next_eval_id: 0,
            next_wait_id: 0,
            next_timer_id: 0,
            contexts: BTreeMap::new(),
            pending_evals: HashMap::new(),
            pending_waits: HashMap::new(),
            timers: HashMap::new(),
            pending_approval: None,
        }
    }

    #[must_use]
    pub(crate) fn is_available(&self) -> bool {
        self.backend.is_available()
    }

    #[must_use]
    pub(crate) fn has_pending_approval(&self) -> bool {
        self.pending_approval.is_some()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn backend(&self) -> &B {
        &self.backend
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn active_context_count(&self) -> usize {
        self.contexts.len()
    }

    /// Explicitly grants `url`'s origin for `context_id` (used only by explicit host approval
    /// paths, never implicitly from caller claims or loopback heuristics).
    #[cfg(test)]
    pub(crate) fn grant_origin_explicit(
        &mut self,
        context_id: AgentContextId,
        url: &Url,
    ) -> Result<(), ToolError> {
        let Some(ctx) = self.contexts.get_mut(&context_id) else {
            return Err(context_unavailable_error());
        };
        self.backend.grant_navigation(&mut ctx.session, url)?;
        ctx.approved_origin = Some(url.origin());
        Ok(())
    }

    /// Dispatches one typed [`AgentOperation`] for `client`.
    pub(crate) fn handle_operation(&mut self, client: ClientId, operation: AgentOperation) {
        if !self.backend.is_available() {
            operation.reject(unavailable_error());
            return;
        }
        match operation {
            AgentOperation::ContextOpen { viewport, reply } => {
                self.handle_context_open(client, viewport, reply);
            }
            AgentOperation::ContextList { reply } => self.handle_context_list(client, reply),
            AgentOperation::ContextClose { context_id, reply } => {
                self.handle_context_close(client, context_id, reply);
            }
            AgentOperation::Navigate {
                context_id,
                url,
                wait_stage,
                timeout_ms,
                reply,
            } => self.handle_navigate(client, context_id, url, wait_stage, timeout_ms, reply),
            AgentOperation::Locate {
                context_id,
                locator,
                max_results,
                reply,
            } => self.handle_locate(client, context_id, &locator, max_results, reply),
            AgentOperation::Act {
                context_id,
                action,
                reply,
            } => self.handle_act(client, context_id, action, reply),
            AgentOperation::Wait {
                context_id,
                condition,
                timeout_ms,
                reply,
            } => self.handle_wait(client, context_id, condition, timeout_ms, reply),
            AgentOperation::Screenshot {
                context_id,
                target,
                max_bytes,
                reply,
            } => self.handle_screenshot(client, context_id, target, max_bytes, reply),
        }
    }

    /// Handles `page_snapshot` targeting `ObservationTarget::Agent(context_id)`.
    pub(crate) fn handle_page_snapshot(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        request: &SnapshotRequest,
        reply: HostReply<PageSnapshotResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        if !self.backend.is_available() {
            reply.reject(unavailable_error());
            return;
        }
        let Some(captured_epoch) = self
            .contexts
            .get(&context_id)
            .filter(|c| c.owner == client)
            .map(|c| c.epoch)
        else {
            reply.reject(context_unavailable_error());
            return;
        };
        let script = match request.kind() {
            SnapshotKind::Dom | SnapshotKind::Text => match snapshot::build_script(request) {
                Ok(s) => s,
                Err(err) => {
                    reply.reject(err);
                    return;
                }
            },
            SnapshotKind::Accessibility => match build_accessibility_fallback_script(request) {
                Ok(s) => s,
                Err(err) => {
                    reply.reject(err);
                    return;
                }
            },
        };
        let eval_id = self.alloc_eval_id();
        let Some(ctx) = self.contexts.get(&context_id) else {
            reply.reject(context_unavailable_error());
            return;
        };
        match self.backend.start_script(eval_id, &ctx.session, &script) {
            Ok(handle) => {
                self.pending_evals.insert(
                    eval_id,
                    PendingEval {
                        client,
                        context_id,
                        captured_epoch,
                        kind: PendingEvalKind::Snapshot {
                            kind: request.kind(),
                            max_bytes: request.max_bytes(),
                            reply,
                        },
                        _handle: handle,
                    },
                );
            }
            Err(err) => reply.reject(err),
        }
    }

    /// Handles `page_events` targeting `ObservationTarget::Agent(context_id)`.
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
        if !self.backend.is_available() {
            reply.reject(unavailable_error());
            return;
        }
        let Some(ctx) = self.contexts.get(&context_id).filter(|c| c.owner == client) else {
            reply.reject(context_unavailable_error());
            return;
        };
        match ctx.events.read(cursor, filter, limit) {
            Ok(batch) => reply.resolve(PageEventsResult {
                batch,
                capabilities: DiagnosticCapabilities {
                    dom: true,
                    accessibility: true,
                    console: true,
                    runtime_errors: true,
                    pending_network: true,
                    completed_network: true,
                    lifecycle: true,
                },
                native_overflow: ctx.events.native_overflow,
                closed: false,
            }),
            Err(err) => reply.reject(err),
        }
    }

    /// Resolves an active native GUI approval prompt.
    pub(crate) fn resolve_approval(&mut self, allow: bool) {
        let Some(pending) = self.pending_approval.take() else {
            return;
        };
        self.backend.hide_approval();
        match pending.kind {
            PendingApprovalKind::Navigate {
                url,
                wait_stage,
                timeout_ms,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let Some(ctx) = self
                    .contexts
                    .get_mut(&pending.context_id)
                    .filter(|c| c.owner == pending.client)
                else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                if !allow {
                    reply.resolve(AgentNavigateResult {
                        context_id: pending.context_id,
                        status: AgentNavigateStatus::Rejected,
                        epoch: ctx.epoch,
                        origin: ctx.origin.clone(),
                        reason: Some("user denied navigation approval".to_owned()),
                    });
                    return;
                }
                if let Err(err) = self.backend.grant_navigation(&mut ctx.session, &url) {
                    reply.reject(err);
                    return;
                }
                ctx.approved_origin = Some(url.origin());
                self.dispatch_approved_navigate(
                    pending.context_id,
                    url,
                    wait_stage,
                    timeout_ms,
                    reply,
                );
            }
            PendingApprovalKind::Act {
                action,
                resolved,
                captured_epoch,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let Some(ctx) = self
                    .contexts
                    .get(&pending.context_id)
                    .filter(|c| c.owner == pending.client)
                else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                if ctx.epoch != captured_epoch {
                    reply.reject(stale_document_error());
                    return;
                }
                if !allow {
                    reply.resolve(AgentActResult {
                        context_id: pending.context_id,
                        status: AgentActStatus::Rejected,
                        epoch: ctx.epoch,
                        origin: ctx.origin.clone(),
                        reason: Some("user denied action approval".to_owned()),
                        observed_state: None,
                    });
                    return;
                }
                self.dispatch_approved_act(pending.context_id, action, resolved, reply);
            }
        }
    }

    /// Handles one inbound `WebKitAutomationSession::message-received` frame for `context_id`.
    pub(crate) fn on_automation_message(&mut self, context_id: AgentContextId, raw: &str) {
        if !self.contexts.contains_key(&context_id) {
            return;
        }
        match parse_inbound_frame(raw) {
            InboundFrame::Ignored => {}
            InboundFrame::BrowsingContextCleared { .. } => {
                if let Some(ctx) = self.contexts.get_mut(&context_id) {
                    ctx.epoch = ctx.epoch.saturating_add(1);
                }
            }
            InboundFrame::InspectorMessage { message, .. } => {
                self.handle_inspector_message(context_id, &message);
            }
            InboundFrame::Reply { id, outcome } => {
                self.handle_command_reply(context_id, id, outcome);
            }
        }
    }

    /// Handles native `WebKitAutomationSession::will-close` or `web-process-terminated`.
    pub(crate) fn on_session_closed(&mut self, context_id: AgentContextId) {
        self.teardown_context(
            context_id,
            &ToolError::new(
                ToolErrorCode::Unavailable,
                "isolated agent session closed unexpectedly",
            ),
        );
    }

    /// Handles completion of an isolated-world script evaluation.
    #[expect(
        clippy::too_many_lines,
        reason = "Exhaustive single-place dispatch across isolated-world evaluation kinds"
    )]
    pub(crate) fn on_script_completed(&mut self, eval_id: u64, result: Result<String, ToolError>) {
        let Some(pending) = self.pending_evals.remove(&eval_id) else {
            return;
        };
        match pending.kind {
            PendingEvalKind::Locate {
                locator_kind,
                max_results,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let Some(ctx) = self
                    .contexts
                    .get(&pending.context_id)
                    .filter(|c| c.owner == pending.client)
                else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                if ctx.epoch != pending.captured_epoch {
                    reply.reject(stale_document_error());
                    return;
                }
                let raw = match result {
                    Ok(raw) => raw,
                    Err(err) => {
                        reply.reject(err);
                        return;
                    }
                };
                let origin = ctx.origin.as_deref().unwrap_or("");
                match parse_locate_reply(
                    &raw,
                    pending.context_id,
                    ctx.epoch,
                    origin,
                    locator_kind,
                    max_results,
                ) {
                    Ok(located) => reply.resolve(located),
                    Err(err) => reply.reject(err),
                }
            }
            PendingEvalKind::ActResolve { action, reply } => {
                if reply.is_closed() {
                    return;
                }
                let Some(ctx) = self
                    .contexts
                    .get(&pending.context_id)
                    .filter(|c| c.owner == pending.client)
                else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                if ctx.epoch != pending.captured_epoch {
                    reply.reject(stale_document_error());
                    return;
                }
                let raw = match result {
                    Ok(raw) => raw,
                    Err(err) => {
                        reply.reject(err);
                        return;
                    }
                };
                let resolved = match parse_resolve_element_reply(&raw) {
                    Ok(resolved) => resolved,
                    Err(err) => {
                        reply.reject(err);
                        return;
                    }
                };
                let needs_approval = resolved.requires_approval
                    || matches!(
                        action,
                        AgentAction::PressKey {
                            key: AgentKey::Enter,
                            ..
                        }
                    );
                if needs_approval {
                    let reason = resolved.approval_reason.clone().unwrap_or_else(|| {
                        "irreversible or ambiguous action requires explicit approval".to_owned()
                    });
                    self.request_act_approval(
                        pending.client,
                        pending.context_id,
                        pending.captured_epoch,
                        action,
                        Some(resolved),
                        reason,
                        reply,
                    );
                    return;
                }
                self.dispatch_approved_act(pending.context_id, action, Some(resolved), reply);
            }
            PendingEvalKind::ActObserve { start_epoch, reply } => {
                if reply.is_closed() {
                    return;
                }
                let Some(ctx) = self
                    .contexts
                    .get(&pending.context_id)
                    .filter(|c| c.owner == pending.client)
                else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                let navigated = ctx.epoch != start_epoch;
                let observed = result
                    .ok()
                    .map(|raw| parse_observed_state_reply(&raw, navigated));
                reply.resolve(AgentActResult {
                    context_id: pending.context_id,
                    status: AgentActStatus::Performed,
                    epoch: ctx.epoch,
                    origin: ctx.origin.clone(),
                    reason: None,
                    observed_state: observed,
                });
            }
            PendingEvalKind::Snapshot {
                kind,
                max_bytes,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let Some(ctx) = self
                    .contexts
                    .get(&pending.context_id)
                    .filter(|c| c.owner == pending.client)
                else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                if ctx.epoch != pending.captured_epoch {
                    reply.reject(stale_document_error());
                    return;
                }
                let raw = match result {
                    Ok(raw) => raw,
                    Err(err) => {
                        reply.reject(err);
                        return;
                    }
                };
                match snapshot::parse_reply(&raw, max_bytes) {
                    Ok(snap) => {
                        let accessibility_source = (kind == SnapshotKind::Accessibility)
                            .then_some(AccessibilitySource::SemanticDomFallback);
                        reply.resolve(PageSnapshotResult {
                            kind,
                            snapshot: BoundedSnapshot::new(ctx.epoch, &snap.data, snap.truncated),
                            accessibility_source,
                        });
                    }
                    Err(err) => reply.reject(err),
                }
            }
            PendingEvalKind::WaitProbe { wait_id } => {
                self.handle_wait_probe_result(wait_id, result);
            }
        }
    }

    /// Handles a scheduled one-shot timer firing on the main context.
    pub(crate) fn on_timer_fired(&mut self, timer_id: u64) {
        let Some(action) = self.timers.remove(&timer_id) else {
            return;
        };
        match action {
            TimerAction::CommandDeadline { context_id, cmd_id } => {
                let Some(ctx) = self.contexts.get_mut(&context_id) else {
                    return;
                };
                let Some(pending) = ctx.pending_cmds.remove(&cmd_id) else {
                    return;
                };
                if matches!(pending.kind, PendingCommandKind::InitHandle { .. }) {
                    ctx.handle_in_flight = false;
                }
                reject_pending_command(
                    pending.kind,
                    ToolError::new(ToolErrorCode::TimedOut, "automation command timed out"),
                );
            }
            TimerAction::WaitRecheck { wait_id } => {
                self.step_wait(wait_id);
            }
        }
    }

    /// Destroys every isolated Agent context and pending request owned by a disconnected client.
    pub(crate) fn on_client_closed(&mut self, client: ClientId) {
        if self
            .pending_approval
            .as_ref()
            .is_some_and(|p| p.client == client)
        {
            self.pending_approval = None;
            self.backend.hide_approval();
        }
        let owned: Vec<AgentContextId> = self
            .contexts
            .iter()
            .filter_map(|(&id, ctx)| (ctx.owner == client).then_some(id))
            .collect();
        for context_id in owned {
            self.teardown_context(
                context_id,
                &ToolError::new(ToolErrorCode::Cancelled, "client disconnected"),
            );
        }
    }

    /// Destroys all isolated Agent contexts and rejects all pending replies during host shutdown.
    pub(crate) fn shutdown(&mut self) {
        if let Some(pending) = self.pending_approval.take() {
            self.backend.hide_approval();
            reject_pending_approval(
                pending.kind,
                ToolError::new(ToolErrorCode::Unavailable, "browser is shutting down"),
            );
        }
        let ids: Vec<AgentContextId> = self.contexts.keys().copied().collect();
        let reason = ToolError::new(ToolErrorCode::Unavailable, "browser is shutting down");
        for id in ids {
            self.teardown_context(id, &reason);
        }
        self.timers.clear();
    }

    fn handle_context_open(
        &mut self,
        client: ClientId,
        viewport: AgentViewport,
        reply: HostReply<AgentContextOpenResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let owned_count = self
            .contexts
            .values()
            .filter(|ctx| ctx.owner == client)
            .count();
        if owned_count >= MAX_CONTEXTS_PER_CLIENT || self.contexts.len() >= MAX_TOTAL_CONTEXTS {
            reply.reject(ToolError::new(
                ToolErrorCode::Busy,
                "isolated agent context capacity reached",
            ));
            return;
        }
        let Some(raw_id) = self.next_context_id.checked_add(1) else {
            reply.reject(ToolError::new(
                ToolErrorCode::Unavailable,
                "agent context identifiers exhausted",
            ));
            return;
        };
        self.next_context_id = raw_id;
        let context_id = AgentContextId::new(raw_id);
        let session_id = format!("quickie-agent-{}-{raw_id}", client.get());
        let session = match self.backend.open_session(context_id, &session_id, viewport) {
            Ok(session) => session,
            Err(err) => {
                reply.reject(err);
                return;
            }
        };
        let initial_epoch = 1;
        self.contexts.insert(
            context_id,
            ContextRuntime {
                owner: client,
                context_id,
                viewport,
                epoch: initial_epoch,
                origin: None,
                approved_origin: None,
                browsing_context_handle: None,
                handle_in_flight: false,
                session,
                next_cmd_id: 0,
                pending_cmds: BTreeMap::new(),
                decoder: ObservationDecoder::new(),
                inspector_bootstrapped: false,
                events: AgentEventRing::new(),
            },
        );
        // Kick off handle discovery eagerly so the context is ready when the first action arrives.
        self.ensure_handle_query(context_id, None);
        reply.resolve(AgentContextOpenResult {
            context_id,
            epoch: initial_epoch,
            viewport,
        });
    }

    fn handle_context_list(&self, client: ClientId, reply: HostReply<AgentContextListResult>) {
        if reply.is_closed() {
            return;
        }
        let contexts = self
            .contexts
            .values()
            .filter(|ctx| ctx.owner == client)
            .map(|ctx| AgentContextSummary {
                context_id: ctx.context_id,
                epoch: ctx.epoch,
                origin: ctx.origin.clone(),
                viewport: ctx.viewport,
            })
            .collect();
        reply.resolve(AgentContextListResult { contexts });
    }

    fn handle_context_close(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        reply: HostReply<AgentContextCloseResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let owned = self
            .contexts
            .get(&context_id)
            .is_some_and(|ctx| ctx.owner == client);
        if !owned {
            reply.reject(context_unavailable_error());
            return;
        }
        self.teardown_context(context_id, &context_unavailable_error());
        reply.resolve(AgentContextCloseResult {
            context_id,
            closed: true,
        });
    }

    fn handle_navigate(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        url: Url,
        wait_stage: AgentWaitStage,
        timeout_ms: u64,
        reply: HostReply<AgentNavigateResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let Some(ctx) = self.contexts.get(&context_id).filter(|c| c.owner == client) else {
            reply.reject(context_unavailable_error());
            return;
        };
        let requested_origin = url.origin();
        if ctx.approved_origin.as_ref() != Some(&requested_origin) {
            let epoch = ctx.epoch;
            let origin = ctx.origin.clone();
            if self.pending_approval.is_some() {
                reply.resolve(AgentNavigateResult {
                    context_id,
                    status: AgentNavigateStatus::ApprovalRequired,
                    epoch,
                    origin,
                    reason: Some("another approval prompt is already active".to_owned()),
                });
                return;
            }
            let prompt = AgentApprovalPrompt {
                context_id,
                summary: format!("Allow Agent navigation to {}?", requested_origin.ascii_serialization()),
                detail: "Isolated Agent browsing requires explicit approval before navigating to a new origin.".to_owned(),
            };
            if self.backend.show_approval(prompt) {
                self.pending_approval = Some(PendingApproval {
                    client,
                    context_id,
                    kind: PendingApprovalKind::Navigate {
                        url,
                        wait_stage,
                        timeout_ms,
                        reply,
                    },
                });
            } else {
                reply.resolve(AgentNavigateResult {
                    context_id,
                    status: AgentNavigateStatus::ApprovalRequired,
                    epoch,
                    origin,
                    reason: Some(
                        "navigation to unapproved origin requires explicit user approval"
                            .to_owned(),
                    ),
                });
            }
            return;
        }
        self.dispatch_approved_navigate(context_id, url, wait_stage, timeout_ms, reply);
    }

    fn dispatch_approved_navigate(
        &mut self,
        context_id: AgentContextId,
        url: Url,
        wait_stage: AgentWaitStage,
        timeout_ms: u64,
        reply: HostReply<AgentNavigateResult>,
    ) {
        let Some(handle) = self
            .contexts
            .get(&context_id)
            .and_then(|ctx| ctx.browsing_context_handle.clone())
        else {
            self.ensure_handle_query(
                context_id,
                Some(DeferredAfterHandle::Navigate {
                    url,
                    wait_stage,
                    timeout_ms,
                    reply,
                }),
            );
            return;
        };
        let timeout_u32 = u32::try_from(timeout_ms).unwrap_or(u32::MAX);
        let command = AutomationWireCommand::Navigate {
            handle,
            url: url.clone(),
            wait_stage,
            timeout_ms: timeout_u32,
        };
        self.send_command(
            context_id,
            &command,
            Some(timeout_ms),
            PendingCommandKind::Navigate { url, reply },
        );
    }

    fn handle_locate(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        locator: &AgentLocator,
        max_results: usize,
        reply: HostReply<AgentLocateResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let Some((doc_token, captured_epoch)) = self
            .contexts
            .get(&context_id)
            .filter(|c| c.owner == client)
            .map(|c| (c.doc_token(), c.epoch))
        else {
            reply.reject(context_unavailable_error());
            return;
        };
        let script = match build_locate_script(locator, max_results, &doc_token) {
            Ok(script) => script,
            Err(err) => {
                reply.reject(err);
                return;
            }
        };
        let eval_id = self.alloc_eval_id();
        let Some(ctx) = self.contexts.get(&context_id) else {
            reply.reject(context_unavailable_error());
            return;
        };
        match self.backend.start_script(eval_id, &ctx.session, &script) {
            Ok(handle) => {
                self.pending_evals.insert(
                    eval_id,
                    PendingEval {
                        client,
                        context_id,
                        captured_epoch,
                        kind: PendingEvalKind::Locate {
                            locator_kind: locator.kind(),
                            max_results,
                            reply,
                        },
                        _handle: handle,
                    },
                );
            }
            Err(err) => reply.reject(err),
        }
    }

    fn handle_act(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        action: AgentAction,
        reply: HostReply<AgentActResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let Some((captured_epoch, doc_token)) = self
            .contexts
            .get(&context_id)
            .filter(|c| c.owner == client)
            .map(|c| (c.epoch, c.doc_token()))
        else {
            reply.reject(context_unavailable_error());
            return;
        };

        // Validate exact document-bound element reference before any dispatch.
        if let Some(elem_ref) = action_element_ref(&action)
            && (elem_ref.context_id != context_id || elem_ref.epoch != captured_epoch)
        {
            reply.reject(stale_document_error());
            return;
        }

        if matches!(action, AgentAction::SelectOption { .. }) {
            reply.reject(ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "native trusted select_option is unavailable in WebKitGTK Automation",
            ));
            return;
        }

        if let Some(elem_ref) = action_element_ref(&action) {
            let script = match build_resolve_element_script(&elem_ref.element_id, &doc_token, true)
            {
                Ok(s) => s,
                Err(err) => {
                    reply.reject(err);
                    return;
                }
            };
            let eval_id = self.alloc_eval_id();
            let Some(ctx) = self.contexts.get(&context_id) else {
                reply.reject(context_unavailable_error());
                return;
            };
            match self.backend.start_script(eval_id, &ctx.session, &script) {
                Ok(handle) => {
                    self.pending_evals.insert(
                        eval_id,
                        PendingEval {
                            client,
                            context_id,
                            captured_epoch,
                            kind: PendingEvalKind::ActResolve { action, reply },
                            _handle: handle,
                        },
                    );
                }
                Err(err) => reply.reject(err),
            }
            return;
        }

        // Actions without an element reference: Enter key is potentially form-submitting.
        if matches!(
            action,
            AgentAction::PressKey {
                key: AgentKey::Enter,
                ..
            }
        ) {
            self.request_act_approval(
                client,
                context_id,
                captured_epoch,
                action,
                None,
                "pressing Enter may submit a form and requires explicit approval".to_owned(),
                reply,
            );
            return;
        }

        self.dispatch_approved_act(context_id, action, None, reply);
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Approval handoff preserves client, context, epoch, action, resolved element, reason, and reply"
    )]
    fn request_act_approval(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        captured_epoch: u64,
        action: AgentAction,
        resolved: Option<ResolvedElement>,
        reason: String,
        reply: HostReply<AgentActResult>,
    ) {
        let Some(ctx) = self.contexts.get(&context_id) else {
            reply.reject(context_unavailable_error());
            return;
        };
        if self.pending_approval.is_some() {
            reply.resolve(AgentActResult {
                context_id,
                status: AgentActStatus::ApprovalRequired,
                epoch: ctx.epoch,
                origin: ctx.origin.clone(),
                reason: Some("another approval prompt is already active".to_owned()),
                observed_state: None,
            });
            return;
        }
        let prompt = AgentApprovalPrompt {
            context_id,
            summary: "Allow irreversible or sensitive Agent action?".to_owned(),
            detail: reason.clone(),
        };
        if self.backend.show_approval(prompt) {
            self.pending_approval = Some(PendingApproval {
                client,
                context_id,
                kind: PendingApprovalKind::Act {
                    action,
                    resolved,
                    captured_epoch,
                    reply,
                },
            });
        } else {
            reply.resolve(AgentActResult {
                context_id,
                status: AgentActStatus::ApprovalRequired,
                epoch: ctx.epoch,
                origin: ctx.origin.clone(),
                reason: Some(reason),
                observed_state: None,
            });
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Exhaustive translation of typed AgentAction variants into native Automation commands"
    )]
    fn dispatch_approved_act(
        &mut self,
        context_id: AgentContextId,
        action: AgentAction,
        resolved: Option<ResolvedElement>,
        reply: HostReply<AgentActResult>,
    ) {
        let Some(ctx) = self.contexts.get(&context_id) else {
            reply.reject(context_unavailable_error());
            return;
        };
        let Some(handle) = ctx.browsing_context_handle.clone() else {
            self.ensure_handle_query(
                context_id,
                Some(DeferredAfterHandle::Act {
                    action,
                    resolved,
                    reply,
                }),
            );
            return;
        };
        let start_epoch = ctx.epoch;
        let viewport = ctx.viewport;
        let action_element = action_element_ref(&action).map(|r| r.element_id.clone());

        let (command, pending_kind) = match action {
            AgentAction::Click {
                button, modifiers, ..
            } => {
                let Some(res) = resolved else {
                    reply.reject(stale_document_error());
                    return;
                };
                (
                    AutomationWireCommand::MouseInteraction {
                        handle,
                        x: res.x,
                        y: res.y,
                        button,
                        interaction: WireMouseInteraction::SingleClick,
                        modifiers,
                    },
                    PendingCommandKind::ActFinish {
                        action_element,
                        start_epoch,
                        reply,
                    },
                )
            }
            AgentAction::DoubleClick {
                button, modifiers, ..
            } => {
                let Some(res) = resolved else {
                    reply.reject(stale_document_error());
                    return;
                };
                (
                    AutomationWireCommand::MouseInteraction {
                        handle,
                        x: res.x,
                        y: res.y,
                        button,
                        interaction: WireMouseInteraction::DoubleClick,
                        modifiers,
                    },
                    PendingCommandKind::ActFinish {
                        action_element,
                        start_epoch,
                        reply,
                    },
                )
            }
            AgentAction::Hover { .. } => {
                let Some(res) = resolved else {
                    reply.reject(stale_document_error());
                    return;
                };
                (
                    AutomationWireCommand::MouseInteraction {
                        handle,
                        x: res.x,
                        y: res.y,
                        button: AgentMouseButton::Left,
                        interaction: WireMouseInteraction::Move,
                        modifiers: Vec::new(),
                    },
                    PendingCommandKind::ActFinish {
                        action_element,
                        start_epoch,
                        reply,
                    },
                )
            }
            AgentAction::Fill { text, .. } => {
                let Some(res) = resolved else {
                    reply.reject(stale_document_error());
                    return;
                };
                if !res.is_editable {
                    reply.resolve(AgentActResult {
                        context_id,
                        status: AgentActStatus::Rejected,
                        epoch: start_epoch,
                        origin: ctx.origin.clone(),
                        reason: Some("target element is not editable".to_owned()),
                        observed_state: None,
                    });
                    return;
                }
                let steps = build_fill_steps(&text);
                (
                    AutomationWireCommand::MouseInteraction {
                        handle,
                        x: res.x,
                        y: res.y,
                        button: AgentMouseButton::Left,
                        interaction: WireMouseInteraction::SingleClick,
                        modifiers: Vec::new(),
                    },
                    PendingCommandKind::ActFocusThenKeyboard {
                        action_element,
                        steps,
                        start_epoch,
                        reply,
                    },
                )
            }
            AgentAction::Type { text, .. } => {
                let steps = vec![WireKeyStep::InsertByKey(text)];
                if let Some(res) = resolved {
                    (
                        AutomationWireCommand::MouseInteraction {
                            handle,
                            x: res.x,
                            y: res.y,
                            button: AgentMouseButton::Left,
                            interaction: WireMouseInteraction::SingleClick,
                            modifiers: Vec::new(),
                        },
                        PendingCommandKind::ActFocusThenKeyboard {
                            action_element,
                            steps,
                            start_epoch,
                            reply,
                        },
                    )
                } else {
                    (
                        AutomationWireCommand::KeyboardInteractions { handle, steps },
                        PendingCommandKind::ActFinish {
                            action_element,
                            start_epoch,
                            reply,
                        },
                    )
                }
            }
            AgentAction::PressKey { key, modifiers, .. } => {
                let steps = build_press_key_steps(key, &modifiers);
                if let Some(res) = resolved {
                    (
                        AutomationWireCommand::MouseInteraction {
                            handle,
                            x: res.x,
                            y: res.y,
                            button: AgentMouseButton::Left,
                            interaction: WireMouseInteraction::SingleClick,
                            modifiers: Vec::new(),
                        },
                        PendingCommandKind::ActFocusThenKeyboard {
                            action_element,
                            steps,
                            start_epoch,
                            reply,
                        },
                    )
                } else {
                    (
                        AutomationWireCommand::KeyboardInteractions { handle, steps },
                        PendingCommandKind::ActFinish {
                            action_element,
                            start_epoch,
                            reply,
                        },
                    )
                }
            }
            AgentAction::Scroll {
                delta_x, delta_y, ..
            } => {
                let (x, y) = resolved.map_or_else(
                    || {
                        (
                            f64::from(viewport.width) / 2.0,
                            f64::from(viewport.height) / 2.0,
                        )
                    },
                    |res| (res.x, res.y),
                );
                (
                    AutomationWireCommand::WheelScroll {
                        handle,
                        x,
                        y,
                        delta_x,
                        delta_y,
                    },
                    PendingCommandKind::ActFinish {
                        action_element,
                        start_epoch,
                        reply,
                    },
                )
            }
            AgentAction::SelectOption { .. } => {
                reply.reject(ToolError::new(
                    ToolErrorCode::UnsupportedCapability,
                    "native trusted select_option is unavailable in WebKitGTK Automation",
                ));
                return;
            }
        };

        self.send_command(
            context_id,
            &command,
            Some(DEFAULT_COMMAND_TIMEOUT_MS),
            pending_kind,
        );
    }

    fn handle_wait(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        condition: AgentWaitCondition,
        timeout_ms: u64,
        reply: HostReply<AgentWaitResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        if self
            .contexts
            .get(&context_id)
            .is_none_or(|c| c.owner != client)
        {
            reply.reject(context_unavailable_error());
            return;
        }
        // Validate condition script up-front so invalid selectors/stages fail immediately.
        if let Err(err) = build_wait_check_script(&condition) {
            reply.reject(err);
            return;
        }
        let now = Instant::now();
        let wait_id = self.alloc_wait_id();
        self.pending_waits.insert(
            wait_id,
            PendingWait {
                client,
                context_id,
                condition,
                started_at: now,
                deadline: now + Duration::from_millis(timeout_ms),
                reply,
                _timer: None,
            },
        );
        self.step_wait(wait_id);
    }

    fn step_wait(&mut self, wait_id: u64) {
        let Some(wait) = self.pending_waits.get(&wait_id) else {
            return;
        };
        if wait.reply.is_closed() {
            let _ = self.pending_waits.remove(&wait_id);
            return;
        }
        let context_id = wait.context_id;
        let client = wait.client;
        let Some(captured_epoch) = self
            .contexts
            .get(&context_id)
            .filter(|c| c.owner == client)
            .map(|c| c.epoch)
        else {
            if let Some(wait) = self.pending_waits.remove(&wait_id) {
                wait.reply.reject(context_unavailable_error());
            }
            return;
        };
        let script = match build_wait_check_script(&wait.condition) {
            Ok(s) => s,
            Err(err) => {
                if let Some(wait) = self.pending_waits.remove(&wait_id) {
                    wait.reply.reject(err);
                }
                return;
            }
        };
        let eval_id = self.alloc_eval_id();
        let Some(ctx) = self.contexts.get(&context_id) else {
            if let Some(wait) = self.pending_waits.remove(&wait_id) {
                wait.reply.reject(context_unavailable_error());
            }
            return;
        };
        match self.backend.start_script(eval_id, &ctx.session, &script) {
            Ok(handle) => {
                self.pending_evals.insert(
                    eval_id,
                    PendingEval {
                        client,
                        context_id,
                        captured_epoch,
                        kind: PendingEvalKind::WaitProbe { wait_id },
                        _handle: handle,
                    },
                );
            }
            Err(err) => {
                if let Some(wait) = self.pending_waits.remove(&wait_id) {
                    wait.reply.reject(err);
                }
            }
        }
    }

    fn handle_wait_probe_result(&mut self, wait_id: u64, result: Result<String, ToolError>) {
        let Some(wait) = self.pending_waits.remove(&wait_id) else {
            return;
        };
        if wait.reply.is_closed() {
            return;
        }
        let Some(ctx) = self
            .contexts
            .get(&wait.context_id)
            .filter(|c| c.owner == wait.client)
        else {
            wait.reply.reject(context_unavailable_error());
            return;
        };
        let elapsed_ms = u64::try_from(wait.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        match result.and_then(|raw| parse_wait_check_reply(&raw)) {
            Ok(true) => {
                wait.reply.resolve(AgentWaitResult {
                    context_id: wait.context_id,
                    epoch: ctx.epoch,
                    status: AgentWaitStatus::Matched,
                    elapsed_ms,
                });
            }
            Ok(false) => {
                let now = Instant::now();
                if now >= wait.deadline {
                    wait.reply.resolve(AgentWaitResult {
                        context_id: wait.context_id,
                        epoch: ctx.epoch,
                        status: AgentWaitStatus::TimedOut,
                        elapsed_ms,
                    });
                    return;
                }
                let remaining_ms =
                    u64::try_from(wait.deadline.saturating_duration_since(now).as_millis())
                        .unwrap_or(WAIT_RECHECK_INTERVAL_MS);
                let delay = remaining_ms.clamp(1, WAIT_RECHECK_INTERVAL_MS);
                let timer_id = self.alloc_timer_id();
                let timer_handle = self.backend.schedule_timer(timer_id, delay);
                self.timers
                    .insert(timer_id, TimerAction::WaitRecheck { wait_id });
                self.pending_waits.insert(
                    wait_id,
                    PendingWait {
                        _timer: Some(timer_handle),
                        ..wait
                    },
                );
            }
            Err(err) => wait.reply.reject(err),
        }
    }

    fn handle_screenshot(
        &mut self,
        client: ClientId,
        context_id: AgentContextId,
        target: AgentScreenshotTarget,
        max_bytes: usize,
        reply: HostReply<AgentScreenshotResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let Some(ctx) = self.contexts.get(&context_id).filter(|c| c.owner == client) else {
            reply.reject(context_unavailable_error());
            return;
        };
        if let AgentScreenshotTarget::Element(elem_ref) = target {
            if elem_ref.context_id != context_id || elem_ref.epoch != ctx.epoch {
                reply.reject(stale_document_error());
            } else {
                reply.reject(ToolError::new(
                    ToolErrorCode::UnsupportedCapability,
                    "element-scoped native screenshots are unavailable in WebKitGTK Automation",
                ));
            }
            return;
        }
        self.dispatch_viewport_screenshot(context_id, max_bytes, reply);
    }

    fn dispatch_viewport_screenshot(
        &mut self,
        context_id: AgentContextId,
        max_bytes: usize,
        reply: HostReply<AgentScreenshotResult>,
    ) {
        let Some(ctx) = self.contexts.get(&context_id) else {
            reply.reject(context_unavailable_error());
            return;
        };
        let captured_epoch = ctx.epoch;
        let Some(handle) = ctx.browsing_context_handle.clone() else {
            self.ensure_handle_query(
                context_id,
                Some(DeferredAfterHandle::Screenshot { max_bytes, reply }),
            );
            return;
        };
        let command = AutomationWireCommand::TakeScreenshot {
            handle,
            clip_to_viewport: true,
        };
        self.send_command(
            context_id,
            &command,
            Some(DEFAULT_COMMAND_TIMEOUT_MS),
            PendingCommandKind::Screenshot {
                captured_epoch,
                max_bytes,
                reply,
            },
        );
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Exhaustive single-place correlation of native Automation command replies"
    )]
    fn handle_command_reply(
        &mut self,
        context_id: AgentContextId,
        id: u64,
        outcome: Result<Value, ToolError>,
    ) {
        let pending = {
            let Some(ctx) = self.contexts.get_mut(&context_id) else {
                return;
            };
            ctx.pending_cmds.remove(&id)
        };
        let Some(pending) = pending else {
            // Duplicate, late, timed-out, or unknown ID: ignore safely.
            return;
        };

        match pending.kind {
            PendingCommandKind::InitHandle { deferred } => {
                if let Some(ctx) = self.contexts.get_mut(&context_id) {
                    ctx.handle_in_flight = false;
                }
                match outcome.and_then(|val| decode_browsing_context_handle(&val)) {
                    Ok(handle) => {
                        if let Some(ctx) = self.contexts.get_mut(&context_id) {
                            ctx.browsing_context_handle = Some(handle.clone());
                        }
                        self.bootstrap_inspector(context_id, &handle);
                        if let Some(next) = deferred {
                            match next {
                                DeferredAfterHandle::Navigate {
                                    url,
                                    wait_stage,
                                    timeout_ms,
                                    reply,
                                } => self.dispatch_approved_navigate(
                                    context_id, url, wait_stage, timeout_ms, reply,
                                ),
                                DeferredAfterHandle::Act {
                                    action,
                                    resolved,
                                    reply,
                                } => {
                                    self.dispatch_approved_act(context_id, action, resolved, reply);
                                }
                                DeferredAfterHandle::Screenshot { max_bytes, reply } => {
                                    self.dispatch_viewport_screenshot(context_id, max_bytes, reply);
                                }
                            }
                        }
                    }
                    Err(err) => {
                        if let Some(next) = deferred {
                            reject_deferred(next, err);
                        }
                    }
                }
            }
            PendingCommandKind::Navigate { url, reply } => {
                if reply.is_closed() {
                    return;
                }
                let Some(ctx) = self.contexts.get_mut(&context_id) else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                match outcome {
                    Ok(_) => {
                        ctx.epoch = ctx.epoch.saturating_add(1);
                        let origin = url.origin().ascii_serialization();
                        ctx.origin = Some(origin.clone());
                        reply.resolve(AgentNavigateResult {
                            context_id,
                            status: AgentNavigateStatus::Committed,
                            epoch: ctx.epoch,
                            origin: Some(origin),
                            reason: None,
                        });
                    }
                    Err(err) => reply.reject(err),
                }
            }
            PendingCommandKind::ActFocusThenKeyboard {
                action_element,
                steps,
                start_epoch,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                if let Err(err) = outcome {
                    reply.reject(err);
                    return;
                }
                let Some(handle) = self
                    .contexts
                    .get(&context_id)
                    .and_then(|ctx| ctx.browsing_context_handle.clone())
                else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                let command = AutomationWireCommand::KeyboardInteractions { handle, steps };
                self.send_command(
                    context_id,
                    &command,
                    Some(DEFAULT_COMMAND_TIMEOUT_MS),
                    PendingCommandKind::ActFinish {
                        action_element,
                        start_epoch,
                        reply,
                    },
                );
            }
            PendingCommandKind::ActFinish {
                action_element,
                start_epoch,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                if let Err(err) = outcome {
                    reply.reject(err);
                    return;
                }
                let Some((doc_token, client, captured_epoch, origin)) = self
                    .contexts
                    .get(&context_id)
                    .map(|ctx| (ctx.doc_token(), ctx.owner, ctx.epoch, ctx.origin.clone()))
                else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                if let Ok(script) =
                    build_observed_state_script(action_element.as_deref(), &doc_token)
                {
                    let eval_id = self.alloc_eval_id();
                    if let Some(ctx) = self.contexts.get(&context_id)
                        && let Ok(handle) =
                            self.backend.start_script(eval_id, &ctx.session, &script)
                    {
                        self.pending_evals.insert(
                            eval_id,
                            PendingEval {
                                client,
                                context_id,
                                captured_epoch,
                                kind: PendingEvalKind::ActObserve { start_epoch, reply },
                                _handle: handle,
                            },
                        );
                        return;
                    }
                }
                reply.resolve(AgentActResult {
                    context_id,
                    status: AgentActStatus::Performed,
                    epoch: captured_epoch,
                    origin,
                    reason: None,
                    observed_state: Some(AgentObservedState {
                        navigated: captured_epoch != start_epoch,
                        ..AgentObservedState::default()
                    }),
                });
            }
            PendingCommandKind::Screenshot {
                captured_epoch,
                max_bytes,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let Some(ctx) = self.contexts.get(&context_id) else {
                    reply.reject(context_unavailable_error());
                    return;
                };
                match outcome.and_then(|val| decode_screenshot_reply(&val, max_bytes)) {
                    Ok((width, height, png_bytes)) => reply.resolve(AgentScreenshotResult {
                        context_id,
                        epoch: ctx.epoch.max(captured_epoch),
                        width,
                        height,
                        png_bytes,
                    }),
                    Err(err) => reply.reject(err),
                }
            }
            PendingCommandKind::InspectorSub => {}
        }
    }

    fn bootstrap_inspector(&mut self, context_id: AgentContextId, handle: &str) {
        let initial_commands = {
            let Some(ctx) = self.contexts.get_mut(&context_id) else {
                return;
            };
            if ctx.inspector_bootstrapped {
                return;
            }
            ctx.inspector_bootstrapped = true;
            ctx.decoder.initial_direct_commands()
        };
        for raw in initial_commands {
            if let Some(command) = parse_allowlisted_command(&raw) {
                let wire = AutomationWireCommand::SendInspectorMessage {
                    handle: handle.to_owned(),
                    command,
                };
                self.send_command(context_id, &wire, None, PendingCommandKind::InspectorSub);
            }
        }
    }

    fn handle_inspector_message(&mut self, context_id: AgentContextId, raw_frame: &str) {
        let (handle, outcome) = {
            let Some(ctx) = self.contexts.get_mut(&context_id) else {
                return;
            };
            let handle = ctx.browsing_context_handle.clone();
            let outcome = ctx.decoder.decode_message(raw_frame);
            let epoch = ctx.epoch;
            for signal in outcome.signals {
                match signal {
                    DiagnosticSignal::Event(data) => ctx.events.push(epoch, data),
                    DiagnosticSignal::Gap(
                        DiagnosticGapReason::NativeQueueOverflow { .. }
                        | DiagnosticGapReason::RequestMapOverflow { .. },
                    ) => {
                        ctx.events.native_overflow = true;
                    }
                    DiagnosticSignal::Gap(DiagnosticGapReason::ChannelClosed) => {}
                }
            }
            (handle, outcome.commands)
        };
        let Some(handle) = handle else {
            return;
        };
        for raw_command in outcome {
            if let Some(command) = parse_allowlisted_command(&raw_command) {
                let wire = AutomationWireCommand::SendInspectorMessage {
                    handle: handle.clone(),
                    command,
                };
                self.send_command(context_id, &wire, None, PendingCommandKind::InspectorSub);
            }
        }
    }

    fn ensure_handle_query(
        &mut self,
        context_id: AgentContextId,
        deferred: Option<DeferredAfterHandle>,
    ) {
        let already_in_flight = self
            .contexts
            .get(&context_id)
            .is_some_and(|ctx| ctx.handle_in_flight);
        if already_in_flight && deferred.is_none() {
            return;
        }
        if let Some(ctx) = self.contexts.get_mut(&context_id) {
            ctx.handle_in_flight = true;
        }
        if !self.send_command(
            context_id,
            &AutomationWireCommand::GetBrowsingContexts,
            Some(DEFAULT_COMMAND_TIMEOUT_MS),
            PendingCommandKind::InitHandle { deferred },
        ) && let Some(ctx) = self.contexts.get_mut(&context_id)
        {
            ctx.handle_in_flight = false;
        }
    }

    fn send_command(
        &mut self,
        context_id: AgentContextId,
        command: &AutomationWireCommand,
        timeout_ms: Option<u64>,
        kind: PendingCommandKind,
    ) -> bool {
        let Some(ctx) = self.contexts.get_mut(&context_id) else {
            reject_pending_command(kind, context_unavailable_error());
            return false;
        };
        if ctx.pending_cmds.len() >= MAX_IN_FLIGHT_COMMANDS {
            reject_pending_command(
                kind,
                ToolError::new(
                    ToolErrorCode::Busy,
                    "too many in-flight automation commands for context",
                ),
            );
            return false;
        }
        let cmd_id = match ctx.next_command_id() {
            Ok(id) => id,
            Err(err) => {
                reject_pending_command(kind, err);
                return false;
            }
        };
        if let Err(err) = self
            .backend
            .dispatch_command(&mut ctx.session, cmd_id, command)
        {
            reject_pending_command(kind, err);
            return false;
        }
        let deadline = timeout_ms.map(|ms| {
            let timer_id = self.next_timer_id.saturating_add(1);
            self.next_timer_id = timer_id;
            let handle = self.backend.schedule_timer(timer_id, ms);
            self.timers.insert(
                timer_id,
                TimerAction::CommandDeadline { context_id, cmd_id },
            );
            handle
        });
        ctx.pending_cmds.insert(
            cmd_id,
            PendingCommand {
                kind,
                _deadline: deadline,
            },
        );
        true
    }

    fn teardown_context(&mut self, context_id: AgentContextId, reason: &ToolError) {
        if self
            .pending_approval
            .as_ref()
            .is_some_and(|p| p.context_id == context_id)
            && let Some(pending) = self.pending_approval.take()
        {
            self.backend.hide_approval();
            reject_pending_approval(pending.kind, reason.clone());
        }

        let eval_ids: Vec<u64> = self
            .pending_evals
            .iter()
            .filter_map(|(&id, ev)| (ev.context_id == context_id).then_some(id))
            .collect();
        for id in eval_ids {
            if let Some(ev) = self.pending_evals.remove(&id) {
                reject_pending_eval(ev.kind, reason.clone());
            }
        }

        let wait_ids: Vec<u64> = self
            .pending_waits
            .iter()
            .filter_map(|(&id, w)| (w.context_id == context_id).then_some(id))
            .collect();
        for id in wait_ids {
            if let Some(wait) = self.pending_waits.remove(&id) {
                wait.reply.reject(reason.clone());
            }
        }

        if let Some(mut ctx) = self.contexts.remove(&context_id) {
            for (_, pending) in std::mem::take(&mut ctx.pending_cmds) {
                reject_pending_command(pending.kind, reason.clone());
            }
            let _ = ctx.decoder.on_channel_closed();
            self.backend.close_session(ctx.session);
        }
    }

    fn alloc_eval_id(&mut self) -> u64 {
        self.next_eval_id = self.next_eval_id.saturating_add(1);
        self.next_eval_id
    }

    fn alloc_wait_id(&mut self) -> u64 {
        self.next_wait_id = self.next_wait_id.saturating_add(1);
        self.next_wait_id
    }

    fn alloc_timer_id(&mut self) -> u64 {
        self.next_timer_id = self.next_timer_id.saturating_add(1);
        self.next_timer_id
    }
}

const fn action_element_ref(action: &AgentAction) -> Option<&AgentElementRef> {
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

fn reject_deferred(deferred: DeferredAfterHandle, error: ToolError) {
    match deferred {
        DeferredAfterHandle::Navigate { reply, .. } => reply.reject(error),
        DeferredAfterHandle::Act { reply, .. } => reply.reject(error),
        DeferredAfterHandle::Screenshot { reply, .. } => reply.reject(error),
    }
}

fn reject_pending_command(kind: PendingCommandKind, error: ToolError) {
    match kind {
        PendingCommandKind::InitHandle { deferred } => {
            if let Some(d) = deferred {
                reject_deferred(d, error);
            }
        }
        PendingCommandKind::Navigate { reply, .. } => reply.reject(error),
        PendingCommandKind::ActFocusThenKeyboard { reply, .. }
        | PendingCommandKind::ActFinish { reply, .. } => reply.reject(error),
        PendingCommandKind::Screenshot { reply, .. } => reply.reject(error),
        PendingCommandKind::InspectorSub => {}
    }
}

fn reject_pending_eval(kind: PendingEvalKind, error: ToolError) {
    match kind {
        PendingEvalKind::Locate { reply, .. } => reply.reject(error),
        PendingEvalKind::ActResolve { reply, .. } | PendingEvalKind::ActObserve { reply, .. } => {
            reply.reject(error);
        }
        PendingEvalKind::Snapshot { reply, .. } => reply.reject(error),
        PendingEvalKind::WaitProbe { .. } => {}
    }
}

fn reject_pending_approval(kind: PendingApprovalKind, error: ToolError) {
    match kind {
        PendingApprovalKind::Navigate { reply, .. } => reply.reject(error),
        PendingApprovalKind::Act { reply, .. } => reply.reject(error),
    }
}

fn unavailable_error() -> ToolError {
    ToolError::new(
        ToolErrorCode::ToolNotAvailable,
        "isolated agent contexts are unavailable in this runtime",
    )
}

fn context_unavailable_error() -> ToolError {
    ToolError::new(
        ToolErrorCode::AttachmentUnavailable,
        "agent context is not available",
    )
}

fn stale_document_error() -> ToolError {
    ToolError::new(
        ToolErrorCode::StaleDocument,
        "element reference or document epoch is stale",
    )
}
