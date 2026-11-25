//! Allowlisted Inspector subscription translator between `ObservationDecoder` and `ObservationLease`.

use serde_json::Value;

/// Allowlisted direct Inspector subscription methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectMethod {
    NetworkEnable,
    NetworkDisable,
    PageEnable,
    PageDisable,
}

/// Allowlisted target-routed Inspector subscription methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetMethod {
    ConsoleEnable,
    ConsoleDisable,
    RuntimeEnable,
    RuntimeDisable,
    NetworkEnable,
    NetworkDisable,
    PageEnable,
    PageDisable,
}

/// Structurally validated subscription command ready for the native GTK observation lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecodedSubscription {
    Direct {
        id: u64,
        method: DirectMethod,
    },
    Target {
        id: u64,
        target_id: String,
        inner_id: u64,
        method: TargetMethod,
    },
}

/// Parses a JSON subscription command emitted by [`quickie_engine_webkit::observation::ObservationDecoder`]
/// into a strictly allowlisted [`DecodedSubscription`].
///
/// Rejects any unknown method, missing ID, or malformed envelope so arbitrary JSON never reaches
/// the native Inspector SPI.
#[must_use]
pub(crate) fn parse_allowlisted_command(raw: &str) -> Option<DecodedSubscription> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let object = value.as_object()?;
    let id = object.get("id")?.as_u64()?;
    let method = object.get("method")?.as_str()?;

    if method == "Target.sendMessageToTarget" {
        let params = object.get("params")?.as_object()?;
        let target_id = params.get("targetId")?.as_str()?.to_owned();
        let inner_raw = params.get("message")?.as_str()?;
        let inner_value: Value = serde_json::from_str(inner_raw).ok()?;
        let inner = inner_value.as_object()?;
        let inner_id = inner.get("id")?.as_u64()?;
        let inner_method = match inner.get("method")?.as_str()? {
            "Console.enable" => TargetMethod::ConsoleEnable,
            "Console.disable" => TargetMethod::ConsoleDisable,
            "Runtime.enable" => TargetMethod::RuntimeEnable,
            "Runtime.disable" => TargetMethod::RuntimeDisable,
            "Network.enable" => TargetMethod::NetworkEnable,
            "Network.disable" => TargetMethod::NetworkDisable,
            "Page.enable" => TargetMethod::PageEnable,
            "Page.disable" => TargetMethod::PageDisable,
            _ => return None,
        };
        return Some(DecodedSubscription::Target {
            id,
            target_id,
            inner_id,
            method: inner_method,
        });
    }

    let direct_method = match method {
        "Network.enable" => DirectMethod::NetworkEnable,
        "Network.disable" => DirectMethod::NetworkDisable,
        "Page.enable" => DirectMethod::PageEnable,
        "Page.disable" => DirectMethod::PageDisable,
        _ => return None,
    };
    Some(DecodedSubscription::Direct {
        id,
        method: direct_method,
    })
}
