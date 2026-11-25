//! Toolkit-thread MCP controller coordinating `DiagnosticState`, consent, leases, and snapshots.

use std::collections::HashMap;

use quickie_application::mcp::{
    AgentContextId, AttachmentId, ClientId, DiagnosticCapabilities, DiagnosticEventData,
    DiagnosticState, DocumentBinding, GrantScope, LifecycleEventKind, NavigationKind,
    ObservationTarget, SnapshotKind, SnapshotRequest,
};
use quickie_application::{DiagnosticGapReason, DiagnosticSignal, DiagnosticSnapshot};
use quickie_domain_browsing::TabId;
use quickie_engine_webkit::observation::ObservationDecoder;
use quickie_mcp::{
    BrowserOperation, BrowserStatus, DiagnosticDetachResult, DiagnosticGrant, ForkCapabilities,
    HostPlatform, HostReply, PageEventsResult, PageSnapshotResult, RuntimeMode, ToolError,
    ToolErrorCode,
};

use crate::mcp::agent::{AgentBackend, AgentManager, UnavailableAgentBackend};

use super::HostEvent;
use super::protocol::{DecodedSubscription, parse_allowlisted_command};

/// Consent prompt details presented by the native GTK modal.
///
/// Only sanitized origin and requested scope are shown; no pre-consent metadata is returned to MCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsentPrompt {
    pub(crate) origin: String,
    pub(crate) scope: GrantScope,
}

/// Failure modes when binding a native observation lease on the toolkit thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeaseAttachError {
    UnsupportedSpi,
    OccupiedInspector,
    ClosedStream,
}

impl LeaseAttachError {
    fn into_tool_error(self) -> ToolError {
        match self {
            Self::UnsupportedSpi => ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "native diagnostic observation requires Quickie's WebKitGTK fork",
            ),
            Self::OccupiedInspector => ToolError::new(
                ToolErrorCode::Busy,
                "Web Inspector or another diagnostic session is already attached to this tab",
            ),
            Self::ClosedStream => ToolError::new(
                ToolErrorCode::Unavailable,
                "the selected tab is no longer available for diagnostics",
            ),
        }
    }
}

/// Platform backend implemented by GTK on Linux and deterministic fakes in unit tests.
pub(crate) trait HostBackend {
    /// Retained native observation lease handle (e.g. `ObservationLease`).
    type Lease;
    /// Retained cancellable asynchronous snapshot handle (e.g. `SnapshotEval`).
    type SnapshotHandle;

    /// Product version string for `browser_status`.
    fn product_version(&self) -> String;

    /// Live engine capabilities (distinguishing Quickie's fork from stock `WebKitGTK`).
    fn capabilities(&self) -> ForkCapabilities;

    /// Returns the currently selected `Profile::Main` tab's exact live binding, if eligible.
    fn active_document(&self) -> Option<DocumentBinding>;

    /// Presents the native modal consent prompt for the captured active tab.
    fn show_consent(&mut self, prompt: ConsentPrompt);

    /// Hides the native modal consent prompt.
    fn hide_consent(&mut self);

    /// Updates the visible connected-diagnostics indicator and explicit Stop affordance.
    fn set_active_indicator(&mut self, active_origins: &[String]);

    /// Attaches a native observation lease for `binding.tab` and associates callbacks with `attachment`.
    fn attach_lease(
        &mut self,
        attachment: AttachmentId,
        binding: &DocumentBinding,
    ) -> Result<Self::Lease, LeaseAttachError>;

    /// Dispatches one allowlisted subscription command to an active native lease.
    fn dispatch_lease_command(
        &mut self,
        lease: &mut Self::Lease,
        command: &DecodedSubscription,
    ) -> bool;

    /// Explicitly stops and drops a native observation lease.
    fn stop_lease(&mut self, lease: Self::Lease);

    /// Starts an isolated-world snapshot evaluation for `binding.tab`, completing with `snapshot_id`.
    fn start_snapshot(
        &mut self,
        snapshot_id: u64,
        binding: &DocumentBinding,
        request: &SnapshotRequest,
    ) -> Result<Self::SnapshotHandle, ToolError>;
}

struct PendingConsent {
    client: ClientId,
    captured: DocumentBinding,
    scope: GrantScope,
    reply: HostReply<DiagnosticGrant>,
}

struct AttachmentRuntime<L> {
    binding: DocumentBinding,
    lease: L,
    decoder: ObservationDecoder,
    native_overflow: bool,
    closed: bool,
}

struct PendingSnapshot<S> {
    client: ClientId,
    attachment: AttachmentId,
    captured: DocumentBinding,
    kind: SnapshotKind,
    reply: HostReply<PageSnapshotResult>,
    _handle: S,
}

/// Main-thread MCP controller owning `DiagnosticState`, active native leases, consent, snapshots,
/// and isolated `Profile::Agent` contexts.
pub(crate) struct McpController<B: HostBackend, A: AgentBackend = UnavailableAgentBackend> {
    backend: B,
    agent: AgentManager<A>,
    state: DiagnosticState,
    leases: HashMap<AttachmentId, AttachmentRuntime<B::Lease>>,
    pending_consent: Option<PendingConsent>,
    next_snapshot_id: u64,
    pending_snapshots: HashMap<u64, PendingSnapshot<B::SnapshotHandle>>,
}

#[cfg(test)]
impl<B: HostBackend> McpController<B, UnavailableAgentBackend> {
    #[must_use]
    pub(crate) fn new(backend: B) -> Self {
        Self::with_agent(backend, UnavailableAgentBackend)
    }
}

impl<B: HostBackend, A: AgentBackend> McpController<B, A> {
    #[must_use]
    pub(crate) fn with_agent(backend: B, agent_backend: A) -> Self {
        Self {
            backend,
            agent: AgentManager::new(agent_backend),
            state: DiagnosticState::new(),
            leases: HashMap::new(),
            pending_consent: None,
            next_snapshot_id: 1,
            pending_snapshots: HashMap::new(),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn backend(&self) -> &B {
        &self.backend
    }

    #[cfg(test)]
    pub(crate) const fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn agent(&self) -> &AgentManager<A> {
        &self.agent
    }

    #[cfg(test)]
    pub(crate) const fn agent_mut(&mut self) -> &mut AgentManager<A> {
        &mut self.agent
    }

    #[must_use]
    pub(crate) fn has_pending_consent(&self) -> bool {
        self.pending_consent.is_some() || self.agent.has_pending_approval()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn active_attachment_count(&self) -> usize {
        self.state.active_count()
    }

    /// Processes one event drained from the bounded [`super::DispatchPort`].
    pub(crate) fn handle_event(&mut self, event: HostEvent) {
        match event {
            HostEvent::ClientClosed(client) => self.handle_client_closed(client),
            HostEvent::Operation { client, operation } => self.handle_operation(client, operation),
        }
    }

    fn handle_operation(&mut self, client: ClientId, operation: BrowserOperation) {
        match operation {
            BrowserOperation::BrowserStatus { reply } => {
                if reply.is_closed() {
                    return;
                }
                let mut capabilities = self.backend.capabilities();
                capabilities.agent_automation = self.agent.is_available();
                reply.resolve(BrowserStatus {
                    platform: HostPlatform::Linux,
                    runtime_mode: RuntimeMode::Gui,
                    product_version: self.backend.product_version(),
                    fork_capabilities: capabilities,
                });
            }
            BrowserOperation::DiagnosticRequest { scope, reply } => {
                self.handle_diagnostic_request(client, scope, reply);
            }
            BrowserOperation::DiagnosticDetach { attachment, reply } => {
                self.handle_diagnostic_detach(client, attachment, reply);
            }
            BrowserOperation::PageSnapshot {
                target,
                request,
                reply,
            } => {
                self.handle_page_snapshot(client, target, &request, reply);
            }
            BrowserOperation::PageEvents {
                target,
                cursor,
                filter,
                limit,
                reply,
            } => {
                self.handle_page_events(client, target, cursor, &filter, limit, reply);
            }
            BrowserOperation::Agent(operation) => {
                self.agent.handle_operation(client, operation);
            }
        }
    }

    fn handle_diagnostic_request(
        &mut self,
        client: ClientId,
        scope: GrantScope,
        reply: HostReply<DiagnosticGrant>,
    ) {
        if reply.is_closed() {
            return;
        }
        let capabilities = self.backend.capabilities();
        if !has_live_diagnostic_channel(capabilities.diagnostic) {
            reply.reject(ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "native diagnostic observation requires Quickie's WebKitGTK fork",
            ));
            return;
        }
        if self.pending_consent.is_some() {
            reply.reject(ToolError::new(
                ToolErrorCode::Busy,
                "another diagnostic consent prompt is already active",
            ));
            return;
        }
        let Some(captured) = self
            .backend
            .active_document()
            .filter(DocumentBinding::is_valid_main)
        else {
            reply.reject(ToolError::new(
                ToolErrorCode::ApprovalDenied,
                "no eligible active tab for diagnostic observation",
            ));
            return;
        };
        let prompt = ConsentPrompt {
            origin: captured.origin.ascii_serialization(),
            scope,
        };
        self.pending_consent = Some(PendingConsent {
            client,
            captured,
            scope,
            reply,
        });
        self.backend.show_consent(prompt);
    }

    /// Resolves the active native consent or agent approval prompt after the user clicks
    /// Allow/Deny or presses Esc/1.
    pub(crate) fn resolve_consent(&mut self, allow: bool) {
        let Some(pending) = self.pending_consent.take() else {
            self.agent.resolve_approval(allow);
            return;
        };
        self.backend.hide_consent();
        if pending.reply.is_closed() {
            return;
        }
        if !allow {
            pending.reply.reject(ToolError::new(
                ToolErrorCode::ApprovalDenied,
                "user denied diagnostic consent",
            ));
            return;
        }

        let live = self.backend.active_document();
        let enabled = match self.state.enable(
            pending.client,
            pending.captured.clone(),
            live.as_ref(),
            pending.scope,
        ) {
            Ok(enabled) => enabled,
            Err(error) => {
                pending.reply.reject(ToolError::from(error));
                return;
            }
        };

        if let Some(replaced) = enabled.replaced {
            self.release_attachment(replaced.attachment);
        }

        let mut lease = match self
            .backend
            .attach_lease(enabled.attachment, &pending.captured)
        {
            Ok(lease) => lease,
            Err(error) => {
                let _ = self.state.revoke(enabled.attachment);
                self.refresh_indicator();
                pending.reply.reject(error.into_tool_error());
                return;
            }
        };

        let mut decoder = ObservationDecoder::new();
        for raw in decoder.initial_direct_commands() {
            if let Some(command) = parse_allowlisted_command(&raw) {
                // Direct enables succeed only when the page controller proxies those domains;
                // target-routed enables follow on `Target.targetCreated`.
                let _ = self.backend.dispatch_lease_command(&mut lease, &command);
            }
        }

        self.leases.insert(
            enabled.attachment,
            AttachmentRuntime {
                binding: pending.captured.clone(),
                lease,
                decoder,
                native_overflow: false,
                closed: false,
            },
        );
        self.refresh_indicator();

        pending.reply.resolve(DiagnosticGrant {
            attachment_id: enabled.attachment,
            scope: pending.scope,
            epoch: pending.captured.epoch,
            origin: pending.captured.origin.ascii_serialization(),
            capabilities: self.backend.capabilities().diagnostic,
        });
    }

    fn handle_diagnostic_detach(
        &mut self,
        client: ClientId,
        attachment: quickie_application::mcp::AttachmentId,
        reply: HostReply<DiagnosticDetachResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        match self.state.detach(client, attachment) {
            Ok(revoked) => {
                self.release_attachment(revoked.attachment);
                self.refresh_indicator();
                reply.resolve(DiagnosticDetachResult {
                    attachment_id: revoked.attachment,
                    detached: true,
                    reason: revoked.reason,
                });
            }
            Err(error) => reply.reject(ToolError::from(error)),
        }
    }

    fn handle_page_snapshot(
        &mut self,
        client: ClientId,
        target: ObservationTarget,
        request: &SnapshotRequest,
        reply: HostReply<PageSnapshotResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let attachment = match target {
            ObservationTarget::Diagnostic(attachment) => attachment,
            ObservationTarget::Agent(context_id) => {
                self.agent
                    .handle_page_snapshot(client, context_id, request, reply);
                return;
            }
        };
        if request.kind() == SnapshotKind::Accessibility {
            reply.reject(ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "native accessibility snapshots are unavailable",
            ));
            return;
        }
        let captured = match self.state.binding_for(client, attachment) {
            Ok(binding) => binding.clone(),
            Err(error) => {
                reply.reject(ToolError::from(error));
                return;
            }
        };
        let snapshot_id = self.next_snapshot_id;
        self.next_snapshot_id = self.next_snapshot_id.saturating_add(1);
        match self.backend.start_snapshot(snapshot_id, &captured, request) {
            Ok(handle) => {
                self.pending_snapshots.insert(
                    snapshot_id,
                    PendingSnapshot {
                        client,
                        attachment,
                        captured,
                        kind: request.kind(),
                        reply,
                        _handle: handle,
                    },
                );
            }
            Err(error) => reply.reject(error),
        }
    }

    /// Completes an asynchronous isolated-world snapshot after re-verifying document identity.
    pub(crate) fn on_snapshot_completed(
        &mut self,
        snapshot_id: u64,
        result: Result<DiagnosticSnapshot, ToolError>,
    ) {
        let Some(pending) = self.pending_snapshots.remove(&snapshot_id) else {
            return;
        };
        if pending.reply.is_closed() {
            return;
        }
        let raw = match result {
            Ok(raw) => raw,
            Err(error) => {
                pending.reply.reject(error);
                return;
            }
        };
        match self.state.complete_snapshot(
            pending.attachment,
            &pending.captured,
            &raw.data,
            raw.truncated,
        ) {
            Ok(snapshot) => pending.reply.resolve(PageSnapshotResult {
                kind: pending.kind,
                snapshot,
                accessibility_source: None,
            }),
            Err(error) => pending.reply.reject(ToolError::from(error)),
        }
    }

    fn handle_page_events(
        &self,
        client: ClientId,
        target: ObservationTarget,
        cursor: u64,
        filter: &quickie_application::mcp::DiagnosticEventFilter,
        limit: usize,
        reply: HostReply<PageEventsResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let attachment = match target {
            ObservationTarget::Diagnostic(attachment) => attachment,
            ObservationTarget::Agent(context_id) => {
                self.agent
                    .handle_page_events(client, context_id, cursor, filter, limit, reply);
                return;
            }
        };
        let batch = match self
            .state
            .read_events(client, attachment, cursor, filter, limit)
        {
            Ok(batch) => batch,
            Err(error) => {
                reply.reject(ToolError::from(error));
                return;
            }
        };
        let (native_overflow, closed) = self
            .leases
            .get(&attachment)
            .map_or((false, true), |rt| (rt.native_overflow, rt.closed));
        reply.resolve(PageEventsResult {
            batch,
            capabilities: self.backend.capabilities().diagnostic,
            native_overflow,
            closed,
        });
    }

    /// Feeds one raw native Inspector protocol frame into the shared decoder and bounded event ring.
    ///
    /// Raw protocol frames never cross out of this boundary into MCP responses.
    pub(crate) fn on_native_frame(&mut self, attachment: AttachmentId, raw_frame: &str) {
        let Some(runtime) = self.leases.get_mut(&attachment) else {
            return;
        };
        if runtime.closed {
            return;
        }
        let binding = runtime.binding.clone();
        let outcome = runtime.decoder.decode_message(raw_frame);
        for raw_command in outcome.commands {
            let dispatched = parse_allowlisted_command(&raw_command).is_some_and(|cmd| {
                self.backend
                    .dispatch_lease_command(&mut runtime.lease, &cmd)
            });
            if !dispatched {
                self.revoke_native_closed(attachment);
                return;
            }
        }
        let mut channel_closed = false;
        for signal in outcome.signals {
            match signal {
                DiagnosticSignal::Event(data) => {
                    if self.state.record_event(attachment, &binding, data).is_err() {
                        channel_closed = true;
                        break;
                    }
                }
                DiagnosticSignal::Gap(
                    DiagnosticGapReason::NativeQueueOverflow { .. }
                    | DiagnosticGapReason::RequestMapOverflow { .. },
                ) => {
                    runtime.native_overflow = true;
                }
                DiagnosticSignal::Gap(DiagnosticGapReason::ChannelClosed) => {
                    channel_closed = true;
                    break;
                }
            }
        }
        if channel_closed {
            self.revoke_native_closed(attachment);
        }
    }

    /// Handles native Inspector closure (including visual Inspector takeover).
    pub(crate) fn on_native_closed(&mut self, attachment: AttachmentId) {
        self.revoke_native_closed(attachment);
    }

    /// Handles active tab switches: dismisses any pending consent for the old tab without
    /// retargeting existing approved diagnostic attachments.
    pub(crate) fn on_active_tab_changed(&mut self, active_tab: Option<TabId>) {
        let stale_consent = self
            .pending_consent
            .as_ref()
            .is_some_and(|pending| Some(pending.captured.tab) != active_tab);
        if stale_consent {
            self.resolve_consent(false);
        }
    }

    /// Applies a main-frame navigation or reload transition on `tab`.
    pub(crate) fn on_navigation(
        &mut self,
        tab: TabId,
        observed: &DocumentBinding,
        kind: NavigationKind,
    ) {
        let stale_consent = self
            .pending_consent
            .as_ref()
            .is_some_and(|pending| pending.captured.tab == tab && pending.captured != *observed);
        if stale_consent {
            self.resolve_consent(false);
        }

        let revoked = self.state.observe_navigation(tab, observed, kind);
        for entry in revoked {
            self.release_attachment(entry.attachment);
        }

        let surviving: Vec<AttachmentId> = self
            .leases
            .iter()
            .filter_map(|(&id, rt)| (rt.binding.tab == tab).then_some(id))
            .collect();
        for id in surviving {
            if let Some(runtime) = self.leases.get_mut(&id) {
                runtime.binding = observed.clone();
                let lifecycle = match kind {
                    NavigationKind::SameDocument => {
                        Some(LifecycleEventKind::SameDocumentNavigation)
                    }
                    NavigationKind::Reload => Some(LifecycleEventKind::ReloadCommitted),
                    NavigationKind::NewDocument => None,
                };
                if let Some(lifecycle_kind) = lifecycle {
                    let _ = self.state.record_event(
                        id,
                        observed,
                        DiagnosticEventData::lifecycle(lifecycle_kind),
                    );
                }
            }
        }
        self.refresh_indicator();
    }

    /// Revokes all attachments and pending requests bound to a closed `tab`.
    pub(crate) fn on_tab_closed(&mut self, tab: TabId) {
        self.cancel_consent_for_tab(tab);
        for revoked in self.state.on_tab_closed(tab) {
            self.release_attachment(revoked.attachment);
        }
        self.refresh_indicator();
    }

    /// Revokes all attachments and pending requests bound to a tab whose web process crashed.
    pub(crate) fn on_process_terminated(&mut self, tab: TabId) {
        self.cancel_consent_for_tab(tab);
        for revoked in self.state.on_process_terminated(tab) {
            self.release_attachment(revoked.attachment);
        }
        self.refresh_indicator();
    }

    /// Revokes any active diagnostic leases on `tab` when the visual Web Inspector opens.
    pub(crate) fn on_inspector_opened(&mut self, tab: TabId) {
        let affected: Vec<AttachmentId> = self
            .leases
            .iter()
            .filter_map(|(&id, rt)| (rt.binding.tab == tab).then_some(id))
            .collect();
        for id in affected {
            self.revoke_native_closed(id);
        }
    }

    /// Revokes every active diagnostic grant when the user clicks the explicit Stop button.
    pub(crate) fn stop_all_user_grants(&mut self) {
        let ids: Vec<AttachmentId> = self.leases.keys().copied().collect();
        for id in ids {
            let _ = self.state.revoke(id);
            self.release_attachment(id);
        }
        self.refresh_indicator();
    }

    /// Tears down all pending consent, snapshots, active leases, and isolated Agent contexts
    /// during GUI shutdown.
    pub(crate) fn shutdown(&mut self) {
        self.agent.shutdown();
        if let Some(pending) = self.pending_consent.take() {
            self.backend.hide_consent();
            pending.reply.reject(ToolError::new(
                ToolErrorCode::Unavailable,
                "browser is shutting down",
            ));
        }
        for (_, pending) in self.pending_snapshots.drain() {
            pending.reply.reject(ToolError::new(
                ToolErrorCode::Unavailable,
                "browser is shutting down",
            ));
        }
        for revoked in self.state.revoke_all_for_shutdown() {
            self.release_attachment(revoked.attachment);
        }
        self.refresh_indicator();
    }

    /// Forwards an inbound `WebKitAutomationSession::message-received` frame to the Agent manager.
    pub(crate) fn on_agent_message(&mut self, context_id: AgentContextId, raw: &str) {
        self.agent.on_automation_message(context_id, raw);
    }

    /// Handles native isolated Agent session closure or web process termination.
    pub(crate) fn on_agent_closed(&mut self, context_id: AgentContextId) {
        self.agent.on_session_closed(context_id);
    }

    /// Completes an asynchronous isolated-world Agent script evaluation.
    pub(crate) fn on_agent_script_completed(
        &mut self,
        eval_id: u64,
        result: Result<String, ToolError>,
    ) {
        self.agent.on_script_completed(eval_id, result);
    }

    /// Fires a scheduled one-shot Agent deadline or wait re-check timer.
    pub(crate) fn on_agent_timer_fired(&mut self, timer_id: u64) {
        self.agent.on_timer_fired(timer_id);
    }

    fn handle_client_closed(&mut self, client: ClientId) {
        self.agent.on_client_closed(client);
        if self
            .pending_consent
            .as_ref()
            .is_some_and(|pending| pending.client == client)
            && let Some(pending) = self.pending_consent.take()
        {
            self.backend.hide_consent();
            drop(pending.reply);
        }
        self.pending_snapshots
            .retain(|_, pending| pending.client != client);
        if let Some(revoked) = self.state.on_client_disconnected(client) {
            self.release_attachment(revoked.attachment);
            self.refresh_indicator();
        }
    }

    fn cancel_consent_for_tab(&mut self, tab: TabId) {
        if self
            .pending_consent
            .as_ref()
            .is_some_and(|pending| pending.captured.tab == tab)
        {
            self.resolve_consent(false);
        }
    }

    fn revoke_native_closed(&mut self, attachment: AttachmentId) {
        let _ = self.state.revoke(attachment);
        self.release_attachment(attachment);
        self.refresh_indicator();
    }

    fn release_attachment(&mut self, attachment: AttachmentId) {
        if let Some(mut runtime) = self.leases.remove(&attachment) {
            runtime.closed = true;
            let _ = runtime.decoder.on_channel_closed();
            self.backend.stop_lease(runtime.lease);
        }
        let cancelled: Vec<u64> = self
            .pending_snapshots
            .iter()
            .filter_map(|(&id, pending)| (pending.attachment == attachment).then_some(id))
            .collect();
        for id in cancelled {
            if let Some(pending) = self.pending_snapshots.remove(&id) {
                pending.reply.reject(ToolError::new(
                    ToolErrorCode::AttachmentUnavailable,
                    "diagnostic attachment is not available",
                ));
            }
        }
    }

    fn refresh_indicator(&mut self) {
        let mut origins: Vec<String> = self
            .leases
            .values()
            .map(|rt| rt.binding.origin.ascii_serialization())
            .collect();
        origins.sort();
        origins.dedup();
        self.backend.set_active_indicator(&origins);
    }
}

const fn has_live_diagnostic_channel(caps: DiagnosticCapabilities) -> bool {
    caps.console
        || caps.runtime_errors
        || caps.pending_network
        || caps.completed_network
        || caps.lifecycle
}
