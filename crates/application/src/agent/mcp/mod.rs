//! Shared read-only diagnostic state and contracts for the in-process MCP broker.
//!
//! This module is pure application state: no async runtime, no JSON or transport encoding, and no
//! platform toolkit calls. It owns diagnostic grants over `Profile::Main` tabs, document bindings,
//! revocation transitions, stale-callback validation, bounded event rings, and redacted network
//! metadata.

mod events;
mod state;
#[cfg(test)]
mod tests;
mod types;

pub use events::{
    ConsoleLevel, DiagnosticEvent, DiagnosticEventBatch, DiagnosticEventData,
    DiagnosticEventFilter, DiagnosticEventKind, LifecycleEventKind, NetworkEndpoint, NetworkEvent,
    NetworkPhase,
};
pub use state::DiagnosticState;
pub use types::{
    AgentContextId, AttachmentId, BoundedSnapshot, ClientId, DiagnosticCapabilities,
    DiagnosticError, DocumentBinding, EnabledAttachment, GrantScope, MAX_CLIENTS,
    MAX_EVENTS_PER_ATTACHMENT, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_DEPTH, MAX_SNAPSHOT_NODES,
    MAX_TEXT_EVENT_BYTES, NavigationKind, ObservationTarget, RevokeReason, RevokedAttachment,
    SnapshotKind, SnapshotRequest,
};
