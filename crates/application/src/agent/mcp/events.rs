//! Diagnostic event types, redacted network metadata, filtering, and bounded ring buffer.

use std::collections::VecDeque;

use url::{Origin, Url};

use super::types::{
    DiagnosticError, MAX_EVENTS_PER_ATTACHMENT, MAX_TEXT_EVENT_BYTES, is_valid_http_origin,
    truncate_utf8,
};

const MAX_METHOD_BYTES: usize = 32;

/// Category of a diagnostic event stored in an attachment's ring buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticEventKind {
    /// Browser console message.
    Console,
    /// Uncaught page runtime error.
    RuntimeError,
    /// Redacted HTTP(S) network request metadata.
    Network,
    /// Document lifecycle transition.
    Lifecycle,
}

/// Severity level of a console event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConsoleLevel {
    /// Verbose debug output.
    Debug,
    /// Standard log or info output.
    Info,
    /// Warning output.
    Warn,
    /// Error output.
    Error,
}

/// Page lifecycle milestone observed on an attached document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleEventKind {
    /// Same-document history transition (`pushState`/`replaceState`/hash).
    SameDocumentNavigation,
    /// Same-origin reload committed under `GrantScope::SameOriginReload`.
    ReloadCommitted,
    /// `DOMContentLoaded` fired.
    DomContentLoaded,
    /// Window `load` fired.
    Load,
}

/// Lifecycle phase of an observed network request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkPhase {
    /// Request is in flight and has not settled.
    Pending,
    /// Request finished with an HTTP response.
    Completed,
    /// Request failed or was aborted before completing.
    Failed,
}

/// Redacted HTTP(S) network endpoint carrying only tuple origin and path.
///
/// Userinfo, query strings, and fragments are discarded at construction time; non-HTTP(S) URLs
/// are rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkEndpoint {
    origin: Origin,
    path: String,
    path_truncated: bool,
}

impl NetworkEndpoint {
    /// Extracts a redacted HTTP(S) endpoint from `url`, dropping userinfo, query, and fragment.
    ///
    /// Returns `None` for non-HTTP(S) schemes or URLs without a valid tuple origin.
    #[must_use]
    pub fn from_url(url: &Url) -> Option<Self> {
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let origin = url.origin();
        if !is_valid_http_origin(&origin) {
            return None;
        }
        let (path, path_truncated) = truncate_utf8(url.path(), MAX_TEXT_EVENT_BYTES);
        Some(Self {
            origin,
            path,
            path_truncated,
        })
    }

    /// The HTTP(S) tuple origin (`scheme`, `host`, `port`).
    #[must_use]
    pub const fn origin(&self) -> &Origin {
        &self.origin
    }

    /// The URL path without query parameters or fragment.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Whether the path was truncated to the event text ceiling.
    #[must_use]
    pub const fn is_path_truncated(&self) -> bool {
        self.path_truncated
    }
}

/// Redacted metadata for a single HTTP(S) network request.
///
/// Carries only endpoint metadata, method, status, and lifecycle phase — never headers, raw
/// query parameters, fragments, URL userinfo, or bodies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkEvent {
    request_id: u64,
    endpoint: NetworkEndpoint,
    method: String,
    status: Option<u16>,
    phase: NetworkPhase,
}

impl NetworkEvent {
    /// Constructs a redacted network event from typed fields and a parsed URL.
    ///
    /// Returns `None` if `url` is not a valid HTTP(S) endpoint or `method` is empty.
    #[must_use]
    pub fn new(
        request_id: u64,
        url: &Url,
        method: &str,
        status: Option<u16>,
        phase: NetworkPhase,
    ) -> Option<Self> {
        let endpoint = NetworkEndpoint::from_url(url)?;
        let trimmed = method.trim();
        if trimmed.is_empty() {
            return None;
        }
        let (bounded_method, _) = truncate_utf8(trimmed, MAX_METHOD_BYTES);
        Some(Self {
            request_id,
            endpoint,
            method: bounded_method.to_ascii_uppercase(),
            status,
            phase,
        })
    }

    /// Host-assigned request identifier.
    #[must_use]
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Redacted HTTP(S) endpoint metadata.
    #[must_use]
    pub const fn endpoint(&self) -> &NetworkEndpoint {
        &self.endpoint
    }

    /// Bounded uppercase HTTP method name.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// HTTP response status code, if received.
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        self.status
    }

    /// Current lifecycle phase of the request.
    #[must_use]
    pub const fn phase(&self) -> NetworkPhase {
        self.phase
    }
}

/// Typed payload of a [`DiagnosticEvent`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticEventData {
    /// Console log message bounded to `MAX_TEXT_EVENT_BYTES`.
    Console {
        /// Console severity level.
        level: ConsoleLevel,
        /// Bounded UTF-8 message text.
        text: String,
        /// Whether `text` was truncated to fit `MAX_TEXT_EVENT_BYTES`.
        truncated: bool,
    },
    /// Uncaught runtime error message bounded to `MAX_TEXT_EVENT_BYTES`.
    RuntimeError {
        /// Bounded UTF-8 error message.
        message: String,
        /// Whether `message` was truncated to fit `MAX_TEXT_EVENT_BYTES`.
        truncated: bool,
    },
    /// Redacted network request metadata.
    Network(NetworkEvent),
    /// Document lifecycle event.
    Lifecycle {
        /// Lifecycle transition kind.
        kind: LifecycleEventKind,
    },
}

impl DiagnosticEventData {
    /// Creates a bounded console event payload.
    #[must_use]
    pub fn console(level: ConsoleLevel, text: &str) -> Self {
        let (bounded, truncated) = truncate_utf8(text, MAX_TEXT_EVENT_BYTES);
        Self::Console {
            level,
            text: bounded,
            truncated,
        }
    }

    /// Creates a bounded runtime error event payload.
    #[must_use]
    pub fn runtime_error(message: &str) -> Self {
        let (bounded, truncated) = truncate_utf8(message, MAX_TEXT_EVENT_BYTES);
        Self::RuntimeError {
            message: bounded,
            truncated,
        }
    }

    /// Creates a lifecycle event payload.
    #[must_use]
    pub const fn lifecycle(kind: LifecycleEventKind) -> Self {
        Self::Lifecycle { kind }
    }

    /// The kind of this event payload.
    #[must_use]
    pub const fn kind(&self) -> DiagnosticEventKind {
        match self {
            Self::Console { .. } => DiagnosticEventKind::Console,
            Self::RuntimeError { .. } => DiagnosticEventKind::RuntimeError,
            Self::Network(_) => DiagnosticEventKind::Network,
            Self::Lifecycle { .. } => DiagnosticEventKind::Lifecycle,
        }
    }
}

/// A single diagnostic event with its attachment-monotonic sequence and captured document epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticEvent {
    /// Monotonically increasing sequence number starting at 1.
    pub seq: u64,
    /// Document epoch active when this event was captured.
    pub epoch: u64,
    /// Event payload.
    pub data: DiagnosticEventData,
}

impl DiagnosticEvent {
    /// The category of this event.
    #[must_use]
    pub const fn kind(&self) -> DiagnosticEventKind {
        self.data.kind()
    }
}

/// Filter applied during cursor-based event replay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticEventFilter {
    /// Event kinds to include; an empty list matches all kinds.
    pub kinds: Vec<DiagnosticEventKind>,
}

impl DiagnosticEventFilter {
    /// Matches every event kind.
    #[must_use]
    pub const fn all() -> Self {
        Self { kinds: Vec::new() }
    }

    /// Matches only the listed event kinds.
    #[must_use]
    pub fn of_kinds(kinds: impl IntoIterator<Item = DiagnosticEventKind>) -> Self {
        Self {
            kinds: kinds.into_iter().collect(),
        }
    }

    /// Whether `event` matches this filter.
    #[must_use]
    pub fn matches(&self, event: &DiagnosticEvent) -> bool {
        self.kinds.is_empty() || self.kinds.contains(&event.kind())
    }
}

/// Paginated batch of diagnostic events returned from a cursor replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticEventBatch {
    /// Matching events in sequence order.
    pub events: Vec<DiagnosticEvent>,
    /// Cursor to pass on the next call to continue advancing.
    pub next_cursor: u64,
    /// Whether additional matching events exist after `next_cursor`.
    pub has_more: bool,
    /// Oldest retained sequence boundary when events after the requested cursor were evicted.
    pub dropped_before: Option<u64>,
}

/// Single ordered ring buffer with hard capacity `MAX_EVENTS_PER_ATTACHMENT`.
#[derive(Debug)]
pub(super) struct EventRing {
    events: VecDeque<DiagnosticEvent>,
    last_seq: u64,
}

impl EventRing {
    pub(super) fn new() -> Self {
        Self {
            events: VecDeque::new(),
            last_seq: 0,
        }
    }

    pub(super) fn push(&mut self, epoch: u64, data: DiagnosticEventData) -> u64 {
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

    pub(super) fn read(
        &self,
        cursor: u64,
        filter: &DiagnosticEventFilter,
        limit: usize,
    ) -> Result<DiagnosticEventBatch, DiagnosticError> {
        if cursor > self.last_seq {
            return Err(DiagnosticError::InvalidCursor);
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
