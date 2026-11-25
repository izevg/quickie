//! Shared MCP test handler and JSON-RPC wire helpers for `quickie-mcp` integration tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use quickie_mcp::rmcp::ErrorData as RmcpError;
use quickie_mcp::rmcp::handler::server::ServerHandler;
use quickie_mcp::rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerConfig, Tool, ToolsCapability,
};
use quickie_mcp::rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value, json};

/// Stateful per-connection MCP handler used to verify independent sessions, `tools/list`, and
/// `tools/call`.
#[derive(Debug)]
pub struct EchoHandler {
    session_id: usize,
    call_count: AtomicUsize,
}

impl EchoHandler {
    #[must_use]
    pub fn new(session_id: usize) -> Self {
        Self {
            session_id,
            call_count: AtomicUsize::new(0),
        }
    }
}

impl ServerHandler for EchoHandler {
    fn get_info(&self) -> ServerConfig {
        let mut config = ServerConfig::default();
        config.capabilities.tools = Some(ToolsCapability::default());
        config
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, RmcpError>> {
        let mut schema = Map::new();
        schema.insert("type".to_owned(), json!("object"));
        let mut props = Map::new();
        props.insert("text".to_owned(), json!({ "type": "string" }));
        schema.insert("properties".to_owned(), Value::Object(props));

        let tool = Tool::new(
            "echo",
            "Echoes input text with the handler's session id and per-session call count",
            Arc::new(schema),
        );
        std::future::ready(Ok(ListToolsResult {
            tools: vec![tool],
            ..ListToolsResult::default()
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, RmcpError>> {
        if request.name != "echo" {
            return std::future::ready(Err(RmcpError::invalid_params(
                format!("unknown tool: {}", request.name),
                None,
            )));
        }
        let call_num = self.call_count.fetch_add(1, Ordering::Relaxed) + 1;
        let text = request
            .arguments
            .as_ref()
            .and_then(|args| args.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let reply = format!("session={} call={} text={text}", self.session_id, call_num);
        std::future::ready(Ok(
            CallToolResult::success(vec![ContentBlock::text(reply)]).into()
        ))
    }
}

/// Serializes an MCP `initialize` JSON-RPC request frame (including trailing `\n`).
#[must_use]
pub fn initialize_frame(id: u64) -> String {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {
                "name": "quickie-mcp-test",
                "version": "0.1.0"
            }
        }
    });
    format!("{msg}\n")
}

/// Serializes an MCP `notifications/initialized` JSON-RPC notification frame (including `\n`).
#[must_use]
pub fn initialized_notification_frame() -> String {
    let msg = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    format!("{msg}\n")
}

/// Serializes an MCP `tools/list` JSON-RPC request frame (including `\n`).
#[must_use]
pub fn tools_list_frame(id: u64) -> String {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": {}
    });
    format!("{msg}\n")
}

/// Serializes an MCP `tools/call` JSON-RPC request frame for the `echo` tool (including `\n`).
#[must_use]
pub fn tools_call_echo_frame(id: u64, text: &str) -> String {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "echo",
            "arguments": { "text": text }
        }
    });
    format!("{msg}\n")
}
