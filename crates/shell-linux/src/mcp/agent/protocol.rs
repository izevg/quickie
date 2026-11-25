//! Pure `WebKit` Automation protocol translation, inbound frame decoding, and PNG bounds checks.

use quickie_mcp::browser::{
    AgentKey, AgentKeyModifier, AgentMouseButton, AgentWaitStage, MAX_SCREENSHOT_BYTES,
    MAX_VIEWPORT_HEIGHT, MAX_VIEWPORT_WIDTH, ToolError, ToolErrorCode,
};
use serde_json::Value;
use url::Url;

use crate::mcp::runtime::protocol::DecodedSubscription;

/// Maximum inbound native Automation JSON frame size (8 MiB).
pub(crate) const MAX_INBOUND_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Maximum UTF-8 byte length for a `WebKit` Automation browsing context handle.
const MAX_HANDLE_BYTES: usize = 256;

/// PNG magic signature.
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Typed mouse interaction dispatched to native `WebKit` Automation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WireMouseInteraction {
    Move,
    SingleClick,
    DoubleClick,
}

/// Typed keyboard step dispatched to native `WebKit` Automation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WireKeyStep {
    InsertByKey(String),
    KeyPress(&'static str),
    KeyRelease(&'static str),
}

/// Strictly allowlisted native `WebKit` Automation command emitted by `AgentManager`.
///
/// Translated directly into `gtk::agent::automation::AutomationRequest` on Linux; never carries
/// caller-supplied raw JSON, JavaScript, or arbitrary Inspector commands.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AutomationWireCommand {
    GetBrowsingContexts,
    Navigate {
        handle: String,
        url: Url,
        wait_stage: AgentWaitStage,
        timeout_ms: u32,
    },
    MouseInteraction {
        handle: String,
        x: f64,
        y: f64,
        button: AgentMouseButton,
        interaction: WireMouseInteraction,
        modifiers: Vec<AgentKeyModifier>,
    },
    KeyboardInteractions {
        handle: String,
        steps: Vec<WireKeyStep>,
    },
    WheelScroll {
        handle: String,
        x: f64,
        y: f64,
        delta_x: i32,
        delta_y: i32,
    },
    TakeScreenshot {
        handle: String,
        clip_to_viewport: bool,
    },
    SendInspectorMessage {
        handle: String,
        command: DecodedSubscription,
    },
}

/// Decoded inbound message from `WebKitAutomationSession::message-received`.
#[derive(Debug, PartialEq)]
pub(crate) enum InboundFrame {
    Reply {
        id: u64,
        outcome: Result<Value, ToolError>,
    },
    BrowsingContextCleared {
        handle: String,
    },
    InspectorMessage {
        handle: String,
        message: String,
    },
    Ignored,
}

/// Parses one raw inbound JSON frame from `WebKitAutomationSession`.
#[must_use]
pub(crate) fn parse_inbound_frame(raw: &str) -> InboundFrame {
    if raw.is_empty() || raw.len() > MAX_INBOUND_FRAME_BYTES {
        return InboundFrame::Ignored;
    }
    let Ok(mut value) = serde_json::from_str::<Value>(raw) else {
        return InboundFrame::Ignored;
    };
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        if let Some(error) = value.get("error") {
            return InboundFrame::Reply {
                id,
                outcome: Err(map_automation_error(error)),
            };
        }
        let result = value.get_mut("result").map_or(Value::Null, Value::take);
        return InboundFrame::Reply {
            id,
            outcome: Ok(result),
        };
    }

    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return InboundFrame::Ignored;
    };
    let Some(params) = value.get("params").and_then(Value::as_object) else {
        return InboundFrame::Ignored;
    };

    match method {
        "Automation.browsingContextCleared" => {
            let Some(handle) = params
                .get("browsingContextHandle")
                .and_then(Value::as_str)
                .filter(|h| is_valid_handle(h))
            else {
                return InboundFrame::Ignored;
            };
            InboundFrame::BrowsingContextCleared {
                handle: handle.to_owned(),
            }
        }
        "Automation.inspectorMessage" => {
            let Some(handle) = params
                .get("browsingContextHandle")
                .and_then(Value::as_str)
                .filter(|h| is_valid_handle(h))
            else {
                return InboundFrame::Ignored;
            };
            let Some(message) = params.get("message").and_then(Value::as_str) else {
                return InboundFrame::Ignored;
            };
            InboundFrame::InspectorMessage {
                handle: handle.to_owned(),
                message: message.to_owned(),
            }
        }
        _ => InboundFrame::Ignored,
    }
}

/// Extracts the primary browsing context handle from `Automation.getBrowsingContexts`.
pub(crate) fn decode_browsing_context_handle(result: &Value) -> Result<String, ToolError> {
    let contexts = result
        .get("contexts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Unavailable,
                "invalid browsing contexts reply from WebKit Automation",
            )
        })?;
    contexts
        .iter()
        .find_map(|entry| {
            entry
                .get("handle")
                .and_then(Value::as_str)
                .filter(|h| is_valid_handle(h))
                .map(str::to_owned)
        })
        .ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Unavailable,
                "no active isolated browsing context handle returned",
            )
        })
}

/// Decodes and validates a bounded PNG screenshot returned by `Automation.takeScreenshot`.
pub(crate) fn decode_screenshot_reply(
    result: &Value,
    max_bytes: usize,
) -> Result<(u32, u32, Vec<u8>), ToolError> {
    let data = result
        .get("data")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::Unavailable,
                "missing screenshot payload from WebKit Automation",
            )
        })?;
    let bound = max_bytes.min(MAX_SCREENSHOT_BYTES);
    let png_bytes = decode_base64_bounded(data, bound)?;
    let (width, height) = parse_png_dimensions(&png_bytes)?;
    Ok((width, height, png_bytes))
}

/// Builds native keyboard interaction steps for `AgentAction::PressKey`.
#[must_use]
pub(crate) fn build_press_key_steps(
    key: AgentKey,
    modifiers: &[AgentKeyModifier],
) -> Vec<WireKeyStep> {
    let mut steps = Vec::with_capacity(modifiers.len() * 2 + 2);
    for &modifier in modifiers {
        steps.push(WireKeyStep::KeyPress(modifier_virtual_key(modifier)));
    }
    match key {
        AgentKey::Character(ch) => {
            steps.push(WireKeyStep::InsertByKey(ch.to_string()));
        }
        virtual_key => {
            let name = agent_virtual_key(virtual_key);
            steps.push(WireKeyStep::KeyPress(name));
            steps.push(WireKeyStep::KeyRelease(name));
        }
    }
    for &modifier in modifiers.iter().rev() {
        steps.push(WireKeyStep::KeyRelease(modifier_virtual_key(modifier)));
    }
    steps
}

/// Builds native keyboard interaction steps for `AgentAction::Fill` (select-all, delete, insert).
#[must_use]
pub(crate) fn build_fill_steps(text: &str) -> Vec<WireKeyStep> {
    let mut steps = vec![
        WireKeyStep::KeyPress("Control"),
        WireKeyStep::InsertByKey("a".to_owned()),
        WireKeyStep::KeyRelease("Control"),
        WireKeyStep::KeyPress("Backspace"),
        WireKeyStep::KeyRelease("Backspace"),
    ];
    if !text.is_empty() {
        steps.push(WireKeyStep::InsertByKey(text.to_owned()));
    }
    steps
}

const fn modifier_virtual_key(modifier: AgentKeyModifier) -> &'static str {
    match modifier {
        AgentKeyModifier::Shift => "Shift",
        AgentKeyModifier::Control => "Control",
        AgentKeyModifier::Alt => "Alternate",
        AgentKeyModifier::Meta => "Meta",
    }
}

const fn agent_virtual_key(key: AgentKey) -> &'static str {
    match key {
        AgentKey::Enter => "Enter",
        AgentKey::Tab => "Tab",
        AgentKey::Escape => "Escape",
        AgentKey::Backspace => "Backspace",
        AgentKey::Delete => "Delete",
        AgentKey::Space | AgentKey::Character(_) => "Space",
        AgentKey::ArrowUp => "UpArrow",
        AgentKey::ArrowDown => "DownArrow",
        AgentKey::ArrowLeft => "LeftArrow",
        AgentKey::ArrowRight => "RightArrow",
        AgentKey::Home => "Home",
        AgentKey::End => "End",
        AgentKey::PageUp => "PageUp",
        AgentKey::PageDown => "PageDown",
    }
}

fn map_automation_error(error: &Value) -> ToolError {
    let raw_message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("automation command failed");
    let code_name = raw_message.split(';').next().map_or(raw_message, str::trim);
    match code_name {
        "Timeout" => ToolError::new(ToolErrorCode::TimedOut, "automation command timed out"),
        "NodeNotFound" | "FrameNotFound" | "WindowNotFound" | "TargetOutOfBounds" => {
            ToolError::new(
                ToolErrorCode::StaleDocument,
                "target document or element is no longer available",
            )
        }
        "InvalidParameter" | "InvalidSelector" | "InvalidElementState" => ToolError::new(
            ToolErrorCode::InvalidParams,
            "invalid automation command target or state",
        ),
        "NotImplemented" => ToolError::new(
            ToolErrorCode::UnsupportedCapability,
            "operation is unsupported by WebKitGTK Automation",
        ),
        _ => ToolError::new(ToolErrorCode::Unavailable, "automation command failed"),
    }
}

fn is_valid_handle(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_HANDLE_BYTES
        && !value.bytes().any(|b| b == 0 || b.is_ascii_control())
}

fn decode_base64_bounded(input: &str, max_bytes: usize) -> Result<Vec<u8>, ToolError> {
    let mut out = Vec::with_capacity((input.len() / 4).saturating_mul(3).min(max_bytes));
    let mut buf = [0_u8; 4];
    let mut count = 0usize;
    let mut saw_pad = false;

    for byte in input.bytes() {
        if matches!(byte, b'\r' | b'\n' | b' ' | b'\t') {
            continue;
        }
        if saw_pad && byte != b'=' {
            return Err(ToolError::new(
                ToolErrorCode::Unavailable,
                "invalid base64 screenshot payload",
            ));
        }
        if byte == b'=' {
            saw_pad = true;
            buf[count] = 0xFF;
        } else {
            let val = decode_b64_digit(byte).ok_or_else(|| {
                ToolError::new(
                    ToolErrorCode::Unavailable,
                    "invalid base64 screenshot payload",
                )
            })?;
            buf[count] = val;
        }
        count += 1;
        if count == 4 {
            let emitted = flush_b64_quad(buf, &mut out)?;
            if out.len() > max_bytes {
                return Err(ToolError::new(
                    ToolErrorCode::BoundsExceeded,
                    "screenshot payload exceeds maximum byte bound",
                ));
            }
            if emitted < 3 {
                saw_pad = true;
            }
            count = 0;
        }
    }

    if count != 0 {
        return Err(ToolError::new(
            ToolErrorCode::Unavailable,
            "truncated base64 screenshot payload",
        ));
    }
    Ok(out)
}

fn decode_b64_digit(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn flush_b64_quad(quad: [u8; 4], out: &mut Vec<u8>) -> Result<usize, ToolError> {
    if quad[0] == 0xFF || quad[1] == 0xFF {
        return Err(ToolError::new(
            ToolErrorCode::Unavailable,
            "invalid base64 padding in screenshot payload",
        ));
    }
    let b0 = (quad[0] << 2) | (quad[1] >> 4);
    out.push(b0);
    if quad[2] == 0xFF {
        if quad[3] != 0xFF {
            return Err(ToolError::new(
                ToolErrorCode::Unavailable,
                "invalid base64 padding in screenshot payload",
            ));
        }
        return Ok(1);
    }
    let b1 = (quad[1] << 4) | (quad[2] >> 2);
    out.push(b1);
    if quad[3] == 0xFF {
        return Ok(2);
    }
    let b2 = (quad[2] << 6) | quad[3];
    out.push(b2);
    Ok(3)
}

fn parse_png_dimensions(bytes: &[u8]) -> Result<(u32, u32), ToolError> {
    if bytes.len() < 24 || !bytes.starts_with(PNG_SIGNATURE) || &bytes[12..16] != b"IHDR" {
        return Err(ToolError::new(
            ToolErrorCode::Unavailable,
            "host returned invalid PNG screenshot payload",
        ));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width == 0 || width > MAX_VIEWPORT_WIDTH || height == 0 || height > MAX_VIEWPORT_HEIGHT {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            "screenshot dimensions exceed allowed bounds",
        ));
    }
    Ok((width, height))
}
