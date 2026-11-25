//! Unit and integration tests for Linux Quickie's in-process MCP host.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use quickie_application::DiagnosticSnapshot;
use quickie_application::mcp::{
    ClientId, DiagnosticCapabilities, DiagnosticEventData, DocumentBinding, NavigationKind,
    SnapshotKind, SnapshotRequest,
};
use quickie_domain_browsing::{Profile, TabId};
use quickie_mcp::{
    BrowserHandler, BrowserHandlerConfig, BrowserHandlerFactory, BrowserHost, ForkCapabilities,
    IsolatedStdio, McpError, RelayConfig, ShutdownTrigger, SocketEndpoint, ToolError,
    ToolErrorCode, run_relay_fds, serve_socket, serve_stdio, serve_stream,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use url::Url;

use super::controller::{ConsentPrompt, HostBackend, LeaseAttachError, McpController};
use super::protocol::{DecodedSubscription, DirectMethod, TargetMethod, parse_allowlisted_command};
use super::{DispatchPort, MainContextWakeup};
use crate::mcp::{run_with_endpoint, snapshot, standalone::StandaloneHost};

struct CountingWakeup {
    count: AtomicUsize,
}

impl CountingWakeup {
    fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
        }
    }

    fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

impl MainContextWakeup for CountingWakeup {
    fn wake(&self) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeLease {
    attachment: quickie_application::mcp::AttachmentId,
    commands: Vec<DecodedSubscription>,
}

struct FakeBackend {
    capabilities: ForkCapabilities,
    active_document: Option<DocumentBinding>,
    shown_prompts: Vec<ConsentPrompt>,
    consent_visible: bool,
    active_indicator: Vec<String>,
    attach_error: Option<LeaseAttachError>,
    dispatch_ok: bool,
    stopped_leases: Vec<quickie_application::mcp::AttachmentId>,
    started_snapshots: Vec<(u64, DocumentBinding, SnapshotRequest)>,
    dispatched_commands: HashMap<quickie_application::mcp::AttachmentId, Vec<DecodedSubscription>>,
}

impl FakeBackend {
    fn fork_ready(active: Option<DocumentBinding>) -> Self {
        Self {
            capabilities: ForkCapabilities {
                diagnostic: DiagnosticCapabilities {
                    dom: true,
                    accessibility: false,
                    console: true,
                    runtime_errors: true,
                    pending_network: true,
                    completed_network: true,
                    lifecycle: true,
                },
                native_accessibility: false,
                visual_inspector_priority: true,
                agent_automation: false,
            },
            active_document: active,
            shown_prompts: Vec::new(),
            consent_visible: false,
            active_indicator: Vec::new(),
            attach_error: None,
            dispatch_ok: true,
            stopped_leases: Vec::new(),
            started_snapshots: Vec::new(),
            dispatched_commands: HashMap::new(),
        }
    }

    fn stock_webkit(active: Option<DocumentBinding>) -> Self {
        Self {
            capabilities: ForkCapabilities {
                diagnostic: DiagnosticCapabilities {
                    dom: true,
                    accessibility: false,
                    console: false,
                    runtime_errors: false,
                    pending_network: false,
                    completed_network: false,
                    lifecycle: false,
                },
                native_accessibility: false,
                visual_inspector_priority: false,
                agent_automation: false,
            },
            ..Self::fork_ready(active)
        }
    }
}

impl HostBackend for FakeBackend {
    type Lease = FakeLease;
    type SnapshotHandle = u64;

    fn product_version(&self) -> String {
        "0.1.0-test".to_owned()
    }

    fn capabilities(&self) -> ForkCapabilities {
        self.capabilities
    }

    fn active_document(&self) -> Option<DocumentBinding> {
        self.active_document.clone()
    }

    fn show_consent(&mut self, prompt: ConsentPrompt) {
        self.consent_visible = true;
        self.shown_prompts.push(prompt);
    }

    fn hide_consent(&mut self) {
        self.consent_visible = false;
    }

    fn set_active_indicator(&mut self, active_origins: &[String]) {
        self.active_indicator = active_origins.to_vec();
    }

    fn attach_lease(
        &mut self,
        attachment: quickie_application::mcp::AttachmentId,
        _binding: &DocumentBinding,
    ) -> Result<Self::Lease, LeaseAttachError> {
        if let Some(error) = self.attach_error {
            return Err(error);
        }
        Ok(FakeLease {
            attachment,
            commands: Vec::new(),
        })
    }

    fn dispatch_lease_command(
        &mut self,
        lease: &mut Self::Lease,
        command: &DecodedSubscription,
    ) -> bool {
        if !self.dispatch_ok {
            return false;
        }
        lease.commands.push(command.clone());
        self.dispatched_commands
            .entry(lease.attachment)
            .or_default()
            .push(command.clone());
        true
    }

    fn stop_lease(&mut self, lease: Self::Lease) {
        self.stopped_leases.push(lease.attachment);
    }

    fn start_snapshot(
        &mut self,
        snapshot_id: u64,
        binding: &DocumentBinding,
        request: &SnapshotRequest,
    ) -> Result<Self::SnapshotHandle, ToolError> {
        self.started_snapshots
            .push((snapshot_id, binding.clone(), request.clone()));
        Ok(snapshot_id)
    }
}

fn binding(tab: u64, url: &str, epoch: u64) -> DocumentBinding {
    DocumentBinding {
        tab: TabId::new(Profile::Main, tab),
        origin: Url::parse(url).expect("valid test url").origin(),
        epoch,
    }
}

fn pipe_pair() -> (OwnedFd, OwnedFd) {
    let (reader, writer) = std::io::pipe().expect("pipe creation");
    (OwnedFd::from(reader), OwnedFd::from(writer))
}

fn send_json_line(writer: &mut impl Write, message: &Value) {
    let line = serde_json::to_string(message).expect("valid JSON");
    writer.write_all(line.as_bytes()).expect("write json line");
    writer.write_all(b"\n").expect("write newline");
    writer.flush().expect("flush line");
}

fn read_json_line(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("read json line");
    assert!(n > 0, "expected non-empty JSON-RPC response line");
    serde_json::from_str(line.trim()).expect("valid JSON-RPC line")
}

/// Asynchronous MCP client over `tokio::io::duplex` driving a `BrowserHandler<DispatchPort<...>>`
/// so tests can step `McpController` deterministically on the test thread.
struct DuplexSession {
    rt: tokio::runtime::Runtime,
    writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    reader: tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    next_id: u64,
    shutdown: ShutdownTrigger,
    server_task: tokio::task::JoinHandle<()>,
}

impl DuplexSession {
    fn start<H: BrowserHost>(client_id: u64, host: Arc<H>) -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let (srv_read, srv_write) = tokio::io::split(server_stream);
        let (cli_read, cli_write) = tokio::io::split(client_stream);
        let (shutdown, signal) = ShutdownTrigger::new();
        let config = BrowserHandlerConfig {
            request_timeout: Duration::from_secs(5),
            max_in_flight: 8,
        };
        let handler = BrowserHandler::with_config(ClientId::new(client_id), host, config);
        let server_task = rt.spawn(async move {
            let _ = serve_stream(handler, srv_read, srv_write, signal).await;
        });
        let mut session = Self {
            rt,
            writer: cli_write,
            reader: tokio::io::BufReader::new(cli_read),
            next_id: 1,
            shutdown,
            server_task,
        };
        session.initialize();
        session
    }

    fn send_raw(&mut self, msg: &Value) {
        let mut line = serde_json::to_string(msg).expect("json");
        line.push('\n');
        self.rt.block_on(async {
            self.writer.write_all(line.as_bytes()).await.expect("write");
            self.writer.flush().await.expect("flush");
        });
    }

    fn recv_raw(&mut self) -> Value {
        self.rt.block_on(async {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).await.expect("read");
            assert!(n > 0, "expected response line");
            serde_json::from_str(line.trim()).expect("valid json")
        })
    }

    fn initialize(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "unit-test", "version": "1.0" }
            }
        }));
        let _ = self.recv_raw();
        self.send_raw(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    }

    fn send_tool_call(&mut self, name: &str, arguments: &Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }));
        // Drive the Tokio runtime briefly so `serve_stream` parses the request and calls `dispatch`.
        self.rt.block_on(async {
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
        });
        id
    }

    fn finish(self) {
        self.shutdown.trigger();
        let _ = self.rt.block_on(self.server_task);
    }
}

fn drain_all<W: MainContextWakeup>(
    port: &DispatchPort<W>,
    controller: &mut McpController<FakeBackend>,
) {
    for event in port.drain() {
        controller.handle_event(event);
    }
}

#[test]
fn relay_attach_only_and_fail_on_other_errors_never_fall_back_to_standalone() {
    // Arrange
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("rt");
    std::fs::create_dir(&root).expect("create root");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    }
    let endpoint =
        SocketEndpoint::in_trusted_root(&root, root.join("quickie/mcp.sock")).expect("endpoint");
    let isolated_called = AtomicUsize::new(0);

    // Act
    let attach_only_missing = run_with_endpoint(
        &endpoint,
        true,
        |ep| Err(McpError::EndpointMissing(ep.socket_path().to_path_buf())),
        || {
            isolated_called.fetch_add(1, Ordering::SeqCst);
            Err(McpError::InvalidEndpoint("should not run".to_owned()))
        },
    );
    let stale_non_attach = run_with_endpoint(
        &endpoint,
        false,
        |ep| Err(McpError::EndpointStale(ep.socket_path().to_path_buf())),
        || {
            isolated_called.fetch_add(1, Ordering::SeqCst);
            Err(McpError::InvalidEndpoint("should not run".to_owned()))
        },
    );
    let permission_denied = run_with_endpoint(
        &endpoint,
        false,
        |ep| Err(McpError::PermissionDenied(ep.socket_path().to_path_buf())),
        || {
            isolated_called.fetch_add(1, Ordering::SeqCst);
            Err(McpError::InvalidEndpoint("should not run".to_owned()))
        },
    );

    // Assert
    assert!(attach_only_missing.is_err());
    assert!(stale_non_attach.is_err());
    assert!(permission_denied.is_err());
    assert_eq!(isolated_called.load(Ordering::SeqCst), 0);
}

#[test]
fn standalone_stdio_serves_real_mcp_without_opening_main_profile() {
    // Arrange
    let (client_to_server_r, client_to_server_w) = pipe_pair();
    let (server_to_client_r, server_to_client_w) = pipe_pair();
    let isolated = IsolatedStdio::from_fds(client_to_server_r, server_to_client_w);
    let host = Arc::new(StandaloneHost::new("0.1.0-standalone"));
    let handler = BrowserHandler::new(ClientId::new(1), host);
    let server = serve_stdio(isolated, handler).expect("serve standalone stdio");

    let mut writer = std::fs::File::from(client_to_server_w);
    let mut reader = BufReader::new(std::fs::File::from(server_to_client_r));

    // Act
    send_json_line(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "test-client", "version": "1.0" }
            }
        }),
    );
    let init_reply = read_json_line(&mut reader);
    send_json_line(
        &mut writer,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );
    send_json_line(
        &mut writer,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    );
    let list_reply = read_json_line(&mut reader);
    send_json_line(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "browser_status", "arguments": {} }
        }),
    );
    let status_reply = read_json_line(&mut reader);
    send_json_line(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "diagnostic_request", "arguments": {} }
        }),
    );
    let diag_reply = read_json_line(&mut reader);
    send_json_line(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "page_snapshot",
                "arguments": { "target": { "kind": "agent", "context_id": 1 }, "kind": "text" }
            }
        }),
    );
    let agent_reply = read_json_line(&mut reader);

    drop(writer);
    server.wait();

    // Assert
    assert_eq!(init_reply["result"]["serverInfo"]["name"], "quickie");
    let tools = list_reply["result"]["tools"]
        .as_array()
        .expect("tools array");
    assert!(tools.iter().any(|tool| tool["name"] == "browser_status"));
    let status = &status_reply["result"]["structuredContent"];
    assert_eq!(status["platform"], "linux");
    assert_eq!(status["runtime_mode"], "standalone");
    assert_eq!(status["product_version"], "0.1.0-standalone");
    assert_eq!(status["fork_capabilities"]["agent_automation"], false);
    assert_eq!(status["fork_capabilities"]["diagnostic"]["console"], false);
    assert_eq!(diag_reply["result"]["isError"], true);
    assert_eq!(
        diag_reply["result"]["structuredContent"]["code"],
        "tool_not_available"
    );
    assert_eq!(agent_reply["result"]["isError"], true);
    assert_eq!(
        agent_reply["result"]["structuredContent"]["code"],
        "tool_not_available"
    );
}

#[test]
fn dispatch_port_enforces_bounds_coalesces_wakeups_and_closes_cleanly() {
    // Arrange
    let port = Arc::new(DispatchPort::with_capacity(2, CountingWakeup::new()));
    let mut session = DuplexSession::start(1, Arc::clone(&port));

    // Act: enqueue 3 concurrent tool calls against capacity 2.
    session.send_tool_call("browser_status", &json!({}));
    session.send_tool_call("browser_status", &json!({}));
    session.send_tool_call("browser_status", &json!({}));
    let busy_reply = session.recv_raw();
    port.client_closed(ClientId::new(99));
    let wakes_before_close = port.wakeup.count();
    port.close();
    let closed_reply_1 = session.recv_raw();
    let closed_reply_2 = session.recv_raw();

    // Assert
    assert_eq!(wakes_before_close, 1);
    assert!(port.is_closed());
    assert_eq!(busy_reply["result"]["structuredContent"]["code"], "busy");
    assert_eq!(
        closed_reply_1["result"]["structuredContent"]["code"],
        "unavailable"
    );
    assert_eq!(
        closed_reply_2["result"]["structuredContent"]["code"],
        "unavailable"
    );
    session.finish();
}

#[test]
fn stock_webkit_reports_unavailable_diagnostics_without_showing_consent() {
    // Arrange
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::new(FakeBackend::stock_webkit(Some(binding(
        1,
        "https://example.com/page",
        1,
    ))));
    let mut session = DuplexSession::start(1, Arc::clone(&port));

    // Act
    session.send_tool_call("browser_status", &json!({}));
    drain_all(&port, &mut controller);
    let status = session.recv_raw();

    session.send_tool_call("diagnostic_request", &json!({}));
    drain_all(&port, &mut controller);
    let diag = session.recv_raw();

    // Assert
    let sc = &status["result"]["structuredContent"];
    assert_eq!(sc["platform"], "linux");
    assert_eq!(sc["runtime_mode"], "gui");
    assert_eq!(sc["fork_capabilities"]["diagnostic"]["dom"], true);
    assert_eq!(sc["fork_capabilities"]["diagnostic"]["console"], false);
    assert_eq!(sc["fork_capabilities"]["visual_inspector_priority"], false);
    assert_eq!(diag["result"]["isError"], true);
    assert_eq!(
        diag["result"]["structuredContent"]["code"],
        "unsupported_capability"
    );
    assert_eq!(controller.backend().shown_prompts.len(), 0);
    assert!(!controller.has_pending_consent());
    session.finish();
}

#[test]
fn consent_verifies_identity_before_on_confirm_and_after_async_snapshot() {
    // Arrange
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::new(FakeBackend::fork_ready(Some(binding(
        1,
        "https://example.com/a",
        1,
    ))));
    let mut session = DuplexSession::start(1, Arc::clone(&port));

    // Act 1: epoch advances before user confirms -> StaleDocument, no lease attached.
    session.send_tool_call("diagnostic_request", &json!({ "scope": "single_document" }));
    drain_all(&port, &mut controller);
    assert!(controller.backend().consent_visible);
    controller.backend_mut().active_document = Some(binding(1, "https://example.com/b", 2));
    controller.resolve_consent(true);
    let stale_reply = session.recv_raw();
    assert_eq!(
        stale_reply["result"]["structuredContent"]["code"],
        "stale_document"
    );
    assert_eq!(controller.active_attachment_count(), 0);

    // Act 2: fresh consent succeeds and starts async snapshot; user clicks Stop before callback.
    session.send_tool_call(
        "diagnostic_request",
        &json!({ "scope": "same_origin_reload" }),
    );
    drain_all(&port, &mut controller);
    controller.resolve_consent(true);
    let granted = session.recv_raw();
    let attachment_id = granted["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");
    assert_eq!(
        controller.backend().active_indicator,
        vec!["https://example.com"]
    );

    session.send_tool_call(
        "page_snapshot",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": attachment_id },
            "kind": "text"
        }),
    );
    drain_all(&port, &mut controller);
    let snap_id = controller.backend().started_snapshots[0].0;
    controller.stop_all_user_grants();
    controller.on_snapshot_completed(
        snap_id,
        Ok(DiagnosticSnapshot {
            data: "stale body".to_owned(),
            truncated: false,
        }),
    );
    let snap_reply = session.recv_raw();

    // Assert
    assert_eq!(
        snap_reply["result"]["structuredContent"]["code"],
        "attachment_unavailable"
    );
    assert_eq!(controller.backend().stopped_leases.len(), 1);
    assert_eq!(controller.backend().active_indicator.len(), 0);
    session.finish();
}

#[test]
fn tab_switch_never_retargets_and_same_origin_reload_survives_only_explicit_reloads() {
    // Arrange
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::new(FakeBackend::fork_ready(Some(binding(
        1,
        "https://example.com/start",
        1,
    ))));
    let mut session = DuplexSession::start(1, Arc::clone(&port));
    session.send_tool_call(
        "diagnostic_request",
        &json!({ "scope": "same_origin_reload" }),
    );
    drain_all(&port, &mut controller);
    controller.resolve_consent(true);
    let _granted = session.recv_raw();

    // Act 1: switching to tab 2 never retargets or revokes tab 1's grant.
    controller.backend_mut().active_document = Some(binding(2, "https://other.test/", 1));
    controller.on_active_tab_changed(Some(TabId::new(Profile::Main, 2)));
    let after_switch = controller.active_attachment_count();

    // Act 2: verified explicit same-origin reload survives with higher epoch.
    controller.on_navigation(
        TabId::new(Profile::Main, 1),
        &binding(1, "https://example.com/start", 2),
        NavigationKind::Reload,
    );
    let after_reload = controller.active_attachment_count();

    // Act 3: non-reload same-origin navigation revokes even under SameOriginReload.
    controller.on_navigation(
        TabId::new(Profile::Main, 1),
        &binding(1, "https://example.com/next", 3),
        NavigationKind::NewDocument,
    );

    // Assert
    assert_eq!(after_switch, 1);
    assert_eq!(after_reload, 1);
    assert_eq!(controller.active_attachment_count(), 0);
    assert_eq!(controller.backend().stopped_leases.len(), 1);
    session.finish();
}

#[test]
fn native_frames_overflow_inspector_takeover_and_protocol_allowlist() {
    // Arrange
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::new(FakeBackend::fork_ready(Some(binding(
        1,
        "https://example.com/",
        1,
    ))));
    let mut session = DuplexSession::start(1, Arc::clone(&port));
    session.send_tool_call("diagnostic_request", &json!({ "scope": "single_document" }));
    drain_all(&port, &mut controller);
    controller.resolve_consent(true);
    let granted = session.recv_raw();
    let raw_id = granted["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");
    let attachment = quickie_application::mcp::AttachmentId::from_wire(raw_id);

    // Act: feed targetCreated, console message, redacted network request, and queue overflow.
    controller.on_native_frame(
        attachment,
        r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"page-1","type":"page","isProvisional":false}}}"#,
    );
    controller.on_native_frame(
        attachment,
        r#"{"method":"Console.messageAdded","params":{"message":{"source":"console-api","level":"log","text":"hello linux"}}}"#,
    );
    controller.on_native_frame(
        attachment,
        r#"{"method":"Network.requestWillBeSent","params":{"requestId":"r1","request":{"url":"https://user:secret@api.example.com/v1/items?token=abc","method":"get"}}}"#,
    );
    controller.on_native_frame(
        attachment,
        r#"{"method":"LocalDiagnostic.queueOverflow","params":{"droppedCount":3,"totalDroppedCount":3}}"#,
    );

    session.send_tool_call(
        "page_events",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": raw_id },
            "cursor": 0,
            "limit": 10
        }),
    );
    drain_all(&port, &mut controller);
    let events_reply = session.recv_raw();

    // Visual Inspector takeover revokes immediately.
    controller.on_inspector_opened(TabId::new(Profile::Main, 1));

    // Assert
    let sc = &events_reply["result"]["structuredContent"];
    assert_eq!(sc["native_overflow"], true);
    assert_eq!(sc["closed"], false);
    let events = sc["events"].as_array().expect("events array");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["text"], "hello linux");
    assert_eq!(events[1]["endpoint"]["origin"], "https://api.example.com");
    assert_eq!(events[1]["endpoint"]["path"], "/v1/items");
    assert_eq!(events[1]["method"], "GET");
    assert!(!events_reply.to_string().contains("secret"));
    assert!(!events_reply.to_string().contains("token=abc"));
    let dispatched = controller
        .backend()
        .dispatched_commands
        .get(&attachment)
        .expect("commands dispatched");
    assert!(dispatched.iter().any(|cmd| matches!(
        cmd,
        DecodedSubscription::Direct {
            method: DirectMethod::NetworkEnable,
            ..
        }
    )));
    assert!(dispatched.iter().any(|cmd| matches!(
        cmd,
        DecodedSubscription::Target {
            target_id,
            method: TargetMethod::ConsoleEnable,
            ..
        } if target_id == "page-1"
    )));
    assert_eq!(controller.active_attachment_count(), 0);
    assert!(
        parse_allowlisted_command(r#"{"id":1,"method":"Runtime.evaluate","params":{}}"#).is_none()
    );
    let _ = DiagnosticEventData::lifecycle(
        quickie_application::mcp::LifecycleEventKind::DomContentLoaded,
    );
    session.finish();
}

struct ChannelWakeup(std::sync::Mutex<std::sync::mpsc::Sender<()>>);

impl MainContextWakeup for ChannelWakeup {
    fn wake(&self) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(());
        }
    }
}

fn spawn_synthetic_gui_loop(
    port: Arc<DispatchPort<ChannelWakeup>>,
    wake_rx: std::sync::mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut controller = McpController::new(FakeBackend::fork_ready(Some(binding(
            1,
            "https://example.com/app",
            1,
        ))));
        let mut completed_snaps = 0usize;
        while !stop.load(Ordering::SeqCst) {
            if wake_rx.recv_timeout(Duration::from_millis(20)).is_ok() {
                for event in port.drain() {
                    controller.handle_event(event);
                    if controller.has_pending_consent() {
                        controller.resolve_consent(true);
                    }
                    while completed_snaps < controller.backend().started_snapshots.len() {
                        let snap_id = controller.backend().started_snapshots[completed_snaps].0;
                        completed_snaps += 1;
                        controller.on_snapshot_completed(
                            snap_id,
                            Ok(DiagnosticSnapshot {
                                data: "synthetic dom".to_owned(),
                                truncated: false,
                            }),
                        );
                    }
                }
            }
        }
        controller.shutdown();
    })
}

fn call_over_stdio(
    writer: &mut impl Write,
    reader: &mut impl BufRead,
    id: u64,
    name: &str,
    arguments: &Value,
) -> Value {
    send_json_line(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
    );
    read_json_line(reader)
}

#[test]
fn real_socket_relay_and_synthetic_existing_tab_roundtrip() {
    // Arrange
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("rt");
    std::fs::create_dir(&root).expect("create root");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    }
    let endpoint =
        SocketEndpoint::in_trusted_root(&root, root.join("quickie/mcp.sock")).expect("endpoint");
    let (wake_tx, wake_rx) = std::sync::mpsc::channel::<()>();
    let port = Arc::new(DispatchPort::new(ChannelWakeup(std::sync::Mutex::new(
        wake_tx,
    ))));
    let factory = Arc::new(BrowserHandlerFactory::new(Arc::clone(&port)));
    let server = serve_socket(&endpoint, move || factory.next_handler()).expect("serve socket");
    let stop_loop = Arc::new(AtomicBool::new(false));
    let gui_thread = spawn_synthetic_gui_loop(Arc::clone(&port), wake_rx, Arc::clone(&stop_loop));

    let (client_to_relay_r, client_to_relay_w) = pipe_pair();
    let (relay_to_client_r, relay_to_client_w) = pipe_pair();
    let relay_endpoint = endpoint.clone();
    let relay_thread = thread::spawn(move || {
        run_relay_fds(
            &relay_endpoint,
            client_to_relay_r,
            relay_to_client_w,
            RelayConfig::default(),
        )
    });

    let mut writer = std::fs::File::from(client_to_relay_w);
    let mut reader = BufReader::new(std::fs::File::from(relay_to_client_r));

    // Act
    send_json_line(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "relay-test", "version": "1.0" }
            }
        }),
    );
    let _init = read_json_line(&mut reader);
    send_json_line(
        &mut writer,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );
    let status = call_over_stdio(&mut writer, &mut reader, 2, "browser_status", &json!({}));
    let granted = call_over_stdio(
        &mut writer,
        &mut reader,
        3,
        "diagnostic_request",
        &json!({ "scope": "single_document" }),
    );
    let attachment_id = granted["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");
    let snap = call_over_stdio(
        &mut writer,
        &mut reader,
        4,
        "page_snapshot",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": attachment_id },
            "kind": "dom"
        }),
    );
    let detached = call_over_stdio(
        &mut writer,
        &mut reader,
        5,
        "diagnostic_detach",
        &json!({ "attachment_id": attachment_id }),
    );

    drop(writer);
    let mut drain = Vec::new();
    let _ = reader.read_to_end(&mut drain);
    relay_thread.join().expect("relay join").expect("relay ok");
    stop_loop.store(true, Ordering::SeqCst);
    port.close();
    gui_thread.join().expect("gui join");
    server.shutdown();

    // Assert
    assert_eq!(status["result"]["structuredContent"]["runtime_mode"], "gui");
    assert_eq!(
        granted["result"]["structuredContent"]["origin"],
        "https://example.com"
    );
    assert_eq!(snap["result"]["structuredContent"]["data"], "synthetic dom");
    assert_eq!(detached["result"]["structuredContent"]["detached"], true);
}

#[test]
fn snapshot_script_semantics_match_appkit_and_reject_accessibility() {
    // Arrange
    let a11y = SnapshotRequest::with_max_bounds(SnapshotKind::Accessibility, None);
    let dom = SnapshotRequest::new(SnapshotKind::Dom, Some("#main".to_owned()), 10, 4, 128);
    let text = SnapshotRequest::new(SnapshotKind::Text, None, 10, 4, 5);

    // Act
    let a11y_err = snapshot::build_script(&a11y).expect_err("a11y unsupported");
    let dom_script = snapshot::build_script(&dom).expect("dom script");
    let text_script = snapshot::build_script(&text).expect("text script");
    let parsed_truncated =
        snapshot::parse_reply(r#"{"data":"hello world","truncated":false}"#, 5).expect("truncated");
    let parsed_missing =
        snapshot::parse_reply(r#"{"error":"selector_not_found"}"#, 64).expect_err("missing");

    // Assert
    assert_eq!(a11y_err.code(), ToolErrorCode::UnsupportedCapability);
    assert!(dom_script.contains(r##"const selector = "#main";"##));
    assert!(text_script.contains("const maxBytes = 5;"));
    assert_eq!(parsed_truncated.data, "hello");
    assert!(parsed_truncated.truncated);
    assert_eq!(parsed_missing.code(), ToolErrorCode::InvalidParams);
}

#[test]
fn tab_close_process_crash_native_close_and_lease_attach_failures_revoke_cleanly() {
    // Arrange
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::new(FakeBackend::fork_ready(Some(binding(
        1,
        "https://example.com/",
        1,
    ))));
    let mut session = DuplexSession::start(1, Arc::clone(&port));

    // Act 1: lease attach failures roll back DiagnosticState and map to typed errors.
    for (attach_err, expected_code) in [
        (LeaseAttachError::UnsupportedSpi, "unsupported_capability"),
        (LeaseAttachError::OccupiedInspector, "busy"),
        (LeaseAttachError::ClosedStream, "unavailable"),
    ] {
        controller.backend_mut().attach_error = Some(attach_err);
        session.send_tool_call("diagnostic_request", &json!({}));
        drain_all(&port, &mut controller);
        controller.resolve_consent(true);
        let reply = session.recv_raw();
        assert_eq!(reply["result"]["structuredContent"]["code"], expected_code);
        assert_eq!(controller.active_attachment_count(), 0);
    }
    controller.backend_mut().attach_error = None;

    // Act 2: tab close revokes active grant and cancels pending consent.
    session.send_tool_call("diagnostic_request", &json!({}));
    drain_all(&port, &mut controller);
    controller.resolve_consent(true);
    let _ = session.recv_raw();
    controller.on_tab_closed(TabId::new(Profile::Main, 1));
    assert_eq!(controller.active_attachment_count(), 0);

    // Act 3: process termination revokes active grant.
    session.send_tool_call("diagnostic_request", &json!({}));
    drain_all(&port, &mut controller);
    controller.resolve_consent(true);
    let _ = session.recv_raw();
    controller.on_process_terminated(TabId::new(Profile::Main, 1));
    assert_eq!(controller.active_attachment_count(), 0);

    // Act 4: native inspector close revokes active grant.
    session.send_tool_call("diagnostic_request", &json!({}));
    drain_all(&port, &mut controller);
    controller.resolve_consent(true);
    let granted = session.recv_raw();
    let raw_id = granted["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");
    controller.on_native_closed(quickie_application::mcp::AttachmentId::from_wire(raw_id));

    // Assert
    assert_eq!(controller.active_attachment_count(), 0);
    assert_eq!(controller.backend().stopped_leases.len(), 3);
    session.finish();
}
