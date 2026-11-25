//! Independent MCP protocol integration tests for [`BrowserHandler`] over real `serve_stdio`
//! and `serve_socket` transports.

#![expect(
    unsafe_code,
    reason = "OS pipe creation via libc::pipe for real serve_stdio integration tests"
)]
#![expect(
    clippy::expect_used,
    reason = "integration tests panic on unexpected setup or I/O failure"
)]

mod agent_tools;
mod fake_host;

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Arc;
use std::time::{Duration, Instant};

use quickie_application::mcp::{
    AttachmentId, ClientId, ConsoleLevel, DiagnosticCapabilities, DiagnosticEventData,
    LifecycleEventKind, MAX_EVENTS_PER_ATTACHMENT, MAX_SNAPSHOT_NODES, NetworkEvent, NetworkPhase,
};
use quickie_mcp::{
    BrowserHandler, BrowserHandlerConfig, BrowserHandlerFactory, HostDispatchError, IsolatedStdio,
    ServerHandle, serve_stdio,
};
use serde_json::{Value, json};
use url::Url;

use self::fake_host::{
    ConsentDecision, FakeBrowserHost, default_scope_test_capabilities, sample_binding,
};

struct StdioSession {
    writer: Option<File>,
    reader: BufReader<File>,
    handle: Option<ServerHandle>,
    next_id: u64,
}

impl StdioSession {
    fn start(handler: BrowserHandler<FakeBrowserHost>) -> Self {
        let (server_stdin_rd, client_stdin_wr) = make_os_pipe();
        let (client_stdout_rd, server_stdout_wr) = make_os_pipe();
        let isolated = IsolatedStdio::from_fds(server_stdin_rd, server_stdout_wr);
        let handle = serve_stdio(isolated, handler).expect("start serve_stdio");

        Self {
            writer: Some(File::from(client_stdin_wr)),
            reader: BufReader::new(File::from(client_stdout_rd)),
            handle: Some(handle),
            next_id: 1,
        }
    }

    fn send_raw(&mut self, msg: &Value) {
        let writer = self.writer.as_mut().expect("session writer already closed");
        let line = format!("{msg}\n");
        writer.write_all(line.as_bytes()).expect("write frame");
        writer.flush().expect("flush frame");
    }

    fn read_response(&mut self) -> Value {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).expect("read response");
        assert!(n > 0, "unexpected EOF reading JSON-RPC response");
        serde_json::from_str(&line).expect("parse JSON-RPC response")
    }

    fn initialize(&mut self) -> Value {
        let id = self.alloc_id();
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {
                    "name": "quickie-browser-handler-test",
                    "version": "0.1.0"
                }
            }
        }));
        let init_resp = self.read_response();
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));
        init_resp
    }

    fn list_tools(&mut self) -> Value {
        let id = self.alloc_id();
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
            "params": {}
        }));
        self.read_response()
    }

    fn send_call_tool(&mut self, name: &str, arguments: &Value) -> u64 {
        let id = self.alloc_id();
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        }));
        id
    }

    fn call_tool(&mut self, name: &str, arguments: &Value) -> Value {
        let _ = self.send_call_tool(name, arguments);
        self.read_response()
    }

    fn send_cancel(&mut self, request_id: u64) {
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {
                "requestId": request_id,
                "reason": "test cancelled"
            }
        }));
    }

    fn close_stdin_and_wait(mut self) {
        drop(self.writer.take());
        if let Some(handle) = self.handle.take() {
            handle.wait();
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

impl Drop for StdioSession {
    fn drop(&mut self) {
        drop(self.writer.take());
        if let Some(handle) = self.handle.take() {
            handle.shutdown();
        }
    }
}

fn make_os_pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0_i32; 2];
    // SAFETY: `fds` is a valid 2-element array for `pipe`.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(rc, 0, "libc::pipe failed");
    // SAFETY: `fds[0]` is a newly created, exclusively owned read descriptor.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: `fds[1]` is a newly created, exclusively owned write descriptor.
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    (read_fd, write_fd)
}

#[test]
fn stdio_initialize_discovers_only_implemented_first_wave_tools_and_reports_status() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(1), Arc::clone(&host)));

    // Act
    let init = session.initialize();
    let list = session.list_tools();
    let status = session.call_tool("browser_status", &json!({}));

    // Assert
    assert_eq!(init["result"]["serverInfo"]["name"], "quickie");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    let tool_names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert_eq!(
        tool_names,
        vec![
            "browser_status",
            "diagnostic_request",
            "diagnostic_detach",
            "page_snapshot",
            "page_events",
            "agent_context_open",
            "agent_context_list",
            "agent_context_close",
            "agent_navigate",
            "agent_locate",
            "agent_act",
            "agent_wait",
            "agent_screenshot",
        ]
    );

    assert_eq!(status["result"]["isError"], false);
    let body = &status["result"]["structuredContent"];
    assert_eq!(body["platform"], "macos");
    assert_eq!(body["runtime_mode"], "gui");
    assert_eq!(body["product_version"], "0.1.0-test");
    assert_eq!(body["fork_capabilities"]["native_accessibility"], true);
    assert_eq!(body["fork_capabilities"]["agent_automation"], false);
    assert_eq!(
        body["supported_operations"],
        json!([
            "browser_status",
            "diagnostic_request",
            "diagnostic_detach",
            "page_snapshot",
            "page_events"
        ])
    );

    session.close_stdin_and_wait();
}

#[test]
fn diagnostic_request_denied_exposes_no_tab_metadata_and_approved_roundtrips_snapshot_and_detach() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    host.set_consent(ConsentDecision::Deny);
    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(1), Arc::clone(&host)));
    let _ = session.initialize();

    // Act (denied)
    let denied = session.call_tool(
        "diagnostic_request",
        &json!({ "scope": "same_origin_reload" }),
    );

    // Assert (denied)
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(
        denied["result"]["structuredContent"]["code"],
        "approval_denied"
    );
    assert!(
        denied["result"]["structuredContent"]
            .get("origin")
            .is_none()
    );
    assert!(denied["result"]["structuredContent"].get("title").is_none());
    assert_eq!(host.active_attachments(), 0);

    // Arrange (approved)
    let binding = sample_binding(7, "https://app.example/private/path?secret=1", 4);
    host.set_consent(ConsentDecision::Approve {
        binding,
        capabilities: default_scope_test_capabilities(),
    });

    // Act (approved -> snapshot -> detach)
    let granted = session.call_tool(
        "diagnostic_request",
        &json!({ "scope": "same_origin_reload" }),
    );
    let attachment_id = granted["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");
    let snapshot = session.call_tool(
        "page_snapshot",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": attachment_id },
            "kind": "accessibility",
            "selector": "main",
            "max_nodes": 120,
            "max_depth": 8,
            "max_bytes": 4096
        }),
    );
    let detached = session.call_tool(
        "diagnostic_detach",
        &json!({ "attachment_id": attachment_id }),
    );

    // Assert (approved -> snapshot -> detach)
    assert_eq!(granted["result"]["isError"], false);
    assert_eq!(
        granted["result"]["structuredContent"]["scope"],
        "same_origin_reload"
    );
    assert_eq!(granted["result"]["structuredContent"]["epoch"], 4);
    assert_eq!(
        granted["result"]["structuredContent"]["origin"],
        "https://app.example"
    );

    assert_eq!(snapshot["result"]["isError"], false);
    assert_eq!(snapshot["result"]["structuredContent"]["epoch"], 4);
    assert_eq!(
        snapshot["result"]["structuredContent"]["kind"],
        "accessibility"
    );
    assert_eq!(
        snapshot["result"]["structuredContent"]["accessibility_source"],
        "native"
    );
    assert_eq!(
        snapshot["result"]["structuredContent"]["data"],
        "<main>synthetic-control</main>"
    );

    assert_eq!(detached["result"]["isError"], false);
    assert_eq!(detached["result"]["structuredContent"]["detached"], true);
    assert_eq!(
        detached["result"]["structuredContent"]["reason"],
        "client_detached"
    );
    assert_eq!(host.active_attachments(), 0);

    session.close_stdin_and_wait();
}

#[test]
fn invalid_inputs_wrong_target_kind_and_caller_supplied_client_id_are_rejected_before_host() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(1), Arc::clone(&host)));
    let _ = session.initialize();

    // Act
    let unknown_tool = session.call_tool("eval_js", &json!({ "code": "1+1" }));
    let forged_client_status = session.call_tool("browser_status", &json!({ "client_id": 999 }));
    let forged_client_request = session.call_tool(
        "diagnostic_request",
        &json!({ "scope": "single_document", "client_id": 999 }),
    );
    let bad_scope = session.call_tool("diagnostic_request", &json!({ "scope": "all_tabs" }));
    let wrong_target_field = session.call_tool(
        "page_snapshot",
        &json!({
            "target": { "kind": "diagnostic", "context_id": 1 },
            "kind": "dom"
        }),
    );
    let wrong_target_kind = session.call_tool(
        "page_snapshot",
        &json!({
            "target": { "kind": "tab", "attachment_id": 1 },
            "kind": "dom"
        }),
    );
    let over_node_bound = session.call_tool(
        "page_snapshot",
        &json!({
            "target": { "kind": "agent", "context_id": 1 },
            "kind": "dom",
            "max_nodes": MAX_SNAPSHOT_NODES + 1
        }),
    );
    let over_event_bound = session.call_tool(
        "page_events",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": 1 },
            "limit": MAX_EVENTS_PER_ATTACHMENT + 1
        }),
    );

    // Assert
    assert!(
        unknown_tool["error"]["code"].is_i64(),
        "unknown tool must return JSON-RPC protocol error: {unknown_tool}"
    );
    assert_eq!(
        forged_client_status["result"]["structuredContent"]["code"],
        "invalid_params"
    );
    assert_eq!(
        forged_client_request["result"]["structuredContent"]["code"],
        "invalid_params"
    );
    assert_eq!(
        bad_scope["result"]["structuredContent"]["code"],
        "invalid_params"
    );
    assert_eq!(
        wrong_target_field["result"]["structuredContent"]["code"],
        "invalid_params"
    );
    assert_eq!(
        wrong_target_kind["result"]["structuredContent"]["code"],
        "invalid_params"
    );
    assert_eq!(
        over_node_bound["result"]["structuredContent"]["code"],
        "bounds_exceeded"
    );
    assert_eq!(
        over_event_bound["result"]["structuredContent"]["code"],
        "bounds_exceeded"
    );
    assert!(
        host.dispatched_clients().is_empty(),
        "none of the invalid calls should reach BrowserHost"
    );

    session.close_stdin_and_wait();
}

fn seed_mixed_events(
    host: &FakeBrowserHost,
    attachment: AttachmentId,
    binding: &quickie_application::mcp::DocumentBinding,
) {
    let sensitive_url =
        Url::parse("https://alice:topsecret@service.example/api/v1/items?token=bearer123#frag")
            .expect("parse sensitive url");
    let pending_net =
        NetworkEvent::new(101, &sensitive_url, "post", None, NetworkPhase::Pending).expect("net");
    let completed_net = NetworkEvent::new(
        101,
        &sensitive_url,
        "post",
        Some(200),
        NetworkPhase::Completed,
    )
    .expect("net");

    host.record_event(
        attachment,
        binding,
        DiagnosticEventData::console(ConsoleLevel::Warn, "should-be-filtered-console"),
    );
    host.record_event(
        attachment,
        binding,
        DiagnosticEventData::Network(pending_net),
    );
    host.record_event(
        attachment,
        binding,
        DiagnosticEventData::runtime_error("TypeError: undefined is not a function"),
    );
    host.record_event(
        attachment,
        binding,
        DiagnosticEventData::Network(completed_net),
    );
    host.record_event(
        attachment,
        binding,
        DiagnosticEventData::lifecycle(LifecycleEventKind::Load),
    );
}

#[test]
fn page_events_redacts_network_metadata_filters_unset_capabilities_and_roundtrips_cursor() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    let binding = sample_binding(11, "https://service.example/dashboard", 2);
    let partial_caps = DiagnosticCapabilities {
        dom: true,
        accessibility: false,
        console: false,
        runtime_errors: true,
        pending_network: false,
        completed_network: true,
        lifecycle: true,
    };
    host.set_consent(ConsentDecision::Approve {
        binding: binding.clone(),
        capabilities: partial_caps,
    });
    host.set_native_overflow(true);

    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(1), Arc::clone(&host)));
    let _ = session.initialize();

    let grant = session.call_tool("diagnostic_request", &json!({ "scope": "single_document" }));
    let raw_attachment = grant["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_id");
    seed_mixed_events(&host, AttachmentId::from_wire(raw_attachment), &binding);

    // Act
    let first_page = session.call_tool(
        "page_events",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": raw_attachment },
            "cursor": 0,
            "limit": 4
        }),
    );
    let next_cursor = first_page["result"]["structuredContent"]["next_cursor"]
        .as_u64()
        .expect("next_cursor");
    let second_page = session.call_tool(
        "page_events",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": raw_attachment },
            "cursor": next_cursor,
            "limit": 4
        }),
    );

    // Assert
    assert_eq!(first_page["result"]["isError"], false);
    assert_eq!(first_page["result"]["structuredContent"]["has_more"], true);
    assert_eq!(
        first_page["result"]["structuredContent"]["native_overflow"],
        true
    );

    let first_events = first_page["result"]["structuredContent"]["events"]
        .as_array()
        .expect("events array");
    // Console (seq 1) and Pending network (seq 2) must be ignored due to unset capabilities.
    assert_eq!(first_events.len(), 2);
    assert_eq!(first_events[0]["category"], "runtime_error");
    assert_eq!(first_events[1]["category"], "network");
    assert_eq!(first_events[1]["phase"], "completed");
    assert_eq!(first_events[1]["method"], "POST");
    assert_eq!(first_events[1]["status"], 200);
    assert_eq!(
        first_events[1]["endpoint"]["origin"],
        "https://service.example"
    );
    assert_eq!(first_events[1]["endpoint"]["path"], "/api/v1/items");

    let serialized_first = first_page.to_string();
    assert!(!serialized_first.contains("topsecret"));
    assert!(!serialized_first.contains("bearer123"));
    assert!(!serialized_first.contains("frag"));
    assert!(!serialized_first.contains("should-be-filtered-console"));

    let second_events = second_page["result"]["structuredContent"]["events"]
        .as_array()
        .expect("second events");
    assert_eq!(second_events.len(), 1);
    assert_eq!(second_events[0]["category"], "lifecycle");
    assert_eq!(second_events[0]["kind"], "load");
    assert_eq!(
        second_page["result"]["structuredContent"]["has_more"],
        false
    );

    session.close_stdin_and_wait();
}

#[test]
fn cross_session_attachment_access_matches_unknown_and_closing_one_session_keeps_other_alive() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    let factory = BrowserHandlerFactory::new(Arc::clone(&host));
    let binding = sample_binding(21, "https://owner.example/page", 1);
    host.set_consent(ConsentDecision::Approve {
        binding,
        capabilities: default_scope_test_capabilities(),
    });

    let mut session_a = StdioSession::start(factory.next_handler());
    let mut session_b = StdioSession::start(factory.next_handler());
    let _ = session_a.initialize();
    let _ = session_b.initialize();

    let grant_a = session_a.call_tool("diagnostic_request", &json!({}));
    let attachment_a = grant_a["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .expect("attachment_a");

    // Act
    let cross_owner = session_b.call_tool(
        "page_snapshot",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": attachment_a },
            "kind": "dom"
        }),
    );
    let unknown_id = session_b.call_tool(
        "page_snapshot",
        &json!({
            "target": { "kind": "diagnostic", "attachment_id": 999_999 },
            "kind": "dom"
        }),
    );

    session_a.close_stdin_and_wait();
    host.wait_for_client_closed(ClientId::new(1), Duration::from_secs(2));
    let still_alive_b = session_b.call_tool("browser_status", &json!({}));

    // Assert
    assert_eq!(cross_owner["result"]["isError"], true);
    assert_eq!(unknown_id["result"]["isError"], true);
    assert_eq!(
        cross_owner["result"]["structuredContent"],
        unknown_id["result"]["structuredContent"]
    );
    assert_eq!(
        cross_owner["result"]["structuredContent"]["code"],
        "attachment_unavailable"
    );
    assert_eq!(still_alive_b["result"]["isError"], false);
    assert_eq!(host.closed_clients(), vec![ClientId::new(1)]);
    assert_eq!(host.active_attachments(), 0);

    session_b.close_stdin_and_wait();
    host.wait_for_client_closed(ClientId::new(2), Duration::from_secs(2));
    assert_eq!(
        host.closed_clients(),
        vec![ClientId::new(1), ClientId::new(2)]
    );
}

#[test]
fn busy_host_closed_host_and_dropped_reply_return_neutral_structured_errors() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(1), Arc::clone(&host)));
    let _ = session.initialize();

    // Act
    host.set_dispatch_error(Some(HostDispatchError::Full));
    let full = session.call_tool("browser_status", &json!({}));

    host.set_dispatch_error(Some(HostDispatchError::Busy));
    let busy = session.call_tool("browser_status", &json!({}));

    host.set_dispatch_error(Some(HostDispatchError::Closed));
    let closed = session.call_tool("browser_status", &json!({}));

    host.set_dispatch_error(None);
    host.set_drop_next_status(true);
    let dropped = session.call_tool("browser_status", &json!({}));

    // Assert
    assert_eq!(full["result"]["isError"], true);
    assert_eq!(full["result"]["structuredContent"]["code"], "busy");
    assert_eq!(busy["result"]["structuredContent"]["code"], "busy");
    assert_eq!(closed["result"]["structuredContent"]["code"], "unavailable");
    assert_eq!(
        dropped["result"]["structuredContent"]["code"],
        "unavailable"
    );

    session.close_stdin_and_wait();
}

#[test]
fn cancellation_timeout_and_in_flight_cap_ignore_late_replies_without_panicking() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    let config = BrowserHandlerConfig {
        request_timeout: Duration::from_millis(80),
        max_in_flight: 1,
    };
    let mut session = StdioSession::start(BrowserHandler::with_config(
        ClientId::new(1),
        Arc::clone(&host),
        config,
    ));
    let _ = session.initialize();

    // Act (in-flight cap + client cancellation)
    host.set_park_next_status(true);
    let req_id = session.send_call_tool("browser_status", &json!({}));
    let parked = host.wait_for_parked(Duration::from_secs(2));

    let flood_resp = session.call_tool("browser_status", &json!({}));
    session.send_cancel(req_id);

    // Wait briefly for cancellation to drop the oneshot receiver, then complete late reply.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !parked.is_closed() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        parked.is_closed(),
        "cancelled request must drop its reply receiver"
    );
    parked.resolve_late();

    // Act (timeout + late reply)
    host.set_park_next_status(true);
    let timeout_resp = session.call_tool("browser_status", &json!({}));
    let timed_out_parked = host.wait_for_parked(Duration::from_secs(2));
    assert!(timed_out_parked.is_closed());
    timed_out_parked.resolve_late();

    // Subsequent normal request succeeds on the same session.
    let healthy = session.call_tool("browser_status", &json!({}));

    // Assert
    assert_eq!(flood_resp["result"]["isError"], true);
    assert_eq!(flood_resp["result"]["structuredContent"]["code"], "busy");
    assert_eq!(timeout_resp["result"]["isError"], true);
    assert_eq!(
        timeout_resp["result"]["structuredContent"]["code"],
        "timed_out"
    );
    assert_eq!(healthy["result"]["isError"], false);
    assert_eq!(healthy["result"]["structuredContent"]["platform"], "macos");

    session.close_stdin_and_wait();
}

#[test]
fn dropped_client_during_pending_request_notifies_host_once_and_ignores_late_reply() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(7), Arc::clone(&host)));
    let _ = session.initialize();

    host.set_park_next_status(true);
    let _ = session.send_call_tool("browser_status", &json!({}));
    let parked = host.wait_for_parked(Duration::from_secs(2));

    // Act: close stdin while the request is still pending in the host.
    let start = Instant::now();
    session.close_stdin_and_wait();
    let elapsed = start.elapsed();

    // Complete the parked reply after the client has disconnected.
    assert!(parked.is_closed());
    parked.resolve_late();

    // Assert
    assert!(
        elapsed < Duration::from_secs(2),
        "serve_stdio must exit promptly when client drops during pending call ({elapsed:?})"
    );
    assert_eq!(host.closed_clients(), vec![ClientId::new(7)]);
}
