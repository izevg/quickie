//! Typed isolated `Profile::Agent` operations, request/response models, and hard bounds.

use quickie_application::mcp::AgentContextId;
use url::Url;

use super::super::host::{HostReply, ToolError};

/// Minimum allowed viewport width in CSS pixels.
pub const MIN_VIEWPORT_WIDTH: u32 = 64;

/// Maximum allowed viewport width in CSS pixels.
pub const MAX_VIEWPORT_WIDTH: u32 = 4_096;

/// Minimum allowed viewport height in CSS pixels.
pub const MIN_VIEWPORT_HEIGHT: u32 = 64;

/// Maximum allowed viewport height in CSS pixels.
pub const MAX_VIEWPORT_HEIGHT: u32 = 4_096;

/// Default viewport width in CSS pixels when omitted on `agent_context_open`.
pub const DEFAULT_VIEWPORT_WIDTH: u32 = 640;

/// Default viewport height in CSS pixels when omitted on `agent_context_open`.
pub const DEFAULT_VIEWPORT_HEIGHT: u32 = 480;

/// Maximum UTF-8 byte length of a navigation URL string.
pub const MAX_URL_BYTES: usize = 2_048;

/// Maximum UTF-8 byte length of an opaque document-bound element reference ID.
pub const MAX_ELEMENT_ID_BYTES: usize = 256;

/// Maximum UTF-8 byte length of locator text or accessible name filters.
pub const MAX_LOCATOR_TEXT_BYTES: usize = 1_024;

/// Maximum UTF-8 byte length of an accessibility role identifier.
pub const MAX_ROLE_BYTES: usize = 64;

/// Default maximum number of located elements returned by `agent_locate`.
pub const DEFAULT_LOCATE_RESULTS: usize = 20;

/// Hard maximum number of located elements returned by `agent_locate`.
pub const MAX_LOCATE_RESULTS: usize = 50;

/// Maximum UTF-8 byte length of text passed to `fill`, `type`, or `select_option`.
pub const MAX_INPUT_TEXT_BYTES: usize = 4_096;

/// Maximum absolute scroll delta in CSS pixels along either axis.
pub const MAX_SCROLL_DELTA: i32 = 16_384;

/// Maximum number of modifier keys attached to a click or key press.
pub const MAX_KEY_MODIFIERS: usize = 4;

/// Minimum allowed timeout in milliseconds for `agent_navigate` and `agent_wait`.
pub const MIN_ACTION_TIMEOUT_MS: u64 = 1;

/// Maximum allowed timeout in milliseconds for `agent_navigate` and `agent_wait`.
pub const MAX_ACTION_TIMEOUT_MS: u64 = 30_000;

/// Default timeout in milliseconds for `agent_navigate`.
pub const DEFAULT_NAVIGATE_TIMEOUT_MS: u64 = 10_000;

/// Default timeout in milliseconds for `agent_wait`.
pub const DEFAULT_ACTION_TIMEOUT_MS: u64 = 5_000;

/// Maximum absolute coordinate or dimension in CSS pixels for element bounding boxes.
pub const MAX_BBOX_COORD: f64 = 16_384.0;

/// Default maximum raw PNG byte length for `agent_screenshot` (512 KiB).
pub const DEFAULT_SCREENSHOT_MAX_BYTES: usize = 512 * 1024;

/// Hard maximum raw PNG byte length for `agent_screenshot` (2 MiB).
pub const MAX_SCREENSHOT_BYTES: usize = 2 * 1024 * 1024;

/// Maximum UTF-8 byte length of a neutral status reason string in structured results.
pub const MAX_REASON_BYTES: usize = 256;

/// Bounded CSS-pixel viewport dimensions for an isolated Agent context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentViewport {
    /// Viewport width in CSS pixels (`MIN_VIEWPORT_WIDTH..=MAX_VIEWPORT_WIDTH`).
    pub width: u32,
    /// Viewport height in CSS pixels (`MIN_VIEWPORT_HEIGHT..=MAX_VIEWPORT_HEIGHT`).
    pub height: u32,
}

impl Default for AgentViewport {
    fn default() -> Self {
        Self {
            width: DEFAULT_VIEWPORT_WIDTH,
            height: DEFAULT_VIEWPORT_HEIGHT,
        }
    }
}

/// Summary of one isolated Agent context owned by the calling MCP client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentContextSummary {
    /// Host-issued isolated Agent context identifier.
    pub context_id: AgentContextId,
    /// Current monotonic main-frame document epoch.
    pub epoch: u64,
    /// Sanitized HTTP(S) tuple origin of the committed document, if any.
    pub origin: Option<String>,
    /// Current viewport dimensions.
    pub viewport: AgentViewport,
}

/// Result returned for `agent_context_open`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentContextOpenResult {
    /// Newly opened isolated Agent context identifier.
    pub context_id: AgentContextId,
    /// Initial monotonic main-frame document epoch.
    pub epoch: u64,
    /// Active viewport dimensions.
    pub viewport: AgentViewport,
}

/// Result returned for `agent_context_list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentContextListResult {
    /// Isolated Agent contexts currently owned by this client.
    pub contexts: Vec<AgentContextSummary>,
}

/// Result returned for `agent_context_close`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentContextCloseResult {
    /// Closed isolated Agent context identifier.
    pub context_id: AgentContextId,
    /// Whether the context was closed.
    pub closed: bool,
}

/// Document load stage to wait for in `agent_navigate` or `agent_wait`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AgentWaitStage {
    /// Return as soon as navigation is dispatched or committed without waiting for subresources.
    None,
    /// Wait until `DOMContentLoaded`.
    DomContentLoaded,
    /// Wait until the document `load` event completes.
    #[default]
    Load,
}

impl AgentWaitStage {
    /// Wire representation of this wait stage.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::DomContentLoaded => "dom_content_loaded",
            Self::Load => "load",
        }
    }
}

/// Ordinary typed navigation outcome for `agent_navigate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentNavigateStatus {
    /// Navigation committed within the approved origin.
    Committed,
    /// Navigation requires explicit user approval under the host's origin/provenance policy.
    ApprovalRequired,
    /// Navigation was rejected by policy or failed before committing.
    Rejected,
}

impl AgentNavigateStatus {
    /// Wire representation of this navigation status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::ApprovalRequired => "approval_required",
            Self::Rejected => "rejected",
        }
    }
}

/// Result returned for `agent_navigate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentNavigateResult {
    /// Owning isolated Agent context.
    pub context_id: AgentContextId,
    /// Ordinary typed navigation status (`committed`, `approval_required`, or `rejected`).
    pub status: AgentNavigateStatus,
    /// Current main-frame document epoch after the navigation attempt.
    pub epoch: u64,
    /// Sanitized committed HTTP(S) tuple origin when available.
    pub origin: Option<String>,
    /// Optional neutral reason when `approval_required` or `rejected`.
    pub reason: Option<String>,
}

/// Exact context- and document-epoch-bound element reference issued by `agent_locate`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentElementRef {
    /// Owning isolated Agent context.
    pub context_id: AgentContextId,
    /// Exact main-frame document epoch in which the element was located.
    pub epoch: u64,
    /// Opaque bounded element identifier scoped to `(context_id, epoch)`.
    pub element_id: String,
}

/// Locator strategy kind for provenance reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentLocatorKind {
    /// CSS selector lookup.
    Css,
    /// Visible text lookup.
    Text,
    /// Accessibility role lookup.
    Role,
}

impl AgentLocatorKind {
    /// Wire representation of this locator kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Css => "css",
            Self::Text => "text",
            Self::Role => "role",
        }
    }
}

/// Typed element locator accepted by `agent_locate` and `agent_wait`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentLocator {
    /// Match elements by CSS selector.
    Css {
        /// Validated non-empty CSS selector (`<= MAX_SELECTOR_BYTES`).
        selector: String,
    },
    /// Match elements by visible text.
    Text {
        /// Validated non-empty visible text (`<= MAX_LOCATOR_TEXT_BYTES`).
        text: String,
        /// Whether matching requires an exact text match rather than substring.
        exact: bool,
    },
    /// Match elements by accessibility role and optional accessible name.
    Role {
        /// Validated accessibility role name (`<= MAX_ROLE_BYTES`).
        role: String,
        /// Optional accessible name filter (`<= MAX_LOCATOR_TEXT_BYTES`).
        name: Option<String>,
        /// Whether `name` matching requires an exact match.
        exact: bool,
    },
}

impl AgentLocator {
    /// Returns the locator strategy kind.
    #[must_use]
    pub const fn kind(&self) -> AgentLocatorKind {
        match self {
            Self::Css { .. } => AgentLocatorKind::Css,
            Self::Text { .. } => AgentLocatorKind::Text,
            Self::Role { .. } => AgentLocatorKind::Role,
        }
    }
}

/// Frame scope where an element was located.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameScope {
    /// Main document frame.
    MainFrame,
}

impl FrameScope {
    /// Wire representation of this frame scope.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MainFrame => "main_frame",
        }
    }
}

/// Proven provenance metadata attached to every located element reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementProvenance {
    /// Sanitized HTTP(S) tuple origin of the document containing the element.
    pub origin: String,
    /// Frame scope where the element was matched.
    pub frame: FrameScope,
    /// Locator strategy that produced this match.
    pub locator_kind: AgentLocatorKind,
}

/// Validated CSS-pixel bounding box for a located element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementBounds {
    /// Horizontal offset in CSS pixels.
    pub x: f64,
    /// Vertical offset in CSS pixels.
    pub y: f64,
    /// Non-negative width in CSS pixels.
    pub width: f64,
    /// Non-negative height in CSS pixels.
    pub height: f64,
}

impl ElementBounds {
    /// Returns `true` when all coordinates are finite, non-negative in size, and within
    /// [`MAX_BBOX_COORD`].
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.x.abs() <= MAX_BBOX_COORD
            && self.y.abs() <= MAX_BBOX_COORD
            && (0.0..=MAX_BBOX_COORD).contains(&self.width)
            && (0.0..=MAX_BBOX_COORD).contains(&self.height)
    }
}

/// One ordered element match returned by `agent_locate`.
#[derive(Clone, Debug, PartialEq)]
pub struct LocatedElement {
    /// Exact `(context_id, epoch, element_id)` reference required for element-targeted actions.
    pub element_ref: AgentElementRef,
    /// Accessibility or semantic role of the matched element.
    pub role: String,
    /// Optional bounded accessible name or visible label.
    pub name: Option<String>,
    /// Optional bounded HTML tag name.
    pub tag: Option<String>,
    /// Optional bounded CSS-pixel bounding box.
    pub bounds: Option<ElementBounds>,
    /// Provenance metadata proving origin, frame, and locator kind.
    pub provenance: ElementProvenance,
}

/// Result returned for `agent_locate`.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentLocateResult {
    /// Owning isolated Agent context.
    pub context_id: AgentContextId,
    /// Document epoch in which all returned references are valid.
    pub epoch: u64,
    /// Bounded ordered element matches (`<= MAX_LOCATE_RESULTS`).
    pub elements: Vec<LocatedElement>,
    /// Whether additional matches were omitted due to `max_results`.
    pub truncated: bool,
}

/// Mouse button for pointer interactions in `agent_act`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AgentMouseButton {
    /// Primary (left) button.
    #[default]
    Left,
    /// Auxiliary (middle) button.
    Middle,
    /// Secondary (right) button.
    Right,
}

/// Key modifier for click or key-press interactions in `agent_act`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentKeyModifier {
    /// Shift key.
    Shift,
    /// Control key.
    Control,
    /// Alt / Option key.
    Alt,
    /// Meta / Command key.
    Meta,
}

/// Allowlisted key specification for `press_key` in `agent_act`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentKey {
    /// Enter / Return key.
    Enter,
    /// Tab key.
    Tab,
    /// Escape key.
    Escape,
    /// Backspace key.
    Backspace,
    /// Delete key.
    Delete,
    /// Space key.
    Space,
    /// Arrow Up key.
    ArrowUp,
    /// Arrow Down key.
    ArrowDown,
    /// Arrow Left key.
    ArrowLeft,
    /// Arrow Right key.
    ArrowRight,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page Up key.
    PageUp,
    /// Page Down key.
    PageDown,
    /// Single printable Unicode scalar value (never a control character).
    Character(char),
}

/// Option selector for `select_option` in `agent_act`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectOptionBy {
    /// Select option matching its `value` attribute.
    Value(String),
    /// Select option matching its visible label text.
    Label(String),
}

/// Typed isolated Agent interaction accepted by `agent_act`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentAction {
    /// Single-click a document-bound element.
    Click {
        /// Exact document-bound element reference.
        element: AgentElementRef,
        /// Mouse button to press.
        button: AgentMouseButton,
        /// Active key modifiers.
        modifiers: Vec<AgentKeyModifier>,
    },
    /// Double-click a document-bound element.
    DoubleClick {
        /// Exact document-bound element reference.
        element: AgentElementRef,
        /// Mouse button to press.
        button: AgentMouseButton,
        /// Active key modifiers.
        modifiers: Vec<AgentKeyModifier>,
    },
    /// Move the pointer over a document-bound element.
    Hover {
        /// Exact document-bound element reference.
        element: AgentElementRef,
    },
    /// Replace the value of a document-bound editable element.
    Fill {
        /// Exact document-bound element reference.
        element: AgentElementRef,
        /// Bounded UTF-8 replacement text without NUL bytes.
        text: String,
    },
    /// Type bounded text into an optional target element (or currently focused element).
    Type {
        /// Optional exact document-bound element reference to focus first.
        element: Option<AgentElementRef>,
        /// Bounded non-empty UTF-8 text without NUL bytes.
        text: String,
    },
    /// Press an allowlisted key with optional modifiers.
    PressKey {
        /// Optional exact document-bound element reference to focus first.
        element: Option<AgentElementRef>,
        /// Allowlisted virtual key or printable character.
        key: AgentKey,
        /// Active key modifiers.
        modifiers: Vec<AgentKeyModifier>,
    },
    /// Scroll the viewport or a document-bound element by bounded CSS-pixel deltas.
    Scroll {
        /// Optional exact document-bound element reference to scroll.
        element: Option<AgentElementRef>,
        /// Horizontal scroll delta in CSS pixels (`-MAX_SCROLL_DELTA..=MAX_SCROLL_DELTA`).
        delta_x: i32,
        /// Vertical scroll delta in CSS pixels (`-MAX_SCROLL_DELTA..=MAX_SCROLL_DELTA`).
        delta_y: i32,
    },
    /// Select an option in a document-bound `<select>` element.
    SelectOption {
        /// Exact document-bound element reference.
        element: AgentElementRef,
        /// Option matching criterion (`value` or `label`).
        option: SelectOptionBy,
    },
}

/// Ordinary typed action outcome for `agent_act`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentActStatus {
    /// Action was performed by trusted engine input.
    Performed,
    /// Action would submit or cross an approval boundary and requires explicit user approval.
    ApprovalRequired,
    /// Action was rejected by policy or target state.
    Rejected,
}

impl AgentActStatus {
    /// Wire representation of this action status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Performed => "performed",
            Self::ApprovalRequired => "approval_required",
            Self::Rejected => "rejected",
        }
    }
}

/// Optional bounded post-action state observed by the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentObservedState {
    /// Whether the action triggered a document navigation.
    pub navigated: bool,
    /// Accessibility role of the currently focused element, if known.
    pub focused_role: Option<String>,
    /// Character length of an updated input value (never raw secret text).
    pub value_length: Option<usize>,
    /// Current scroll offset in CSS pixels, if observed.
    pub scroll_x: Option<i32>,
    /// Current scroll offset in CSS pixels, if observed.
    pub scroll_y: Option<i32>,
}

/// Result returned for `agent_act`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentActResult {
    /// Owning isolated Agent context.
    pub context_id: AgentContextId,
    /// Ordinary typed action outcome (`performed`, `approval_required`, or `rejected`).
    pub status: AgentActStatus,
    /// Current main-frame document epoch after the action attempt.
    pub epoch: u64,
    /// Sanitized HTTP(S) tuple origin when available.
    pub origin: Option<String>,
    /// Optional neutral reason when `approval_required` or `rejected`.
    pub reason: Option<String>,
    /// Optional observed post-action state.
    pub observed_state: Option<AgentObservedState>,
}

/// Typed wait condition accepted by `agent_wait`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentWaitCondition {
    /// Wait until at least one element matching `locator` is visible.
    LocatorVisible {
        /// Typed element locator.
        locator: AgentLocator,
    },
    /// Wait until no element matching `locator` is visible.
    LocatorHidden {
        /// Typed element locator.
        locator: AgentLocator,
    },
    /// Wait until visible text appears in the document.
    TextPresent {
        /// Validated non-empty text (`<= MAX_LOCATOR_TEXT_BYTES`).
        text: String,
        /// Whether matching requires an exact text match.
        exact: bool,
    },
    /// Wait until the document reaches the requested load stage.
    LoadStage {
        /// Required document load stage (`dom_content_loaded` or `load`).
        stage: AgentWaitStage,
    },
}

/// Outcome of `agent_wait`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentWaitStatus {
    /// The condition matched before the deadline.
    Matched,
    /// The deadline elapsed before the condition matched.
    TimedOut,
}

impl AgentWaitStatus {
    /// Wire representation of this wait outcome.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::TimedOut => "timed_out",
        }
    }
}

/// Result returned for `agent_wait`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentWaitResult {
    /// Owning isolated Agent context.
    pub context_id: AgentContextId,
    /// Document epoch at completion.
    pub epoch: u64,
    /// Whether the condition matched or timed out.
    pub status: AgentWaitStatus,
    /// Elapsed wait time in milliseconds.
    pub elapsed_ms: u64,
}

/// Target region for `agent_screenshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentScreenshotTarget {
    /// Capture the current visual viewport.
    Viewport,
    /// Capture a specific document-bound element.
    Element(AgentElementRef),
}

/// Bounded PNG screenshot payload returned by the host for `agent_screenshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentScreenshotResult {
    /// Owning isolated Agent context.
    pub context_id: AgentContextId,
    /// Document epoch captured in the screenshot.
    pub epoch: u64,
    /// Image width in pixels (`1..=MAX_VIEWPORT_WIDTH`).
    pub width: u32,
    /// Image height in pixels (`1..=MAX_VIEWPORT_HEIGHT`).
    pub height: u32,
    /// Raw PNG bytes (`<= max_bytes <= MAX_SCREENSHOT_BYTES`, starting with the PNG signature).
    pub png_bytes: Vec<u8>,
}

/// Nested typed isolated `Profile::Agent` operation carried by [`super::super::host::BrowserOperation::Agent`].
///
/// Hosts that have not yet wired isolated Agent support can reject the entire branch in one arm
/// via [`Self::reject`].
#[derive(Debug)]
pub enum AgentOperation {
    /// Open a new isolated Agent browsing context for the calling client.
    ContextOpen {
        /// Validated initial viewport dimensions.
        viewport: AgentViewport,
        /// One-shot reply completed with the created context details.
        reply: HostReply<AgentContextOpenResult>,
    },
    /// List isolated Agent contexts owned by the calling client.
    ContextList {
        /// One-shot reply completed with the client's contexts.
        reply: HostReply<AgentContextListResult>,
    },
    /// Close an isolated Agent context owned by the calling client.
    ContextClose {
        /// Target isolated Agent context identifier.
        context_id: AgentContextId,
        /// One-shot reply completed with closure status.
        reply: HostReply<AgentContextCloseResult>,
    },
    /// Navigate an isolated Agent context to a validated credential-free HTTP(S) URL.
    Navigate {
        /// Target isolated Agent context identifier.
        context_id: AgentContextId,
        /// Validated credential-free HTTP(S) destination URL.
        url: Url,
        /// Requested document load stage to wait for.
        wait_stage: AgentWaitStage,
        /// Bounded navigation timeout in milliseconds (`MIN_ACTION_TIMEOUT_MS..=MAX_ACTION_TIMEOUT_MS`).
        timeout_ms: u64,
        /// One-shot reply completed with navigation outcome.
        reply: HostReply<AgentNavigateResult>,
    },
    /// Locate bounded, ordered, document-bound element references in an isolated Agent context.
    Locate {
        /// Target isolated Agent context identifier.
        context_id: AgentContextId,
        /// Typed locator (`css`, `text`, or `role`).
        locator: AgentLocator,
        /// Bounded maximum number of matches (`1..=MAX_LOCATE_RESULTS`).
        max_results: usize,
        /// One-shot reply completed with located element references.
        reply: HostReply<AgentLocateResult>,
    },
    /// Perform a typed trusted interaction in an isolated Agent context.
    Act {
        /// Target isolated Agent context identifier.
        context_id: AgentContextId,
        /// Typed interaction with exact document-bound element references where required.
        action: AgentAction,
        /// One-shot reply completed with action outcome.
        reply: HostReply<AgentActResult>,
    },
    /// Wait for a typed condition in an isolated Agent context within a bounded timeout.
    Wait {
        /// Target isolated Agent context identifier.
        context_id: AgentContextId,
        /// Typed wait condition.
        condition: AgentWaitCondition,
        /// Bounded wait timeout in milliseconds (`MIN_ACTION_TIMEOUT_MS..=MAX_ACTION_TIMEOUT_MS`).
        timeout_ms: u64,
        /// One-shot reply completed with matched or timed-out status.
        reply: HostReply<AgentWaitResult>,
    },
    /// Capture a bounded PNG screenshot of the viewport or a document-bound element.
    Screenshot {
        /// Target isolated Agent context identifier.
        context_id: AgentContextId,
        /// Viewport or exact document-bound element target.
        target: AgentScreenshotTarget,
        /// Upper bound on raw PNG byte length (`1..=MAX_SCREENSHOT_BYTES`).
        max_bytes: usize,
        /// One-shot reply completed with bounded PNG bytes and dimensions.
        reply: HostReply<AgentScreenshotResult>,
    },
}

impl AgentOperation {
    /// Rejects the pending one-shot reply for any isolated Agent operation with `error`.
    ///
    /// Allows native hosts to return `ToolErrorCode::ToolNotAvailable` cleanly before isolated
    /// Agent support is wired.
    pub fn reject(self, error: ToolError) {
        match self {
            Self::ContextOpen { reply, .. } => reply.reject(error),
            Self::ContextList { reply } => reply.reject(error),
            Self::ContextClose { reply, .. } => reply.reject(error),
            Self::Navigate { reply, .. } => reply.reject(error),
            Self::Locate { reply, .. } => reply.reject(error),
            Self::Act { reply, .. } => reply.reject(error),
            Self::Wait { reply, .. } => reply.reject(error),
            Self::Screenshot { reply, .. } => reply.reject(error),
        }
    }
}
