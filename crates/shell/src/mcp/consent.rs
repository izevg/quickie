//! `AppKit` native diagnostic consent sheet, active-tab stop indicator, and `Browser` wiring.
//!
//! - Uses the existing [`DialogSheet`] to prompt for explicit user consent on the selected tab's
//!   window without stealing window focus or exposing tab labels/content before consent.
//! - Uses the existing [`AgentBadge`] to show a user-visible `MCP diagnostics │ Stop ⌘.` indicator
//!   only while an attached tab is displayed, and revokes on `⌘.` ([`Browser::stop_mcp_diagnostics`]).
//! - Owns the GUI's private Unix socket [`ServerHandle`] and [`MacBrowserHost`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use objc2::MainThreadMarker;
use quickie_application::mcp::{
    AttachmentId, ClientId, DiagnosticCapabilities, GrantScope, ObservationTarget, SnapshotRequest,
};
use quickie_application::{
    DiagnosticLease, DiagnosticSignal, DiagnosticSnapshot, EngineError, PageContent,
};
use quickie_domain_browsing::{Profile, TabId, WindowId};
use quickie_domain_commands::KeyStroke;
use quickie_mcp::{
    BrowserHandlerFactory, BrowserOperation, ServerHandle, SocketEndpoint, serve_socket,
};
use quickie_ui_widgets::{AgentBadge, DialogChoice, DialogKind, DialogRequest, DialogSheet};
use url::{Origin, Url};

use crate::browser::Browser;
use crate::input::keystroke::{SheetKey, sheet_key};
use crate::updates::INSTALLED_VERSION;

use super::agent::native::NativeAgentBackend;
use super::agent::{AgentApprovalPrompt, AgentCoordinator, AgentDispatchEffect};
use super::bridge::{
    ConsentPrompt, DiagnosticBackend, DispatchEffect, MacBrowserHost, McpCoordinator,
};

/// Minimal English copy for the active-tab stop indicator.
pub(crate) const ATTACHED_BADGE_TEXT: &str = "MCP diagnostics";

/// Builds the [`DialogRequest`] shown in [`DialogSheet`] for a diagnostic consent prompt.
///
/// Uses only the sanitized HTTP(S) tuple origin (never the tab title, URL path, or query string).
#[must_use]
pub(crate) fn consent_dialog_request(origin: &Origin, scope: GrantScope) -> Option<DialogRequest> {
    let origin_url = Url::parse(&origin.ascii_serialization()).ok()?;
    let message = match scope {
        GrantScope::SingleDocument => {
            "Allow read-only MCP diagnostics on this tab? Access ends when this page navigates or reloads."
        }
        GrantScope::SameOriginReload => {
            "Allow read-only MCP diagnostics on this tab? Access continues across same-origin reloads of this page."
        }
    };
    Some(DialogRequest {
        origin: origin_url,
        kind: DialogKind::Confirm,
        message: message.to_owned(),
        offer_block: false,
    })
}

/// Builds the [`DialogRequest`] shown in [`DialogSheet`] for an isolated Agent approval prompt.
#[must_use]
pub(crate) fn agent_approval_dialog_request(
    origin: &Origin,
    message: &str,
) -> Option<DialogRequest> {
    let origin_url = Url::parse(&origin.ascii_serialization()).ok()?;
    Some(DialogRequest {
        origin: origin_url,
        kind: DialogKind::Confirm,
        message: message.to_owned(),
        offer_block: false,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShownPrompt {
    Diagnostic { window: WindowId, id: u64 },
    Agent { window: WindowId, id: u64 },
}

impl ShownPrompt {
    const fn window(self) -> WindowId {
        match self {
            Self::Diagnostic { window, .. } | Self::Agent { window, .. } => window,
        }
    }
}

/// GUI-owned MCP server, coordinator, isolated Agent coordinator, consent sheet, and indicator.
pub(crate) struct McpUi {
    coordinator: RefCell<McpCoordinator>,
    agent: RefCell<AgentCoordinator>,
    sheet: DialogSheet,
    shown: RefCell<Option<ShownPrompt>>,
    badge: AgentBadge,
    host: RefCell<Option<Arc<MacBrowserHost>>>,
    server: RefCell<Option<ServerHandle>>,
}

impl McpUi {
    /// Creates the GUI MCP state on the main thread.
    #[must_use]
    pub(crate) fn new(mtm: MainThreadMarker) -> Self {
        Self {
            coordinator: RefCell::new(McpCoordinator::new()),
            agent: RefCell::new(AgentCoordinator::new()),
            sheet: DialogSheet::new(mtm),
            shown: RefCell::new(None),
            badge: AgentBadge::new(mtm),
            host: RefCell::new(None),
            server: RefCell::new(None),
        }
    }
}

/// Live `Browser` adapter implementing [`DiagnosticBackend`] via the application's [`PageContent`].
struct BrowserBackend<'a> {
    browser: &'a Browser,
}

impl DiagnosticBackend for BrowserBackend<'_> {
    fn diagnostic_capabilities(&self) -> DiagnosticCapabilities {
        let Ok(browsing) = self.browser.browsing.try_borrow() else {
            return DiagnosticCapabilities::default();
        };
        browsing.engine().diagnostic_capabilities()
    }

    fn agent_automation_supported(&self) -> bool {
        NativeAgentBackend::probe_capabilities().isolated_automation
    }

    fn active_main_tab(&self) -> Option<(WindowId, TabId, Url)> {
        let (window, _) = self.browser.current()?;
        let browsing = self.browser.browsing.try_borrow().ok()?;
        let tab_id = browsing.session().active(window)?;
        if tab_id.profile() != Profile::Main || !browsing.has_page(tab_id) {
            return None;
        }
        let url = browsing.session().tab(tab_id)?.url.clone();
        Some((window, tab_id, url))
    }

    fn live_tab_url(&self, tab: TabId) -> Option<Url> {
        if tab.profile() != Profile::Main {
            return None;
        }
        let browsing = self.browser.browsing.try_borrow().ok()?;
        if !browsing.has_page(tab) {
            return None;
        }
        Some(browsing.session().tab(tab)?.url.clone())
    }

    fn observe_diagnostics(
        &self,
        tab: TabId,
        on_signal: Box<dyn Fn(DiagnosticSignal) + 'static>,
    ) -> Result<DiagnosticLease, EngineError> {
        let browsing = self
            .browser
            .browsing
            .try_borrow()
            .map_err(|_| EngineError("browser state is temporarily busy".to_owned()))?;
        browsing.engine().observe_diagnostics(tab, on_signal)
    }

    fn diagnostic_snapshot(
        &self,
        tab: TabId,
        request: &SnapshotRequest,
        done: Box<dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static>,
    ) -> Result<(), EngineError> {
        let browsing = self
            .browser
            .browsing
            .try_borrow()
            .map_err(|_| EngineError("browser state is temporarily busy".to_owned()))?;
        browsing.engine().diagnostic_snapshot(tab, request, done)
    }
}

impl Browser {
    /// Starts the GUI's private Unix domain socket MCP listener.
    ///
    /// Failure to bind (for example, when another Quickie instance already owns the socket) never
    /// blocks standard GUI startup.
    pub(crate) fn wire_mcp(&self) {
        let Ok(endpoint) = SocketEndpoint::default_endpoint() else {
            return;
        };
        let host = Arc::new(MacBrowserHost::new(self.this.clone(), self.mtm));
        let factory = Arc::new(
            BrowserHandlerFactory::new(Arc::clone(&host)).with_product_version(INSTALLED_VERSION),
        );
        let factory_for_server = Arc::clone(&factory);
        if let Ok(handle) = serve_socket(&endpoint, move || factory_for_server.next_handler()) {
            *self.mcp.host.borrow_mut() = Some(host);
            *self.mcp.server.borrow_mut() = Some(handle);
        }
    }

    /// Shuts down the MCP socket listener, revokes all active attachments and isolated Agent
    /// contexts, and rejects pending consent prompts cleanly before joining the I/O thread.
    pub(crate) fn shutdown_mcp(&self) {
        let host = self.mcp.host.borrow_mut().take();
        if let Some(host) = host.as_ref() {
            host.close();
        }
        self.mcp.coordinator.borrow_mut().shutdown();
        self.mcp.agent.borrow_mut().shutdown();
        self.hide_mcp_sheet();
        self.mcp.badge.hide();
        if let Some(server) = self.mcp.server.borrow_mut().take() {
            server.shutdown();
        }
        drop(host);
    }

    /// Dispatches one typed MCP operation on the main thread.
    pub(crate) fn mcp_dispatch(&self, client: ClientId, operation: BrowserOperation) {
        match operation {
            BrowserOperation::Agent(agent_op) => {
                let backend = NativeAgentBackend::new(self.mtm);
                let active_window = self.current().map(|(window, _)| window);
                let effect = self.mcp.agent.borrow_mut().handle_operation(
                    client,
                    agent_op,
                    &backend,
                    active_window,
                );
                if effect == AgentDispatchEffect::ApprovalChanged {
                    self.present_mcp_consent();
                }
            }
            BrowserOperation::PageSnapshot {
                target: ObservationTarget::Agent(context_id),
                request,
                reply,
            } => {
                self.mcp
                    .agent
                    .borrow()
                    .handle_page_snapshot(client, context_id, &request, reply);
            }
            BrowserOperation::PageEvents {
                target: ObservationTarget::Agent(context_id),
                cursor,
                filter,
                limit,
                reply,
            } => {
                self.mcp
                    .agent
                    .borrow()
                    .handle_page_events(client, context_id, cursor, &filter, limit, reply);
            }
            other => {
                let backend = BrowserBackend { browser: self };
                let this = self.this.clone();
                let effect =
                    self.mcp.coordinator.borrow_mut().handle_operation(
                        client,
                        other,
                        &backend,
                        move |attachment, binding, kind, reply| {
                            Box::new(move |result| {
                                if let Some(browser) = this.upgrade() {
                                    browser.mcp.coordinator.borrow().complete_snapshot(
                                        attachment, &binding, kind, result, reply,
                                    );
                                }
                            })
                        },
                    );
                self.apply_mcp_effect(effect);
            }
        }
    }

    /// Releases all MCP resources owned by a disconnected client without affecting GUI lifetime.
    pub(crate) fn mcp_client_closed(&self, client: ClientId) {
        let diag_changed = self.mcp.coordinator.borrow_mut().on_client_closed(client);
        let agent_changed = self.mcp.agent.borrow_mut().on_client_closed(client);
        if diag_changed || agent_changed {
            self.present_mcp_consent();
            self.refresh_mcp_indicator();
        }
    }

    /// Records explicit shell reload intent (`Reload` / `HardReload`) on `tab`.
    pub(crate) fn mcp_reload_requested(&self, tab: TabId) {
        let url = {
            let Ok(browsing) = self.browsing.try_borrow() else {
                return;
            };
            browsing.session().tab(tab).map(|t| t.url.clone())
        };
        if let Some(url) = url {
            self.mcp
                .coordinator
                .borrow_mut()
                .on_reload_requested(tab, &url);
        }
    }

    /// Clears explicit shell reload intent when a non-reload navigation starts on `tab`.
    pub(crate) fn mcp_navigation_requested(&self, tab: TabId) {
        self.mcp
            .coordinator
            .borrow_mut()
            .on_navigation_requested(tab);
    }

    /// Notifies MCP diagnostics that `tab` committed a main-frame navigation to `url`.
    pub(crate) fn mcp_tab_committed(&self, tab: TabId, url: &Url) {
        if self.mcp.coordinator.borrow_mut().on_tab_committed(tab, url) {
            self.present_mcp_consent();
            self.refresh_mcp_indicator();
        }
    }

    /// Notifies MCP diagnostics that `tab` closed.
    pub(crate) fn mcp_tab_closed(&self, tab: TabId) {
        if self.mcp.coordinator.borrow_mut().on_tab_closed(tab) {
            self.present_mcp_consent();
            self.refresh_mcp_indicator();
        }
    }

    /// Notifies MCP diagnostics that `tab`'s web content process terminated.
    pub(crate) fn mcp_process_terminated(&self, tab: TabId) {
        if self.mcp.coordinator.borrow_mut().on_process_terminated(tab) {
            self.present_mcp_consent();
            self.refresh_mcp_indicator();
        }
    }

    /// Preempts MCP diagnostics on `tab` when the visual Web Inspector takes priority.
    pub(crate) fn mcp_inspector_takeover(&self, tab: TabId) {
        if self.mcp.coordinator.borrow_mut().on_inspector_takeover(tab) {
            self.present_mcp_consent();
            self.refresh_mcp_indicator();
        }
    }

    /// Stops all active MCP diagnostic attachments on explicit user request (`⌘.`).
    pub(crate) fn stop_mcp_diagnostics(&self) -> bool {
        let count = self.mcp.coordinator.borrow_mut().user_revoke_all();
        if count == 0 {
            return false;
        }
        self.refresh_mcp_indicator();
        self.toast("MCP diagnostics stopped");
        true
    }

    /// True while the MCP diagnostic consent or Agent approval sheet is shown in `window`.
    pub(crate) fn mcp_sheet_open(&self, window: WindowId) -> bool {
        self.mcp
            .shown
            .borrow()
            .is_some_and(|shown| shown.window() == window)
    }

    /// Routes `⏎` (allow) and `⎋` (cancel) when the MCP consent sheet is open in the current
    /// window.
    pub(crate) fn mcp_sheet_key(&self, stroke: KeyStroke) -> bool {
        let Some((window, _)) = self.current() else {
            return false;
        };
        let Some(shown) = *self.mcp.shown.borrow() else {
            return false;
        };
        if shown.window() != window {
            return false;
        }
        match sheet_key(stroke) {
            Some(SheetKey::Accept) => self.answer_mcp_prompt(shown, DialogChoice::Ok),
            Some(SheetKey::Dismiss) => self.answer_mcp_prompt(shown, DialogChoice::Cancel),
            _ => {}
        }
        true
    }

    /// Shows the `MCP diagnostics │ Stop ⌘.` badge on the current window only when its active tab
    /// has an active diagnostic attachment; hides it otherwise.
    pub(crate) fn refresh_mcp_indicator(&self) {
        let active_attached = self.current().and_then(|(window, ns_window)| {
            let tab = self.active(window)?;
            if !self.mcp.coordinator.borrow().has_attachments_for_tab(tab) {
                return None;
            }
            if self.is_window_fullscreen(window) {
                return None;
            }
            ns_window.contentView()
        });

        if let Some(host) = active_attached {
            let width = host.bounds().size.width;
            self.mcp.badge.show(&host, ATTACHED_BADGE_TEXT, width);
        } else {
            self.mcp.badge.hide();
        }
    }

    fn apply_mcp_effect(&self, effect: DispatchEffect) {
        match effect {
            DispatchEffect::None => {}
            DispatchEffect::ConsentChanged => self.present_mcp_consent(),
            DispatchEffect::AttachmentsChanged => {
                self.present_mcp_consent();
                self.refresh_mcp_indicator();
            }
        }
    }

    fn present_mcp_consent(&self) {
        let next_diag = self.mcp.coordinator.borrow_mut().front_consent();
        let next_agent = if next_diag.is_none() {
            self.mcp.agent.borrow_mut().front_approval()
        } else {
            None
        };
        let current = *self.mcp.shown.borrow();
        match (current, next_diag, next_agent) {
            (Some(ShownPrompt::Diagnostic { id, .. }), Some(prompt), _) if id == prompt.id => {}
            (_, Some(prompt), _) => self.show_mcp_consent(&prompt),
            (Some(ShownPrompt::Agent { id, .. }), None, Some(prompt)) if id == prompt.id => {}
            (_, None, Some(prompt)) => self.show_agent_approval(&prompt),
            (Some(_), None, None) => self.hide_mcp_sheet(),
            (None, None, None) => {}
        }
    }

    fn show_mcp_consent(&self, prompt: &ConsentPrompt) {
        self.hide_mcp_sheet();
        let Some(ns_window) = self.window(prompt.window) else {
            self.answer_mcp_consent(prompt.id, DialogChoice::Cancel);
            return;
        };
        let Some(host) = ns_window.contentView() else {
            self.answer_mcp_consent(prompt.id, DialogChoice::Cancel);
            return;
        };
        let Some(request) = consent_dialog_request(&prompt.origin, prompt.scope) else {
            self.answer_mcp_consent(prompt.id, DialogChoice::Cancel);
            return;
        };

        // Never steal window focus (`makeKeyAndOrderFront` / `activate` are intentionally omitted).
        *self.mcp.shown.borrow_mut() = Some(ShownPrompt::Diagnostic {
            window: prompt.window,
            id: prompt.id,
        });
        let consent_id = prompt.id;
        let this = self.this.clone();
        self.mcp
            .sheet
            .show(&host, &ns_window, &request, move |choice| {
                if let Some(browser) = this.upgrade() {
                    browser.answer_mcp_consent(consent_id, choice);
                    browser.drain();
                }
            });
    }

    fn show_agent_approval(&self, prompt: &AgentApprovalPrompt) {
        self.hide_mcp_sheet();
        let Some(ns_window) = self.window(prompt.window) else {
            self.answer_agent_approval(prompt.id, DialogChoice::Cancel);
            return;
        };
        let Some(host) = ns_window.contentView() else {
            self.answer_agent_approval(prompt.id, DialogChoice::Cancel);
            return;
        };
        let Some(request) = agent_approval_dialog_request(&prompt.origin, &prompt.message) else {
            self.answer_agent_approval(prompt.id, DialogChoice::Cancel);
            return;
        };

        // Never steal window focus (`makeKeyAndOrderFront` / `activate` are intentionally omitted).
        *self.mcp.shown.borrow_mut() = Some(ShownPrompt::Agent {
            window: prompt.window,
            id: prompt.id,
        });
        let approval_id = prompt.id;
        let this = self.this.clone();
        self.mcp
            .sheet
            .show(&host, &ns_window, &request, move |choice| {
                if let Some(browser) = this.upgrade() {
                    browser.answer_agent_approval(approval_id, choice);
                    browser.drain();
                }
            });
    }

    fn hide_mcp_sheet(&self) {
        *self.mcp.shown.borrow_mut() = None;
        self.mcp.sheet.hide();
    }

    fn answer_mcp_prompt(&self, shown: ShownPrompt, choice: DialogChoice) {
        match shown {
            ShownPrompt::Diagnostic { id, .. } => self.answer_mcp_consent(id, choice),
            ShownPrompt::Agent { id, .. } => self.answer_agent_approval(id, choice),
        }
    }

    fn answer_agent_approval(&self, approval_id: u64, choice: DialogChoice) {
        let shown_window = {
            let mut shown = self.mcp.shown.borrow_mut();
            if matches!(*shown, Some(ShownPrompt::Agent { id, .. }) if id == approval_id) {
                let window = shown.map(ShownPrompt::window);
                *shown = None;
                window
            } else {
                None
            }
        };
        if shown_window.is_some() {
            self.mcp.sheet.hide();
        }
        let approved = choice == DialogChoice::Ok;
        let effect = self
            .mcp
            .agent
            .borrow_mut()
            .resolve_approval(approval_id, approved);
        if effect == AgentDispatchEffect::ApprovalChanged {
            self.present_mcp_consent();
        }
        if let Some(window) = shown_window {
            self.settle(window);
        }
    }

    fn answer_mcp_consent(&self, consent_id: u64, choice: DialogChoice) {
        let shown_window = {
            let mut shown = self.mcp.shown.borrow_mut();
            if matches!(*shown, Some(ShownPrompt::Diagnostic { id, .. }) if id == consent_id) {
                let window = shown.map(ShownPrompt::window);
                *shown = None;
                window
            } else {
                None
            }
        };
        if shown_window.is_some() {
            self.mcp.sheet.hide();
        }

        let approved = choice == DialogChoice::Ok;
        let backend = BrowserBackend { browser: self };
        let this = self.this.clone();
        let effect = self.mcp.coordinator.borrow_mut().resolve_consent(
            consent_id,
            approved,
            &backend,
            move |attachment_slot: Rc<Cell<Option<AttachmentId>>>, tab: TabId| {
                Box::new(move |signal| {
                    let Some(attachment_id) = attachment_slot.get() else {
                        return;
                    };
                    if let Some(browser) = this.upgrade() {
                        browser.on_mcp_native_signal(attachment_id, tab, signal);
                    }
                })
            },
        );
        self.apply_mcp_effect(effect);
        if let Some(window) = shown_window {
            self.settle(window);
        }
    }

    fn on_mcp_native_signal(&self, attachment: AttachmentId, tab: TabId, signal: DiagnosticSignal) {
        let backend = BrowserBackend { browser: self };
        let changed = self
            .mcp
            .coordinator
            .borrow_mut()
            .on_native_signal(attachment, tab, signal, &backend);
        if changed {
            self.refresh_mcp_indicator();
        }
    }
}
