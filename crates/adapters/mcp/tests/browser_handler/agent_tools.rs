//! Isolated `Profile::Agent` MCP protocol tests over `BrowserHandler` and `serve_stdio`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use quickie_application::mcp::ClientId;
use quickie_mcp::browser::{
    MAX_ACTION_TIMEOUT_MS, MAX_INPUT_TEXT_BYTES, MAX_LOCATE_RESULTS, MAX_SCREENSHOT_BYTES,
    MAX_SCROLL_DELTA, MAX_VIEWPORT_WIDTH,
};
use quickie_mcp::{BrowserHandler, BrowserHandlerConfig, BrowserHandlerFactory};
use serde_json::{Value, json};

use super::StdioSession;
use super::fake_host::{FAKE_AGENT_APPROVED_ORIGIN, FakeBrowserHost, MINIMAL_VALID_PNG};

fn open_navigated_fixture_context(session: &mut StdioSession) -> (u64, u64, Value, Value) {
    let opened = session.call_tool(
        "agent_context_open",
        &json!({ "viewport": { "width": 640, "height": 480 } }),
    );
    assert_eq!(opened["result"]["isError"], false);
    let ctx_id = opened["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("context_id");
    assert_eq!(opened["result"]["structuredContent"]["epoch"], 1);
    assert_eq!(
        opened["result"]["structuredContent"]["viewport"],
        json!({ "width": 640, "height": 480 })
    );

    let nav_ok = session.call_tool(
        "agent_navigate",
        &json!({
            "context_id": ctx_id,
            "url": format!("{FAKE_AGENT_APPROVED_ORIGIN}/control"),
            "wait_stage": "load",
            "timeout_ms": 5000
        }),
    );
    assert_eq!(nav_ok["result"]["isError"], false);
    assert_eq!(nav_ok["result"]["structuredContent"]["status"], "committed");
    assert_eq!(
        nav_ok["result"]["structuredContent"]["origin"],
        FAKE_AGENT_APPROVED_ORIGIN
    );
    let epoch = nav_ok["result"]["structuredContent"]["epoch"]
        .as_u64()
        .expect("epoch");
    assert_eq!(epoch, 2);

    let located = session.call_tool(
        "agent_locate",
        &json!({
            "context_id": ctx_id,
            "locator": { "kind": "role", "role": "button", "name": "Submit", "exact": true },
            "max_results": 2
        }),
    );
    assert_eq!(located["result"]["isError"], false);
    let elements = located["result"]["structuredContent"]["elements"]
        .as_array()
        .expect("elements");
    assert_eq!(elements.len(), 2);
    assert_eq!(
        elements[0]["provenance"],
        json!({
            "origin": FAKE_AGENT_APPROVED_ORIGIN,
            "frame": "main_frame",
            "locator_kind": "role"
        })
    );
    assert_eq!(elements[0]["bounds"]["width"], 240.0);
    (
        ctx_id,
        epoch,
        elements[0]["element_ref"].clone(),
        elements[1]["element_ref"].clone(),
    )
}

#[test]
fn false_agent_capability_omits_action_tools_from_status_and_returns_tool_not_available() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    host.set_agent_automation(false);
    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(1), Arc::clone(&host)));
    let _ = session.initialize();

    // Act
    let status = session.call_tool("browser_status", &json!({}));
    let open = session.call_tool("agent_context_open", &json!({}));
    let list = session.call_tool("agent_context_list", &json!({}));

    // Assert
    assert_eq!(status["result"]["isError"], false);
    assert_eq!(
        status["result"]["structuredContent"]["fork_capabilities"]["agent_automation"],
        false
    );
    assert_eq!(
        status["result"]["structuredContent"]["supported_operations"],
        json!([
            "browser_status",
            "diagnostic_request",
            "diagnostic_detach",
            "page_snapshot",
            "page_events"
        ])
    );
    assert_eq!(open["result"]["isError"], true);
    assert_eq!(
        open["result"]["structuredContent"]["code"],
        "tool_not_available"
    );
    assert_eq!(list["result"]["isError"], true);
    assert_eq!(
        list["result"]["structuredContent"]["code"],
        "tool_not_available"
    );

    session.close_stdin_and_wait();
}

#[test]
fn agent_tools_reject_forged_client_diagnostic_target_raw_commands_and_invalid_urls_before_host() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    host.set_agent_automation(true);
    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(1), Arc::clone(&host)));
    let _ = session.initialize();

    // Act
    let cases = [
        session.call_tool("agent_context_open", &json!({ "client_id": 999 })),
        session.call_tool("agent_context_close", &json!({ "attachment_id": 1 })),
        session.call_tool(
            "agent_act",
            &json!({
                "target": { "kind": "diagnostic", "attachment_id": 1 },
                "action": { "kind": "scroll", "delta_x": 0, "delta_y": 100 }
            }),
        ),
        session.call_tool(
            "agent_navigate",
            &json!({
                "context_id": 1,
                "url": "http://127.0.0.1:4173/fixture",
                "approved": true
            }),
        ),
        session.call_tool(
            "agent_act",
            &json!({
                "context_id": 1,
                "action": { "kind": "eval_js", "script": "document.cookie" }
            }),
        ),
        session.call_tool(
            "agent_navigate",
            &json!({ "context_id": 1, "url": "file:///etc/hosts" }),
        ),
        session.call_tool(
            "agent_navigate",
            &json!({ "context_id": 1, "url": "http://user:pass@127.0.0.1:4173/fixture" }),
        ),
        session.call_tool(
            "agent_act",
            &json!({
                "context_id": 1,
                "action": {
                    "kind": "click",
                    "element": { "context_id": 2, "epoch": 1, "element_id": "el-input" }
                }
            }),
        ),
        session.call_tool(
            "agent_act",
            &json!({
                "context_id": 1,
                "action": {
                    "kind": "click",
                    "element": { "context_id": 1, "element_id": "el-input" }
                }
            }),
        ),
    ];

    // Assert
    for resp in &cases {
        assert_eq!(resp["result"]["isError"], true, "expected error: {resp}");
        assert_eq!(
            resp["result"]["structuredContent"]["code"], "invalid_params",
            "expected invalid_params: {resp}"
        );
    }
    assert!(
        host.dispatched_clients().is_empty(),
        "none of the invalid agent calls should reach BrowserHost"
    );

    session.close_stdin_and_wait();
}

#[test]
fn agent_tools_enforce_viewport_timeout_locator_text_scroll_and_screenshot_bounds_before_host() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    host.set_agent_automation(true);
    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(1), Arc::clone(&host)));
    let _ = session.initialize();

    // Act
    let cases = [
        session.call_tool(
            "agent_context_open",
            &json!({ "viewport": { "width": MAX_VIEWPORT_WIDTH + 1, "height": 480 } }),
        ),
        session.call_tool(
            "agent_wait",
            &json!({
                "context_id": 1,
                "condition": { "kind": "load_stage", "stage": "load" },
                "timeout_ms": MAX_ACTION_TIMEOUT_MS + 1
            }),
        ),
        session.call_tool(
            "agent_locate",
            &json!({
                "context_id": 1,
                "locator": { "kind": "css", "selector": "button" },
                "max_results": MAX_LOCATE_RESULTS + 1
            }),
        ),
        session.call_tool(
            "agent_act",
            &json!({
                "context_id": 1,
                "action": { "kind": "type", "text": "x".repeat(MAX_INPUT_TEXT_BYTES + 1) }
            }),
        ),
        session.call_tool(
            "agent_act",
            &json!({
                "context_id": 1,
                "action": { "kind": "scroll", "delta_x": 0, "delta_y": MAX_SCROLL_DELTA + 1 }
            }),
        ),
        session.call_tool(
            "agent_screenshot",
            &json!({ "context_id": 1, "max_bytes": MAX_SCREENSHOT_BYTES + 1 }),
        ),
    ];

    // Assert
    for resp in &cases {
        assert_eq!(resp["result"]["isError"], true, "expected error: {resp}");
        assert_eq!(
            resp["result"]["structuredContent"]["code"], "bounds_exceeded",
            "expected bounds_exceeded: {resp}"
        );
    }
    assert!(
        host.dispatched_clients().is_empty(),
        "out-of-bounds agent requests must not reach BrowserHost"
    );

    session.close_stdin_and_wait();
}

fn assert_agent_actions(
    host: &FakeBrowserHost,
    session: &mut StdioSession,
    ctx_id: u64,
    epoch: u64,
    input_ref: &Value,
    submit_ref: &Value,
) {
    let fill = session.call_tool(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": { "kind": "fill", "element": input_ref, "text": "hello" }
        }),
    );
    assert_eq!(fill["result"]["structuredContent"]["status"], "performed");
    assert_eq!(
        fill["result"]["structuredContent"]["observed_state"]["value_length"],
        5
    );

    let scroll = session.call_tool(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": { "kind": "scroll", "delta_x": 0, "delta_y": 120 }
        }),
    );
    assert_eq!(
        scroll["result"]["structuredContent"]["observed_state"]["scroll_y"],
        120
    );

    let press_key = session.call_tool(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": { "kind": "press_key", "key": "Enter", "modifiers": ["shift"] }
        }),
    );
    assert_eq!(
        press_key["result"]["structuredContent"]["status"],
        "performed"
    );

    let submit_approval = session.call_tool(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": { "kind": "click", "element": submit_ref }
        }),
    );
    assert_eq!(submit_approval["result"]["isError"], false);
    assert_eq!(
        submit_approval["result"]["structuredContent"]["status"],
        "approval_required"
    );

    let stale_click = session.call_tool(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": {
                "kind": "click",
                "element": { "context_id": ctx_id, "epoch": epoch - 1, "element_id": "el-input" }
            }
        }),
    );
    assert_eq!(
        stale_click["result"]["structuredContent"]["code"],
        "stale_document"
    );

    host.set_unsupported_select_option(true);
    let unsupported_select = session.call_tool(
        "agent_act",
        &json!({
            "context_id": ctx_id,
            "action": { "kind": "select_option", "element": input_ref, "value": "opt-1" }
        }),
    );
    assert_eq!(
        unsupported_select["result"]["structuredContent"]["code"],
        "unsupported_capability"
    );
}

#[test]
fn isolated_agent_lifecycle_roundtrips_navigation_locate_actions_approval_and_isolation() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    host.set_agent_automation(true);
    let factory = BrowserHandlerFactory::new(Arc::clone(&host));
    let mut session_a = StdioSession::start(factory.next_handler());
    let mut session_b = StdioSession::start(factory.next_handler());
    let _ = session_a.initialize();
    let _ = session_b.initialize();

    // Act & Assert
    let status = session_a.call_tool("browser_status", &json!({}));
    assert_eq!(
        status["result"]["structuredContent"]["fork_capabilities"]["agent_automation"],
        true
    );
    assert_eq!(
        status["result"]["structuredContent"]["supported_operations"]
            .as_array()
            .expect("supported_operations array")
            .len(),
        13
    );

    let (ctx_id, epoch, input_ref, submit_ref) = open_navigated_fixture_context(&mut session_a);
    assert_eq!(
        session_a.call_tool("agent_context_list", &json!({}))["result"]["structuredContent"]
            ["contexts"]
            .as_array()
            .expect("contexts")
            .len(),
        1
    );
    assert_eq!(
        session_b.call_tool("agent_context_list", &json!({}))["result"]["structuredContent"]["contexts"],
        json!([])
    );

    let nav_approval = session_a.call_tool(
        "agent_navigate",
        &json!({ "context_id": ctx_id, "url": "https://other.example/checkout" }),
    );
    assert_eq!(nav_approval["result"]["isError"], false);
    assert_eq!(
        nav_approval["result"]["structuredContent"]["status"],
        "approval_required"
    );

    assert_agent_actions(
        &host,
        &mut session_a,
        ctx_id,
        epoch,
        &input_ref,
        &submit_ref,
    );

    let cross_close = session_b.call_tool("agent_context_close", &json!({ "context_id": ctx_id }));
    assert_eq!(
        cross_close["result"]["structuredContent"]["code"],
        "attachment_unavailable"
    );

    let closed = session_a.call_tool("agent_context_close", &json!({ "context_id": ctx_id }));
    assert_eq!(closed["result"]["structuredContent"]["closed"], true);
    assert_eq!(host.active_agent_contexts(), 0);

    session_a.close_stdin_and_wait();
    session_b.close_stdin_and_wait();
}

#[test]
fn isolated_agent_wait_png_screenshot_and_read_only_observation_targets_work() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    host.set_agent_automation(true);
    let mut session = StdioSession::start(BrowserHandler::new(ClientId::new(1), Arc::clone(&host)));
    let _ = session.initialize();
    let (ctx_id, epoch, _input_ref, _submit_ref) = open_navigated_fixture_context(&mut session);

    // Act
    let wait_ok = session.call_tool(
        "agent_wait",
        &json!({
            "context_id": ctx_id,
            "condition": { "kind": "text_present", "text": "Submit" },
            "timeout_ms": 1000
        }),
    );
    let wait_timeout = session.call_tool(
        "agent_wait",
        &json!({
            "context_id": ctx_id,
            "condition": { "kind": "text_present", "text": "never-appears" },
            "timeout_ms": 100
        }),
    );
    let shot = session.call_tool(
        "agent_screenshot",
        &json!({
            "context_id": ctx_id,
            "target": { "kind": "viewport" },
            "max_bytes": 4096
        }),
    );
    let tight_shot = session.call_tool(
        "agent_screenshot",
        &json!({ "context_id": ctx_id, "max_bytes": 16 }),
    );
    host.set_screenshot_override(Some(b"not-a-png".to_vec()));
    let bad_png = session.call_tool("agent_screenshot", &json!({ "context_id": ctx_id }));
    host.set_screenshot_override(None);

    let snap = session.call_tool(
        "page_snapshot",
        &json!({
            "target": { "kind": "agent", "context_id": ctx_id },
            "kind": "accessibility"
        }),
    );
    let events = session.call_tool(
        "page_events",
        &json!({
            "target": { "kind": "agent", "context_id": ctx_id },
            "cursor": 0,
            "limit": 10
        }),
    );

    // Assert
    assert_eq!(wait_ok["result"]["structuredContent"]["matched"], true);
    assert_eq!(
        wait_timeout["result"]["structuredContent"]["timed_out"],
        true
    );
    assert_eq!(shot["result"]["isError"], false);
    assert_eq!(shot["result"]["content"][0]["type"], "image");
    assert_eq!(shot["result"]["content"][0]["mimeType"], "image/png");
    assert!(
        shot["result"]["content"][0]["data"]
            .as_str()
            .expect("base64 data")
            .starts_with("iVBORw0KGgo")
    );
    assert_eq!(
        shot["result"]["structuredContent"]["byte_len"],
        MINIMAL_VALID_PNG.len()
    );
    assert_eq!(shot["result"]["structuredContent"]["width"], 640);
    assert_eq!(shot["result"]["structuredContent"]["height"], 480);
    assert_eq!(
        tight_shot["result"]["structuredContent"]["code"],
        "bounds_exceeded"
    );
    assert_eq!(
        bad_png["result"]["structuredContent"]["code"],
        "unavailable"
    );
    assert_eq!(snap["result"]["isError"], false);
    assert_eq!(snap["result"]["structuredContent"]["epoch"], epoch);
    assert_eq!(events["result"]["isError"], false);
    assert_eq!(
        events["result"]["structuredContent"]["events"]
            .as_array()
            .expect("events")
            .len(),
        1
    );

    session.close_stdin_and_wait();
}

#[test]
fn agent_wait_cancellation_and_client_disconnect_clean_up_isolated_contexts() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    host.set_agent_automation(true);
    let config = BrowserHandlerConfig {
        request_timeout: Duration::from_secs(5),
        max_in_flight: 2,
    };
    let mut session = StdioSession::start(BrowserHandler::with_config(
        ClientId::new(42),
        Arc::clone(&host),
        config,
    ));
    let _ = session.initialize();

    let opened = session.call_tool("agent_context_open", &json!({}));
    let ctx_id = opened["result"]["structuredContent"]["context_id"]
        .as_u64()
        .expect("context_id");
    assert_eq!(host.active_agent_contexts(), 1);

    // Act
    host.set_park_next_agent_wait(true);
    let req_id = session.send_call_tool(
        "agent_wait",
        &json!({
            "context_id": ctx_id,
            "condition": { "kind": "load_stage", "stage": "load" },
            "timeout_ms": 2000
        }),
    );
    let parked = host.wait_for_parked_agent_wait(Duration::from_secs(2));
    session.send_cancel(req_id);

    let deadline = Instant::now() + Duration::from_secs(2);
    while !parked.is_closed() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }

    // Assert
    assert!(
        parked.is_closed(),
        "cancelled agent_wait must drop its one-shot receiver"
    );
    parked.resolve_late();

    session.close_stdin_and_wait();
    host.wait_for_client_closed(ClientId::new(42), Duration::from_secs(2));
    assert_eq!(host.active_agent_contexts(), 0);
}

#[test]
fn initialize_reports_configured_product_version_without_host_roundtrip() {
    // Arrange
    let host = Arc::new(FakeBrowserHost::new());
    let factory = BrowserHandlerFactory::new(Arc::clone(&host)).with_product_version("0.4.2");
    let mut from_factory = StdioSession::start(factory.next_handler());
    let mut direct = StdioSession::start(
        BrowserHandler::new(ClientId::new(9), Arc::clone(&host)).with_product_version("0.5.0-rc1"),
    );

    // Act
    let init_factory = from_factory.initialize();
    let init_direct = direct.initialize();

    // Assert
    assert_eq!(init_factory["result"]["serverInfo"]["name"], "quickie");
    assert_eq!(init_factory["result"]["serverInfo"]["version"], "0.4.2");
    assert_eq!(init_direct["result"]["serverInfo"]["version"], "0.5.0-rc1");
    assert!(
        host.dispatched_clients().is_empty(),
        "initialize must not dispatch to BrowserHost"
    );

    from_factory.close_stdin_and_wait();
    direct.close_stdin_and_wait();
}
