//! Pure containment, approval policy, PNG validation, and bounded event ring for isolated
//! `Profile::Agent` MCP contexts.

use std::collections::VecDeque;

use quickie_application::mcp::{
    DiagnosticCapabilities, DiagnosticEvent, DiagnosticEventBatch, DiagnosticEventData,
    DiagnosticEventFilter, MAX_EVENTS_PER_ATTACHMENT,
};
use quickie_mcp::browser::{
    AgentAction, AgentKey, ElementBounds, MAX_VIEWPORT_HEIGHT, MAX_VIEWPORT_WIDTH,
};
use quickie_mcp::{ToolError, ToolErrorCode};
use url::{Host, Origin, Url};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const PNG_MIN_IHDR_LEN: usize = 24;

/// Honest runtime capabilities for isolated `Profile::Agent` contexts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AgentCapabilities {
    /// Whether native isolated automation (`_WKAutomationSession`) is available.
    pub(crate) isolated_automation: bool,
    /// Diagnostic observation capabilities available on isolated agent contexts.
    pub(crate) observations: DiagnosticCapabilities,
}

/// Neutral engine error returned by [`super::AgentSession`] implementations.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "main-webkit"),
    expect(
        dead_code,
        reason = "variants are constructed by the WebKit backend when main-webkit is enabled"
    )
)]
pub(crate) enum AgentEngineError {
    Unsupported(&'static str),
    InvalidCommand(&'static str),
    OriginMismatch,
    Busy,
    Timeout,
    Cancelled,
    Disconnected,
    BoundsExceeded(&'static str),
    Failed(String),
}

impl AgentEngineError {
    /// Maps an engine error to a neutral structured MCP [`ToolError`].
    #[must_use]
    pub(crate) fn into_tool_error(self) -> ToolError {
        match self {
            Self::Unsupported(msg) => ToolError::new(ToolErrorCode::UnsupportedCapability, msg),
            Self::InvalidCommand(msg) => ToolError::new(ToolErrorCode::InvalidParams, msg),
            Self::OriginMismatch => ToolError::new(
                ToolErrorCode::ApprovalDenied,
                "navigation URL does not match the approved origin",
            ),
            Self::Busy => ToolError::new(
                ToolErrorCode::Busy,
                "agent context is busy with another operation",
            ),
            Self::Timeout => ToolError::new(ToolErrorCode::TimedOut, "agent operation timed out"),
            Self::Cancelled => {
                ToolError::new(ToolErrorCode::Cancelled, "agent operation was cancelled")
            }
            Self::Disconnected => ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "agent context is closed or disconnected",
            ),
            Self::BoundsExceeded(msg) => ToolError::new(ToolErrorCode::BoundsExceeded, msg),
            Self::Failed(msg) => ToolError::new(ToolErrorCode::Unavailable, msg),
        }
    }
}

/// Typed containment/provenance signals extracted from an isolated-world DOM element.
#[derive(Clone, Debug, Default, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag represents an independent typed DOM containment signal"
)]
pub(crate) struct AgentSignals {
    pub(crate) connected: bool,
    pub(crate) visible: bool,
    pub(crate) is_submit_control: bool,
    pub(crate) in_form: bool,
    pub(crate) is_file_input: bool,
    pub(crate) is_editable: bool,
    pub(crate) is_select: bool,
    pub(crate) link_url: Option<String>,
    pub(crate) link_opens_new_window: bool,
    pub(crate) action_point: Option<(f64, f64)>,
}

/// One element match produced by [`super::AgentSession::locate_elements`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentLocatedItem {
    pub(crate) element_id: String,
    pub(crate) role: String,
    pub(crate) name: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) bounds: Option<ElementBounds>,
    pub(crate) signals: AgentSignals,
}

/// Bounded locator result returned by [`super::AgentSession::locate_elements`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentLocateOutput {
    pub(crate) elements: Vec<AgentLocatedItem>,
    pub(crate) truncated: bool,
}

/// Bounded post-action page state returned by [`super::AgentSession::observe_state`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AgentObservedOutput {
    pub(crate) focused_role: Option<String>,
    pub(crate) value_length: Option<usize>,
    pub(crate) scroll_x: Option<i32>,
    pub(crate) scroll_y: Option<i32>,
}

/// Containment and approval decision for an isolated Agent navigation or action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalDecision {
    /// Allowed immediately within the context's already-approved origin boundary.
    Allowed,
    /// Requires explicit local user approval before proceeding.
    RequiresApproval { origin: Origin, message: String },
    /// Disallowed by containment policy.
    Rejected { reason: String },
}

/// Validates that `url` is a credential-free HTTP(S) URL with a valid tuple origin.
#[must_use]
pub(crate) fn is_valid_http_url(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https")
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    matches!(
        url.origin(),
        Origin::Tuple(scheme, host, _)
            if matches!(scheme.as_str(), "http" | "https")
                && !matches!(&host, Host::Domain(domain) if domain.is_empty())
    )
}

/// Evaluates Quickie's origin approval policy for `agent_navigate`.
///
/// Initial activation is always explicit: neither remote origins nor loopback (`127.0.0.1`,
/// `localhost`, `[::1]`) are implicitly authorized. Once an origin is explicitly approved for
/// this context, subsequent same-origin HTTP(S) navigations proceed without re-prompting.
#[must_use]
pub(crate) fn evaluate_navigation_policy(
    approved_origin: Option<&Origin>,
    url: &Url,
) -> ApprovalDecision {
    if !is_valid_http_url(url) {
        return ApprovalDecision::Rejected {
            reason: "navigation requires a credential-free HTTP(S) URL".to_owned(),
        };
    }
    let origin = url.origin();
    if approved_origin == Some(&origin) {
        return ApprovalDecision::Allowed;
    }
    ApprovalDecision::RequiresApproval {
        message: format!(
            "Allow isolated Agent context to navigate to {}?",
            origin.ascii_serialization()
        ),
        origin,
    }
}

/// Evaluates Quickie's containment and approval policy for `agent_act` using typed DOM signals.
#[must_use]
pub(crate) fn evaluate_action_policy(
    action: &AgentAction,
    signals: &AgentSignals,
    approved_origin: Option<&Origin>,
) -> ApprovalDecision {
    if signals.is_file_input {
        return ApprovalDecision::Rejected {
            reason: "file chooser input is not permitted in isolated agent contexts".to_owned(),
        };
    }

    match action {
        AgentAction::Click { .. } | AgentAction::DoubleClick { .. } => {
            if signals.is_submit_control {
                return approval_or_reject(
                    approved_origin,
                    "Allow isolated Agent to submit a form on this page?",
                );
            }
            if signals.link_opens_new_window {
                return approval_or_reject(
                    approved_origin,
                    "Allow isolated Agent to activate a link that opens a new window?",
                );
            }
            if let Some(raw_href) = &signals.link_url {
                let Ok(parsed) = Url::parse(raw_href) else {
                    return ApprovalDecision::Rejected {
                        reason: "link target URL is invalid".to_owned(),
                    };
                };
                if !is_valid_http_url(&parsed) {
                    return ApprovalDecision::Rejected {
                        reason: "non-HTTP(S) link navigation is not permitted".to_owned(),
                    };
                }
                let link_origin = parsed.origin();
                if approved_origin != Some(&link_origin) {
                    return ApprovalDecision::RequiresApproval {
                        message: format!(
                            "Allow isolated Agent to follow a cross-origin link to {}?",
                            link_origin.ascii_serialization()
                        ),
                        origin: link_origin,
                    };
                }
            }
            ApprovalDecision::Allowed
        }
        AgentAction::Fill { .. } => {
            if !signals.is_editable {
                return ApprovalDecision::Rejected {
                    reason: "target element is not editable".to_owned(),
                };
            }
            ApprovalDecision::Allowed
        }
        AgentAction::PressKey {
            key: AgentKey::Enter,
            ..
        } => {
            if signals.in_form || signals.is_submit_control {
                return approval_or_reject(
                    approved_origin,
                    "Allow isolated Agent to press Enter inside a form?",
                );
            }
            ApprovalDecision::Allowed
        }
        AgentAction::SelectOption { .. } => {
            if !signals.is_select {
                return ApprovalDecision::Rejected {
                    reason: "target element is not a select control".to_owned(),
                };
            }
            ApprovalDecision::Allowed
        }
        AgentAction::Hover { .. }
        | AgentAction::Type { .. }
        | AgentAction::PressKey { .. }
        | AgentAction::Scroll { .. } => ApprovalDecision::Allowed,
    }
}

fn approval_or_reject(approved_origin: Option<&Origin>, message: &str) -> ApprovalDecision {
    match approved_origin {
        Some(origin) => ApprovalDecision::RequiresApproval {
            origin: origin.clone(),
            message: message.to_owned(),
        },
        None => ApprovalDecision::Rejected {
            reason: "no approved HTTP(S) origin is active in this agent context".to_owned(),
        },
    }
}

/// Decodes a base64 PNG screenshot payload, enforces `max_bytes`, validates the PNG signature,
/// and extracts `(width, height)` from the `IHDR` header.
///
/// # Errors
///
/// Returns [`ToolError`] with `BoundsExceeded` if the decoded PNG exceeds `max_bytes`, or
/// `Unavailable` if the base64 or PNG header is malformed.
pub(crate) fn decode_png_base64(
    encoded: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, u32, u32), ToolError> {
    let bytes = base64_decode(encoded, max_bytes)?;
    if bytes.len() < PNG_MIN_IHDR_LEN || &bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err(ToolError::new(
            ToolErrorCode::Unavailable,
            "engine returned an invalid PNG screenshot payload",
        ));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width == 0 || height == 0 || width > MAX_VIEWPORT_WIDTH || height > MAX_VIEWPORT_HEIGHT {
        return Err(ToolError::new(
            ToolErrorCode::BoundsExceeded,
            "screenshot dimensions exceed allowed bounds",
        ));
    }
    Ok((bytes, width, height))
}

fn base64_decode(input: &str, max_bytes: usize) -> Result<Vec<u8>, ToolError> {
    let mut out = Vec::with_capacity((input.len() / 4).saturating_mul(3).min(max_bytes));
    let mut buf = [0_u8; 4];
    let mut count = 0_usize;
    let mut saw_pad = false;

    for byte in input.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'=' {
            saw_pad = true;
            buf[count] = 0;
            count += 1;
        } else {
            if saw_pad {
                return Err(ToolError::new(
                    ToolErrorCode::Unavailable,
                    "invalid base64 padding in screenshot",
                ));
            }
            let val = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => {
                    return Err(ToolError::new(
                        ToolErrorCode::Unavailable,
                        "invalid base64 character in screenshot",
                    ));
                }
            };
            buf[count] = val;
            count += 1;
        }

        if count == 4 {
            let b0 = (buf[0] << 2) | (buf[1] >> 4);
            let b1 = (buf[1] << 4) | (buf[2] >> 2);
            let b2 = (buf[2] << 6) | buf[3];
            let emit = if saw_pad {
                // Determine exact emitted byte count from trailing '=' count in input.
                if input.trim_end().ends_with("==") {
                    1
                } else {
                    2
                }
            } else {
                3
            };
            if out.len().saturating_add(emit) > max_bytes {
                return Err(ToolError::new(
                    ToolErrorCode::BoundsExceeded,
                    "screenshot exceeds requested max_bytes",
                ));
            }
            out.push(b0);
            if emit >= 2 {
                out.push(b1);
            }
            if emit == 3 {
                out.push(b2);
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

/// Bounded per-context diagnostic event ring matching `DiagnosticState`'s replay semantics.
#[derive(Debug, Default)]
pub(crate) struct AgentEventBuffer {
    events: VecDeque<DiagnosticEvent>,
    last_seq: u64,
}

impl AgentEventBuffer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, epoch: u64, data: DiagnosticEventData) -> u64 {
        self.last_seq = self.last_seq.saturating_add(1);
        if self.events.len() == MAX_EVENTS_PER_ATTACHMENT {
            self.events.pop_front();
        }
        self.events.push_back(DiagnosticEvent {
            seq: self.last_seq,
            epoch,
            data,
        });
        self.last_seq
    }

    /// Reads an ordered page of events after `cursor`.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] with `InvalidParams` if `cursor` is ahead of the stream.
    pub(crate) fn read(
        &self,
        cursor: u64,
        filter: &DiagnosticEventFilter,
        limit: usize,
    ) -> Result<DiagnosticEventBatch, ToolError> {
        if cursor > self.last_seq {
            return Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "diagnostic event cursor is ahead of stream",
            ));
        }

        let dropped_before = self
            .events
            .front()
            .map(|oldest| oldest.seq)
            .filter(|&oldest| cursor.saturating_add(1) < oldest);

        let mut events = Vec::new();
        let mut has_more = false;

        for event in self.events.iter().filter(|event| event.seq > cursor) {
            if !filter.matches(event) {
                continue;
            }
            if events.len() < limit {
                events.push(event.clone());
            } else {
                has_more = true;
                break;
            }
        }

        let next_cursor = if has_more {
            events.last().map_or(cursor, |event| event.seq)
        } else {
            self.last_seq
        };

        Ok(DiagnosticEventBatch {
            events,
            next_cursor,
            has_more,
            dropped_before,
        })
    }
}
