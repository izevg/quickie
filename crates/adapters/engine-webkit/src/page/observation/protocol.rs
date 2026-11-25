//! Typed read-only `WebKit` Inspector commands and bounded JSON envelope helpers.
//!
//! Protocol method names and payload fields match the pinned `WebKit` revision
//! (`702da9c837a3e24f7d87f4824a5d12e9ff3db923`) and the fork's `LocalDiagnosticInspectorPolicy.h`.

use quickie_application::DiagnosticGapReason;
use serde_json::{Map, Value, json};

/// Maximum accepted UTF-8 byte length of a single incoming Inspector protocol JSON message.
pub const MAX_PROTOCOL_MESSAGE_BYTES: usize = 256 * 1024;

/// Maximum accepted UTF-8 byte length of an Inspector target identifier.
pub const MAX_TARGET_ID_BYTES: usize = 256;

/// Read-only Inspector protocol domains the local diagnostic observer may subscribe to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObservationDomain {
    /// JavaScript console messages and uncaught script errors (`Console.messageAdded`).
    Console,
    /// JavaScript execution context lifecycle (`Runtime.enable` / `Runtime.disable`).
    Runtime,
    /// Redacted HTTP(S) request metadata (`Network.*`).
    Network,
    /// Document load and DOM-ready milestones (`Page.*`).
    Page,
}

/// Fixed, allowlisted read-only observation subscription commands.
///
/// Clients never pass raw JSON or arbitrary Inspector methods; every outgoing command is built
/// from this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObserveCommand {
    /// Enables event delivery for a read-only observation domain.
    Enable(ObservationDomain),
    /// Disables event delivery for a read-only observation domain.
    Disable(ObservationDomain),
}

impl ObserveCommand {
    /// Exact `WebKit` Inspector protocol method name for this command.
    #[must_use]
    pub const fn method_name(self) -> &'static str {
        match self {
            Self::Enable(ObservationDomain::Console) => "Console.enable",
            Self::Disable(ObservationDomain::Console) => "Console.disable",
            Self::Enable(ObservationDomain::Runtime) => "Runtime.enable",
            Self::Disable(ObservationDomain::Runtime) => "Runtime.disable",
            Self::Enable(ObservationDomain::Network) => "Network.enable",
            Self::Disable(ObservationDomain::Network) => "Network.disable",
            Self::Enable(ObservationDomain::Page) => "Page.enable",
            Self::Disable(ObservationDomain::Page) => "Page.disable",
        }
    }

    /// Whether `WebKit`'s local diagnostic policy allows sending this command directly (outside
    /// `Target.sendMessageToTarget`) when the page controller proxies the agent.
    #[must_use]
    pub const fn is_direct_allowed(self) -> bool {
        matches!(
            self,
            Self::Enable(ObservationDomain::Network | ObservationDomain::Page)
                | Self::Disable(ObservationDomain::Network | ObservationDomain::Page)
        )
    }

    /// All target-scoped read-only enable commands issued when a live page or frame target attaches.
    #[must_use]
    pub const fn target_enables() -> [Self; 4] {
        [
            Self::Enable(ObservationDomain::Console),
            Self::Enable(ObservationDomain::Runtime),
            Self::Enable(ObservationDomain::Network),
            Self::Enable(ObservationDomain::Page),
        ]
    }

    /// Direct page-controller enable commands allowed by the local diagnostic policy.
    #[must_use]
    pub const fn direct_enables() -> [Self; 2] {
        [
            Self::Enable(ObservationDomain::Network),
            Self::Enable(ObservationDomain::Page),
        ]
    }
}

/// Encodes an allowlisted direct Inspector command (only `Network.*` and `Page.*`).
#[must_use]
pub(super) fn encode_direct(id: u64, command: ObserveCommand) -> Option<String> {
    if !command.is_direct_allowed() {
        return None;
    }
    Some(
        json!({
            "id": id,
            "method": command.method_name(),
        })
        .to_string(),
    )
}

/// Encodes an allowlisted target-scoped Inspector command wrapped in `Target.sendMessageToTarget`.
#[must_use]
pub(super) fn encode_for_target(
    outer_id: u64,
    target_id: &str,
    inner_id: u64,
    command: ObserveCommand,
) -> Option<String> {
    if !is_valid_target_id(target_id) {
        return None;
    }
    let inner = json!({
        "id": inner_id,
        "method": command.method_name(),
    })
    .to_string();
    Some(
        json!({
            "id": outer_id,
            "method": "Target.sendMessageToTarget",
            "params": {
                "targetId": target_id,
                "message": inner,
            },
        })
        .to_string(),
    )
}

/// Validates an Inspector target identifier against the fork's length and non-empty bounds.
#[must_use]
pub(super) const fn is_valid_target_id(target_id: &str) -> bool {
    !target_id.is_empty() && target_id.len() <= MAX_TARGET_ID_BYTES
}

/// Parses a bounded JSON protocol object, rejecting empty, oversized, or non-object inputs.
#[must_use]
pub(super) fn parse_bounded_object(raw: &str) -> Option<Map<String, Value>> {
    if raw.is_empty() || raw.len() > MAX_PROTOCOL_MESSAGE_BYTES {
        return None;
    }
    match serde_json::from_str::<Value>(raw).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Decodes a native `LocalDiagnostic.*` notification emitted by the `WebKit` fork SPI.
#[must_use]
pub(super) fn decode_local_diagnostic_gap(
    method: &str,
    params: Option<&Value>,
) -> Option<DiagnosticGapReason> {
    let suffix = method.strip_prefix("LocalDiagnostic.")?;
    if suffix == "queueOverflow" {
        let dropped = params
            .and_then(|p| p.get("droppedCount"))
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let total_dropped = params
            .and_then(|p| p.get("totalDroppedCount"))
            .and_then(Value::as_u64)
            .unwrap_or(dropped);
        return Some(DiagnosticGapReason::NativeQueueOverflow {
            dropped,
            total_dropped,
        });
    }
    Some(DiagnosticGapReason::ChannelClosed)
}
