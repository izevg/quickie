//! Strict JSON argument decoding and structured MCP output encoding for browser tools.
//!
//! Rejects unknown fields (including any caller-supplied `client_id`), malformed targets, and
//! out-of-bounds limits before invoking [`super::BrowserHost`]. Filters output events whose
//! corresponding capability flag is unset.

use quickie_application::mcp::{
    AgentContextId, AttachmentId, ConsoleLevel, DiagnosticCapabilities, DiagnosticEvent,
    DiagnosticEventData, DiagnosticEventFilter, DiagnosticEventKind, GrantScope,
    LifecycleEventKind, MAX_EVENTS_PER_ATTACHMENT, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_DEPTH,
    MAX_SNAPSHOT_NODES, NetworkEvent, NetworkPhase, ObservationTarget, RevokeReason, SnapshotKind,
    SnapshotRequest,
};
use rmcp::model::{CallToolResult, JsonObject};
use serde_json::{Map, Value, json};

use super::catalog::supported_operation_names_for;
use super::host::{
    BrowserStatus, DiagnosticDetachResult, DiagnosticGrant, PageEventsResult, PageSnapshotResult,
    ToolError, ToolErrorCode,
};

/// Default event batch limit when `page_events` omits `limit`.
pub const DEFAULT_EVENTS_LIMIT: usize = 100;

/// Maximum UTF-8 byte length allowed for an optional CSS selector in `page_snapshot`.
pub const MAX_SELECTOR_BYTES: usize = 1_024;

/// Decoded arguments for `page_snapshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageSnapshotArgs {
    pub(crate) target: ObservationTarget,
    pub(crate) request: SnapshotRequest,
}

/// Decoded arguments for `page_events`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageEventsArgs {
    pub(crate) target: ObservationTarget,
    pub(crate) cursor: u64,
    pub(crate) filter: DiagnosticEventFilter,
    pub(crate) limit: usize,
}

pub(crate) fn decode_browser_status_args(args: Option<&JsonObject>) -> Result<(), ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &[])
}

pub(crate) fn decode_diagnostic_request_args(
    args: Option<&JsonObject>,
) -> Result<GrantScope, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["scope"])?;
    match map.get("scope") {
        None | Some(Value::Null) => Ok(GrantScope::SingleDocument),
        Some(Value::String(raw)) => match raw.as_str() {
            "single_document" => Ok(GrantScope::SingleDocument),
            "same_origin_reload" => Ok(GrantScope::SameOriginReload),
            _ => Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "invalid diagnostic scope; expected single_document or same_origin_reload",
            )),
        },
        Some(_) => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "scope must be a string",
        )),
    }
}

pub(crate) fn decode_diagnostic_detach_args(
    args: Option<&JsonObject>,
) -> Result<AttachmentId, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["attachment_id"])?;
    let id = require_u64(map, "attachment_id")?;
    Ok(AttachmentId::from_wire(id))
}

pub(crate) fn decode_page_snapshot_args(
    args: Option<&JsonObject>,
) -> Result<PageSnapshotArgs, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(
        map,
        &[
            "target",
            "kind",
            "selector",
            "max_nodes",
            "max_depth",
            "max_bytes",
        ],
    )?;

    let target = decode_target(map.get("target"))?;
    let kind = decode_snapshot_kind(map.get("kind"))?;
    let selector = decode_optional_selector(map.get("selector"))?;
    let max_nodes = decode_bounded_usize(
        map.get("max_nodes"),
        "max_nodes",
        MAX_SNAPSHOT_NODES,
        MAX_SNAPSHOT_NODES,
    )?;
    let max_depth = decode_bounded_usize(
        map.get("max_depth"),
        "max_depth",
        MAX_SNAPSHOT_DEPTH,
        MAX_SNAPSHOT_DEPTH,
    )?;
    let max_bytes = decode_bounded_usize(
        map.get("max_bytes"),
        "max_bytes",
        MAX_SNAPSHOT_BYTES,
        MAX_SNAPSHOT_BYTES,
    )?;

    let request = SnapshotRequest::new(kind, selector, max_nodes, max_depth, max_bytes);
    Ok(PageSnapshotArgs { target, request })
}

pub(crate) fn decode_page_events_args(
    args: Option<&JsonObject>,
) -> Result<PageEventsArgs, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["target", "cursor", "categories", "limit"])?;

    let target = decode_target(map.get("target"))?;
    let cursor = match map.get("cursor") {
        None | Some(Value::Null) => 0,
        Some(value) => parse_u64(value, "cursor")?,
    };
    let filter = decode_event_filter(map.get("categories"))?;
    let limit = decode_bounded_usize(
        map.get("limit"),
        "limit",
        DEFAULT_EVENTS_LIMIT,
        MAX_EVENTS_PER_ATTACHMENT,
    )?;

    Ok(PageEventsArgs {
        target,
        cursor,
        filter,
        limit,
    })
}

pub(crate) fn encode_browser_status(status: &BrowserStatus) -> CallToolResult {
    CallToolResult::structured(json!({
        "platform": status.platform.as_str(),
        "runtime_mode": status.runtime_mode.as_str(),
        "product_version": status.product_version,
        "fork_capabilities": {
            "diagnostic": encode_capabilities(status.fork_capabilities.diagnostic),
            "native_accessibility": status.fork_capabilities.native_accessibility,
            "visual_inspector_priority": status.fork_capabilities.visual_inspector_priority,
            "agent_automation": status.fork_capabilities.agent_automation,
        },
        "supported_operations": supported_operation_names_for(status.fork_capabilities),
    }))
}

pub(crate) fn encode_diagnostic_grant(grant: &DiagnosticGrant) -> CallToolResult {
    CallToolResult::structured(json!({
        "attachment_id": grant.attachment_id.get(),
        "scope": grant_scope_str(grant.scope),
        "epoch": grant.epoch,
        "origin": grant.origin,
        "capabilities": encode_capabilities(grant.capabilities),
    }))
}

pub(crate) fn encode_diagnostic_detach(result: DiagnosticDetachResult) -> CallToolResult {
    CallToolResult::structured(json!({
        "attachment_id": result.attachment_id.get(),
        "detached": result.detached,
        "reason": revoke_reason_str(result.reason),
    }))
}

pub(crate) fn encode_page_snapshot(result: &PageSnapshotResult) -> CallToolResult {
    let mut obj = Map::new();
    obj.insert("kind".to_owned(), json!(snapshot_kind_str(result.kind)));
    obj.insert("epoch".to_owned(), json!(result.snapshot.epoch()));
    obj.insert("data".to_owned(), json!(result.snapshot.data()));
    obj.insert(
        "truncated".to_owned(),
        json!(result.snapshot.is_truncated()),
    );
    if let Some(source) = result.accessibility_source {
        obj.insert("accessibility_source".to_owned(), json!(source.as_str()));
    }
    CallToolResult::structured(Value::Object(obj))
}

pub(crate) fn encode_page_events(result: &PageEventsResult) -> CallToolResult {
    let events: Vec<Value> = result
        .batch
        .events
        .iter()
        .filter(|event| event_allowed_by_capabilities(event, result.capabilities))
        .map(encode_event)
        .collect();

    let mut obj = Map::new();
    obj.insert("events".to_owned(), Value::Array(events));
    obj.insert("next_cursor".to_owned(), json!(result.batch.next_cursor));
    obj.insert("has_more".to_owned(), json!(result.batch.has_more));
    if let Some(dropped_before) = result.batch.dropped_before {
        obj.insert("dropped_before".to_owned(), json!(dropped_before));
    }
    obj.insert("native_overflow".to_owned(), json!(result.native_overflow));
    obj.insert("closed".to_owned(), json!(result.closed));
    CallToolResult::structured(Value::Object(obj))
}

pub(crate) fn encode_tool_error(error: &ToolError) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "code": error.code().as_str(),
        "message": error.message(),
    }))
}

pub(crate) fn empty_or_args(args: Option<&JsonObject>) -> &JsonObject {
    static EMPTY: std::sync::LazyLock<JsonObject> = std::sync::LazyLock::new(Map::new);
    args.unwrap_or(&EMPTY)
}

pub(crate) fn reject_unknown_keys(map: &JsonObject, allowed: &[&str]) -> Result<(), ToolError> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "unexpected parameter in tool arguments",
            ));
        }
    }
    Ok(())
}

fn decode_target(value: Option<&Value>) -> Result<ObservationTarget, ToolError> {
    let Some(Value::Object(map)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "target must be an object",
        ));
    };

    let Some(Value::String(kind)) = map.get("kind") else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "target.kind must be 'diagnostic' or 'agent'",
        ));
    };

    match kind.as_str() {
        "diagnostic" => {
            reject_unknown_keys(map, &["kind", "attachment_id"])?;
            let id = require_u64(map, "attachment_id")?;
            Ok(ObservationTarget::Diagnostic(AttachmentId::from_wire(id)))
        }
        "agent" => {
            reject_unknown_keys(map, &["kind", "context_id"])?;
            let id = require_u64(map, "context_id")?;
            Ok(ObservationTarget::Agent(AgentContextId::new(id)))
        }
        _ => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "target.kind must be 'diagnostic' or 'agent'",
        )),
    }
}

fn decode_snapshot_kind(value: Option<&Value>) -> Result<SnapshotKind, ToolError> {
    let Some(Value::String(raw)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "kind must be 'dom', 'accessibility', or 'text'",
        ));
    };
    match raw.as_str() {
        "dom" => Ok(SnapshotKind::Dom),
        "accessibility" => Ok(SnapshotKind::Accessibility),
        "text" => Ok(SnapshotKind::Text),
        _ => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "kind must be 'dom', 'accessibility', or 'text'",
        )),
    }
}

fn decode_optional_selector(value: Option<&Value>) -> Result<Option<String>, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(ToolError::new(
                    ToolErrorCode::InvalidParams,
                    "selector must not be empty",
                ));
            }
            if trimmed.len() > MAX_SELECTOR_BYTES {
                return Err(ToolError::new(
                    ToolErrorCode::BoundsExceeded,
                    "selector exceeds maximum length",
                ));
            }
            Ok(Some(trimmed.to_owned()))
        }
        Some(_) => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "selector must be a string",
        )),
    }
}

fn decode_event_filter(value: Option<&Value>) -> Result<DiagnosticEventFilter, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(DiagnosticEventFilter::all()),
        Some(Value::Array(items)) => {
            let mut kinds = Vec::with_capacity(items.len());
            for item in items {
                let Some(raw) = item.as_str() else {
                    return Err(ToolError::new(
                        ToolErrorCode::InvalidParams,
                        "categories entries must be strings",
                    ));
                };
                let kind = match raw {
                    "console" => DiagnosticEventKind::Console,
                    "runtime_error" => DiagnosticEventKind::RuntimeError,
                    "network" => DiagnosticEventKind::Network,
                    "lifecycle" => DiagnosticEventKind::Lifecycle,
                    _ => {
                        return Err(ToolError::new(
                            ToolErrorCode::InvalidParams,
                            "invalid event category",
                        ));
                    }
                };
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
            Ok(DiagnosticEventFilter::of_kinds(kinds))
        }
        Some(_) => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "categories must be an array",
        )),
    }
}

pub(crate) fn decode_bounded_usize(
    value: Option<&Value>,
    field: &str,
    default: usize,
    max: usize,
) -> Result<usize, ToolError> {
    let Some(raw) = value else {
        return Ok(default);
    };
    if raw.is_null() {
        return Ok(default);
    }
    let parsed = parse_u64(raw, field)?;
    if parsed == 0 {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            format!("{field} must be at least 1"),
        ));
    }
    let Ok(as_usize) = usize::try_from(parsed) else {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            format!("{field} exceeds maximum bound"),
        ));
    };
    if as_usize > max {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            format!("{field} exceeds maximum bound"),
        ));
    }
    Ok(as_usize)
}

pub(crate) fn require_u64(map: &JsonObject, field: &str) -> Result<u64, ToolError> {
    let Some(value) = map.get(field) else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            format!("missing required parameter {field}"),
        ));
    };
    parse_u64(value, field)
}

pub(crate) fn parse_u64(value: &Value, field: &str) -> Result<u64, ToolError> {
    value.as_u64().ok_or_else(|| {
        ToolError::new(
            ToolErrorCode::InvalidParams,
            format!("{field} must be a non-negative integer"),
        )
    })
}

fn encode_capabilities(caps: DiagnosticCapabilities) -> Value {
    json!({
        "dom": caps.dom,
        "accessibility": caps.accessibility,
        "console": caps.console,
        "runtime_errors": caps.runtime_errors,
        "pending_network": caps.pending_network,
        "completed_network": caps.completed_network,
        "lifecycle": caps.lifecycle,
    })
}

fn event_allowed_by_capabilities(event: &DiagnosticEvent, caps: DiagnosticCapabilities) -> bool {
    match &event.data {
        DiagnosticEventData::Console { .. } => caps.console,
        DiagnosticEventData::RuntimeError { .. } => caps.runtime_errors,
        DiagnosticEventData::Network(net) => match net.phase() {
            NetworkPhase::Pending => caps.pending_network,
            NetworkPhase::Completed | NetworkPhase::Failed => caps.completed_network,
        },
        DiagnosticEventData::Lifecycle { .. } => caps.lifecycle,
    }
}

fn encode_event(event: &DiagnosticEvent) -> Value {
    let mut obj = Map::new();
    obj.insert("seq".to_owned(), json!(event.seq));
    obj.insert("epoch".to_owned(), json!(event.epoch));

    match &event.data {
        DiagnosticEventData::Console {
            level,
            text,
            truncated,
        } => {
            obj.insert("category".to_owned(), json!("console"));
            obj.insert("level".to_owned(), json!(console_level_str(*level)));
            obj.insert("text".to_owned(), json!(text));
            obj.insert("truncated".to_owned(), json!(truncated));
        }
        DiagnosticEventData::RuntimeError { message, truncated } => {
            obj.insert("category".to_owned(), json!("runtime_error"));
            obj.insert("message".to_owned(), json!(message));
            obj.insert("truncated".to_owned(), json!(truncated));
        }
        DiagnosticEventData::Network(net) => {
            obj.insert("category".to_owned(), json!("network"));
            encode_network_fields(&mut obj, net);
        }
        DiagnosticEventData::Lifecycle { kind } => {
            obj.insert("category".to_owned(), json!("lifecycle"));
            obj.insert("kind".to_owned(), json!(lifecycle_kind_str(*kind)));
        }
    }

    Value::Object(obj)
}

fn encode_network_fields(obj: &mut Map<String, Value>, net: &NetworkEvent) {
    let endpoint = net.endpoint();
    obj.insert("request_id".to_owned(), json!(net.request_id()));
    obj.insert("phase".to_owned(), json!(network_phase_str(net.phase())));
    obj.insert("method".to_owned(), json!(net.method()));
    if let Some(status) = net.status() {
        obj.insert("status".to_owned(), json!(status));
    }
    obj.insert(
        "endpoint".to_owned(),
        json!({
            "origin": endpoint.origin().ascii_serialization(),
            "path": endpoint.path(),
            "path_truncated": endpoint.is_path_truncated(),
        }),
    );
}

const fn grant_scope_str(scope: GrantScope) -> &'static str {
    match scope {
        GrantScope::SingleDocument => "single_document",
        GrantScope::SameOriginReload => "same_origin_reload",
    }
}

const fn snapshot_kind_str(kind: SnapshotKind) -> &'static str {
    match kind {
        SnapshotKind::Dom => "dom",
        SnapshotKind::Accessibility => "accessibility",
        SnapshotKind::Text => "text",
    }
}

const fn console_level_str(level: ConsoleLevel) -> &'static str {
    match level {
        ConsoleLevel::Debug => "debug",
        ConsoleLevel::Info => "info",
        ConsoleLevel::Warn => "warn",
        ConsoleLevel::Error => "error",
    }
}

const fn network_phase_str(phase: NetworkPhase) -> &'static str {
    match phase {
        NetworkPhase::Pending => "pending",
        NetworkPhase::Completed => "completed",
        NetworkPhase::Failed => "failed",
    }
}

const fn lifecycle_kind_str(kind: LifecycleEventKind) -> &'static str {
    match kind {
        LifecycleEventKind::SameDocumentNavigation => "same_document_navigation",
        LifecycleEventKind::ReloadCommitted => "reload_committed",
        LifecycleEventKind::DomContentLoaded => "dom_content_loaded",
        LifecycleEventKind::Load => "load",
    }
}

const fn revoke_reason_str(reason: RevokeReason) -> &'static str {
    match reason {
        RevokeReason::UserRevoked => "user_revoked",
        RevokeReason::ClientDetached => "client_detached",
        RevokeReason::Replaced => "replaced",
        RevokeReason::NavigatedAway => "navigated_away",
        RevokeReason::CrossOriginNavigation => "cross_origin_navigation",
        RevokeReason::TabClosed => "tab_closed",
        RevokeReason::ProcessCrashed => "process_crashed",
        RevokeReason::ClientDisconnected => "client_disconnected",
        RevokeReason::BrowserShuttingDown => "browser_shutting_down",
    }
}
