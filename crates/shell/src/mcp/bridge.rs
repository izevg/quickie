//! Main-thread MCP host bridge and pure diagnostic session coordinator.
//!
//! - [`MacBrowserHost`] implements [`BrowserHost`] for the Tokio socket thread using a bounded
//!   non-blocking queue and `MainThreadBound<Weak<Browser>>` wakeups on `DispatchQueue::main()`.
//!   No toolkit object is ever stored in Tokio.
//! - [`McpCoordinator`] owns [`DiagnosticState`], per-tab document epochs, explicit shell reload
//!   intent, active [`DiagnosticLease`]s, and pending consent requests on the main thread. It is
//!   parameterized over [`DiagnosticBackend`] so both the real `AppKit` [`Browser`] and synthetic
//!   GUI tests run the exact same state machine.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::MainThreadMarker;
use quickie_application::mcp::{
    AttachmentId, ClientId, DiagnosticCapabilities, DiagnosticError, DiagnosticEventData,
    DiagnosticEventFilter, DiagnosticState, DocumentBinding, GrantScope, LifecycleEventKind,
    NavigationKind, ObservationTarget, RevokeReason, SnapshotKind, SnapshotRequest,
};
use quickie_application::{
    DiagnosticGapReason, DiagnosticLease, DiagnosticSignal, DiagnosticSnapshot, EngineError,
};
use quickie_domain_browsing::{Profile, TabId, WindowId};
use quickie_mcp::browser::AgentOperation;
use quickie_mcp::{
    BrowserHost, BrowserOperation, BrowserStatus, DiagnosticDetachResult, DiagnosticGrant,
    ForkCapabilities, HostDispatchError, HostPlatform, HostReply, PageEventsResult,
    PageSnapshotResult, RuntimeMode, ToolError, ToolErrorCode,
};
use url::{Origin, Url};

use crate::browser::Browser;
use crate::updates::INSTALLED_VERSION;

/// Hard ceiling on queued MCP operations waiting for the main `DispatchQueue`.
pub(crate) const MAX_QUEUED_OPS: usize = 64;

/// Hard ceiling on concurrent pending native consent prompts.
pub(crate) const MAX_PENDING_CONSENTS: usize = 16;

/// One operation enqueued from the MCP I/O thread for the main thread.
struct QueuedOp {
    client: ClientId,
    operation: BrowserOperation,
}

#[derive(Default)]
struct DispatchQueueState {
    closed: bool,
    scheduled: bool,
    pending: VecDeque<QueuedOp>,
    closed_clients: VecDeque<ClientId>,
}

/// Thread-safe [`BrowserHost`] owned by the GUI and shared with [`quickie_mcp::BrowserHandler`].
pub(crate) struct MacBrowserHost {
    queue: Arc<Mutex<DispatchQueueState>>,
    browser: Arc<MainThreadBound<Weak<Browser>>>,
}

impl MacBrowserHost {
    /// Creates a host bridge bound weakly to `browser` on the main thread.
    #[must_use]
    pub(crate) fn new(browser: Weak<Browser>, mtm: MainThreadMarker) -> Self {
        Self {
            queue: Arc::new(Mutex::new(DispatchQueueState::default())),
            browser: Arc::new(MainThreadBound::new(browser, mtm)),
        }
    }

    /// Marks the host closed and drops any queued operations so callers fail promptly.
    pub(crate) fn close(&self) {
        let drained = {
            let Ok(mut state) = self.queue.lock() else {
                return;
            };
            state.closed = true;
            state.closed_clients.clear();
            std::mem::take(&mut state.pending)
        };
        drop(drained);
    }

    fn schedule_drain(&self) {
        let queue = Arc::clone(&self.queue);
        let browser = Arc::clone(&self.browser);
        DispatchQueue::main().exec_async(move || {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let (closed_clients, ops) = {
                let Ok(mut state) = queue.lock() else {
                    return;
                };
                state.scheduled = false;
                if state.closed {
                    state.closed_clients.clear();
                    state.pending.clear();
                    return;
                }
                (
                    std::mem::take(&mut state.closed_clients),
                    std::mem::take(&mut state.pending),
                )
            };

            let Some(browser) = browser.get(mtm).upgrade() else {
                if let Ok(mut state) = queue.lock() {
                    state.closed = true;
                }
                drop(ops);
                return;
            };

            for client in closed_clients {
                browser.mcp_client_closed(client);
            }
            for item in ops {
                if is_reply_closed(&item.operation) {
                    continue;
                }
                browser.mcp_dispatch(item.client, item.operation);
            }
            browser.drain();
        });
    }
}

impl BrowserHost for MacBrowserHost {
    fn dispatch(
        &self,
        client: ClientId,
        operation: BrowserOperation,
    ) -> Result<(), HostDispatchError> {
        let should_schedule = {
            let Ok(mut state) = self.queue.lock() else {
                return Err(HostDispatchError::Closed);
            };
            if state.closed {
                return Err(HostDispatchError::Closed);
            }
            if state.pending.len() >= MAX_QUEUED_OPS {
                return Err(HostDispatchError::Full);
            }
            state.pending.push_back(QueuedOp { client, operation });
            !std::mem::replace(&mut state.scheduled, true)
        };
        if should_schedule {
            self.schedule_drain();
        }
        Ok(())
    }

    fn client_closed(&self, client: ClientId) {
        let (should_schedule, dropped_ops) = {
            let Ok(mut state) = self.queue.lock() else {
                return;
            };
            if state.closed {
                return;
            }
            // Drop any not-yet-dispatched operations from this disconnected client immediately.
            let mut remaining = VecDeque::with_capacity(state.pending.len());
            let mut dropped = Vec::new();
            for item in state.pending.drain(..) {
                if item.client == client {
                    dropped.push(item);
                } else {
                    remaining.push_back(item);
                }
            }
            state.pending = remaining;
            state.closed_clients.push_back(client);
            (!std::mem::replace(&mut state.scheduled, true), dropped)
        };
        drop(dropped_ops);
        if should_schedule {
            self.schedule_drain();
        }
    }
}

pub(crate) fn is_reply_closed(operation: &BrowserOperation) -> bool {
    match operation {
        BrowserOperation::BrowserStatus { reply } => reply.is_closed(),
        BrowserOperation::DiagnosticRequest { reply, .. } => reply.is_closed(),
        BrowserOperation::DiagnosticDetach { reply, .. } => reply.is_closed(),
        BrowserOperation::PageSnapshot { reply, .. } => reply.is_closed(),
        BrowserOperation::PageEvents { reply, .. } => reply.is_closed(),
        BrowserOperation::Agent(agent_op) => match agent_op {
            AgentOperation::ContextOpen { reply, .. } => reply.is_closed(),
            AgentOperation::ContextList { reply } => reply.is_closed(),
            AgentOperation::ContextClose { reply, .. } => reply.is_closed(),
            AgentOperation::Navigate { reply, .. } => reply.is_closed(),
            AgentOperation::Locate { reply, .. } => reply.is_closed(),
            AgentOperation::Act { reply, .. } => reply.is_closed(),
            AgentOperation::Wait { reply, .. } => reply.is_closed(),
            AgentOperation::Screenshot { reply, .. } => reply.is_closed(),
        },
    }
}

/// Engine and session surface required by [`McpCoordinator`] on the main thread.
pub(crate) trait DiagnosticBackend {
    /// Engine diagnostic capabilities on the current runtime.
    fn diagnostic_capabilities(&self) -> DiagnosticCapabilities;

    /// Whether isolated `Profile::Agent` automation is available on the current runtime.
    fn agent_automation_supported(&self) -> bool {
        false
    }

    /// Returns `(window, tab, url)` for the selected tab in the active Quickie window when it
    /// belongs to `Profile::Main` and has a live page.
    fn active_main_tab(&self) -> Option<(WindowId, TabId, Url)>;

    /// Returns the live URL for `tab` when it still belongs to `Profile::Main` and has a live page.
    fn live_tab_url(&self, tab: TabId) -> Option<Url>;

    /// Connects a read-only native diagnostic observer to `tab`'s page.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the tab has no page, the SPI is unavailable, or another observer
    /// or visual Inspector is already attached.
    fn observe_diagnostics(
        &self,
        tab: TabId,
        on_signal: Box<dyn Fn(DiagnosticSignal) + 'static>,
    ) -> Result<DiagnosticLease, EngineError>;

    /// Captures a bounded read-only snapshot from `tab`'s page in an isolated world.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the tab has no page or the snapshot kind is unsupported.
    fn diagnostic_snapshot(
        &self,
        tab: TabId,
        request: &SnapshotRequest,
        done: Box<dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static>,
    ) -> Result<(), EngineError>;
}

/// A consent prompt waiting for the user's explicit Allow/Cancel decision.
#[derive(Debug)]
pub(crate) struct PendingConsent {
    pub(crate) id: u64,
    pub(crate) client: ClientId,
    pub(crate) scope: GrantScope,
    pub(crate) window: WindowId,
    pub(crate) approved: DocumentBinding,
    pub(crate) reply: HostReply<DiagnosticGrant>,
}

/// Public snapshot of the front pending consent prompt for UI presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConsentPrompt {
    pub(crate) id: u64,
    pub(crate) window: WindowId,
    pub(crate) tab: TabId,
    pub(crate) origin: Origin,
    pub(crate) scope: GrantScope,
}

#[derive(Clone, Debug)]
struct PendingReload {
    origin: Origin,
    url: Url,
}

struct AttachmentRecord {
    client: ClientId,
    tab: TabId,
    capabilities: DiagnosticCapabilities,
    native_overflow: bool,
    _lease: DiagnosticLease,
}

/// Outcome of dispatching a [`BrowserOperation`] in [`McpCoordinator`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DispatchEffect {
    /// No UI refresh needed.
    None,
    /// Consent queue changed; UI should present or hide the consent sheet.
    ConsentChanged,
    /// Active attachments changed; UI should refresh the attached-tab indicator.
    AttachmentsChanged,
}

/// Pure main-thread coordinator for GUI MCP diagnostics.
#[derive(Default)]
pub(crate) struct McpCoordinator {
    state: DiagnosticState,
    epochs: HashMap<TabId, u64>,
    pending_reload: HashMap<TabId, PendingReload>,
    attachments: HashMap<AttachmentId, AttachmentRecord>,
    next_consent_id: u64,
    pending_consents: VecDeque<PendingConsent>,
}

impl McpCoordinator {
    /// Creates an empty coordinator.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Number of currently active diagnostic attachments.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn active_attachment_count(&self) -> usize {
        self.state.active_count()
    }

    /// Whether `tab` currently has an active diagnostic attachment.
    #[must_use]
    pub(crate) fn has_attachments_for_tab(&self, tab: TabId) -> bool {
        self.state.has_attachments_for_tab(tab)
    }

    /// Returns the front pending consent prompt, if any, pruning closed replies first.
    pub(crate) fn front_consent(&mut self) -> Option<ConsentPrompt> {
        self.prune_closed_consents();
        self.pending_consents.front().map(|item| ConsentPrompt {
            id: item.id,
            window: item.window,
            tab: item.approved.tab,
            origin: item.approved.origin.clone(),
            scope: item.scope,
        })
    }

    /// Builds honest [`BrowserStatus`] for the running GUI host.
    #[must_use]
    pub(crate) fn browser_status(backend: &impl DiagnosticBackend) -> BrowserStatus {
        let diagnostic = backend.diagnostic_capabilities();
        let has_diagnostic_spi = diagnostic.console
            || diagnostic.runtime_errors
            || diagnostic.pending_network
            || diagnostic.completed_network
            || diagnostic.lifecycle;
        BrowserStatus {
            platform: HostPlatform::Macos,
            runtime_mode: RuntimeMode::Gui,
            product_version: INSTALLED_VERSION.to_owned(),
            fork_capabilities: ForkCapabilities {
                diagnostic,
                native_accessibility: diagnostic.accessibility,
                visual_inspector_priority: has_diagnostic_spi,
                agent_automation: backend.agent_automation_supported(),
            },
        }
    }

    /// Handles a typed [`BrowserOperation`] on the main thread.
    ///
    /// For `PageSnapshot`, `on_snapshot_complete` is invoked by the engine callback and should
    /// route back to [`Self::complete_snapshot`].
    pub(crate) fn handle_operation(
        &mut self,
        client: ClientId,
        operation: BrowserOperation,
        backend: &impl DiagnosticBackend,
        make_snapshot_done: impl FnOnce(
            AttachmentId,
            DocumentBinding,
            SnapshotKind,
            HostReply<PageSnapshotResult>,
        ) -> Box<
            dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static,
        >,
    ) -> DispatchEffect {
        match operation {
            BrowserOperation::BrowserStatus { reply } => {
                reply.resolve(Self::browser_status(backend));
                DispatchEffect::None
            }
            BrowserOperation::DiagnosticRequest { scope, reply } => {
                self.enqueue_diagnostic_request(client, scope, reply, backend)
            }
            BrowserOperation::DiagnosticDetach { attachment, reply } => {
                self.handle_detach(client, attachment, reply)
            }
            BrowserOperation::PageSnapshot {
                target,
                request,
                reply,
            } => {
                self.handle_page_snapshot(
                    client,
                    target,
                    &request,
                    reply,
                    backend,
                    make_snapshot_done,
                );
                DispatchEffect::None
            }
            BrowserOperation::PageEvents {
                target,
                cursor,
                filter,
                limit,
                reply,
            } => {
                self.handle_page_events(client, target, cursor, &filter, limit, reply);
                DispatchEffect::None
            }
            BrowserOperation::Agent(agent_op) => {
                agent_op.reject(ToolError::new(
                    ToolErrorCode::ToolNotAvailable,
                    "isolated agent contexts are not available in this host",
                ));
                DispatchEffect::None
            }
        }
    }

    fn enqueue_diagnostic_request(
        &mut self,
        client: ClientId,
        scope: GrantScope,
        reply: HostReply<DiagnosticGrant>,
        backend: &impl DiagnosticBackend,
    ) -> DispatchEffect {
        if reply.is_closed() {
            return DispatchEffect::None;
        }
        self.prune_closed_consents();

        let caps = backend.diagnostic_capabilities();
        if !has_any_diagnostic_capability(caps) {
            reply.reject(ToolError::new(
                ToolErrorCode::UnsupportedCapability,
                "diagnostic observation is unavailable on this engine",
            ));
            return DispatchEffect::None;
        }

        let Some((window, tab, url)) = backend.active_main_tab() else {
            reply.reject(ToolError::new(
                ToolErrorCode::ApprovalDenied,
                "no eligible web page is active for diagnostics",
            ));
            return DispatchEffect::None;
        };

        let binding = DocumentBinding {
            tab,
            origin: url.origin(),
            epoch: self.epoch_for(tab),
        };
        if !binding.is_valid_main() {
            reply.reject(ToolError::new(
                ToolErrorCode::ApprovalDenied,
                "active tab is not an eligible HTTP(S) page",
            ));
            return DispatchEffect::None;
        }

        if self.pending_consents.len() >= MAX_PENDING_CONSENTS {
            reply.reject(ToolError::new(
                ToolErrorCode::Busy,
                "too many pending diagnostic consent requests",
            ));
            return DispatchEffect::None;
        }

        self.next_consent_id = self.next_consent_id.saturating_add(1);
        self.pending_consents.push_back(PendingConsent {
            id: self.next_consent_id,
            client,
            scope,
            window,
            approved: binding,
            reply,
        });
        DispatchEffect::ConsentChanged
    }

    /// Resolves a pending consent request after the user clicked Allow or Cancel.
    pub(crate) fn resolve_consent(
        &mut self,
        consent_id: u64,
        approved_by_user: bool,
        backend: &impl DiagnosticBackend,
        make_signal_cb: impl FnOnce(
            Rc<Cell<Option<AttachmentId>>>,
            TabId,
        ) -> Box<dyn Fn(DiagnosticSignal) + 'static>,
    ) -> DispatchEffect {
        let Some(pos) = self
            .pending_consents
            .iter()
            .position(|item| item.id == consent_id)
        else {
            return DispatchEffect::None;
        };
        let Some(consent) = self.pending_consents.remove(pos) else {
            return DispatchEffect::None;
        };

        if consent.reply.is_closed() {
            return DispatchEffect::ConsentChanged;
        }

        if !approved_by_user {
            consent.reply.reject(ToolError::new(
                ToolErrorCode::ApprovalDenied,
                "diagnostic request denied by user",
            ));
            return DispatchEffect::ConsentChanged;
        }

        let live = self.live_binding_for(consent.approved.tab, backend);
        if live.as_ref() != Some(&consent.approved) || !consent.approved.is_valid_main() {
            consent.reply.reject(ToolError::new(
                ToolErrorCode::StaleDocument,
                "active document changed before diagnostic consent was confirmed",
            ));
            return DispatchEffect::ConsentChanged;
        }

        // Another client already owns an active diagnostic lease on this tab.
        if self
            .attachments
            .values()
            .any(|rec| rec.tab == consent.approved.tab && rec.client != consent.client)
        {
            consent.reply.reject(ToolError::new(
                ToolErrorCode::Busy,
                "tab already has an active diagnostic attachment",
            ));
            return DispatchEffect::ConsentChanged;
        }

        // If this same client already holds an attachment on the same tab, release its native
        // lease first so `_WKInspector` accepts the replacement cleanly.
        let previous_same_tab = self
            .attachments
            .iter()
            .find(|(_, rec)| rec.client == consent.client && rec.tab == consent.approved.tab)
            .map(|(&id, _)| id);
        if let Some(old_id) = previous_same_tab {
            let _ = self.attachments.remove(&old_id);
            let _ = self.state.detach(consent.client, old_id);
        }

        let caps = backend.diagnostic_capabilities();
        let attachment_slot = Rc::new(Cell::new(None));
        let on_signal = make_signal_cb(Rc::clone(&attachment_slot), consent.approved.tab);
        let lease = match backend.observe_diagnostics(consent.approved.tab, on_signal) {
            Ok(lease) => lease,
            Err(error) => {
                consent.reply.reject(classify_observe_error(&error, caps));
                return DispatchEffect::AttachmentsChanged;
            }
        };

        match self.state.enable(
            consent.client,
            consent.approved.clone(),
            live.as_ref(),
            consent.scope,
        ) {
            Ok(enabled) => {
                attachment_slot.set(Some(enabled.attachment));
                if let Some(replaced) = enabled.replaced {
                    let _ = self.attachments.remove(&replaced.attachment);
                }
                self.attachments.insert(
                    enabled.attachment,
                    AttachmentRecord {
                        client: consent.client,
                        tab: consent.approved.tab,
                        capabilities: caps,
                        native_overflow: false,
                        _lease: lease,
                    },
                );
                consent.reply.resolve(DiagnosticGrant {
                    attachment_id: enabled.attachment,
                    scope: consent.scope,
                    epoch: consent.approved.epoch,
                    origin: consent.approved.origin.ascii_serialization(),
                    capabilities: caps,
                });
                DispatchEffect::AttachmentsChanged
            }
            Err(error) => {
                drop(lease);
                consent.reply.reject(ToolError::from(error));
                DispatchEffect::AttachmentsChanged
            }
        }
    }

    fn handle_detach(
        &mut self,
        client: ClientId,
        attachment: AttachmentId,
        reply: HostReply<DiagnosticDetachResult>,
    ) -> DispatchEffect {
        match self.state.detach(client, attachment) {
            Ok(revoked) => {
                let _ = self.attachments.remove(&revoked.attachment);
                reply.resolve(DiagnosticDetachResult {
                    attachment_id: revoked.attachment,
                    detached: true,
                    reason: revoked.reason,
                });
                DispatchEffect::AttachmentsChanged
            }
            Err(error) => {
                reply.reject(ToolError::from(error));
                DispatchEffect::None
            }
        }
    }

    fn handle_page_snapshot(
        &self,
        client: ClientId,
        target: ObservationTarget,
        request: &SnapshotRequest,
        reply: HostReply<PageSnapshotResult>,
        backend: &impl DiagnosticBackend,
        make_snapshot_done: impl FnOnce(
            AttachmentId,
            DocumentBinding,
            SnapshotKind,
            HostReply<PageSnapshotResult>,
        ) -> Box<
            dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static,
        >,
    ) {
        let attachment = match target {
            ObservationTarget::Diagnostic(id) => id,
            ObservationTarget::Agent(_) => {
                reply.reject(ToolError::new(
                    ToolErrorCode::ToolNotAvailable,
                    "isolated agent contexts are not available in this host",
                ));
                return;
            }
        };

        let binding = match self.state.binding_for(client, attachment) {
            Ok(binding) => binding.clone(),
            Err(error) => {
                reply.reject(ToolError::from(error));
                return;
            }
        };

        let Some(record) = self.attachments.get(&attachment) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "diagnostic attachment is not available",
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

        let live = self.live_binding_for(binding.tab, backend);
        if live.as_ref() != Some(&binding) {
            reply.reject(ToolError::new(
                ToolErrorCode::StaleDocument,
                "bound document changed before snapshot started",
            ));
            return;
        }

        let tab = binding.tab;
        let kind = request.kind();
        let done = make_snapshot_done(attachment, binding, kind, reply);
        if let Err(error) = backend.diagnostic_snapshot(tab, request, done) {
            // `done` was moved into `diagnostic_snapshot`, so dropping it notifies the caller if
            // the backend did not invoke it.
            let _ = error;
        }
    }

    /// Revalidates `attachment` and `captured` binding on snapshot completion and resolves `reply`.
    pub(crate) fn complete_snapshot(
        &self,
        attachment: AttachmentId,
        captured: &DocumentBinding,
        kind: SnapshotKind,
        result: Result<DiagnosticSnapshot, EngineError>,
        reply: HostReply<PageSnapshotResult>,
    ) {
        if reply.is_closed() {
            return;
        }
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                reply.reject(classify_snapshot_error(&error));
                return;
            }
        };

        if !self.attachments.contains_key(&attachment) {
            reply.reject(ToolError::new(
                ToolErrorCode::StaleDocument,
                "diagnostic attachment was revoked before snapshot completed",
            ));
            return;
        }

        match self
            .state
            .complete_snapshot(attachment, captured, &snapshot.data, snapshot.truncated)
        {
            Ok(bounded) => reply.resolve(PageSnapshotResult {
                kind,
                snapshot: bounded,
                accessibility_source: None,
            }),
            Err(DiagnosticError::AttachmentUnavailable) => reply.reject(ToolError::new(
                ToolErrorCode::StaleDocument,
                "bound document changed before snapshot completed",
            )),
            Err(error) => reply.reject(ToolError::from(error)),
        }
    }

    fn handle_page_events(
        &self,
        client: ClientId,
        target: ObservationTarget,
        cursor: u64,
        filter: &DiagnosticEventFilter,
        limit: usize,
        reply: HostReply<PageEventsResult>,
    ) {
        let attachment = match target {
            ObservationTarget::Diagnostic(id) => id,
            ObservationTarget::Agent(_) => {
                reply.reject(ToolError::new(
                    ToolErrorCode::ToolNotAvailable,
                    "isolated agent contexts are not available in this host",
                ));
                return;
            }
        };

        let Some(record) = self.attachments.get(&attachment) else {
            reply.reject(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "diagnostic attachment is not available",
            ));
            return;
        };

        match self
            .state
            .read_events(client, attachment, cursor, filter, limit)
        {
            Ok(batch) => reply.resolve(PageEventsResult {
                batch,
                capabilities: record.capabilities,
                native_overflow: record.native_overflow,
                closed: false,
            }),
            Err(error) => reply.reject(ToolError::from(error)),
        }
    }

    /// Processes a native [`DiagnosticSignal`] for `attachment` on `tab`.
    ///
    /// Returns `true` when the active attachment set changed (for example on channel closure).
    pub(crate) fn on_native_signal(
        &mut self,
        attachment: AttachmentId,
        tab: TabId,
        signal: DiagnosticSignal,
        backend: &impl DiagnosticBackend,
    ) -> bool {
        let Some(record) = self.attachments.get_mut(&attachment) else {
            return false;
        };
        if record.tab != tab {
            return false;
        }

        match signal {
            DiagnosticSignal::Event(data) => {
                if let Some(live) = self.live_binding_for(tab, backend) {
                    let _ = self.state.record_event(attachment, &live, data);
                }
                false
            }
            DiagnosticSignal::Gap(
                DiagnosticGapReason::NativeQueueOverflow { .. }
                | DiagnosticGapReason::RequestMapOverflow { .. },
            ) => {
                record.native_overflow = true;
                false
            }
            DiagnosticSignal::Gap(DiagnosticGapReason::ChannelClosed) => {
                let _ = self.attachments.remove(&attachment);
                let _ = self.state.revoke(attachment);
                true
            }
        }
    }

    /// Records explicit shell reload intent (`Reload` / `HardReload`) for `tab`.
    pub(crate) fn on_reload_requested(&mut self, tab: TabId, current_url: &Url) {
        self.pending_reload.insert(
            tab,
            PendingReload {
                origin: current_url.origin(),
                url: current_url.clone(),
            },
        );
    }

    /// Clears explicit shell reload intent when a non-reload navigation starts on `tab`.
    pub(crate) fn on_navigation_requested(&mut self, tab: TabId) {
        self.pending_reload.remove(&tab);
    }

    /// Advances `tab`'s document epoch on main-frame commit and revokes out-of-scope attachments.
    ///
    /// Returns `true` if any active attachment or pending consent was revoked.
    pub(crate) fn on_tab_committed(&mut self, tab: TabId, url: &Url) -> bool {
        let next_epoch = self.bump_epoch(tab);
        let observed = DocumentBinding {
            tab,
            origin: url.origin(),
            epoch: next_epoch,
        };
        let kind = match self.pending_reload.remove(&tab) {
            Some(pending)
                if pending.origin == observed.origin && same_reload_target(&pending.url, url) =>
            {
                NavigationKind::Reload
            }
            _ => NavigationKind::NewDocument,
        };

        let revoked = self.state.observe_navigation(tab, &observed, kind);
        let mut changed = !revoked.is_empty();
        for entry in revoked {
            let _ = self.attachments.remove(&entry.attachment);
        }

        // Record `ReloadCommitted` for any attachment that survived a verified same-origin reload.
        if kind == NavigationKind::Reload {
            let survivors: Vec<AttachmentId> = self
                .attachments
                .iter()
                .filter(|(_, rec)| rec.tab == tab && rec.capabilities.lifecycle)
                .map(|(&id, _)| id)
                .collect();
            for id in survivors {
                let _ = self.state.record_event(
                    id,
                    &observed,
                    DiagnosticEventData::lifecycle(LifecycleEventKind::ReloadCommitted),
                );
            }
        }

        if self.reject_consents_for_tab(
            tab,
            ToolErrorCode::StaleDocument,
            "active document navigated before diagnostic consent was confirmed",
        ) {
            changed = true;
        }
        changed
    }

    /// Revokes all attachments and pending consents for a closed `tab`.
    pub(crate) fn on_tab_closed(&mut self, tab: TabId) -> bool {
        self.epochs.remove(&tab);
        self.pending_reload.remove(&tab);
        let revoked = self.state.on_tab_closed(tab);
        let mut changed = !revoked.is_empty();
        for entry in revoked {
            let _ = self.attachments.remove(&entry.attachment);
        }
        if self.reject_consents_for_tab(
            tab,
            ToolErrorCode::AttachmentUnavailable,
            "target tab closed before diagnostic consent was confirmed",
        ) {
            changed = true;
        }
        changed
    }

    /// Revokes all attachments and pending consents for a tab whose web process crashed.
    pub(crate) fn on_process_terminated(&mut self, tab: TabId) -> bool {
        self.pending_reload.remove(&tab);
        let _ = self.bump_epoch(tab);
        let revoked = self.state.on_process_terminated(tab);
        let mut changed = !revoked.is_empty();
        for entry in revoked {
            let _ = self.attachments.remove(&entry.attachment);
        }
        if self.reject_consents_for_tab(
            tab,
            ToolErrorCode::StaleDocument,
            "page process terminated before diagnostic consent was confirmed",
        ) {
            changed = true;
        }
        changed
    }

    /// Preempts all MCP diagnostic attachments on `tab` when the visual Web Inspector takes over.
    pub(crate) fn on_inspector_takeover(&mut self, tab: TabId) -> bool {
        let targets: Vec<AttachmentId> = self
            .attachments
            .iter()
            .filter(|(_, rec)| rec.tab == tab)
            .map(|(&id, _)| id)
            .collect();
        let mut changed = false;
        for id in targets {
            let _ = self.attachments.remove(&id);
            if self.state.revoke(id).is_some() {
                changed = true;
            }
        }
        if self.reject_consents_for_tab(
            tab,
            ToolErrorCode::Busy,
            "visual Web Inspector took priority on this tab",
        ) {
            changed = true;
        }
        changed
    }

    /// Releases all attachments and pending consents owned by a disconnected `client`.
    pub(crate) fn on_client_closed(&mut self, client: ClientId) -> bool {
        let mut changed = false;
        if let Some(revoked) = self.state.on_client_disconnected(client) {
            let _ = self.attachments.remove(&revoked.attachment);
            changed = true;
        }
        let before = self.pending_consents.len();
        self.pending_consents.retain(|item| item.client != client);
        if self.pending_consents.len() != before {
            changed = true;
        }
        changed
    }

    /// Explicitly revokes all active attachments when the user triggers Stop (`⌘.`).
    pub(crate) fn user_revoke_all(&mut self) -> usize {
        let ids: Vec<AttachmentId> = self.attachments.keys().copied().collect();
        let mut count = 0;
        for id in ids {
            let _ = self.attachments.remove(&id);
            if self.state.revoke(id).is_some() {
                count += 1;
            }
        }
        count
    }

    /// Revokes all active attachments and rejects all pending consents for browser shutdown.
    pub(crate) fn shutdown(&mut self) {
        let revoked = self.state.revoke_all_for_shutdown();
        for entry in revoked {
            let _ = self.attachments.remove(&entry.attachment);
        }
        self.attachments.clear();
        for consent in self.pending_consents.drain(..) {
            consent.reply.reject(ToolError::new(
                ToolErrorCode::Unavailable,
                "browser is shutting down",
            ));
        }
    }

    /// Returns the current live [`DocumentBinding`] for `tab`, if it still has a live `Main` page.
    pub(crate) fn live_binding_for(
        &self,
        tab: TabId,
        backend: &impl DiagnosticBackend,
    ) -> Option<DocumentBinding> {
        if tab.profile() != Profile::Main {
            return None;
        }
        let url = backend.live_tab_url(tab)?;
        Some(DocumentBinding {
            tab,
            origin: url.origin(),
            epoch: self.current_epoch(tab),
        })
    }

    fn epoch_for(&mut self, tab: TabId) -> u64 {
        *self.epochs.entry(tab).or_insert(1)
    }

    fn current_epoch(&self, tab: TabId) -> u64 {
        self.epochs.get(&tab).copied().unwrap_or(1)
    }

    fn bump_epoch(&mut self, tab: TabId) -> u64 {
        let entry = self.epochs.entry(tab).or_insert(1);
        *entry = entry.saturating_add(1);
        *entry
    }

    fn prune_closed_consents(&mut self) {
        self.pending_consents.retain(|item| !item.reply.is_closed());
    }

    fn reject_consents_for_tab(&mut self, tab: TabId, code: ToolErrorCode, message: &str) -> bool {
        let mut kept = VecDeque::with_capacity(self.pending_consents.len());
        let mut removed = false;
        for consent in self.pending_consents.drain(..) {
            if consent.approved.tab == tab {
                consent.reply.reject(ToolError::new(code, message));
                removed = true;
            } else {
                kept.push_back(consent);
            }
        }
        self.pending_consents = kept;
        removed
    }
}

/// Returns `true` only when `observed` stays on the same path and query as `requested` (ignoring
/// URL fragment), so a same-origin redirect during reload fails closed as `NewDocument`.
fn same_reload_target(requested: &Url, observed: &Url) -> bool {
    requested.path() == observed.path() && requested.query() == observed.query()
}

fn has_any_diagnostic_capability(caps: DiagnosticCapabilities) -> bool {
    caps.dom
        || caps.accessibility
        || caps.console
        || caps.runtime_errors
        || caps.pending_network
        || caps.completed_network
        || caps.lifecycle
}

fn classify_observe_error(error: &EngineError, caps: DiagnosticCapabilities) -> ToolError {
    let lower = error.0.to_ascii_lowercase();
    if lower.contains("requires")
        || lower.contains("unavailable")
        || !caps.console && !caps.lifecycle
    {
        return ToolError::new(ToolErrorCode::UnsupportedCapability, &error.0);
    }
    if lower.contains("attach") || lower.contains("occupied") || lower.contains("busy") {
        return ToolError::new(ToolErrorCode::Busy, &error.0);
    }
    ToolError::new(ToolErrorCode::Unavailable, &error.0)
}

fn classify_snapshot_error(error: &EngineError) -> ToolError {
    let lower = error.0.to_ascii_lowercase();
    if lower.contains("selector") {
        return ToolError::new(ToolErrorCode::InvalidParams, &error.0);
    }
    if lower.contains("unavailable") {
        return ToolError::new(ToolErrorCode::UnsupportedCapability, &error.0);
    }
    ToolError::new(ToolErrorCode::Unavailable, &error.0)
}

/// Ensures [`RevokeReason`] stays referenced in this module's public contract.
const _: Option<RevokeReason> = None;
