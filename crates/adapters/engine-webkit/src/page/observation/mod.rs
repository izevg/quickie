//! Cross-platform `WebKit` Inspector read-only observation codec and platform adapter leaf.
//!
//! The protocol command allowlist and event decoder ([`ObservationDecoder`]) compile on every
//! platform using `serde_json` only, so macOS and Linux share one exact implementation of target
//! lifecycle tracking, request correlation, URL redaction, and explicit trace-gap propagation.
//! The macOS-only Cocoa SPI binding lives in a private submodule.

#[cfg(target_os = "macos")]
pub(crate) mod cocoa;
mod decoder;
mod protocol;
#[cfg(test)]
mod tests;

pub use decoder::{DecodeOutcome, MAX_ACTIVE_TARGETS, MAX_TRACKED_REQUESTS, ObservationDecoder};
pub use protocol::{
    MAX_PROTOCOL_MESSAGE_BYTES, MAX_TARGET_ID_BYTES, ObservationDomain, ObserveCommand,
};
