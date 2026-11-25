use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::thread;

use quickie_application::mcp::{
    AttachmentId, ClientId, ConsoleLevel, DiagnosticCapabilities, DiagnosticEventData,
    DocumentBinding, GrantScope, SnapshotKind, SnapshotRequest,
};
use quickie_application::{
    DiagnosticGapReason, DiagnosticLease, DiagnosticSignal, DiagnosticSnapshot, EngineError,
};
use quickie_domain_browsing::{Profile, TabId, WindowId};
use quickie_mcp::{
    BrowserHandlerFactory, BrowserHost, BrowserOperation, HostDispatchError, HostPlatform,
    HostReply, PageSnapshotResult, RuntimeMode, ServerHandle, SocketEndpoint, ToolErrorCode,
    serve_socket,
};
use quickie_ui_widgets::DialogKind;
use serde_json::{Value, json};
use url::Url;

use crate::mcp::bridge::{DiagnosticBackend, DispatchEffect, McpCoordinator};
use crate::mcp::consent::consent_dialog_request;

type SignalCallback = Box<dyn Fn(DiagnosticSignal) + 'static>;
type SnapshotCallback = Box<dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static>;
type PendingSnapshotEntry = (
    AttachmentId,
    DocumentBinding,
    SnapshotKind,
    HostReply<PageSnapshotResult>,
    SnapshotCallback,
);
type ImmediateSnapshot = Rc<
    RefCell<
        Option<(
            AttachmentId,
            DocumentBinding,
            SnapshotKind,
            HostReply<PageSnapshotResult>,
            Result<DiagnosticSnapshot, EngineError>,
        )>,
    >,
>;

fn url(raw: &str) -> Url {
    Url::parse(raw).expect("valid test URL")
}

fn full_capabilities() -> DiagnosticCapabilities {
    DiagnosticCapabilities {
        dom: true,
        accessibility: false,
        console: true,
        runtime_errors: true,
        pending_network: true,
        completed_network: true,
        lifecycle: true,
    }
}

struct SyntheticBackend {
    capabilities: DiagnosticCapabilities,
    active_window: WindowId,
    active_tab: Option<TabId>,
    tabs: RefCell<HashMap<TabId, Url>>,
    active_leases: Rc<Cell<usize>>,
    defer_snapshots: bool,
    pending_snapshots: RefCell<Vec<SnapshotCallback>>,
}

impl SyntheticBackend {
    fn new(capabilities: DiagnosticCapabilities) -> Self {
        let tab = TabId::new(Profile::Main, 1);
        let mut tabs = HashMap::new();
        tabs.insert(tab, url("https://example.com/app?secret=1"));
        Self {
            capabilities,
            active_window: WindowId::new(Profile::Main, 1),
            active_tab: Some(tab),
            tabs: RefCell::new(tabs),
            active_leases: Rc::new(Cell::new(0)),
            defer_snapshots: false,
            pending_snapshots: RefCell::new(Vec::new()),
        }
    }
}

impl DiagnosticBackend for SyntheticBackend {
    fn diagnostic_capabilities(&self) -> DiagnosticCapabilities {
        self.capabilities
    }

    fn active_main_tab(&self) -> Option<(WindowId, TabId, Url)> {
        let tab = self.active_tab?;
        if tab.profile() != Profile::Main {
            return None;
        }
        let url = self.tabs.borrow().get(&tab)?.clone();
        Some((self.active_window, tab, url))
    }

    fn live_tab_url(&self, tab: TabId) -> Option<Url> {
        if tab.profile() != Profile::Main {
            return None;
        }
        self.tabs.borrow().get(&tab).cloned()
    }

    fn observe_diagnostics(
        &self,
        tab: TabId,
        _on_signal: SignalCallback,
    ) -> Result<DiagnosticLease, EngineError> {
        if !self.tabs.borrow().contains_key(&tab) {
            return Err(EngineError("tab has no page".to_owned()));
        }
        self.active_leases.set(self.active_leases.get() + 1);
        let leases = Rc::clone(&self.active_leases);
        Ok(DiagnosticLease::new(move || {
            leases.set(leases.get().saturating_sub(1));
        }))
    }

    fn diagnostic_snapshot(
        &self,
        tab: TabId,
        request: &SnapshotRequest,
        done: SnapshotCallback,
    ) -> Result<(), EngineError> {
        if !self.tabs.borrow().contains_key(&tab) {
            return Err(EngineError("tab has no page".to_owned()));
        }
        if request.kind() == SnapshotKind::Accessibility {
            return Err(EngineError(
                "native accessibility snapshots are unavailable".to_owned(),
            ));
        }
        if self.defer_snapshots {
            self.pending_snapshots.borrow_mut().push(done);
        } else {
            done(Ok(DiagnosticSnapshot {
                data: "<html><body>ok</body></html>".to_owned(),
                truncated: false,
            }));
        }
        Ok(())
    }
}

enum UiMsg {
    Dispatch(ClientId, BrowserOperation),
    ClientClosed(ClientId),
    Control(Box<dyn FnOnce(&mut SyntheticUiState) + Send + 'static>),
    Shutdown,
}

struct SyntheticUiState {
    auto_approve: Option<bool>,
    backend: SyntheticBackend,
    coordinator: McpCoordinator,
    deferred_snapshot_completions: Vec<PendingSnapshotEntry>,
}

/// Synthetic GUI host running [`McpCoordinator`] on a dedicated non-Tokio UI thread, matching the
/// real macOS main-queue threading model (`DiagnosticLease` stays strictly on the UI thread).
struct SyntheticUiHost {
    tx: mpsc::Sender<UiMsg>,
}

impl SyntheticUiHost {
    fn spawn(
        capabilities: DiagnosticCapabilities,
        auto_approve: Option<bool>,
    ) -> (Arc<Self>, thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<UiMsg>();
        let join = thread::Builder::new()
            .name("quickie-mcp-synthetic-ui".to_owned())
            .spawn(move || {
                let mut ui = SyntheticUiState {
                    auto_approve,
                    backend: SyntheticBackend::new(capabilities),
                    coordinator: McpCoordinator::new(),
                    deferred_snapshot_completions: Vec::new(),
                };
                while let Ok(msg) = rx.recv() {
                    match msg {
                        UiMsg::Dispatch(client, op) => ui.dispatch(client, op),
                        UiMsg::ClientClosed(client) => {
                            let _ = ui.coordinator.on_client_closed(client);
                        }
                        UiMsg::Control(action) => action(&mut ui),
                        UiMsg::Shutdown => {
                            ui.coordinator.shutdown();
                            break;
                        }
                    }
                }
            })
            .expect("spawn synthetic UI thread");
        (Arc::new(Self { tx }), join)
    }

    fn run_on_ui<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut SyntheticUiState) -> T + Send + 'static,
    ) -> T {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(UiMsg::Control(Box::new(move |ui| {
                let _ = done_tx.send(f(ui));
            })))
            .expect("send control message");
        done_rx.recv().expect("receive control reply")
    }

    fn shutdown(&self) {
        let _ = self.tx.send(UiMsg::Shutdown);
    }
}

impl SyntheticUiState {
    fn dispatch(&mut self, client: ClientId, op: BrowserOperation) {
        let immediate: ImmediateSnapshot = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&immediate);

        let effect = if self.backend.defer_snapshots {
            let mut deferred_entry = None;
            let eff = self.coordinator.handle_operation(
                client,
                op,
                &self.backend,
                |attachment, binding, kind, reply| {
                    deferred_entry = Some((attachment, binding, kind, reply));
                    Box::new(|_| {})
                },
            );
            if let (Some((attachment, binding, kind, reply)), Some(done)) = (
                deferred_entry,
                self.backend.pending_snapshots.borrow_mut().pop(),
            ) {
                self.deferred_snapshot_completions
                    .push((attachment, binding, kind, reply, done));
            }
            eff
        } else {
            self.coordinator.handle_operation(
                client,
                op,
                &self.backend,
                move |attachment, binding, kind, reply| {
                    Box::new(move |res| {
                        *slot.borrow_mut() = Some((attachment, binding, kind, reply, res));
                    })
                },
            )
        };

        if let Some((attachment, binding, kind, reply, res)) = immediate.borrow_mut().take() {
            self.coordinator
                .complete_snapshot(attachment, &binding, kind, res, reply);
        }

        if effect == DispatchEffect::ConsentChanged
            && let Some(approved) = self.auto_approve
            && let Some(prompt) = self.coordinator.front_consent()
        {
            let _ = self.coordinator.resolve_consent(
                prompt.id,
                approved,
                &self.backend,
                |_slot, _tab| Box::new(|_| {}),
            );
        }
    }
}

impl BrowserHost for SyntheticUiHost {
    fn dispatch(
        &self,
        client: ClientId,
        operation: BrowserOperation,
    ) -> Result<(), HostDispatchError> {
        self.tx
            .send(UiMsg::Dispatch(client, operation))
            .map_err(|_| HostDispatchError::Closed)
    }

    fn client_closed(&self, client: ClientId) {
        let _ = self.tx.send(UiMsg::ClientClosed(client));
    }
}

struct TestGuiHarness {
    _temp: tempfile::TempDir,
    endpoint: SocketEndpoint,
    host: Arc<SyntheticUiHost>,
    server: Option<ServerHandle>,
    ui_thread: Option<thread::JoinHandle<()>>,
}

impl TestGuiHarness {
    fn start(capabilities: DiagnosticCapabilities, auto_approve: Option<bool>) -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700))
                .expect("chmod 0700");
        }
        let sock_path = temp.path().join("quickie").join("mcp.sock");
        let endpoint = SocketEndpoint::in_trusted_root(temp.path(), &sock_path).expect("endpoint");
        let (host, ui_thread) = SyntheticUiHost::spawn(capabilities, auto_approve);
        let factory = Arc::new(BrowserHandlerFactory::new(Arc::clone(&host)));
        let factory_for_server = Arc::clone(&factory);
        let server = serve_socket(&endpoint, move || factory_for_server.next_handler())
            .expect("serve_socket");
        Self {
            _temp: temp,
            endpoint,
            host,
            server: Some(server),
            ui_thread: Some(ui_thread),
        }
    }

    fn connect(&self, name: &str) -> (UnixStream, BufReader<UnixStream>) {
        let stream = UnixStream::connect(self.endpoint.socket_path()).expect("connect");
        let reader_stream = stream.try_clone().expect("clone stream");
        let mut writer = stream;
        let mut reader = BufReader::new(reader_stream);
        let init = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": name, "version": "0.1.0" }
            }
        });
        writeln!(writer, "{init}").expect("write init");
        writer.flush().expect("flush");
        let mut line = String::new();
        reader.read_line(&mut line).expect("read init");
        let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        writeln!(writer, "{initialized}").expect("write initialized");
        writer.flush().expect("flush");
        (writer, reader)
    }
}

impl Drop for TestGuiHarness {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.shutdown();
        }
        self.host.shutdown();
        if let Some(join) = self.ui_thread.take() {
            let _ = join.join();
        }
    }
}

fn call_tool(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    id: u64,
    tool: &str,
    args: &Value,
) -> Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": tool, "arguments": args }
    });
    writeln!(writer, "{req}").expect("write call");
    writer.flush().expect("flush");
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("read call");
    assert!(n > 0, "unexpected EOF");
    serde_json::from_str(&line).expect("json response")
}

#[test]
fn consent_dialog_exposes_only_sanitized_origin_and_scope_copy() {
    // Arrange
    let origin = url("https://app.example.com:8443/private/path?token=secret#frag").origin();

    // Act
    let single = consent_dialog_request(&origin, GrantScope::SingleDocument).expect("request");
    let reload = consent_dialog_request(&origin, GrantScope::SameOriginReload).expect("request");

    // Assert
    assert_eq!(single.origin.as_str(), "https://app.example.com:8443/");
    assert_eq!(single.kind, DialogKind::Confirm);
    assert!(!single.offer_block);
    assert!(!single.message.contains("secret"));
    assert!(!single.message.contains("/private"));
    assert!(single.message.contains("navigates or reloads"));
    assert!(reload.message.contains("same-origin reloads"));
}

#[test]
fn non_webkit_default_capabilities_report_honestly_and_reject_diagnostic_request() {
    // Arrange
    let harness = TestGuiHarness::start(DiagnosticCapabilities::default(), Some(true));
    let (mut writer, mut reader) = harness.connect("non-webkit");

    // Act
    let status = call_tool(&mut writer, &mut reader, 2, "browser_status", &json!({}));
    let req = call_tool(
        &mut writer,
        &mut reader,
        3,
        "diagnostic_request",
        &json!({}),
    );

    // Assert
    assert_eq!(
        status["result"]["structuredContent"]["platform"],
        HostPlatform::Macos.as_str()
    );
    assert_eq!(
        status["result"]["structuredContent"]["runtime_mode"],
        RuntimeMode::Gui.as_str()
    );
    assert_eq!(
        status["result"]["structuredContent"]["fork_capabilities"]["diagnostic"]["dom"],
        false
    );
    assert_eq!(req["result"]["isError"], true);
    assert_eq!(
        req["result"]["structuredContent"]["code"],
        ToolErrorCode::UnsupportedCapability.as_str()
    );
}

#[test]
fn per_client_scope_isolates_attachments_between_clients() {
    // Arrange
    let harness = TestGuiHarness::start(full_capabilities(), Some(true));
    let (mut client_a, mut reader_a) = harness.connect("client-a");
    let (mut client_b, mut reader_b) = harness.connect("client-b");
    let grant = call_tool(
        &mut client_a,
        &mut reader_a,
        2,
        "diagnostic_request",
        &json!({ "scope": "single_document" }),
    );
    let attachment_id = grant["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");

    // Act
    let wrong_owner = call_tool(
        &mut client_b,
        &mut reader_b,
        2,
        "page_snapshot",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": attachment_id },
            "kind": "dom"
        }),
    );
    let owner_ok = call_tool(
        &mut client_a,
        &mut reader_a,
        3,
        "page_snapshot",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": attachment_id },
            "kind": "dom"
        }),
    );

    // Assert
    assert_eq!(wrong_owner["result"]["isError"], true);
    assert_eq!(
        wrong_owner["result"]["structuredContent"]["code"],
        ToolErrorCode::AttachmentUnavailable.as_str()
    );
    assert_eq!(owner_ok["result"]["isError"], false);
    assert_eq!(owner_ok["result"]["structuredContent"]["epoch"], 1);
}

#[test]
fn same_origin_reload_survives_verified_reload_and_fails_closed_on_redirect() {
    // Arrange
    let harness = TestGuiHarness::start(full_capabilities(), Some(true));
    let tab = TabId::new(Profile::Main, 1);
    let (mut writer, mut reader) = harness.connect("client-reload");
    let grant = call_tool(
        &mut writer,
        &mut reader,
        2,
        "diagnostic_request",
        &json!({ "scope": "same_origin_reload" }),
    );
    let attachment_id = grant["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");

    // Act 1: verified shell reload on exact same URL survives and records events + overflow.
    harness.host.run_on_ui(move |ui| {
        let same = url("https://example.com/app?secret=1");
        ui.coordinator.on_reload_requested(tab, &same);
        let _ = ui.coordinator.on_tab_committed(tab, &same);
        let _ = ui.coordinator.on_native_signal(
            AttachmentId::from_wire(attachment_id),
            tab,
            DiagnosticSignal::Event(DiagnosticEventData::console(ConsoleLevel::Warn, "reloaded")),
            &ui.backend,
        );
        let _ = ui.coordinator.on_native_signal(
            AttachmentId::from_wire(attachment_id),
            tab,
            DiagnosticSignal::Gap(DiagnosticGapReason::NativeQueueOverflow {
                dropped: 2,
                total_dropped: 2,
            }),
            &ui.backend,
        );
    });
    let events_resp = call_tool(
        &mut writer,
        &mut reader,
        3,
        "page_events",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": attachment_id },
            "cursor": 0
        }),
    );

    // Act 2: same-origin redirect during reload (`/app?secret=1` -> `/other`) fails closed.
    let revoked_state = harness.host.run_on_ui(move |ui| {
        ui.coordinator
            .on_reload_requested(tab, &url("https://example.com/app?secret=1"));
        let revoked = ui
            .coordinator
            .on_tab_committed(tab, &url("https://example.com/other"));
        (
            revoked,
            ui.coordinator.active_attachment_count(),
            ui.backend.active_leases.get(),
        )
    });
    let after_redirect = call_tool(
        &mut writer,
        &mut reader,
        4,
        "page_events",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": attachment_id },
            "cursor": 0
        }),
    );

    // Assert
    assert_eq!(events_resp["result"]["isError"], false);
    assert_eq!(
        events_resp["result"]["structuredContent"]["native_overflow"],
        true
    );
    let events = events_resp["result"]["structuredContent"]["events"]
        .as_array()
        .expect("events array");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["category"], "lifecycle");
    assert_eq!(events[0]["kind"], "reload_committed");
    assert_eq!(events[1]["text"], "reloaded");
    assert_eq!(revoked_state, (true, 0, 0));
    assert_eq!(after_redirect["result"]["isError"], true);
    assert_eq!(
        after_redirect["result"]["structuredContent"]["code"],
        ToolErrorCode::AttachmentUnavailable.as_str()
    );
}

#[test]
fn dropped_client_revokes_its_attachment_and_keeps_gui_alive() {
    // Arrange
    let harness = TestGuiHarness::start(full_capabilities(), Some(true));
    let (mut client_a, mut reader_a) = harness.connect("client-a");
    let (mut client_b, mut reader_b) = harness.connect("client-b");
    let grant = call_tool(
        &mut client_a,
        &mut reader_a,
        2,
        "diagnostic_request",
        &json!({ "scope": "single_document" }),
    );
    assert_eq!(grant["result"]["isError"], false);
    assert_eq!(
        harness.host.run_on_ui(|ui| ui.backend.active_leases.get()),
        1
    );

    // Act: drop client A while client B continues talking to the GUI server.
    drop((client_a, reader_a));
    let status_b = call_tool(
        &mut client_b,
        &mut reader_b,
        2,
        "browser_status",
        &json!({}),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let state = harness.host.run_on_ui(|ui| {
            (
                ui.coordinator.active_attachment_count(),
                ui.backend.active_leases.get(),
            )
        });
        if state == (0, 0) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "dropped client attachment/lease was not released"
        );
        thread::sleep(std::time::Duration::from_millis(10));
    }

    // Assert
    assert_eq!(status_b["result"]["isError"], false);
    assert_eq!(
        status_b["result"]["structuredContent"]["runtime_mode"],
        "gui"
    );
}

#[test]
fn denied_consent_rejects_without_acquiring_lease() {
    // Arrange
    let harness = TestGuiHarness::start(full_capabilities(), Some(false));
    let (mut writer, mut reader) = harness.connect("client-deny");

    // Act
    let denied = call_tool(
        &mut writer,
        &mut reader,
        2,
        "diagnostic_request",
        &json!({}),
    );

    // Assert
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(
        denied["result"]["structuredContent"]["code"],
        ToolErrorCode::ApprovalDenied.as_str()
    );
    assert_eq!(
        harness.host.run_on_ui(|ui| ui.backend.active_leases.get()),
        0
    );
}

#[test]
fn changed_document_before_snapshot_completion_returns_stale_document() {
    // Arrange
    let harness = TestGuiHarness::start(full_capabilities(), Some(true));
    let tab = TabId::new(Profile::Main, 1);
    harness.host.run_on_ui(|ui| {
        ui.backend.defer_snapshots = true;
    });
    let (mut writer, mut reader) = harness.connect("client-stale");
    let grant = call_tool(
        &mut writer,
        &mut reader,
        2,
        "diagnostic_request",
        &json!({ "scope": "single_document" }),
    );
    let attachment_id = grant["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");

    // Act: start snapshot, then navigate the bound tab before completing the callback.
    let snap_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "page_snapshot",
            "arguments": {
                "target": { "kind": "diagnostic", "attachment_id": attachment_id },
                "kind": "dom"
            }
        }
    });
    writeln!(writer, "{snap_req}").expect("write snapshot");
    writer.flush().expect("flush");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let ready = harness.host.run_on_ui(move |ui| {
            let Some((att, binding, kind, reply, _done)) = ui.deferred_snapshot_completions.pop()
            else {
                return false;
            };
            let _ = ui
                .coordinator
                .on_tab_committed(tab, &url("https://example.com/next"));
            ui.coordinator.complete_snapshot(
                att,
                &binding,
                kind,
                Ok(DiagnosticSnapshot {
                    data: "<html/>".to_owned(),
                    truncated: false,
                }),
                reply,
            );
            true
        });
        if ready {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "deferred snapshot did not arrive"
        );
        thread::sleep(std::time::Duration::from_millis(10));
    }

    let mut line = String::new();
    reader.read_line(&mut line).expect("read snapshot reply");
    let snap_resp: Value = serde_json::from_str(&line).expect("json");

    // Assert
    assert_eq!(snap_resp["result"]["isError"], true);
    assert_eq!(
        snap_resp["result"]["structuredContent"]["code"],
        ToolErrorCode::StaleDocument.as_str()
    );
    assert_eq!(
        harness.host.run_on_ui(|ui| ui.backend.active_leases.get()),
        0
    );
}

#[test]
fn channel_closed_revokes_attachment_and_releases_lease() {
    // Arrange
    let harness = TestGuiHarness::start(full_capabilities(), Some(true));
    let tab = TabId::new(Profile::Main, 1);
    let (mut writer, mut reader) = harness.connect("client-closed-channel");
    let grant = call_tool(
        &mut writer,
        &mut reader,
        2,
        "diagnostic_request",
        &json!({ "scope": "single_document" }),
    );
    let att_id = grant["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");

    // Act: native observer signals `ChannelClosed` (for example visual Inspector takeover).
    let changed = harness.host.run_on_ui(move |ui| {
        ui.coordinator.on_native_signal(
            AttachmentId::from_wire(att_id),
            tab,
            DiagnosticSignal::Gap(DiagnosticGapReason::ChannelClosed),
            &ui.backend,
        )
    });
    let after_closed = call_tool(
        &mut writer,
        &mut reader,
        3,
        "page_events",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": att_id },
            "cursor": 0
        }),
    );

    // Assert
    assert!(changed);
    assert_eq!(
        harness.host.run_on_ui(|ui| ui.backend.active_leases.get()),
        0
    );
    assert_eq!(after_closed["result"]["isError"], true);
    assert_eq!(
        after_closed["result"]["structuredContent"]["code"],
        ToolErrorCode::AttachmentUnavailable.as_str()
    );
}
