//! Typed host boundary for the shared Quickie MCP browser handler.
//!
//! Composition roots (`AppKit` on macOS, `GTK` on Linux) implement [`BrowserHost`] to accept
//! typed [`BrowserOperation`] requests without blocking and complete their owned [`HostReply`]
//! handles on the native main loop.

use super::agent::AgentOperation;
use quickie_application::mcp::{
    AttachmentId, BoundedSnapshot, ClientId, DiagnosticCapabilities, DiagnosticError,
    DiagnosticEventBatch, DiagnosticEventFilter, GrantScope, ObservationTarget, RevokeReason,
    SnapshotKind, SnapshotRequest,
};
use thiserror::Error;
use tokio::sync::oneshot;

/// Maximum UTF-8 byte length of a neutral structured tool error message.
pub const MAX_ERROR_MESSAGE_BYTES: usize = 256;

/// Platform reported by the host runtime in `browser_status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostPlatform {
    /// macOS (`AppKit` + `WebKit`).
    Macos,
    /// Linux (`GTK` + `WebKitGTK`).
    Linux,
}

impl HostPlatform {
    /// Wire representation of the host platform.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }
}

/// Process runtime mode reported by the host in `browser_status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeMode {
    /// Connected to the running GUI browser process.
    Gui,
    /// Running as a standalone isolated agent process without opening the main profile.
    Standalone,
}

impl RuntimeMode {
    /// Wire representation of the runtime mode.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gui => "gui",
            Self::Standalone => "standalone",
        }
    }
}

/// Engine and fork capabilities reported honestly by the host runtime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForkCapabilities {
    /// Diagnostic observation channel capabilities.
    pub diagnostic: DiagnosticCapabilities,
    /// Whether native accessibility tree extraction is supported without DOM fallback.
    pub native_accessibility: bool,
    /// Whether the visual Web Inspector takes priority over MCP diagnostic attachments.
    pub visual_inspector_priority: bool,
    /// Whether isolated `Profile::Agent` automation contexts are supported.
    pub agent_automation: bool,
}

/// Host status returned for `browser_status`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserStatus {
    /// Host operating system / toolkit platform.
    pub platform: HostPlatform,
    /// Whether this server is running in the GUI or standalone mode.
    pub runtime_mode: RuntimeMode,
    /// Product version string reported by the host.
    pub product_version: String,
    /// Engine and fork capabilities available in this runtime.
    pub fork_capabilities: ForkCapabilities,
}

/// Approved read-only diagnostic attachment returned for `diagnostic_request`.
///
/// Carries only the sanitized HTTP(S) tuple origin and never exposes pre-approval tab titles,
/// URL paths, query strings, or page content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticGrant {
    /// Broker-issued diagnostic attachment identifier owned by this client.
    pub attachment_id: AttachmentId,
    /// Approved navigation scope for this attachment.
    pub scope: GrantScope,
    /// Monotonic main-frame document epoch captured at approval time.
    pub epoch: u64,
    /// Sanitized HTTP(S) tuple origin (for example `https://example.com`).
    pub origin: String,
    /// Diagnostic capabilities enabled for this attachment.
    pub capabilities: DiagnosticCapabilities,
}

/// Result returned for `diagnostic_detach`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticDetachResult {
    /// The detached attachment identifier.
    pub attachment_id: AttachmentId,
    /// Whether the attachment was detached.
    pub detached: bool,
    /// Revocation reason recorded by `DiagnosticState`.
    pub reason: RevokeReason,
}

/// Indicates whether an accessibility snapshot used native engine data or a semantic DOM view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessibilitySource {
    /// Native engine accessibility tree.
    Native,
    /// Semantic DOM fallback when native accessibility data is unavailable.
    SemanticDomFallback,
}

impl AccessibilitySource {
    /// Wire representation of the accessibility snapshot source.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::SemanticDomFallback => "semantic_dom_fallback",
        }
    }
}

/// Bounded read-only snapshot result returned for `page_snapshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageSnapshotResult {
    /// Requested snapshot projection kind.
    pub kind: SnapshotKind,
    /// Bounded UTF-8 snapshot tied to a specific document epoch.
    pub snapshot: BoundedSnapshot,
    /// Present for `SnapshotKind::Accessibility` to distinguish native vs semantic fallback.
    pub accessibility_source: Option<AccessibilitySource>,
}

/// Paginated diagnostic event replay result returned for `page_events`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageEventsResult {
    /// Ordered event batch and cursor metadata from `DiagnosticState`.
    pub batch: DiagnosticEventBatch,
    /// Active diagnostic capabilities; events for unset capabilities are filtered before output.
    pub capabilities: DiagnosticCapabilities,
    /// Whether the native observer reported upstream event overflow before reaching the ring.
    pub native_overflow: bool,
    /// Whether the target stream has closed or been revoked.
    pub closed: bool,
}

/// Stable short error codes serialized in structured MCP tool errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolErrorCode {
    /// Invalid or malformed tool arguments or cursor.
    InvalidParams,
    /// Requested capability or snapshot/event feature is not supported by the host.
    UnsupportedCapability,
    /// User denied diagnostic consent or no eligible document was approved.
    ApprovalDenied,
    /// Requested tool is not available in this runtime.
    ToolNotAvailable,
    /// Diagnostic attachment or target is unknown, revoked, or owned by another client.
    AttachmentUnavailable,
    /// Bound document changed or became stale before the operation completed.
    StaleDocument,
    /// Request exceeded hard snapshot or event bounds.
    BoundsExceeded,
    /// Host queue or per-client in-flight budget is full.
    Busy,
    /// Host did not complete the request within the configured timeout.
    TimedOut,
    /// Request was cancelled by the MCP client or session shutdown.
    Cancelled,
    /// Host is closed or dropped the request before completing it.
    Unavailable,
}

impl ToolErrorCode {
    /// Stable wire string for this error code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParams => "invalid_params",
            Self::UnsupportedCapability => "unsupported_capability",
            Self::ApprovalDenied => "approval_denied",
            Self::ToolNotAvailable => "tool_not_available",
            Self::AttachmentUnavailable => "attachment_unavailable",
            Self::StaleDocument => "stale_document",
            Self::BoundsExceeded => "bounds_exceeded",
            Self::Busy => "busy",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Neutral structured tool error returned across the host boundary.
///
/// Messages are sanitized and bounded so raw web content or control characters never leak into
/// MCP error responses.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct ToolError {
    code: ToolErrorCode,
    message: String,
}

impl ToolError {
    /// Creates a neutral structured tool error with a bounded, single-line message.
    #[must_use]
    pub fn new(code: ToolErrorCode, message: impl AsRef<str>) -> Self {
        Self {
            code,
            message: sanitize_error_message(message.as_ref()),
        }
    }

    /// Stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> ToolErrorCode {
        self.code
    }

    /// Short neutral human-readable error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<DiagnosticError> for ToolError {
    fn from(error: DiagnosticError) -> Self {
        let code = match error {
            DiagnosticError::AttachmentUnavailable => ToolErrorCode::AttachmentUnavailable,
            DiagnosticError::InvalidApproval => ToolErrorCode::StaleDocument,
            DiagnosticError::InvalidCursor => ToolErrorCode::InvalidParams,
            DiagnosticError::ClientCapacityReached => ToolErrorCode::Busy,
            DiagnosticError::AttachmentIdExhausted => ToolErrorCode::Unavailable,
        };
        Self::new(code, error.to_string())
    }
}

impl From<HostDispatchError> for ToolError {
    fn from(error: HostDispatchError) -> Self {
        match error {
            HostDispatchError::Full | HostDispatchError::Busy => {
                Self::new(ToolErrorCode::Busy, "browser host is busy")
            }
            HostDispatchError::Closed => {
                Self::new(ToolErrorCode::Unavailable, "browser host is unavailable")
            }
        }
    }
}

/// Immediate non-blocking failure returned when [`BrowserHost::dispatch`] cannot enqueue work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum HostDispatchError {
    /// The host's bounded dispatch queue is full.
    #[error("browser host queue is full")]
    Full,
    /// The host is temporarily busy and cannot accept another request.
    #[error("browser host is busy")]
    Busy,
    /// The host is shutting down or has closed its dispatch port.
    #[error("browser host is closed")]
    Closed,
}

/// Owned one-shot reply handle completed by the native host without blocking.
///
/// Completing a handle whose caller already cancelled, timed out, or disconnected is a no-op and
/// never panics.
#[derive(Debug)]
pub struct HostReply<T> {
    tx: oneshot::Sender<Result<T, ToolError>>,
}

impl<T> HostReply<T> {
    pub(crate) const fn new(tx: oneshot::Sender<Result<T, ToolError>>) -> Self {
        Self { tx }
    }

    /// Completes the pending tool request with `result`.
    ///
    /// Safe to call from any thread (including the native toolkit main loop). Late replies after
    /// cancellation, timeout, or disconnect are silently discarded.
    pub fn complete(self, result: Result<T, ToolError>) {
        let _ = self.tx.send(result);
    }

    /// Completes the pending tool request with a successful payload.
    pub fn resolve(self, value: T) {
        self.complete(Ok(value));
    }

    /// Completes the pending tool request with a structured [`ToolError`].
    pub fn reject(self, error: ToolError) {
        self.complete(Err(error));
    }

    /// Returns `true` if the waiting MCP request has already been cancelled, timed out, or closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

/// Typed browser operation dispatched from [`super::BrowserHandler`] to [`BrowserHost`].
///
/// Carries only application-owned data and an owned one-shot [`HostReply`]; never carries raw
/// JSON, platform handles, or untrusted caller-supplied client IDs.
#[derive(Debug)]
pub enum BrowserOperation {
    /// Query platform, runtime mode, product version, and fork capabilities.
    BrowserStatus {
        /// One-shot reply completed by the host.
        reply: HostReply<BrowserStatus>,
    },
    /// Request explicit native user consent to attach read-only diagnostics to the active tab.
    DiagnosticRequest {
        /// Requested navigation lifetime scope (`single_document` or `same_origin_reload`).
        scope: GrantScope,
        /// One-shot reply completed after native consent resolves.
        reply: HostReply<DiagnosticGrant>,
    },
    /// Detach an active diagnostic attachment owned by this client.
    DiagnosticDetach {
        /// Untrusted wire attachment ID; the host validates ownership against `ClientId`.
        attachment: AttachmentId,
        /// One-shot reply completed after detachment.
        reply: HostReply<DiagnosticDetachResult>,
    },
    /// Capture a bounded read-only snapshot from a diagnostic attachment or agent context.
    PageSnapshot {
        /// Tagged observation target (`diagnostic` or `agent`).
        target: ObservationTarget,
        /// Validated, bounded snapshot request.
        request: SnapshotRequest,
        /// One-shot reply completed with the bounded snapshot.
        reply: HostReply<PageSnapshotResult>,
    },
    /// Replay ordered diagnostic events from a diagnostic attachment or agent context.
    PageEvents {
        /// Tagged observation target (`diagnostic` or `agent`).
        target: ObservationTarget,
        /// Cursor starting at `0`.
        cursor: u64,
        /// Category filter (`kinds` empty means all categories).
        filter: DiagnosticEventFilter,
        /// Maximum number of events to return (`1..=MAX_EVENTS_PER_ATTACHMENT`).
        limit: usize,
        /// One-shot reply completed with the paginated event batch.
        reply: HostReply<PageEventsResult>,
    },
    /// Typed isolated `Profile::Agent` browsing context operation.
    Agent(AgentOperation),
}

/// External host interface implemented by the `AppKit` and `GTK` composition roots.
///
/// Implementations must be `Send + Sync`, accept or reject operations immediately without
/// blocking the MCP I/O thread, and notify/complete replies from their native main loop.
pub trait BrowserHost: Send + Sync + 'static {
    /// Immediately enqueues `operation` on behalf of `client` without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`HostDispatchError::Full`], [`HostDispatchError::Busy`], or
    /// [`HostDispatchError::Closed`] if the operation cannot be accepted.
    fn dispatch(
        &self,
        client: ClientId,
        operation: BrowserOperation,
    ) -> Result<(), HostDispatchError>;

    /// Notifies the host that `client` has disconnected and will not issue further operations.
    ///
    /// Called at most once on last handler ownership release, without blocking.
    fn client_closed(&self, client: ClientId);
}

fn sanitize_error_message(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return "tool request failed".to_owned();
    }
    if trimmed.len() <= MAX_ERROR_MESSAGE_BYTES {
        return trimmed.to_owned();
    }
    let end = trimmed.floor_char_boundary(MAX_ERROR_MESSAGE_BYTES);
    trimmed[..end].to_owned()
}
