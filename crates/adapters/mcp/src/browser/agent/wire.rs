//! Strict JSON argument decoding and structured MCP output encoding for isolated Agent tools.

use quickie_application::mcp::AgentContextId;
use rmcp::model::{CallToolResult, ContentBlock, JsonObject};
use serde_json::{Map, Value, json};
use url::{Host, Origin, Url};

use super::super::host::{ToolError, ToolErrorCode};
use super::super::wire::{
    MAX_SELECTOR_BYTES, decode_bounded_usize, empty_or_args, encode_tool_error, parse_u64,
    reject_unknown_keys, require_u64,
};
use super::types::{
    AgentActResult, AgentAction, AgentContextCloseResult, AgentContextListResult,
    AgentContextOpenResult, AgentContextSummary, AgentElementRef, AgentKey, AgentKeyModifier,
    AgentLocateResult, AgentLocator, AgentMouseButton, AgentNavigateResult, AgentObservedState,
    AgentScreenshotResult, AgentScreenshotTarget, AgentViewport, AgentWaitCondition,
    AgentWaitResult, AgentWaitStage, AgentWaitStatus, DEFAULT_ACTION_TIMEOUT_MS,
    DEFAULT_LOCATE_RESULTS, DEFAULT_NAVIGATE_TIMEOUT_MS, DEFAULT_SCREENSHOT_MAX_BYTES,
    ElementBounds, LocatedElement, MAX_ACTION_TIMEOUT_MS, MAX_ELEMENT_ID_BYTES,
    MAX_INPUT_TEXT_BYTES, MAX_KEY_MODIFIERS, MAX_LOCATE_RESULTS, MAX_LOCATOR_TEXT_BYTES,
    MAX_REASON_BYTES, MAX_ROLE_BYTES, MAX_SCREENSHOT_BYTES, MAX_SCROLL_DELTA, MAX_URL_BYTES,
    MAX_VIEWPORT_HEIGHT, MAX_VIEWPORT_WIDTH, MIN_ACTION_TIMEOUT_MS, MIN_VIEWPORT_HEIGHT,
    MIN_VIEWPORT_WIDTH, SelectOptionBy,
};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentNavigateArgs {
    pub(crate) context_id: AgentContextId,
    pub(crate) url: Url,
    pub(crate) wait_stage: AgentWaitStage,
    pub(crate) timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentLocateArgs {
    pub(crate) context_id: AgentContextId,
    pub(crate) locator: AgentLocator,
    pub(crate) max_results: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentActArgs {
    pub(crate) context_id: AgentContextId,
    pub(crate) action: AgentAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentWaitArgs {
    pub(crate) context_id: AgentContextId,
    pub(crate) condition: AgentWaitCondition,
    pub(crate) timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentScreenshotArgs {
    pub(crate) context_id: AgentContextId,
    pub(crate) target: AgentScreenshotTarget,
    pub(crate) max_bytes: usize,
}

pub(crate) fn decode_context_open_args(
    args: Option<&JsonObject>,
) -> Result<AgentViewport, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["viewport"])?;
    match map.get("viewport") {
        None | Some(Value::Null) => Ok(AgentViewport::default()),
        Some(Value::Object(vp)) => {
            reject_unknown_keys(vp, &["width", "height"])?;
            let width = decode_bounded_u32(
                vp.get("width"),
                "viewport.width",
                MIN_VIEWPORT_WIDTH,
                MAX_VIEWPORT_WIDTH,
            )?;
            let height = decode_bounded_u32(
                vp.get("height"),
                "viewport.height",
                MIN_VIEWPORT_HEIGHT,
                MAX_VIEWPORT_HEIGHT,
            )?;
            Ok(AgentViewport { width, height })
        }
        Some(_) => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "viewport must be an object",
        )),
    }
}

pub(crate) fn decode_context_list_args(args: Option<&JsonObject>) -> Result<(), ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &[])
}

pub(crate) fn decode_context_close_args(
    args: Option<&JsonObject>,
) -> Result<AgentContextId, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["context_id"])?;
    Ok(AgentContextId::new(require_u64(map, "context_id")?))
}

pub(crate) fn decode_navigate_args(
    args: Option<&JsonObject>,
) -> Result<AgentNavigateArgs, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["context_id", "url", "wait_stage", "timeout_ms"])?;
    let context_id = AgentContextId::new(require_u64(map, "context_id")?);
    let url = decode_http_url(map.get("url"))?;
    let wait_stage = decode_wait_stage(map.get("wait_stage"), true)?;
    let timeout_ms = decode_timeout_ms(map.get("timeout_ms"), DEFAULT_NAVIGATE_TIMEOUT_MS)?;
    Ok(AgentNavigateArgs {
        context_id,
        url,
        wait_stage,
        timeout_ms,
    })
}

pub(crate) fn decode_locate_args(args: Option<&JsonObject>) -> Result<AgentLocateArgs, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["context_id", "locator", "max_results"])?;
    let context_id = AgentContextId::new(require_u64(map, "context_id")?);
    let locator = decode_locator(map.get("locator"))?;
    let max_results = decode_bounded_usize(
        map.get("max_results"),
        "max_results",
        DEFAULT_LOCATE_RESULTS,
        MAX_LOCATE_RESULTS,
    )?;
    Ok(AgentLocateArgs {
        context_id,
        locator,
        max_results,
    })
}

pub(crate) fn decode_act_args(args: Option<&JsonObject>) -> Result<AgentActArgs, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["context_id", "action"])?;
    let context_id = AgentContextId::new(require_u64(map, "context_id")?);
    let action = decode_action(map.get("action"), context_id)?;
    Ok(AgentActArgs { context_id, action })
}

pub(crate) fn decode_wait_args(args: Option<&JsonObject>) -> Result<AgentWaitArgs, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["context_id", "condition", "timeout_ms"])?;
    let context_id = AgentContextId::new(require_u64(map, "context_id")?);
    let condition = decode_wait_condition(map.get("condition"))?;
    let timeout_ms = decode_timeout_ms(map.get("timeout_ms"), DEFAULT_ACTION_TIMEOUT_MS)?;
    Ok(AgentWaitArgs {
        context_id,
        condition,
        timeout_ms,
    })
}

pub(crate) fn decode_screenshot_args(
    args: Option<&JsonObject>,
) -> Result<AgentScreenshotArgs, ToolError> {
    let map = empty_or_args(args);
    reject_unknown_keys(map, &["context_id", "target", "max_bytes"])?;
    let context_id = AgentContextId::new(require_u64(map, "context_id")?);
    let target = decode_screenshot_target(map.get("target"), context_id)?;
    let max_bytes = decode_bounded_usize(
        map.get("max_bytes"),
        "max_bytes",
        DEFAULT_SCREENSHOT_MAX_BYTES,
        MAX_SCREENSHOT_BYTES,
    )?;
    Ok(AgentScreenshotArgs {
        context_id,
        target,
        max_bytes,
    })
}

pub(crate) fn encode_context_open(result: &AgentContextOpenResult) -> CallToolResult {
    CallToolResult::structured(json!({
        "context_id": result.context_id.get(),
        "epoch": result.epoch,
        "viewport": encode_viewport(result.viewport),
    }))
}

pub(crate) fn encode_context_list(result: &AgentContextListResult) -> CallToolResult {
    let contexts: Vec<Value> = result.contexts.iter().map(encode_context_summary).collect();
    CallToolResult::structured(json!({
        "contexts": contexts,
    }))
}

pub(crate) fn encode_context_close(result: AgentContextCloseResult) -> CallToolResult {
    CallToolResult::structured(json!({
        "context_id": result.context_id.get(),
        "closed": result.closed,
    }))
}

pub(crate) fn encode_navigate(result: &AgentNavigateResult) -> CallToolResult {
    let mut obj = Map::new();
    obj.insert("context_id".to_owned(), json!(result.context_id.get()));
    obj.insert("status".to_owned(), json!(result.status.as_str()));
    obj.insert("epoch".to_owned(), json!(result.epoch));
    if let Some(origin) = &result.origin {
        obj.insert("origin".to_owned(), json!(origin));
    }
    if let Some(reason) = &result.reason {
        obj.insert("reason".to_owned(), json!(truncate_neutral(reason)));
    }
    CallToolResult::structured(Value::Object(obj))
}

pub(crate) fn encode_locate(result: &AgentLocateResult, max_results: usize) -> CallToolResult {
    let limit = max_results.min(MAX_LOCATE_RESULTS);
    let truncated = result.truncated || result.elements.len() > limit;
    let elements: Vec<Value> = result
        .elements
        .iter()
        .take(limit)
        .map(encode_located_element)
        .collect();
    CallToolResult::structured(json!({
        "context_id": result.context_id.get(),
        "epoch": result.epoch,
        "elements": elements,
        "truncated": truncated,
    }))
}

pub(crate) fn encode_act(result: &AgentActResult) -> CallToolResult {
    let mut obj = Map::new();
    obj.insert("context_id".to_owned(), json!(result.context_id.get()));
    obj.insert("status".to_owned(), json!(result.status.as_str()));
    obj.insert("epoch".to_owned(), json!(result.epoch));
    if let Some(origin) = &result.origin {
        obj.insert("origin".to_owned(), json!(origin));
    }
    if let Some(reason) = &result.reason {
        obj.insert("reason".to_owned(), json!(truncate_neutral(reason)));
    }
    if let Some(state) = &result.observed_state {
        obj.insert("observed_state".to_owned(), encode_observed_state(state));
    }
    CallToolResult::structured(Value::Object(obj))
}

pub(crate) fn encode_wait(result: AgentWaitResult) -> CallToolResult {
    CallToolResult::structured(json!({
        "context_id": result.context_id.get(),
        "epoch": result.epoch,
        "status": result.status.as_str(),
        "matched": result.status == AgentWaitStatus::Matched,
        "timed_out": result.status == AgentWaitStatus::TimedOut,
        "elapsed_ms": result.elapsed_ms,
    }))
}

pub(crate) fn encode_screenshot(
    result: &AgentScreenshotResult,
    max_bytes: usize,
) -> CallToolResult {
    let bound = max_bytes.min(MAX_SCREENSHOT_BYTES);
    if result.png_bytes.len() > bound {
        return encode_tool_error(&ToolError::new(
            ToolErrorCode::BoundsExceeded,
            "screenshot payload exceeds maximum byte bound",
        ));
    }
    if !result.png_bytes.starts_with(PNG_SIGNATURE) {
        return encode_tool_error(&ToolError::new(
            ToolErrorCode::Unavailable,
            "host returned invalid PNG screenshot payload",
        ));
    }
    if result.width == 0
        || result.width > MAX_VIEWPORT_WIDTH
        || result.height == 0
        || result.height > MAX_VIEWPORT_HEIGHT
    {
        return encode_tool_error(&ToolError::new(
            ToolErrorCode::BoundsExceeded,
            "screenshot dimensions exceed allowed bounds",
        ));
    }

    let encoded = encode_base64(&result.png_bytes);
    let metadata = json!({
        "context_id": result.context_id.get(),
        "epoch": result.epoch,
        "mime_type": "image/png",
        "width": result.width,
        "height": result.height,
        "byte_len": result.png_bytes.len(),
        "base64_len": encoded.len(),
    });

    let mut out = CallToolResult::structured(metadata);
    out.content
        .insert(0, ContentBlock::image(encoded, "image/png"));
    out
}

fn decode_http_url(value: Option<&Value>) -> Result<Url, ToolError> {
    let Some(Value::String(raw)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "url must be a string",
        ));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "url must not be empty",
        ));
    }
    if trimmed.len() > MAX_URL_BYTES {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            "url exceeds maximum length",
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "url must not contain control characters",
        ));
    }
    let parsed = Url::parse(trimmed).map_err(|_| {
        ToolError::new(
            ToolErrorCode::InvalidParams,
            "url must be a valid HTTP or HTTPS URL",
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "url scheme must be http or https",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "url must not contain credentials",
        ));
    }
    match parsed.origin() {
        Origin::Tuple(scheme, host, _)
            if matches!(scheme.as_str(), "http" | "https")
                && !matches!(&host, Host::Domain(domain) if domain.is_empty()) =>
        {
            Ok(parsed)
        }
        _ => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "url must have a valid HTTP or HTTPS host",
        )),
    }
}

fn decode_wait_stage(value: Option<&Value>, allow_none: bool) -> Result<AgentWaitStage, ToolError> {
    match value {
        None | Some(Value::Null) if allow_none => Ok(AgentWaitStage::Load),
        Some(Value::String(raw)) => match raw.as_str() {
            "none" if allow_none => Ok(AgentWaitStage::None),
            "dom_content_loaded" => Ok(AgentWaitStage::DomContentLoaded),
            "load" => Ok(AgentWaitStage::Load),
            _ => Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "invalid wait stage",
            )),
        },
        _ => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "wait stage must be a valid string",
        )),
    }
}

fn decode_timeout_ms(value: Option<&Value>, default: u64) -> Result<u64, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(default),
        Some(raw) => {
            let ms = parse_u64(raw, "timeout_ms")?;
            if ms < MIN_ACTION_TIMEOUT_MS {
                return Err(ToolError::new(
                    ToolErrorCode::InvalidParams,
                    "timeout_ms must be at least 1",
                ));
            }
            if ms > MAX_ACTION_TIMEOUT_MS {
                return Err(ToolError::new(
                    ToolErrorCode::BoundsExceeded,
                    "timeout_ms exceeds maximum bound",
                ));
            }
            Ok(ms)
        }
    }
}

fn decode_bounded_u32(
    value: Option<&Value>,
    field: &str,
    min: u32,
    max: u32,
) -> Result<u32, ToolError> {
    let Some(raw) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            format!("missing required parameter {field}"),
        ));
    };
    let parsed = parse_u64(raw, field)?;
    let Ok(as_u32) = u32::try_from(parsed) else {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            format!("{field} exceeds maximum bound"),
        ));
    };
    if as_u32 < min || as_u32 > max {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            format!("{field} is outside allowed bounds"),
        ));
    }
    Ok(as_u32)
}

fn decode_locator(value: Option<&Value>) -> Result<AgentLocator, ToolError> {
    let Some(Value::Object(map)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "locator must be an object",
        ));
    };
    let Some(Value::String(kind)) = map.get("kind") else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "locator.kind must be 'css', 'text', or 'role'",
        ));
    };
    match kind.as_str() {
        "css" => {
            reject_unknown_keys(map, &["kind", "selector"])?;
            let selector =
                decode_required_bounded_str(map.get("selector"), "selector", MAX_SELECTOR_BYTES)?;
            Ok(AgentLocator::Css { selector })
        }
        "text" => {
            reject_unknown_keys(map, &["kind", "text", "exact"])?;
            let text =
                decode_required_bounded_str(map.get("text"), "text", MAX_LOCATOR_TEXT_BYTES)?;
            let exact = decode_optional_bool(map.get("exact"), "exact", false)?;
            Ok(AgentLocator::Text { text, exact })
        }
        "role" => {
            reject_unknown_keys(map, &["kind", "role", "name", "exact"])?;
            let role = decode_role(map.get("role"))?;
            let name = match map.get("name") {
                None | Some(Value::Null) => None,
                Some(v) => Some(decode_required_bounded_str(
                    Some(v),
                    "name",
                    MAX_LOCATOR_TEXT_BYTES,
                )?),
            };
            let exact = decode_optional_bool(map.get("exact"), "exact", false)?;
            Ok(AgentLocator::Role { role, name, exact })
        }
        _ => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "locator.kind must be 'css', 'text', or 'role'",
        )),
    }
}

fn decode_role(value: Option<&Value>) -> Result<String, ToolError> {
    let role = decode_required_bounded_str(value, "role", MAX_ROLE_BYTES)?;
    if !role
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "role must be a lowercase ASCII identifier",
        ));
    }
    Ok(role)
}

fn decode_element_ref(
    value: Option<&Value>,
    expected_context: AgentContextId,
) -> Result<AgentElementRef, ToolError> {
    let Some(Value::Object(map)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "element must be an object with context_id, epoch, and element_id",
        ));
    };
    reject_unknown_keys(map, &["context_id", "epoch", "element_id"])?;
    let context_id = AgentContextId::new(require_u64(map, "context_id")?);
    if context_id != expected_context {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "element.context_id does not match request context_id",
        ));
    }
    let epoch = require_u64(map, "epoch")?;
    let element_id =
        decode_required_bounded_str(map.get("element_id"), "element_id", MAX_ELEMENT_ID_BYTES)?;
    if element_id.chars().any(char::is_control) {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "element_id must not contain control characters",
        ));
    }
    Ok(AgentElementRef {
        context_id,
        epoch,
        element_id,
    })
}

fn decode_optional_element_ref(
    value: Option<&Value>,
    expected_context: AgentContextId,
) -> Result<Option<AgentElementRef>, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(decode_element_ref(Some(v), expected_context)?)),
    }
}

fn decode_pointer_click(
    map: &JsonObject,
    expected_context: AgentContextId,
    double_click: bool,
) -> Result<AgentAction, ToolError> {
    reject_unknown_keys(map, &["kind", "element", "button", "modifiers"])?;
    let element = decode_element_ref(map.get("element"), expected_context)?;
    let button = decode_mouse_button(map.get("button"))?;
    let modifiers = decode_modifiers(map.get("modifiers"))?;
    if double_click {
        Ok(AgentAction::DoubleClick {
            element,
            button,
            modifiers,
        })
    } else {
        Ok(AgentAction::Click {
            element,
            button,
            modifiers,
        })
    }
}

fn decode_action(
    value: Option<&Value>,
    expected_context: AgentContextId,
) -> Result<AgentAction, ToolError> {
    let Some(Value::Object(map)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "action must be an object",
        ));
    };
    let Some(Value::String(kind)) = map.get("kind") else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "action.kind must be a supported interaction string",
        ));
    };
    match kind.as_str() {
        "click" => decode_pointer_click(map, expected_context, false),
        "double_click" => decode_pointer_click(map, expected_context, true),
        "hover" => {
            reject_unknown_keys(map, &["kind", "element"])?;
            let element = decode_element_ref(map.get("element"), expected_context)?;
            Ok(AgentAction::Hover { element })
        }
        "fill" => {
            reject_unknown_keys(map, &["kind", "element", "text"])?;
            let element = decode_element_ref(map.get("element"), expected_context)?;
            let text = decode_input_text(map.get("text"), true)?;
            Ok(AgentAction::Fill { element, text })
        }
        "type" => {
            reject_unknown_keys(map, &["kind", "element", "text"])?;
            let element = decode_optional_element_ref(map.get("element"), expected_context)?;
            let text = decode_input_text(map.get("text"), false)?;
            Ok(AgentAction::Type { element, text })
        }
        "press_key" => {
            reject_unknown_keys(map, &["kind", "element", "key", "modifiers"])?;
            let element = decode_optional_element_ref(map.get("element"), expected_context)?;
            let key = decode_key(map.get("key"))?;
            let modifiers = decode_modifiers(map.get("modifiers"))?;
            Ok(AgentAction::PressKey {
                element,
                key,
                modifiers,
            })
        }
        "scroll" => {
            reject_unknown_keys(map, &["kind", "element", "delta_x", "delta_y"])?;
            let element = decode_optional_element_ref(map.get("element"), expected_context)?;
            let delta_x = decode_scroll_delta(map.get("delta_x"), "delta_x")?;
            let delta_y = decode_scroll_delta(map.get("delta_y"), "delta_y")?;
            if delta_x == 0 && delta_y == 0 {
                return Err(ToolError::new(
                    ToolErrorCode::InvalidParams,
                    "scroll delta_x and delta_y must not both be zero",
                ));
            }
            Ok(AgentAction::Scroll {
                element,
                delta_x,
                delta_y,
            })
        }
        "select_option" => {
            reject_unknown_keys(map, &["kind", "element", "value", "label"])?;
            let element = decode_element_ref(map.get("element"), expected_context)?;
            let option = match (map.get("value"), map.get("label")) {
                (Some(v), None | Some(Value::Null)) => SelectOptionBy::Value(
                    decode_required_bounded_str(Some(v), "value", MAX_INPUT_TEXT_BYTES)?,
                ),
                (None | Some(Value::Null), Some(l)) => SelectOptionBy::Label(
                    decode_required_bounded_str(Some(l), "label", MAX_INPUT_TEXT_BYTES)?,
                ),
                _ => {
                    return Err(ToolError::new(
                        ToolErrorCode::InvalidParams,
                        "select_option requires exactly one of value or label",
                    ));
                }
            };
            Ok(AgentAction::SelectOption { element, option })
        }
        _ => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "unsupported action kind",
        )),
    }
}

fn decode_wait_condition(value: Option<&Value>) -> Result<AgentWaitCondition, ToolError> {
    let Some(Value::Object(map)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "condition must be an object",
        ));
    };
    let Some(Value::String(kind)) = map.get("kind") else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "condition.kind must be a supported wait condition",
        ));
    };
    match kind.as_str() {
        "locator_visible" => {
            reject_unknown_keys(map, &["kind", "locator"])?;
            Ok(AgentWaitCondition::LocatorVisible {
                locator: decode_locator(map.get("locator"))?,
            })
        }
        "locator_hidden" => {
            reject_unknown_keys(map, &["kind", "locator"])?;
            Ok(AgentWaitCondition::LocatorHidden {
                locator: decode_locator(map.get("locator"))?,
            })
        }
        "text_present" => {
            reject_unknown_keys(map, &["kind", "text", "exact"])?;
            let text =
                decode_required_bounded_str(map.get("text"), "text", MAX_LOCATOR_TEXT_BYTES)?;
            let exact = decode_optional_bool(map.get("exact"), "exact", false)?;
            Ok(AgentWaitCondition::TextPresent { text, exact })
        }
        "load_stage" => {
            reject_unknown_keys(map, &["kind", "stage"])?;
            let stage = decode_wait_stage(map.get("stage"), false)?;
            Ok(AgentWaitCondition::LoadStage { stage })
        }
        _ => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "unsupported wait condition kind",
        )),
    }
}

fn decode_screenshot_target(
    value: Option<&Value>,
    expected_context: AgentContextId,
) -> Result<AgentScreenshotTarget, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(AgentScreenshotTarget::Viewport),
        Some(Value::Object(map)) => {
            let Some(Value::String(kind)) = map.get("kind") else {
                return Err(ToolError::new(
                    ToolErrorCode::InvalidParams,
                    "target.kind must be 'viewport' or 'element'",
                ));
            };
            match kind.as_str() {
                "viewport" => {
                    reject_unknown_keys(map, &["kind"])?;
                    Ok(AgentScreenshotTarget::Viewport)
                }
                "element" => {
                    reject_unknown_keys(map, &["kind", "element"])?;
                    let element = decode_element_ref(map.get("element"), expected_context)?;
                    Ok(AgentScreenshotTarget::Element(element))
                }
                _ => Err(ToolError::new(
                    ToolErrorCode::InvalidParams,
                    "target.kind must be 'viewport' or 'element'",
                )),
            }
        }
        Some(_) => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "target must be an object",
        )),
    }
}

fn decode_mouse_button(value: Option<&Value>) -> Result<AgentMouseButton, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(AgentMouseButton::Left),
        Some(Value::String(raw)) => match raw.as_str() {
            "left" => Ok(AgentMouseButton::Left),
            "middle" => Ok(AgentMouseButton::Middle),
            "right" => Ok(AgentMouseButton::Right),
            _ => Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "button must be 'left', 'middle', or 'right'",
            )),
        },
        Some(_) => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "button must be a string",
        )),
    }
}

fn decode_modifiers(value: Option<&Value>) -> Result<Vec<AgentKeyModifier>, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            if items.len() > MAX_KEY_MODIFIERS {
                return Err(ToolError::new(
                    ToolErrorCode::BoundsExceeded,
                    "modifiers exceeds maximum length",
                ));
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(raw) = item.as_str() else {
                    return Err(ToolError::new(
                        ToolErrorCode::InvalidParams,
                        "modifiers entries must be strings",
                    ));
                };
                let modifier = match raw {
                    "shift" => AgentKeyModifier::Shift,
                    "control" => AgentKeyModifier::Control,
                    "alt" => AgentKeyModifier::Alt,
                    "meta" => AgentKeyModifier::Meta,
                    _ => {
                        return Err(ToolError::new(
                            ToolErrorCode::InvalidParams,
                            "invalid key modifier",
                        ));
                    }
                };
                if out.contains(&modifier) {
                    return Err(ToolError::new(
                        ToolErrorCode::InvalidParams,
                        "duplicate key modifier",
                    ));
                }
                out.push(modifier);
            }
            Ok(out)
        }
        Some(_) => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "modifiers must be an array",
        )),
    }
}

fn decode_key(value: Option<&Value>) -> Result<AgentKey, ToolError> {
    let Some(Value::String(raw)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "key must be a string",
        ));
    };
    let named = match raw.as_str() {
        "Enter" => Some(AgentKey::Enter),
        "Tab" => Some(AgentKey::Tab),
        "Escape" => Some(AgentKey::Escape),
        "Backspace" => Some(AgentKey::Backspace),
        "Delete" => Some(AgentKey::Delete),
        "Space" => Some(AgentKey::Space),
        "ArrowUp" => Some(AgentKey::ArrowUp),
        "ArrowDown" => Some(AgentKey::ArrowDown),
        "ArrowLeft" => Some(AgentKey::ArrowLeft),
        "ArrowRight" => Some(AgentKey::ArrowRight),
        "Home" => Some(AgentKey::Home),
        "End" => Some(AgentKey::End),
        "PageUp" => Some(AgentKey::PageUp),
        "PageDown" => Some(AgentKey::PageDown),
        _ => None,
    };
    if let Some(key) = named {
        return Ok(key);
    }
    let mut chars = raw.chars();
    if let (Some(ch), None) = (chars.next(), chars.next())
        && !ch.is_control()
    {
        return Ok(AgentKey::Character(ch));
    }
    Err(ToolError::new(
        ToolErrorCode::InvalidParams,
        "key must be an allowlisted named key or single printable character",
    ))
}

fn decode_scroll_delta(value: Option<&Value>, field: &str) -> Result<i32, ToolError> {
    let Some(raw) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            format!("missing required parameter {field}"),
        ));
    };
    let Some(parsed) = raw.as_i64() else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            format!("{field} must be an integer"),
        ));
    };
    let Ok(delta) = i32::try_from(parsed) else {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            format!("{field} exceeds scroll bound"),
        ));
    };
    if !(-MAX_SCROLL_DELTA..=MAX_SCROLL_DELTA).contains(&delta) {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            format!("{field} exceeds scroll bound"),
        ));
    }
    Ok(delta)
}

fn decode_input_text(value: Option<&Value>, allow_empty: bool) -> Result<String, ToolError> {
    let Some(Value::String(raw)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "text must be a string",
        ));
    };
    if !allow_empty && raw.is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "text must not be empty",
        ));
    }
    if raw.contains('\0') {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            "text must not contain NUL bytes",
        ));
    }
    if raw.len() > MAX_INPUT_TEXT_BYTES {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            "text exceeds maximum length",
        ));
    }
    Ok(raw.clone())
}

fn decode_required_bounded_str(
    value: Option<&Value>,
    field: &str,
    max_bytes: usize,
) -> Result<String, ToolError> {
    let Some(Value::String(raw)) = value else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            format!("{field} must be a string"),
        ));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            format!("{field} must not be empty"),
        ));
    }
    if trimmed.len() > max_bytes {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            format!("{field} exceeds maximum length"),
        ));
    }
    Ok(trimmed.to_owned())
}

fn decode_optional_bool(
    value: Option<&Value>,
    field: &str,
    default: bool,
) -> Result<bool, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(ToolError::new(
            ToolErrorCode::InvalidParams,
            format!("{field} must be a boolean"),
        )),
    }
}

fn encode_viewport(viewport: AgentViewport) -> Value {
    json!({
        "width": viewport.width,
        "height": viewport.height,
    })
}

fn encode_context_summary(summary: &AgentContextSummary) -> Value {
    let mut obj = Map::new();
    obj.insert("context_id".to_owned(), json!(summary.context_id.get()));
    obj.insert("epoch".to_owned(), json!(summary.epoch));
    if let Some(origin) = &summary.origin {
        obj.insert("origin".to_owned(), json!(origin));
    }
    obj.insert("viewport".to_owned(), encode_viewport(summary.viewport));
    Value::Object(obj)
}

fn encode_located_element(element: &LocatedElement) -> Value {
    let mut obj = Map::new();
    obj.insert(
        "element_ref".to_owned(),
        json!({
            "context_id": element.element_ref.context_id.get(),
            "epoch": element.element_ref.epoch,
            "element_id": element.element_ref.element_id,
        }),
    );
    obj.insert("role".to_owned(), json!(element.role));
    if let Some(name) = &element.name {
        obj.insert("name".to_owned(), json!(name));
    }
    if let Some(tag) = &element.tag {
        obj.insert("tag".to_owned(), json!(tag));
    }
    if let Some(bounds) = element.bounds.filter(|b| b.is_valid()) {
        obj.insert("bounds".to_owned(), encode_bounds(bounds));
    }
    obj.insert(
        "provenance".to_owned(),
        json!({
            "origin": element.provenance.origin,
            "frame": element.provenance.frame.as_str(),
            "locator_kind": element.provenance.locator_kind.as_str(),
        }),
    );
    Value::Object(obj)
}

fn encode_bounds(bounds: ElementBounds) -> Value {
    json!({
        "x": bounds.x,
        "y": bounds.y,
        "width": bounds.width,
        "height": bounds.height,
    })
}

fn encode_observed_state(state: &AgentObservedState) -> Value {
    let mut obj = Map::new();
    obj.insert("navigated".to_owned(), json!(state.navigated));
    if let Some(role) = &state.focused_role {
        obj.insert("focused_role".to_owned(), json!(role));
    }
    if let Some(len) = state.value_length {
        obj.insert("value_length".to_owned(), json!(len));
    }
    if let Some(scroll_x) = state.scroll_x {
        obj.insert("scroll_x".to_owned(), json!(scroll_x));
    }
    if let Some(scroll_y) = state.scroll_y {
        obj.insert("scroll_y".to_owned(), json!(scroll_y));
    }
    Value::Object(obj)
}

fn truncate_neutral(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.len() <= MAX_REASON_BYTES {
        return trimmed.to_owned();
    }
    let end = trimmed.floor_char_boundary(MAX_REASON_BYTES);
    trimmed[..end].to_owned()
}

/// Standard RFC 4648 base64 encoder avoiding an extra crate dependency for bounded PNG output.
fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[((triple >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((triple >> 12) & 0x3f) as usize]));
        if chunk.len() > 1 {
            out.push(char::from(TABLE[((triple >> 6) & 0x3f) as usize]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(TABLE[(triple & 0x3f) as usize]));
        } else {
            out.push('=');
        }
    }
    out
}
