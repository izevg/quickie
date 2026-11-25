//! Single tool catalog driving both MCP `tools/list` discovery and `tools/call` dispatch.
//!
//! Advertises the shared discovery, read-only diagnostic, and isolated Agent tools implemented by
//! [`super::BrowserHandler`]. Runtime status reports isolated Agent operations as supported only
//! when [`super::ForkCapabilities::agent_automation`] is enabled by the host.

use std::sync::Arc;

use quickie_application::mcp::{
    MAX_EVENTS_PER_ATTACHMENT, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_DEPTH, MAX_SNAPSHOT_NODES,
};
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};

use super::agent::catalog as agent_catalog;
use super::host::ForkCapabilities;

/// MCP tools implemented by [`super::BrowserHandler`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolId {
    /// `browser_status`
    BrowserStatus,
    /// `diagnostic_request`
    DiagnosticRequest,
    /// `diagnostic_detach`
    DiagnosticDetach,
    /// `page_snapshot`
    PageSnapshot,
    /// `page_events`
    PageEvents,
    /// `agent_context_open`
    AgentContextOpen,
    /// `agent_context_list`
    AgentContextList,
    /// `agent_context_close`
    AgentContextClose,
    /// `agent_navigate`
    AgentNavigate,
    /// `agent_locate`
    AgentLocate,
    /// `agent_act`
    AgentAct,
    /// `agent_wait`
    AgentWait,
    /// `agent_screenshot`
    AgentScreenshot,
}

/// Static catalog entry for one MCP tool.
#[derive(Clone, Copy, Debug)]
pub struct ToolSpec {
    /// Typed tool identifier.
    pub id: ToolId,
    /// Wire tool name exposed in `tools/list` and matched in `tools/call`.
    pub name: &'static str,
    /// Short human-readable title.
    pub title: &'static str,
    /// Concise description of tool behavior.
    pub description: &'static str,
    /// Whether the tool is read-only (MCP annotation hint).
    pub read_only: bool,
    /// Whether the tool is idempotent (MCP annotation hint).
    pub idempotent: bool,
    /// Whether this tool requires isolated `Profile::Agent` host support (`agent_automation`).
    pub requires_agent_automation: bool,
}

/// Canonical ordered catalog of all tools implemented by [`super::BrowserHandler`].
pub const TOOL_CATALOG: &[ToolSpec] = &[
    ToolSpec {
        id: ToolId::BrowserStatus,
        name: "browser_status",
        title: "Browser Status",
        description: "Reports Quickie platform, runtime mode, product version, fork capabilities, and supported operations.",
        read_only: true,
        idempotent: true,
        requires_agent_automation: false,
    },
    ToolSpec {
        id: ToolId::DiagnosticRequest,
        name: "diagnostic_request",
        title: "Diagnostic Request",
        description: "Requests explicit user consent to attach read-only diagnostics to the active tab.",
        read_only: false,
        idempotent: false,
        requires_agent_automation: false,
    },
    ToolSpec {
        id: ToolId::DiagnosticDetach,
        name: "diagnostic_detach",
        title: "Diagnostic Detach",
        description: "Detaches an active read-only diagnostic attachment owned by this MCP client.",
        read_only: false,
        idempotent: true,
        requires_agent_automation: false,
    },
    ToolSpec {
        id: ToolId::PageSnapshot,
        name: "page_snapshot",
        title: "Page Snapshot",
        description: "Captures a bounded read-only DOM, accessibility, or visible text snapshot for an authorized target.",
        read_only: true,
        idempotent: true,
        requires_agent_automation: false,
    },
    ToolSpec {
        id: ToolId::PageEvents,
        name: "page_events",
        title: "Page Events",
        description: "Replays ordered, filtered diagnostic events with cursor pagination for an authorized target.",
        read_only: true,
        idempotent: true,
        requires_agent_automation: false,
    },
    ToolSpec {
        id: ToolId::AgentContextOpen,
        name: "agent_context_open",
        title: "Agent Context Open",
        description: "Opens an isolated Profile::Agent browsing context owned by this MCP client.",
        read_only: false,
        idempotent: false,
        requires_agent_automation: true,
    },
    ToolSpec {
        id: ToolId::AgentContextList,
        name: "agent_context_list",
        title: "Agent Context List",
        description: "Lists isolated Profile::Agent browsing contexts owned by this MCP client.",
        read_only: true,
        idempotent: true,
        requires_agent_automation: true,
    },
    ToolSpec {
        id: ToolId::AgentContextClose,
        name: "agent_context_close",
        title: "Agent Context Close",
        description: "Closes an isolated Profile::Agent browsing context owned by this MCP client.",
        read_only: false,
        idempotent: true,
        requires_agent_automation: true,
    },
    ToolSpec {
        id: ToolId::AgentNavigate,
        name: "agent_navigate",
        title: "Agent Navigate",
        description: "Navigates an isolated Profile::Agent context to a validated credential-free HTTP(S) URL under host origin approval.",
        read_only: false,
        idempotent: false,
        requires_agent_automation: true,
    },
    ToolSpec {
        id: ToolId::AgentLocate,
        name: "agent_locate",
        title: "Agent Locate",
        description: "Locates bounded, ordered, document-bound element references in an isolated Profile::Agent context.",
        read_only: true,
        idempotent: true,
        requires_agent_automation: true,
    },
    ToolSpec {
        id: ToolId::AgentAct,
        name: "agent_act",
        title: "Agent Act",
        description: "Performs a typed trusted interaction in an isolated Profile::Agent context using exact document-bound references.",
        read_only: false,
        idempotent: false,
        requires_agent_automation: true,
    },
    ToolSpec {
        id: ToolId::AgentWait,
        name: "agent_wait",
        title: "Agent Wait",
        description: "Waits for a typed locator, text, or load-stage condition in an isolated Profile::Agent context within a bounded timeout.",
        read_only: true,
        idempotent: true,
        requires_agent_automation: true,
    },
    ToolSpec {
        id: ToolId::AgentScreenshot,
        name: "agent_screenshot",
        title: "Agent Screenshot",
        description: "Captures a bounded PNG screenshot of the viewport or a document-bound element in an isolated Profile::Agent context.",
        read_only: true,
        idempotent: true,
        requires_agent_automation: true,
    },
];

/// Resolves a wire tool name against [`TOOL_CATALOG`].
#[must_use]
pub fn lookup_tool(name: &str) -> Option<ToolId> {
    TOOL_CATALOG
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.id)
}

/// Returns the ordered wire names of all tools registered in [`TOOL_CATALOG`].
#[must_use]
pub fn supported_operation_names() -> Vec<&'static str> {
    TOOL_CATALOG.iter().map(|spec| spec.name).collect()
}

/// Returns the ordered wire names of operations backed by `capabilities`.
///
/// Isolated Agent tools are included only when `capabilities.agent_automation` is `true`.
#[must_use]
pub fn supported_operation_names_for(capabilities: ForkCapabilities) -> Vec<&'static str> {
    TOOL_CATALOG
        .iter()
        .filter(|spec| !spec.requires_agent_automation || capabilities.agent_automation)
        .map(|spec| spec.name)
        .collect()
}

/// Builds the `tools/list` [`Tool`] definitions directly from [`TOOL_CATALOG`].
#[must_use]
pub fn list_tools() -> Vec<Tool> {
    TOOL_CATALOG.iter().map(build_tool).collect()
}

fn build_tool(spec: &ToolSpec) -> Tool {
    let annotations = ToolAnnotations::with_title(spec.title)
        .read_only(spec.read_only)
        .destructive(false)
        .idempotent(spec.idempotent)
        .open_world(false);

    Tool::new(spec.name, spec.description, Arc::new(input_schema(spec.id)))
        .with_title(spec.title)
        .with_annotations(annotations)
}

fn target_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "attachment_id"],
                "properties": {
                    "kind": { "const": "diagnostic" },
                    "attachment_id": { "type": "integer", "minimum": 0 }
                }
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "context_id"],
                "properties": {
                    "kind": { "const": "agent" },
                    "context_id": { "type": "integer", "minimum": 0 }
                }
            }
        ]
    })
}

fn input_schema(id: ToolId) -> JsonObject {
    let value = match id {
        ToolId::BrowserStatus => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        }),
        ToolId::DiagnosticRequest => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "scope": {
                    "type": "string",
                    "enum": ["single_document", "same_origin_reload"],
                    "default": "single_document"
                }
            }
        }),
        ToolId::DiagnosticDetach => json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["attachment_id"],
            "properties": {
                "attachment_id": {
                    "type": "integer",
                    "minimum": 0
                }
            }
        }),
        ToolId::PageSnapshot => json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["target", "kind"],
            "properties": {
                "target": target_schema(),
                "kind": {
                    "type": "string",
                    "enum": ["dom", "accessibility", "text"]
                },
                "selector": {
                    "type": "string",
                    "minLength": 1
                },
                "max_nodes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_SNAPSHOT_NODES
                },
                "max_depth": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_SNAPSHOT_DEPTH
                },
                "max_bytes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_SNAPSHOT_BYTES
                }
            }
        }),
        ToolId::PageEvents => json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["target"],
            "properties": {
                "target": target_schema(),
                "cursor": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 0
                },
                "categories": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["console", "runtime_error", "network", "lifecycle"]
                    }
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_EVENTS_PER_ATTACHMENT,
                    "default": 100
                }
            }
        }),
        ToolId::AgentContextOpen => agent_catalog::schema_agent_context_open(),
        ToolId::AgentContextList => agent_catalog::schema_agent_context_list(),
        ToolId::AgentContextClose => agent_catalog::schema_agent_context_close(),
        ToolId::AgentNavigate => agent_catalog::schema_agent_navigate(),
        ToolId::AgentLocate => agent_catalog::schema_agent_locate(),
        ToolId::AgentAct => agent_catalog::schema_agent_act(),
        ToolId::AgentWait => agent_catalog::schema_agent_wait(),
        ToolId::AgentScreenshot => agent_catalog::schema_agent_screenshot(),
    };

    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}
