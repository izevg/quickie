//! Core identifiers, bindings, capabilities, snapshot bounds, and errors for MCP diagnostics.

use std::fmt;

use quickie_domain_browsing::{Profile, TabId};
use url::{Host, Origin};

/// Maximum number of concurrent diagnostic clients retained by [`super::DiagnosticState`].
pub const MAX_CLIENTS: usize = 16;

/// Maximum number of events retained per active diagnostic attachment.
pub const MAX_EVENTS_PER_ATTACHMENT: usize = 1_024;

/// Maximum UTF-8 byte length of a textual diagnostic event payload (4 KiB).
pub const MAX_TEXT_EVENT_BYTES: usize = 4 * 1024;

/// Maximum number of nodes a bounded snapshot may request or return.
pub const MAX_SNAPSHOT_NODES: usize = 1_500;

/// Maximum tree depth a bounded snapshot may request or return.
pub const MAX_SNAPSHOT_DEPTH: usize = 24;

/// Maximum UTF-8 byte length of serialized snapshot data (64 KiB).
pub const MAX_SNAPSHOT_BYTES: usize = 64 * 1024;

/// Host-assigned identifier for a connected MCP client session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(u64);

impl ClientId {
    /// Creates a client identifier issued by the host when a client connects.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The underlying numeric identifier.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "client#{}", self.0)
    }
}

/// Identifier for an approved read-only diagnostic attachment on a `Main` tab.
///
/// Issued only by [`super::DiagnosticState`]. A raw wire value can be wrapped via
/// [`Self::from_wire`], but every lookup requires the owning [`ClientId`]; knowledge of the
/// numeric value alone grants nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttachmentId(u64);

impl AttachmentId {
    /// Allocates a broker-issued attachment identifier.
    pub(super) const fn issue(raw: u64) -> Self {
        Self(raw)
    }

    /// Wraps an untrusted numeric value received over the wire for ownership-checked lookup.
    #[must_use]
    pub const fn from_wire(raw: u64) -> Self {
        Self(raw)
    }

    /// The underlying numeric identifier for wire serialization.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AttachmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "attachment#{}", self.0)
    }
}

/// Host-assigned identifier for an isolated browsing context inside `Profile::Agent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentContextId(u64);

impl AgentContextId {
    /// Creates an identifier for a host-created isolated Agent context.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The underlying numeric identifier.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AgentContextId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "agent-context#{}", self.0)
    }
}

/// Distinguishes read-only `Main` diagnostic attachments from isolated `Agent` contexts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObservationTarget {
    /// An explicitly granted read-only diagnostic attachment on a `Main` tab.
    Diagnostic(AttachmentId),
    /// A host-created isolated browsing context in `Profile::Agent`.
    Agent(AgentContextId),
}

/// How long a user-approved diagnostic grant survives main-frame navigations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantScope {
    /// Revokes on every new main-frame document, including reloads.
    SingleDocument,
    /// Survives a same-origin, same-tab reload with a strictly higher document epoch.
    SameOriginReload,
}

/// The exact tab, HTTP(S) tuple origin, and document epoch a diagnostic grant is bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentBinding {
    /// The bound tab; must belong to `Profile::Main`.
    pub tab: TabId,
    /// The bound HTTP(S) tuple origin.
    pub origin: Origin,
    /// Monotonic main-frame document epoch within the tab.
    pub epoch: u64,
}

impl DocumentBinding {
    /// Whether this binding targets a `Profile::Main` tab with a valid HTTP(S) tuple origin.
    #[must_use]
    pub fn is_valid_main(&self) -> bool {
        self.tab.profile() == Profile::Main && is_valid_http_origin(&self.origin)
    }
}

/// Classification of a main-frame navigation observed on a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationKind {
    /// Same-document history mutation (`pushState`, `replaceState`, fragment change).
    SameDocument,
    /// Explicit or user-triggered reload of the current document.
    Reload,
    /// Navigation committing a new main-frame document.
    NewDocument,
}

/// Why an active diagnostic attachment was revoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevokeReason {
    /// Explicitly revoked by the user in native UI.
    UserRevoked,
    /// Voluntarily detached by the owning client.
    ClientDetached,
    /// Replaced by a newer approved attachment for the same client.
    Replaced,
    /// Revoked because the tab navigated to a new document outside the grant scope.
    NavigatedAway,
    /// Revoked because the tab navigated to a different or non-HTTP(S) origin.
    CrossOriginNavigation,
    /// Revoked because the bound tab closed.
    TabClosed,
    /// Revoked because the tab's web content process terminated.
    ProcessCrashed,
    /// Revoked because the owning MCP client disconnected.
    ClientDisconnected,
    /// Revoked because the browser is shutting down.
    BrowserShuttingDown,
}

/// Handle returned to the host whenever an attachment is released so it can detach observers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RevokedAttachment {
    /// The retired attachment identifier (never reassigned).
    pub attachment: AttachmentId,
    /// The client that owned the attachment.
    pub client: ClientId,
    /// The `Main` tab the attachment observed.
    pub tab: TabId,
    /// Why the attachment was revoked.
    pub reason: RevokeReason,
}

/// Result of enabling a confirmed diagnostic grant for a client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnabledAttachment {
    /// The newly issued attachment identifier.
    pub attachment: AttachmentId,
    /// The client's previous attachment if one was active and replaced.
    pub replaced: Option<RevokedAttachment>,
}

/// Diagnostic observation capabilities supported by the host runtime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag represents an independent diagnostic channel capability"
)]
pub struct DiagnosticCapabilities {
    /// Read-only DOM tree snapshots.
    pub dom: bool,
    /// Read-only accessibility tree snapshots.
    pub accessibility: bool,
    /// Console message events.
    pub console: bool,
    /// Uncaught runtime error events.
    pub runtime_errors: bool,
    /// In-flight/pending network request metadata.
    pub pending_network: bool,
    /// Completed network request metadata.
    pub completed_network: bool,
    /// Page lifecycle events.
    pub lifecycle: bool,
}

/// Which read-only snapshot projection to capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotKind {
    /// Bounded DOM tree.
    Dom,
    /// Bounded accessibility tree.
    Accessibility,
    /// Bounded visible text.
    Text,
}

/// Bounded request for a read-only page snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRequest {
    kind: SnapshotKind,
    selector: Option<String>,
    max_nodes: usize,
    max_depth: usize,
    max_bytes: usize,
}

impl SnapshotRequest {
    /// Creates a snapshot request with node, depth, and byte limits clamped to the hard ceilings.
    #[must_use]
    pub fn new(
        kind: SnapshotKind,
        selector: Option<String>,
        max_nodes: usize,
        max_depth: usize,
        max_bytes: usize,
    ) -> Self {
        Self {
            kind,
            selector,
            max_nodes: max_nodes.min(MAX_SNAPSHOT_NODES),
            max_depth: max_depth.min(MAX_SNAPSHOT_DEPTH),
            max_bytes: max_bytes.min(MAX_SNAPSHOT_BYTES),
        }
    }

    /// Creates a snapshot request using the maximum allowed bounds.
    #[must_use]
    pub fn with_max_bounds(kind: SnapshotKind, selector: Option<String>) -> Self {
        Self::new(
            kind,
            selector,
            MAX_SNAPSHOT_NODES,
            MAX_SNAPSHOT_DEPTH,
            MAX_SNAPSHOT_BYTES,
        )
    }

    /// Requested snapshot projection.
    #[must_use]
    pub const fn kind(&self) -> SnapshotKind {
        self.kind
    }

    /// Optional root CSS selector.
    #[must_use]
    pub fn selector(&self) -> Option<&str> {
        self.selector.as_deref()
    }

    /// Clamped node ceiling (`<= MAX_SNAPSHOT_NODES`).
    #[must_use]
    pub const fn max_nodes(&self) -> usize {
        self.max_nodes
    }

    /// Clamped depth ceiling (`<= MAX_SNAPSHOT_DEPTH`).
    #[must_use]
    pub const fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// Clamped serialized byte ceiling (`<= MAX_SNAPSHOT_BYTES`).
    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

/// Bounded read-only snapshot payload tied to a specific document epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedSnapshot {
    epoch: u64,
    data: String,
    truncated: bool,
}

impl BoundedSnapshot {
    /// Constructs a snapshot, clamping `data` to `MAX_SNAPSHOT_BYTES` on a UTF-8 boundary.
    #[must_use]
    pub fn new(epoch: u64, data: &str, truncated: bool) -> Self {
        let (bounded, did_truncate) = truncate_utf8(data, MAX_SNAPSHOT_BYTES);
        Self {
            epoch,
            data: bounded,
            truncated: truncated || did_truncate,
        }
    }

    /// The document epoch from which this snapshot was captured.
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The bounded UTF-8 snapshot data (`len <= MAX_SNAPSHOT_BYTES`).
    #[must_use]
    pub fn data(&self) -> &str {
        &self.data
    }

    /// Whether the snapshot was truncated by node, depth, or byte bounds.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// Typed errors returned by [`super::DiagnosticState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiagnosticError {
    /// The attachment does not exist, was revoked, or belongs to a different client.
    #[error("diagnostic attachment is not available")]
    AttachmentUnavailable,
    /// The approval was stale, targeted a non-Main tab, or had an invalid origin.
    #[error("diagnostic approval is invalid or stale")]
    InvalidApproval,
    /// The requested event replay cursor is ahead of the attachment's stream.
    #[error("diagnostic event cursor is invalid")]
    InvalidCursor,
    /// The fixed concurrent client limit has been reached.
    #[error("diagnostic client capacity reached")]
    ClientCapacityReached,
    /// The monotonic attachment identifier space has been exhausted.
    #[error("diagnostic attachment identifier space exhausted")]
    AttachmentIdExhausted,
}

/// Returns whether `origin` is a valid HTTP or HTTPS tuple origin with a non-empty host.
pub(super) fn is_valid_http_origin(origin: &Origin) -> bool {
    match origin {
        Origin::Tuple(scheme, host, _port) => {
            matches!(scheme.as_str(), "http" | "https")
                && !matches!(host, Host::Domain(domain) if domain.is_empty())
        }
        Origin::Opaque(_) => false,
    }
}

/// Truncates `text` to at most `max_bytes` along a valid UTF-8 character boundary.
pub(super) fn truncate_utf8(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_owned(), false);
    }
    let end = text.floor_char_boundary(max_bytes);
    (text[..end].to_owned(), true)
}
