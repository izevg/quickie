//! Comprehensive AAA unit and stdio/duplex integration tests for isolated GTK Agent MCP tools.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use quickie_application::mcp::{
    AgentContextId, ClientId, DiagnosticCapabilities, DocumentBinding, SnapshotKind,
    SnapshotRequest,
};
use quickie_mcp::browser::{AgentLocator, AgentLocatorKind, AgentViewport};
use quickie_mcp::{
    BrowserHandler, BrowserHandlerConfig, BrowserHost, ForkCapabilities, IsolatedStdio,
    ShutdownTrigger, ToolError, ToolErrorCode, serve_stream,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use url::Url;

use crate::mcp::agent::protocol::{
    AutomationWireCommand, InboundFrame, WireKeyStep, WireMouseInteraction,
    decode_screenshot_reply, parse_inbound_frame,
};
use crate::mcp::agent::scripts::{
    build_accessibility_fallback_script, build_locate_script, build_resolve_element_script,
    parse_locate_reply, parse_resolve_element_reply,
};
use crate::mcp::agent::{AgentApprovalPrompt, AgentBackend, MAX_IN_FLIGHT_COMMANDS};
use crate::mcp::runtime::controller::{
    ConsentPrompt, HostBackend, LeaseAttachError, McpController,
};
use crate::mcp::runtime::protocol::DecodedSubscription;
use crate::mcp::runtime::{DispatchPort, MainContextWakeup};
use crate::mcp::{serve_standalone_stdio_with_version, standalone::StandaloneHost};

/// Valid 1x1 PNG (67 bytes) with a real IHDR chunk (`width = 1`, `height = 1`).
const VALID_1X1_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

struct CountingWakeup(AtomicUsize);

impl CountingWakeup {
    fn new() -> Self {
        Self(AtomicUsize::new(0))
    }
}

impl MainContextWakeup for CountingWakeup {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct StubHostBackend {
    version: String,
}

impl StubHostBackend {
    fn new(version: &str) -> Self {
        Self {
            version: version.to_owned(),
        }
    }
}

impl HostBackend for StubHostBackend {
    type Lease = ();
    type SnapshotHandle = ();

    fn product_version(&self) -> String {
        self.version.clone()
    }

    fn capabilities(&self) -> ForkCapabilities {
        ForkCapabilities {
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
        }
    }

    fn active_document(&self) -> Option<DocumentBinding> {
        None
    }

    fn show_consent(&mut self, _prompt: ConsentPrompt) {}
    fn hide_consent(&mut self) {}
    fn set_active_indicator(&mut self, _active_origins: &[String]) {}

    fn attach_lease(
        &mut self,
        _attachment: quickie_application::mcp::AttachmentId,
        _binding: &DocumentBinding,
    ) -> Result<Self::Lease, LeaseAttachError> {
        Err(LeaseAttachError::ClosedStream)
    }

    fn dispatch_lease_command(
        &mut self,
        _lease: &mut Self::Lease,
        _command: &DecodedSubscription,
    ) -> bool {
        false
    }

    fn stop_lease(&mut self, _lease: Self::Lease) {}

    fn start_snapshot(
        &mut self,
        _snapshot_id: u64,
        _binding: &DocumentBinding,
        _request: &SnapshotRequest,
    ) -> Result<Self::SnapshotHandle, ToolError> {
        Err(ToolError::new(
            ToolErrorCode::Unavailable,
            "not used in agent tests",
        ))
    }
}

#[derive(Debug, Clone)]
struct FakeSession {
    context_id: AgentContextId,
    session_id: String,
    viewport: AgentViewport,
}

struct FakeAgentBackend {
    available: bool,
    interactive_ui: bool,
    opened_sessions: Vec<AgentContextId>,
    closed_sessions: Vec<AgentContextId>,
    granted_urls: Vec<(AgentContextId, Url)>,
    dispatched: Vec<(AgentContextId, u64, AutomationWireCommand)>,
    started_scripts: Vec<(u64, AgentContextId, String)>,
    scheduled_timers: Vec<(u64, u64)>,
    shown_approvals: Vec<AgentApprovalPrompt>,
    approval_visible: bool,
    open_error: Option<ToolError>,
}

impl FakeAgentBackend {
    fn fork_gui() -> Self {
        Self {
            available: true,
            interactive_ui: true,
            opened_sessions: Vec::new(),
            closed_sessions: Vec::new(),
            granted_urls: Vec::new(),
            dispatched: Vec::new(),
            started_scripts: Vec::new(),
            scheduled_timers: Vec::new(),
            shown_approvals: Vec::new(),
            approval_visible: false,
            open_error: None,
        }
    }

    fn fork_standalone() -> Self {
        Self {
            interactive_ui: false,
            ..Self::fork_gui()
        }
    }

    fn stock_unavailable() -> Self {
        Self {
            available: false,
            ..Self::fork_gui()
        }
    }
}

impl AgentBackend for FakeAgentBackend {
    type Session = FakeSession;
    type EvalHandle = u64;
    type TimerHandle = u64;

    fn is_available(&self) -> bool {
        self.available
    }

    fn open_session(
        &mut self,
        context_id: AgentContextId,
        session_id: &str,
        viewport: AgentViewport,
    ) -> Result<Self::Session, ToolError> {
        if let Some(err) = &self.open_error {
            return Err(err.clone());
        }
        self.opened_sessions.push(context_id);
        Ok(FakeSession {
            context_id,
            session_id: session_id.to_owned(),
            viewport,
        })
    }

    fn close_session(&mut self, session: Self::Session) {
        let _ = (&session.session_id, session.viewport);
        self.closed_sessions.push(session.context_id);
    }

    fn grant_navigation(
        &mut self,
        session: &mut Self::Session,
        url: &Url,
    ) -> Result<(), ToolError> {
        self.granted_urls.push((session.context_id, url.clone()));
        Ok(())
    }

    fn dispatch_command(
        &mut self,
        session: &mut Self::Session,
        request_id: u64,
        command: &AutomationWireCommand,
    ) -> Result<(), ToolError> {
        self.dispatched
            .push((session.context_id, request_id, command.clone()));
        Ok(())
    }

    fn start_script(
        &mut self,
        eval_id: u64,
        session: &Self::Session,
        script: &str,
    ) -> Result<Self::EvalHandle, ToolError> {
        self.started_scripts
            .push((eval_id, session.context_id, script.to_owned()));
        Ok(eval_id)
    }

    fn schedule_timer(&mut self, timer_id: u64, delay_ms: u64) -> Self::TimerHandle {
        self.scheduled_timers.push((timer_id, delay_ms));
        timer_id
    }

    fn show_approval(&mut self, prompt: AgentApprovalPrompt) -> bool {
        if !self.interactive_ui {
            return false;
        }
        self.approval_visible = true;
        self.shown_approvals.push(prompt);
        true
    }

    fn hide_approval(&mut self) {
        self.approval_visible = false;
    }
}

struct DuplexClient {
    rt: tokio::runtime::Runtime,
    writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    reader: tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    next_id: u64,
    shutdown: ShutdownTrigger,
    server_task: tokio::task::JoinHandle<()>,
    init_reply: Value,
}

impl DuplexClient {
    fn start<H: BrowserHost>(client_id: u64, host: Arc<H>, version: &str) -> Self {
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
            max_in_flight: 32,
        };
        let handler = BrowserHandler::with_config(ClientId::new(client_id), host, config)
            .with_product_version(version);
        let server_task = rt.spawn(async move {
            let _ = serve_stream(handler, srv_read, srv_write, signal).await;
        });
        let mut client = Self {
            rt,
            writer: cli_write,
            reader: tokio::io::BufReader::new(cli_read),
            next_id: 1,
            shutdown,
            server_task,
            init_reply: Value::Null,
        };
        let init = client.initialize();
        client.init_reply = init;
        client
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

    fn initialize(&mut self) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "agent-unit-test", "version": "1.0" }
            }
        }));
        let reply = self.recv_raw();
        self.send_raw(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        reply
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
        self.rt.block_on(async {
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
        });
        id
    }

    fn disconnect_eof(mut self) {
        let _ = self.rt.block_on(self.writer.shutdown());
        drop(self.writer);
        drop(self.reader);
        let _ = self.rt.block_on(self.server_task);
    }

    fn finish(self) {
        self.shutdown.trigger();
        let _ = self.rt.block_on(self.server_task);
    }
}

fn drain<W: MainContextWakeup>(
    port: &DispatchPort<W>,
    controller: &mut McpController<StubHostBackend, FakeAgentBackend>,
) {
    for event in port.drain() {
        controller.handle_event(event);
    }
}

fn pipe_pair() -> (OwnedFd, OwnedFd) {
    let (reader, writer) = std::io::pipe().expect("pipe creation");
    (OwnedFd::from(reader), OwnedFd::from(writer))
}

fn send_json_line(writer: &mut impl Write, message: &Value) {
    let line = serde_json::to_string(message).expect("valid JSON");
    writer.write_all(line.as_bytes()).expect("write");
    writer.write_all(b"\n").expect("newline");
    writer.flush().expect("flush");
}

fn read_json_line(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("read");
    assert!(n > 0, "expected JSON-RPC line");
    serde_json::from_str(line.trim()).expect("valid JSON")
}

#[test]
fn initialize_and_browser_status_supply_identical_product_version_in_gui_and_standalone() {
    // Arrange
    let version = "0.7.3-linux";
    let (c2s_r, c2s_w) = pipe_pair();
    let (s2c_r, s2c_w) = pipe_pair();
    let isolated = IsolatedStdio::from_fds(c2s_r, s2c_w);
    let standalone_host = Arc::new(StandaloneHost::new(version));
    let standalone_thread = std::thread::spawn(move || {
        serve_standalone_stdio_with_version(isolated, standalone_host, version)
    });
    let mut writer = std::fs::File::from(c2s_w);
    let mut reader = BufReader::new(std::fs::File::from(s2c_r));

    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller =
        McpController::with_agent(StubHostBackend::new(version), FakeAgentBackend::fork_gui());
    let mut gui_client = DuplexClient::start(1, Arc::clone(&port), version);

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
                "clientInfo": { "name": "stdio-test", "version": "1.0" }
            }
        }),
    );
    let standalone_init = read_json_line(&mut reader);
    send_json_line(
        &mut writer,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );
    send_json_line(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "browser_status", "arguments": {} }
        }),
    );
    let standalone_status = read_json_line(&mut reader);
    drop(writer);
    standalone_thread.join().expect("join").expect("serve ok");

    gui_client.send_tool_call("browser_status", &json!({}));
    drain(&port, &mut controller);
    let gui_status = gui_client.recv_raw();

    // Assert
    assert_eq!(standalone_init["result"]["serverInfo"]["version"], version);
    assert_eq!(
        standalone_status["result"]["structuredContent"]["product_version"],
        version
    );
    assert_eq!(
        gui_client.init_reply["result"]["serverInfo"]["version"],
        version
    );
    assert_eq!(
        gui_status["result"]["structuredContent"]["product_version"],
        version
    );
    assert_eq!(
        gui_status["result"]["structuredContent"]["fork_capabilities"]["agent_automation"],
        true
    );
    assert_eq!(
        gui_status["result"]["structuredContent"]["fork_capabilities"]["native_accessibility"],
        false
    );
    gui_client.finish();
}

#[test]
fn context_ownership_nonreuse_eof_and_process_failure_teardown() {
    // Arrange
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::with_agent(
        StubHostBackend::new("0.1.0"),
        FakeAgentBackend::fork_standalone(),
    );
    let mut client_a = DuplexClient::start(10, Arc::clone(&port), "0.1.0");
    let mut client_b = DuplexClient::start(20, Arc::clone(&port), "0.1.0");

    // Act 1: Client A opens two contexts, closes first, opens a third -> IDs never reused.
    client_a.send_tool_call("agent_context_open", &json!({}));
    drain(&port, &mut controller);
    let open1 = client_a.recv_raw();
    let ctx1 = open1["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("ctx1");

    client_a.send_tool_call("agent_context_close", &json!({ "context_id": ctx1 }));
    drain(&port, &mut controller);
    let close1 = client_a.recv_raw();

    client_a.send_tool_call(
        "agent_context_open",
        &json!({ "viewport": { "width": 800, "height": 600 } }),
    );
    drain(&port, &mut controller);
    let open2 = client_a.recv_raw();
    let ctx2 = open2["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("ctx2");

    // Act 2: Client B cannot list, close, or observe Client A's context.
    client_b.send_tool_call("agent_context_list", &json!({}));
    drain(&port, &mut controller);
    let list_b = client_b.recv_raw();

    client_b.send_tool_call("agent_context_close", &json!({ "context_id": ctx2 }));
    drain(&port, &mut controller);
    let close_b_err = client_b.recv_raw();

    client_b.send_tool_call(
        "page_snapshot",
        &json!({ "target": { "kind": "agent", "context_id": ctx2 }, "kind": "text" }),
    );
    drain(&port, &mut controller);
    let snap_b_err = client_b.recv_raw();

    // Client B opens its own context, then Client A disconnects via EOF.
    client_b.send_tool_call("agent_context_open", &json!({}));
    drain(&port, &mut controller);
    let open_b = client_b.recv_raw();
    let ctx_b = open_b["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("ctx_b");

    client_a.disconnect_eof();
    drain(&port, &mut controller);
    let after_a_eof = controller.agent().active_context_count();

    // Native process crash destroys Client B's context cleanly.
    controller.on_agent_closed(AgentContextId::new(ctx_b));
    let after_crash = controller.agent().active_context_count();

    // Assert
    assert_eq!(close1["result"]["structuredContent"]["closed"], true);
    assert!(ctx2 > ctx1, "AgentContextId must never be reused");
    assert_eq!(
        list_b["result"]["structuredContent"]["contexts"]
            .as_array()
            .expect("contexts")
            .len(),
        0
    );
    assert_eq!(
        close_b_err["result"]["structuredContent"]["code"],
        "attachment_unavailable"
    );
    assert_eq!(
        snap_b_err["result"]["structuredContent"]["code"],
        "attachment_unavailable"
    );
    assert_eq!(
        after_a_eof, 1,
        "Client A EOF must destroy only A's contexts"
    );
    assert_eq!(after_crash, 0, "process failure must destroy context");
    assert_eq!(controller.agent().backend().closed_sessions.len(), 3);
    client_b.finish();
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "End-to-end approval scenario compares standalone loopback, GUI deny, GUI allow, and same-origin reuse"
)]
fn loopback_and_cross_origin_navigation_require_explicit_approval_in_standalone_and_gui() {
    // Arrange: standalone (no GUI prompt) and GUI (with interactive approval modal).
    let port_st = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut ctrl_st = McpController::with_agent(
        StubHostBackend::new("0.1.0"),
        FakeAgentBackend::fork_standalone(),
    );
    let mut cli_st = DuplexClient::start(1, Arc::clone(&port_st), "0.1.0");
    cli_st.send_tool_call("agent_context_open", &json!({}));
    drain(&port_st, &mut ctrl_st);
    let ctx_st = cli_st.recv_raw()["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("ctx_st");
    ctrl_st.on_agent_message(
        AgentContextId::new(ctx_st),
        r#"{"id":1,"result":{"contexts":[{"handle":"page-st"}]}}"#,
    );

    let port_gui = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut ctrl_gui =
        McpController::with_agent(StubHostBackend::new("0.1.0"), FakeAgentBackend::fork_gui());
    let mut cli_gui = DuplexClient::start(2, Arc::clone(&port_gui), "0.1.0");
    cli_gui.send_tool_call("agent_context_open", &json!({}));
    drain(&port_gui, &mut ctrl_gui);
    let ctx_gui = cli_gui.recv_raw()["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("ctx_gui");
    ctrl_gui.on_agent_message(
        AgentContextId::new(ctx_gui),
        r#"{"id":1,"result":{"contexts":[{"handle":"page-gui"}]}}"#,
    );

    // Act 1: Standalone loopback navigation MUST NOT implicitly approve.
    cli_st.send_tool_call(
        "agent_navigate",
        &json!({ "context_id": ctx_st, "url": "http://127.0.0.1:8080/fixture" }),
    );
    drain(&port_st, &mut ctrl_st);
    let st_nav = cli_st.recv_raw();

    // Act 2: GUI denies first, then approves second, then same-origin navigates without prompt.
    cli_gui.send_tool_call(
        "agent_navigate",
        &json!({ "context_id": ctx_gui, "url": "http://127.0.0.1:8080/a" }),
    );
    drain(&port_gui, &mut ctrl_gui);
    assert!(ctrl_gui.has_pending_consent());
    ctrl_gui.resolve_consent(false);
    let gui_denied = cli_gui.recv_raw();

    cli_gui.send_tool_call(
        "agent_navigate",
        &json!({ "context_id": ctx_gui, "url": "http://127.0.0.1:8080/a", "wait_stage": "dom_content_loaded" }),
    );
    drain(&port_gui, &mut ctrl_gui);
    assert!(ctrl_gui.has_pending_consent());
    ctrl_gui.resolve_consent(true);
    let nav_cmd_id = ctrl_gui
        .agent()
        .backend()
        .dispatched
        .iter()
        .rev()
        .find_map(|(_, id, cmd)| {
            matches!(cmd, AutomationWireCommand::Navigate { .. }).then_some(*id)
        })
        .expect("navigate command dispatched");
    ctrl_gui.on_agent_message(
        AgentContextId::new(ctx_gui),
        &format!(r#"{{"id":{nav_cmd_id},"result":{{}}}}"#),
    );
    let gui_committed = cli_gui.recv_raw();

    // Same-origin second navigation requires no second prompt.
    cli_gui.send_tool_call(
        "agent_navigate",
        &json!({ "context_id": ctx_gui, "url": "http://127.0.0.1:8080/b" }),
    );
    drain(&port_gui, &mut ctrl_gui);
    let prompted_for_same_origin = ctrl_gui.has_pending_consent();
    let nav2_id = ctrl_gui
        .agent()
        .backend()
        .dispatched
        .iter()
        .rev()
        .find_map(|(_, id, cmd)| {
            matches!(cmd, AutomationWireCommand::Navigate { .. }).then_some(*id)
        })
        .expect("second navigate command");
    ctrl_gui.on_agent_message(
        AgentContextId::new(ctx_gui),
        &format!(r#"{{"id":{nav2_id},"result":{{}}}}"#),
    );
    let gui_committed_2 = cli_gui.recv_raw();

    // Assert
    assert_eq!(
        st_nav["result"]["structuredContent"]["status"],
        "approval_required"
    );
    assert!(
        ctrl_st.agent().backend().granted_urls.is_empty(),
        "loopback must never grant navigation implicitly"
    );
    assert_eq!(
        gui_denied["result"]["structuredContent"]["status"],
        "rejected"
    );
    assert_eq!(
        gui_committed["result"]["structuredContent"]["status"],
        "committed"
    );
    assert_eq!(gui_committed["result"]["structuredContent"]["epoch"], 2);
    assert!(!prompted_for_same_origin);
    assert_eq!(
        gui_committed_2["result"]["structuredContent"]["status"],
        "committed"
    );
    assert_eq!(gui_committed_2["result"]["structuredContent"]["epoch"], 3);
    cli_st.finish();
    cli_gui.finish();
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "End-to-end act scenario exercises stale refs, irreversible approval, safe native click, and unsupported operations"
)]
fn stale_refs_irreversible_act_approval_safe_trusted_input_and_unsupported_ops() {
    // Arrange
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::with_agent(
        StubHostBackend::new("0.1.0"),
        FakeAgentBackend::fork_standalone(),
    );
    let mut client = DuplexClient::start(1, Arc::clone(&port), "0.1.0");
    client.send_tool_call("agent_context_open", &json!({}));
    drain(&port, &mut controller);
    let ctx_id = client.recv_raw()["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("ctx_id");
    let ctx = AgentContextId::new(ctx_id);
    controller.on_agent_message(
        ctx,
        r#"{"id":1,"result":{"contexts":[{"handle":"page-1"}]}}"#,
    );
    let url = Url::parse("https://example.com/form").expect("url");
    controller
        .agent_mut()
        .grant_origin_explicit(ctx, &url)
        .expect("explicit grant");

    // Navigate to bump epoch to 2.
    client.send_tool_call(
        "agent_navigate",
        &json!({ "context_id": ctx_id, "url": "https://example.com/form" }),
    );
    drain(&port, &mut controller);
    let nav_id = controller
        .agent()
        .backend()
        .dispatched
        .iter()
        .rev()
        .find_map(|(_, id, cmd)| {
            matches!(cmd, AutomationWireCommand::Navigate { .. }).then_some(*id)
        })
        .expect("nav cmd");
    controller.on_agent_message(ctx, &format!(r#"{{"id":{nav_id},"result":{{}}}}"#));
    let _ = client.recv_raw();

    // Act 1: Stale epoch element ref -> StaleDocument without touching WebView.
    client.send_tool_call(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": {
                "kind": "click",
                "element": { "context_id": ctx_id, "epoch": 1, "element_id": "el-old" }
            }
        }),
    );
    drain(&port, &mut controller);
    let stale_reply = client.recv_raw();

    // Act 2: Irreversible submit button -> approval_required in standalone.
    client.send_tool_call(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": {
                "kind": "click",
                "element": { "context_id": ctx_id, "epoch": 2, "element_id": "el-submit" }
            }
        }),
    );
    drain(&port, &mut controller);
    let eval_submit = controller
        .agent()
        .backend()
        .started_scripts
        .last()
        .expect("eval")
        .0;
    controller.on_agent_script_completed(
        eval_submit,
        Ok(r#"{"x":100,"y":50,"role":"button","tag":"button","requires_approval":true,"approval_reason":"form submission button requires explicit approval","is_editable":false}"#.to_owned()),
    );
    let submit_reply = client.recv_raw();

    // Act 3: Safe click -> dispatches native MouseInteraction::SingleClick + reads observed state.
    client.send_tool_call(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": {
                "kind": "click",
                "element": { "context_id": ctx_id, "epoch": 2, "element_id": "el-safe" }
            }
        }),
    );
    drain(&port, &mut controller);
    let eval_safe = controller
        .agent()
        .backend()
        .started_scripts
        .last()
        .expect("eval")
        .0;
    controller.on_agent_script_completed(
        eval_safe,
        Ok(r#"{"x":42.5,"y":88.0,"role":"button","tag":"button","requires_approval":false,"approval_reason":null,"is_editable":false}"#.to_owned()),
    );
    let click_cmd_id = controller
        .agent()
        .backend()
        .dispatched
        .iter()
        .rev()
        .find_map(|(_, id, cmd)| {
            matches!(
                cmd,
                AutomationWireCommand::MouseInteraction {
                    interaction: WireMouseInteraction::SingleClick,
                    ..
                }
            )
            .then_some(*id)
        })
        .expect("click dispatched");
    controller.on_agent_message(ctx, &format!(r#"{{"id":{click_cmd_id},"result":{{}}}}"#));
    let eval_obs = controller
        .agent()
        .backend()
        .started_scripts
        .last()
        .expect("obs")
        .0;
    controller.on_agent_script_completed(
        eval_obs,
        Ok(
            r#"{"focused_role":"button","value_length":null,"scroll_x":0,"scroll_y":120}"#
                .to_owned(),
        ),
    );
    let safe_reply = client.recv_raw();

    // Act 4: Truly unsupported operations return typed unsupported_capability.
    client.send_tool_call(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": {
                "kind": "select_option",
                "element": { "context_id": ctx_id, "epoch": 2, "element_id": "el-sel" },
                "value": "opt1"
            }
        }),
    );
    drain(&port, &mut controller);
    let select_reply = client.recv_raw();

    client.send_tool_call(
        "agent_screenshot",
        &json!({
            "context_id": ctx_id,
            "target": {
                "kind": "element",
                "element": { "context_id": ctx_id, "epoch": 2, "element_id": "el-safe" }
            }
        }),
    );
    drain(&port, &mut controller);
    let elem_shot_reply = client.recv_raw();

    // Assert
    assert_eq!(
        stale_reply["result"]["structuredContent"]["code"],
        "stale_document"
    );
    assert_eq!(
        submit_reply["result"]["structuredContent"]["status"],
        "approval_required"
    );
    assert_eq!(
        safe_reply["result"]["structuredContent"]["status"],
        "performed"
    );
    assert_eq!(
        safe_reply["result"]["structuredContent"]["observed_state"]["focused_role"],
        "button"
    );
    assert_eq!(
        safe_reply["result"]["structuredContent"]["observed_state"]["scroll_y"],
        120
    );
    assert_eq!(
        select_reply["result"]["structuredContent"]["code"],
        "unsupported_capability"
    );
    assert_eq!(
        elem_shot_reply["result"]["structuredContent"]["code"],
        "unsupported_capability"
    );
    client.finish();
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "End-to-end observation scenario tests screenshot, duplicates, Inspector decoder redaction, AX fallback, and timeout"
)]
fn screenshot_snapshot_fallback_inspector_events_and_duplicates_timeout() {
    // Arrange
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::with_agent(
        StubHostBackend::new("0.1.0"),
        FakeAgentBackend::fork_standalone(),
    );
    let mut client = DuplexClient::start(1, Arc::clone(&port), "0.1.0");
    client.send_tool_call("agent_context_open", &json!({}));
    drain(&port, &mut controller);
    let ctx_id = client.recv_raw()["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("ctx_id");
    let ctx = AgentContextId::new(ctx_id);
    controller.on_agent_message(
        ctx,
        r#"{"id":1,"result":{"contexts":[{"handle":"page-1"}]}}"#,
    );

    // Act 1: Viewport screenshot + duplicate reply ignored + Inspector events redacted.
    client.send_tool_call(
        "agent_screenshot",
        &json!({ "context_id": ctx_id, "target": { "kind": "viewport" }, "max_bytes": 4096 }),
    );
    drain(&port, &mut controller);
    let shot_cmd_id = controller
        .agent()
        .backend()
        .dispatched
        .iter()
        .rev()
        .find_map(|(_, id, cmd)| {
            matches!(cmd, AutomationWireCommand::TakeScreenshot { .. }).then_some(*id)
        })
        .expect("screenshot dispatched");
    let shot_frame = format!(r#"{{"id":{shot_cmd_id},"result":{{"data":"{VALID_1X1_PNG_B64}"}}}}"#);
    controller.on_agent_message(ctx, &shot_frame);
    // Duplicate reply with the same ID must be safely ignored.
    controller.on_agent_message(ctx, &shot_frame);
    let shot_reply = client.recv_raw();

    // Feed Automation.inspectorMessage frames through shared ObservationDecoder.
    controller.on_agent_message(
        ctx,
        r#"{"method":"Automation.inspectorMessage","params":{"browsingContextHandle":"page-1","message":"{\"method\":\"Console.messageAdded\",\"params\":{\"message\":{\"source\":\"console-api\",\"level\":\"log\",\"text\":\"agent hello\"}}}"}}"#,
    );
    controller.on_agent_message(
        ctx,
        r#"{"method":"Automation.inspectorMessage","params":{"browsingContextHandle":"page-1","message":"{\"method\":\"Network.requestWillBeSent\",\"params\":{\"requestId\":\"r1\",\"request\":{\"url\":\"https://alice:pw@api.example.com/data?secret=1\",\"method\":\"post\"}}}"}}"#,
    );
    client.send_tool_call(
        "page_events",
        &json!({
            "target": { "kind": "agent", "context_id": ctx_id },
            "cursor": 0,
            "limit": 10
        }),
    );
    drain(&port, &mut controller);
    let events_reply = client.recv_raw();

    // Act 2: Accessibility snapshot on Agent reports SemanticDomFallback honestly.
    client.send_tool_call(
        "page_snapshot",
        &json!({
            "target": { "kind": "agent", "context_id": ctx_id },
            "kind": "accessibility"
        }),
    );
    drain(&port, &mut controller);
    let snap_eval_id = controller
        .agent()
        .backend()
        .started_scripts
        .last()
        .expect("snap eval")
        .0;
    controller.on_agent_script_completed(
        snap_eval_id,
        Ok(r#"{"data":"main\n  button \"Go\"","truncated":false}"#.to_owned()),
    );
    let snap_reply = client.recv_raw();

    // Act 3: Deadline timer fires for a stalled screenshot command -> TimedOut.
    client.send_tool_call("agent_screenshot", &json!({ "context_id": ctx_id }));
    drain(&port, &mut controller);
    let timer_id = controller
        .agent()
        .backend()
        .scheduled_timers
        .last()
        .expect("deadline timer")
        .0;
    controller.on_agent_timer_fired(timer_id);
    let timeout_reply = client.recv_raw();

    // Assert
    assert_eq!(shot_reply["result"]["isError"], false);
    assert_eq!(shot_reply["result"]["structuredContent"]["width"], 1);
    assert_eq!(shot_reply["result"]["structuredContent"]["height"], 1);
    assert_eq!(shot_reply["result"]["content"][0]["mimeType"], "image/png");

    let events = events_reply["result"]["structuredContent"]["events"]
        .as_array()
        .expect("events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["text"], "agent hello");
    assert_eq!(events[1]["endpoint"]["origin"], "https://api.example.com");
    assert!(!events_reply.to_string().contains("alice"));
    assert!(!events_reply.to_string().contains("secret=1"));

    assert_eq!(
        snap_reply["result"]["structuredContent"]["accessibility_source"],
        "semantic_dom_fallback"
    );
    assert_eq!(
        timeout_reply["result"]["structuredContent"]["code"],
        "timed_out"
    );
    client.finish();
}

#[test]
fn stock_webkit_reports_agent_unavailable_truthfully() {
    // Arrange
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::with_agent(
        StubHostBackend::new("0.1.0"),
        FakeAgentBackend::stock_unavailable(),
    );
    let mut client = DuplexClient::start(1, Arc::clone(&port), "0.1.0");

    // Act
    client.send_tool_call("browser_status", &json!({}));
    drain(&port, &mut controller);
    let status = client.recv_raw();

    client.send_tool_call("agent_context_open", &json!({}));
    drain(&port, &mut controller);
    let open_err = client.recv_raw();

    // Assert
    assert_eq!(
        status["result"]["structuredContent"]["fork_capabilities"]["agent_automation"],
        false
    );
    assert_eq!(
        open_err["result"]["structuredContent"]["code"],
        "tool_not_available"
    );
    client.finish();
}

#[test]
fn protocol_scripts_and_command_queue_bounds_are_enforced() {
    // Arrange
    let locate_css = build_locate_script(
        &AgentLocator::Css {
            selector: "button.primary".to_owned(),
        },
        5,
        "c1-e2",
    )
    .expect("css script");
    let resolve_js = build_resolve_element_script("el-c1-e2-1", "c1-e2", true).expect("resolve");
    let a11y_js = build_accessibility_fallback_script(&SnapshotRequest::with_max_bounds(
        SnapshotKind::Accessibility,
        None,
    ))
    .expect("a11y fallback");

    // Act & Assert
    assert!(locate_css.contains("button.primary"));
    assert!(resolve_js.contains("el-c1-e2-1"));
    assert!(a11y_js.contains("inferRole"));

    let parsed_loc = parse_locate_reply(
        r#"{"elements":[{"element_id":"el-c1-e2-1","role":"button","name":"Run","tag":"button","bounds":{"x":1,"y":2,"width":50,"height":20}}],"truncated":false}"#,
        AgentContextId::new(1),
        2,
        "https://example.com",
        AgentLocatorKind::Css,
        5,
    )
    .expect("parsed locate");
    assert_eq!(parsed_loc.elements.len(), 1);
    assert_eq!(
        parsed_loc.elements[0].provenance.origin,
        "https://example.com"
    );

    let stale_err = parse_resolve_element_reply(r#"{"error":"stale_element"}"#).expect_err("stale");
    assert_eq!(stale_err.code(), ToolErrorCode::StaleDocument);

    let png_err =
        decode_screenshot_reply(&json!({ "data": VALID_1X1_PNG_B64 }), 10).expect_err("too big");
    assert_eq!(png_err.code(), ToolErrorCode::BoundsExceeded);

    assert_eq!(
        parse_inbound_frame(r#"{"id":7,"error":{"message":"Timeout; expired"}}"#),
        InboundFrame::Reply {
            id: 7,
            outcome: Err(ToolError::new(
                ToolErrorCode::TimedOut,
                "automation command timed out"
            ))
        }
    );

    // Bounded in-flight command queue returns Busy past MAX_IN_FLIGHT_COMMANDS.
    let port = Arc::new(DispatchPort::new(CountingWakeup::new()));
    let mut controller = McpController::with_agent(
        StubHostBackend::new("0.1.0"),
        FakeAgentBackend::fork_standalone(),
    );
    let mut client = DuplexClient::start(1, Arc::clone(&port), "0.1.0");
    client.send_tool_call("agent_context_open", &json!({}));
    drain(&port, &mut controller);
    let ctx_id = client.recv_raw()["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("ctx_id");
    controller.on_agent_message(
        AgentContextId::new(ctx_id),
        r#"{"id":1,"result":{"contexts":[{"handle":"page-1"}]}}"#,
    );
    for _ in 0..MAX_IN_FLIGHT_COMMANDS {
        client.send_tool_call("agent_screenshot", &json!({ "context_id": ctx_id }));
    }
    drain(&port, &mut controller);
    client.send_tool_call("agent_screenshot", &json!({ "context_id": ctx_id }));
    drain(&port, &mut controller);
    let busy_reply = client.recv_raw();
    assert_eq!(busy_reply["result"]["structuredContent"]["code"], "busy");
    let _ = WireKeyStep::KeyPress("Enter");
    controller.shutdown();
    client.finish();
}
