use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;

use quickie_mcp::{BrowserHandlerFactory, IsolatedStdio, McpError, SocketEndpoint, serve_stdio};
use serde_json::{Value, json};

use crate::mcp::cli::{
    CliAction, McpCliOptions, StandaloneHost, parse_cli_args, resolve_endpoint,
    should_fallback_to_standalone,
};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn pipe_pair() -> (OwnedFd, OwnedFd) {
    let mut fds = [0_i32; 2];
    // SAFETY: `fds` is a valid two-element array for `pipe`.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(rc, 0, "pipe() failed");
    // SAFETY: `fds[0]` is a newly allocated valid read end from `pipe`.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: `fds[1]` is a newly allocated valid write end from `pipe`.
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    (read_fd, write_fd)
}

#[test]
fn cli_parses_mcp_subcommand_and_flags() {
    // Arrange & Act & Assert
    assert_eq!(parse_cli_args(&[]), CliAction::NotMcp);
    assert_eq!(
        parse_cli_args(&strings(&["https://example.com"])),
        CliAction::NotMcp
    );
    assert_eq!(
        parse_cli_args(&strings(&["mcp"])),
        CliAction::RunMcp(McpCliOptions {
            attach_only: false,
            socket: None,
        })
    );
    assert_eq!(
        parse_cli_args(&strings(&["--mcp", "--attach-only"])),
        CliAction::RunMcp(McpCliOptions {
            attach_only: true,
            socket: None,
        })
    );
    assert_eq!(
        parse_cli_args(&strings(&["mcp", "--socket", "/tmp/x/mcp.sock"])),
        CliAction::RunMcp(McpCliOptions {
            attach_only: false,
            socket: Some(PathBuf::from("/tmp/x/mcp.sock")),
        })
    );
    assert_eq!(
        parse_cli_args(&strings(&["--mcp", "--socket=/tmp/x/mcp.sock"])),
        CliAction::RunMcp(McpCliOptions {
            attach_only: false,
            socket: Some(PathBuf::from("/tmp/x/mcp.sock")),
        })
    );
    assert_eq!(
        parse_cli_args(&strings(&["mcp", "--help"])),
        CliAction::ShowHelp
    );
    assert!(matches!(
        parse_cli_args(&strings(&["mcp", "--bogus"])),
        CliAction::InvalidArgs(_)
    ));
    assert!(matches!(
        parse_cli_args(&strings(&["mcp", "--socket"])),
        CliAction::InvalidArgs(_)
    ));
}

#[test]
fn endpoint_validation_rejects_paths_outside_trusted_runtime_root() {
    // Arrange
    let options = McpCliOptions {
        attach_only: true,
        socket: Some(PathBuf::from("/tmp/untrusted/mcp.sock")),
    };

    // Act
    let result = resolve_endpoint(&options);

    // Assert
    assert!(matches!(result, Err(McpError::InvalidEndpoint(_))));
    assert!(SocketEndpoint::default_endpoint().is_ok());
}

#[test]
fn standalone_fallback_is_allowed_only_for_verified_missing_endpoint() {
    // Arrange
    let missing = McpError::EndpointMissing(PathBuf::from("/tmp/a/mcp.sock"));
    let stale = McpError::EndpointStale(PathBuf::from("/tmp/a/mcp.sock"));
    let invalid = McpError::InvalidEndpoint("bad".to_owned());
    let denied = McpError::PermissionDenied(PathBuf::from("/tmp/a/mcp.sock"));
    let uid_mismatch = McpError::PeerUidMismatch {
        expected: 501,
        actual: 502,
    };

    // Act & Assert
    assert!(should_fallback_to_standalone(false, &missing));
    assert!(!should_fallback_to_standalone(true, &missing));
    assert!(!should_fallback_to_standalone(false, &stale));
    assert!(!should_fallback_to_standalone(false, &invalid));
    assert!(!should_fallback_to_standalone(false, &denied));
    assert!(!should_fallback_to_standalone(false, &uid_mismatch));
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end standalone stdio handshake test covers initialize, tools/list, browser_status, and diagnostic rejection"
)]
fn standalone_stdio_handshake_reports_status_and_unavailable_live_tabs() {
    // Arrange: isolated pipe pair driving `StandaloneHost` via `serve_stdio` without AppKit.
    let (server_in, client_out) = pipe_pair();
    let (client_in, server_out) = pipe_pair();
    let isolated = IsolatedStdio::from_fds(server_in, server_out);
    let factory = BrowserHandlerFactory::new(Arc::new(StandaloneHost::new()))
        .with_product_version(crate::updates::INSTALLED_VERSION);
    let handle = serve_stdio(isolated, factory.next_handler()).expect("serve_stdio starts");

    let mut writer = File::from(client_out);
    let mut reader = BufReader::new(File::from(client_in));

    let send = |writer: &mut File, msg: &Value| {
        writeln!(writer, "{msg}").expect("write json line");
        writer.flush().expect("flush json line");
    };
    let recv = |reader: &mut BufReader<File>| -> Value {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read json line");
        assert!(n > 0, "unexpected EOF from standalone MCP server");
        serde_json::from_str(&line).expect("valid JSON-RPC frame")
    };

    // Act 1: initialize + initialized notification.
    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "test-client", "version": "0.1.0" }
            }
        }),
    );
    let init_resp = recv(&mut reader);
    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    );

    // Act 2: tools/list.
    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );
    let list_resp = recv(&mut reader);

    // Act 3: browser_status.
    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "browser_status", "arguments": {} }
        }),
    );
    let status_resp = recv(&mut reader);

    // Act 4: diagnostic_request in standalone mode.
    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "diagnostic_request", "arguments": {} }
        }),
    );
    let diag_resp = recv(&mut reader);

    // Close client stdin -> standalone server must exit cleanly.
    drop(writer);
    handle.wait();

    // Assert
    assert_eq!(init_resp["result"]["serverInfo"]["name"], "quickie");
    assert_eq!(
        init_resp["result"]["serverInfo"]["version"],
        crate::updates::INSTALLED_VERSION
    );
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tool_names.contains(&"browser_status"));
    assert!(tool_names.contains(&"diagnostic_request"));
    assert!(tool_names.contains(&"agent_context_open"));

    let status = &status_resp["result"]["structuredContent"];
    assert_eq!(status["platform"], "macos");
    assert_eq!(status["runtime_mode"], "standalone");
    assert_eq!(status["fork_capabilities"]["agent_automation"], false);
    assert_eq!(status["fork_capabilities"]["diagnostic"]["dom"], false);
    let supported = status["supported_operations"]
        .as_array()
        .expect("supported_operations array");
    assert!(
        !supported
            .iter()
            .any(|v| v.as_str() == Some("agent_context_open"))
    );

    assert_eq!(diag_resp["result"]["isError"], true);
    assert_eq!(
        diag_resp["result"]["structuredContent"]["code"],
        "tool_not_available"
    );
}
