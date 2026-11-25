//! Isolated native `WebKitGTK` local Automation session for Agent contexts.
//!
//! Constructs and retains one private `WebKitWebContext` with an in-process
//! `WebKitAutomationSession` and one offscreen controlled `WebKitWebView` backed by the
//! local context's ephemeral network session. Never shares `Main`'s context or storage,
//! never opens a TCP listener, never creates or presents visible windows, and accepts
//! only engine-generated bounded protocol requests.

#![expect(
    dead_code,
    reason = "Wired by the sibling MCP automation transport slice"
)]

mod native;
mod session;
#[cfg(test)]
mod tests;

use std::fmt;

use quickie_engine_webkit::download_name::wants_download;
use serde_json::{Map, Value, json};
use url::{Origin, Url};

#[expect(
    unused_imports,
    reason = "Wired by the sibling MCP automation transport slice"
)]
pub(crate) use session::{AutomationSession, availability, is_available};

/// Default offscreen viewport dimensions matching the synthetic Wayland screenshot baseline.
pub(crate) const DEFAULT_VIEWPORT: ViewportSize = ViewportSize {
    width: 640,
    height: 480,
};

/// Minimum allowed offscreen viewport dimension in logical pixels.
pub(crate) const MIN_VIEWPORT_DIMENSION: i32 = 1;

/// Maximum allowed offscreen viewport dimension in logical pixels.
pub(crate) const MAX_VIEWPORT_DIMENSION: i32 = 4096;

/// Maximum length in bytes for session identifiers and automation handles.
pub(crate) const MAX_HANDLE_BYTES: usize = 256;

/// Maximum length in bytes for a single keyboard interaction text payload.
pub(crate) const MAX_KEYBOARD_TEXT_BYTES: usize = 1024;

/// Maximum number of keyboard interactions in a single request.
pub(crate) const MAX_INTERACTIONS: usize = 64;

/// Maximum serialized byte size of an engine-generated automation protocol request.
pub(crate) const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// Availability of the fork's native `WebKitGTK` local Automation SPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutomationAvailability {
    /// On the GTK main thread and all required local Automation symbols and signals exist.
    Available,
    /// The loaded `WebKitGTK` binary lacks one or more required local Automation symbols or signals.
    UnsupportedSpi,
    /// Local Automation sessions may only be inspected and driven on the GTK main thread.
    WrongThread,
}

/// Distinct failure modes for native `WebKitGTK` local Automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutomationError {
    /// The loaded `WebKitGTK` binary does not expose the complete local Automation SPI.
    UnsupportedSpi,
    /// The operation was attempted off the GTK main thread.
    WrongThread,
    /// The supplied session identifier is empty, too long, or contains control characters.
    InvalidSessionId,
    /// The requested viewport dimensions are out of bounds.
    InvalidViewport,
    /// The requested navigation URL is not an ordinary HTTP/HTTPS URL without credentials.
    InvalidUrl,
    /// Navigation was attempted without an explicit approved caller grant for its origin.
    NavigationNotApproved,
    /// `WebKitGTK` failed to construct the isolated local Automation context or session.
    SessionCreationFailed,
    /// `WebKitGTK` failed to construct or verify the isolated controlled `WebKitWebView`.
    ControlledViewFailed,
    /// The session has already been closed or detached.
    ClosedSession,
    /// `WebKitGTK` rejected or failed to dispatch the bounded automation protocol request.
    DispatchFailed,
}

impl fmt::Display for AutomationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSpi => f.write_str("WebKitGTK local Automation SPI is unavailable"),
            Self::WrongThread => {
                f.write_str("local Automation requires the GTK toolkit main thread")
            }
            Self::InvalidSessionId => f.write_str("invalid local Automation session identifier"),
            Self::InvalidViewport => f.write_str("offscreen viewport dimensions are out of bounds"),
            Self::InvalidUrl => {
                f.write_str("automation requires an HTTP or HTTPS URL without credentials")
            }
            Self::NavigationNotApproved => {
                f.write_str("navigation requires an explicit approved caller origin grant")
            }
            Self::SessionCreationFailed => {
                f.write_str("WebKitGTK failed to create the local Automation session")
            }
            Self::ControlledViewFailed => {
                f.write_str("WebKitGTK failed to create an isolated controlled WebView")
            }
            Self::ClosedSession => f.write_str("local Automation session is closed"),
            Self::DispatchFailed => {
                f.write_str("WebKitGTK rejected the local Automation protocol request")
            }
        }
    }
}

impl std::error::Error for AutomationError {}

/// Bounded offscreen viewport dimensions applied via `gtk_widget_allocate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ViewportSize {
    width: i32,
    height: i32,
}

impl ViewportSize {
    /// Creates a validated bounded viewport size.
    pub(crate) const fn new(width: i32, height: i32) -> Result<Self, AutomationError> {
        if width < MIN_VIEWPORT_DIMENSION
            || width > MAX_VIEWPORT_DIMENSION
            || height < MIN_VIEWPORT_DIMENSION
            || height > MAX_VIEWPORT_DIMENSION
        {
            return Err(AutomationError::InvalidViewport);
        }
        Ok(Self { width, height })
    }

    pub(crate) const fn width(self) -> i32 {
        self.width
    }

    pub(crate) const fn height(self) -> i32 {
        self.height
    }
}

impl Default for ViewportSize {
    fn default() -> Self {
        DEFAULT_VIEWPORT
    }
}

/// Validates an ordinary HTTP/HTTPS URL without embedded credentials.
pub(crate) fn validate_ordinary_url(url: &Url) -> Result<(), AutomationError> {
    if matches!(url.scheme(), "http" | "https")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
    {
        Ok(())
    } else {
        Err(AutomationError::InvalidUrl)
    }
}

/// An explicitly approved origin grant for the isolated Agent browsing context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigationGrant {
    origin: Origin,
    initial_url: Url,
}

impl NavigationGrant {
    /// Creates an origin grant after validating the approved HTTP/HTTPS URL.
    pub(crate) fn new(url: &Url) -> Result<Self, AutomationError> {
        validate_ordinary_url(url)?;
        Ok(Self {
            origin: url.origin(),
            initial_url: url.clone(),
        })
    }

    pub(crate) const fn origin(&self) -> &Origin {
        &self.origin
    }

    pub(crate) const fn initial_url(&self) -> &Url {
        &self.initial_url
    }
}

/// Navigation permission policy enforced on the isolated controlled `WebKitWebView`.
///
/// Starts locked so no document can load without an explicit approved caller action.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct NavigationPolicy {
    grant: Option<NavigationGrant>,
}

impl NavigationPolicy {
    /// Creates a locked policy that denies every navigation until an explicit grant is installed.
    pub(crate) const fn locked() -> Self {
        Self { grant: None }
    }

    /// Installs an explicit caller-approved origin grant.
    pub(crate) fn grant_origin(&mut self, url: &Url) -> Result<NavigationGrant, AutomationError> {
        let grant = NavigationGrant::new(url)?;
        self.grant = Some(grant.clone());
        Ok(grant)
    }

    /// Revokes the active origin grant, returning the policy to its locked state.
    pub(crate) fn revoke(&mut self) {
        self.grant = None;
    }

    /// Returns the currently granted origin, if any.
    pub(crate) const fn current_grant(&self) -> Option<&NavigationGrant> {
        self.grant.as_ref()
    }

    /// Checks whether an explicit navigation request URL is covered by the active grant.
    pub(crate) fn permits_approved_url(&self, url: &Url) -> bool {
        self.grant
            .as_ref()
            .is_some_and(|grant| validate_ordinary_url(url).is_ok() && url.origin() == grant.origin)
    }

    /// Evaluates a `WebKitWebView::decide-policy` navigation action.
    ///
    /// Denies new windows, non-GET actions, non-HTTP(S) schemes, credentialed URLs,
    /// cross-origin navigations/redirects, and any load before an explicit grant is installed.
    pub(crate) fn permits_navigation(
        &self,
        url: Option<&Url>,
        method: Option<&str>,
        is_new_window: bool,
    ) -> bool {
        if is_new_window {
            return false;
        }
        let Some(grant) = &self.grant else {
            return false;
        };
        method == Some("GET")
            && url.is_some_and(|candidate| {
                validate_ordinary_url(candidate).is_ok() && candidate.origin() == grant.origin
            })
    }

    /// Evaluates a `WebKitWebView::decide-policy` response decision.
    ///
    /// Denies downloads, unsupported attachment responses, and cross-origin responses.
    pub(crate) fn permits_response(
        &self,
        url: Option<&Url>,
        mime_supported: bool,
        content_disposition: Option<&str>,
    ) -> bool {
        let Some(grant) = &self.grant else {
            return false;
        };
        !wants_download(mime_supported, content_disposition)
            && url.is_some_and(|candidate| {
                validate_ordinary_url(candidate).is_ok() && candidate.origin() == grant.origin
            })
    }
}

/// Page load completion strategy for `Automation.navigateBrowsingContext`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageLoadStrategy {
    None,
    Eager,
    Normal,
}

impl PageLoadStrategy {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Eager => "Eager",
            Self::Normal => "Normal",
        }
    }
}

/// Coordinate system for `Automation.computeElementLayout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoordinateSystem {
    Page,
    LayoutViewport,
}

impl CoordinateSystem {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::Page => "Page",
            Self::LayoutViewport => "LayoutViewport",
        }
    }
}

/// Mouse button for `Automation.performMouseInteraction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseButton {
    None,
    Left,
    Middle,
    Right,
}

impl MouseButton {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Left => "Left",
            Self::Middle => "Middle",
            Self::Right => "Right",
        }
    }
}

/// Mouse interaction kind for `Automation.performMouseInteraction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseInteraction {
    Move,
    Down,
    Up,
    SingleClick,
    DoubleClick,
}

impl MouseInteraction {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Down => "Down",
            Self::Up => "Up",
            Self::SingleClick => "SingleClick",
            Self::DoubleClick => "DoubleClick",
        }
    }
}

/// Keyboard modifier for `Automation.performMouseInteraction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyModifier {
    CapsLock,
    Control,
    Shift,
    Meta,
    Alt,
}

impl KeyModifier {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::CapsLock => "CapsLock",
            Self::Control => "Control",
            Self::Shift => "Shift",
            Self::Meta => "Meta",
            Self::Alt => "Alt",
        }
    }
}

/// Bounded keyboard interaction step for `Automation.performKeyboardInteractions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyboardInteraction {
    InsertByKey { text: String },
    KeyPress { key: String },
    KeyRelease { key: String },
}

impl KeyboardInteraction {
    fn to_wire(&self) -> Option<Value> {
        match self {
            Self::InsertByKey { text } => {
                if text.is_empty()
                    || text.len() > MAX_KEYBOARD_TEXT_BYTES
                    || text.bytes().any(|byte| byte == 0)
                {
                    return None;
                }
                Some(json!({
                    "type": "InsertByKey",
                    "text": text,
                }))
            }
            Self::KeyPress { key } => {
                if !is_valid_handle(key) {
                    return None;
                }
                Some(json!({
                    "type": "KeyPress",
                    "key": key,
                }))
            }
            Self::KeyRelease { key } => {
                if !is_valid_handle(key) {
                    return None;
                }
                Some(json!({
                    "type": "KeyRelease",
                    "key": key,
                }))
            }
        }
    }
}

/// Allowlisted read-only Inspector observation methods permitted through `Automation.sendInspectorMessage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectorObservationMethod {
    ConsoleEnable,
    ConsoleDisable,
    RuntimeEnable,
    RuntimeDisable,
    NetworkEnable,
    NetworkDisable,
    PageEnable,
    PageDisable,
    PageGetResourceTree,
    DomGetDocument,
}

impl InspectorObservationMethod {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::ConsoleEnable => "Console.enable",
            Self::ConsoleDisable => "Console.disable",
            Self::RuntimeEnable => "Runtime.enable",
            Self::RuntimeDisable => "Runtime.disable",
            Self::NetworkEnable => "Network.enable",
            Self::NetworkDisable => "Network.disable",
            Self::PageEnable => "Page.enable",
            Self::PageDisable => "Page.disable",
            Self::PageGetResourceTree => "Page.getResourceTree",
            Self::DomGetDocument => "DOM.getDocument",
        }
    }
}

/// Bounded read-only Inspector command routed inside `Automation.sendInspectorMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InspectorCommand {
    payload: String,
}

impl InspectorCommand {
    /// Builds `Target.setPauseOnStart`.
    pub(crate) fn target_set_pause_on_start(id: u64, pause_on_start: bool) -> Option<Self> {
        if id == 0 {
            return None;
        }
        Some(Self {
            payload: json!({
                "id": id,
                "method": "Target.setPauseOnStart",
                "params": { "pauseOnStart": pause_on_start },
            })
            .to_string(),
        })
    }

    /// Builds `Target.resume`.
    pub(crate) fn target_resume(id: u64, target_id: &str) -> Option<Self> {
        if id == 0 || !is_valid_handle(target_id) {
            return None;
        }
        Some(Self {
            payload: json!({
                "id": id,
                "method": "Target.resume",
                "params": { "targetId": target_id },
            })
            .to_string(),
        })
    }

    /// Builds a direct allowlisted read-only Inspector observation command.
    pub(crate) fn direct(id: u64, method: InspectorObservationMethod) -> Option<Self> {
        if id == 0 {
            return None;
        }
        Some(Self {
            payload: json!({
                "id": id,
                "method": method.as_wire(),
            })
            .to_string(),
        })
    }

    /// Builds a `Target.sendMessageToTarget` wrapper around an allowlisted read-only method.
    pub(crate) fn for_target(
        id: u64,
        target_id: &str,
        inner_id: u64,
        method: InspectorObservationMethod,
    ) -> Option<Self> {
        if id == 0 || inner_id == 0 || !is_valid_handle(target_id) {
            return None;
        }
        let inner = json!({
            "id": inner_id,
            "method": method.as_wire(),
        })
        .to_string();
        Some(Self {
            payload: json!({
                "id": id,
                "method": "Target.sendMessageToTarget",
                "params": {
                    "targetId": target_id,
                    "message": inner,
                },
            })
            .to_string(),
        })
    }

    pub(crate) fn as_json(&self) -> &str {
        &self.payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RequestKind {
    Navigate(Url),
    Screenshot,
    Other,
}

/// An engine-generated, structurally bounded `WebKit` Automation protocol request.
///
/// Can only be constructed via typed builders; raw JSON from MCP clients is never accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutomationRequest {
    payload: String,
    kind: RequestKind,
}

impl AutomationRequest {
    fn from_parts(
        id: u64,
        method: &str,
        params: Option<Map<String, Value>>,
        kind: RequestKind,
    ) -> Option<Self> {
        if id == 0 {
            return None;
        }
        let mut root = Map::new();
        root.insert("id".into(), Value::from(id));
        root.insert("method".into(), Value::from(method));
        if let Some(params) = params {
            root.insert("params".into(), Value::Object(params));
        }
        let payload = Value::Object(root).to_string();
        if payload.len() > MAX_REQUEST_BYTES || payload.bytes().any(|byte| byte == 0) {
            return None;
        }
        Some(Self { payload, kind })
    }

    /// Builds `Automation.getBrowsingContexts`.
    pub(crate) fn get_browsing_contexts(id: u64) -> Option<Self> {
        Self::from_parts(
            id,
            "Automation.getBrowsingContexts",
            None,
            RequestKind::Other,
        )
    }

    /// Builds `Automation.getBrowsingContext`.
    pub(crate) fn get_browsing_context(id: u64, handle: &str) -> Option<Self> {
        if !is_valid_handle(handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        Self::from_parts(
            id,
            "Automation.getBrowsingContext",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.closeBrowsingContext`.
    pub(crate) fn close_browsing_context(id: u64, handle: &str) -> Option<Self> {
        if !is_valid_handle(handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        Self::from_parts(
            id,
            "Automation.closeBrowsingContext",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.resolveBrowsingContext`.
    pub(crate) fn resolve_browsing_context(id: u64, browsing_context_handle: &str) -> Option<Self> {
        if !is_valid_handle(browsing_context_handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert(
            "browsingContextHandle".into(),
            Value::from(browsing_context_handle),
        );
        Self::from_parts(
            id,
            "Automation.resolveBrowsingContext",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.switchToBrowsingContext`.
    pub(crate) fn switch_to_browsing_context(
        id: u64,
        browsing_context_handle: &str,
    ) -> Option<Self> {
        if !is_valid_handle(browsing_context_handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert(
            "browsingContextHandle".into(),
            Value::from(browsing_context_handle),
        );
        Self::from_parts(
            id,
            "Automation.switchToBrowsingContext",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.waitForNavigationToComplete`.
    pub(crate) fn wait_for_navigation_to_complete(
        id: u64,
        browsing_context_handle: &str,
    ) -> Option<Self> {
        if !is_valid_handle(browsing_context_handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert(
            "browsingContextHandle".into(),
            Value::from(browsing_context_handle),
        );
        Self::from_parts(
            id,
            "Automation.waitForNavigationToComplete",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.navigateBrowsingContext` for an ordinary HTTP/HTTPS URL without credentials.
    pub(crate) fn navigate_browsing_context(
        id: u64,
        handle: &str,
        url: &Url,
        page_load_strategy: Option<PageLoadStrategy>,
        page_load_timeout_ms: Option<u32>,
    ) -> Result<Self, AutomationError> {
        validate_ordinary_url(url)?;
        if id == 0 || !is_valid_handle(handle) {
            return Err(AutomationError::DispatchFailed);
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        params.insert("url".into(), Value::from(url.as_str()));
        if let Some(strategy) = page_load_strategy {
            params.insert("pageLoadStrategy".into(), Value::from(strategy.as_wire()));
        }
        if let Some(timeout) = page_load_timeout_ms {
            params.insert("pageLoadTimeout".into(), Value::from(timeout));
        }
        Self::from_parts(
            id,
            "Automation.navigateBrowsingContext",
            Some(params),
            RequestKind::Navigate(url.clone()),
        )
        .ok_or(AutomationError::DispatchFailed)
    }

    /// Builds `Automation.goBackInBrowsingContext`.
    pub(crate) fn go_back(id: u64, handle: &str) -> Option<Self> {
        if !is_valid_handle(handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        Self::from_parts(
            id,
            "Automation.goBackInBrowsingContext",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.goForwardInBrowsingContext`.
    pub(crate) fn go_forward(id: u64, handle: &str) -> Option<Self> {
        if !is_valid_handle(handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        Self::from_parts(
            id,
            "Automation.goForwardInBrowsingContext",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.reloadBrowsingContext`.
    pub(crate) fn reload(id: u64, handle: &str) -> Option<Self> {
        if !is_valid_handle(handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        Self::from_parts(
            id,
            "Automation.reloadBrowsingContext",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.computeElementLayout`.
    pub(crate) fn compute_element_layout(
        id: u64,
        browsing_context_handle: &str,
        node_handle: &str,
        coordinate_system: CoordinateSystem,
        frame_handle: Option<&str>,
        scroll_into_view: bool,
    ) -> Option<Self> {
        if !is_valid_handle(browsing_context_handle) || !is_valid_handle(node_handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert(
            "browsingContextHandle".into(),
            Value::from(browsing_context_handle),
        );
        params.insert("nodeHandle".into(), Value::from(node_handle));
        params.insert(
            "coordinateSystem".into(),
            Value::from(coordinate_system.as_wire()),
        );
        params.insert(
            "scrollIntoViewIfNeeded".into(),
            Value::from(scroll_into_view),
        );
        if let Some(frame) = frame_handle {
            if !is_valid_handle(frame) {
                return None;
            }
            params.insert("frameHandle".into(), Value::from(frame));
        }
        Self::from_parts(
            id,
            "Automation.computeElementLayout",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.getComputedRole`.
    pub(crate) fn get_computed_role(
        id: u64,
        browsing_context_handle: &str,
        node_handle: &str,
        frame_handle: Option<&str>,
    ) -> Option<Self> {
        Self::computed_accessibility(
            id,
            "Automation.getComputedRole",
            browsing_context_handle,
            node_handle,
            frame_handle,
        )
    }

    /// Builds `Automation.getComputedLabel`.
    pub(crate) fn get_computed_label(
        id: u64,
        browsing_context_handle: &str,
        node_handle: &str,
        frame_handle: Option<&str>,
    ) -> Option<Self> {
        Self::computed_accessibility(
            id,
            "Automation.getComputedLabel",
            browsing_context_handle,
            node_handle,
            frame_handle,
        )
    }

    fn computed_accessibility(
        id: u64,
        method: &str,
        browsing_context_handle: &str,
        node_handle: &str,
        frame_handle: Option<&str>,
    ) -> Option<Self> {
        if !is_valid_handle(browsing_context_handle) || !is_valid_handle(node_handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert(
            "browsingContextHandle".into(),
            Value::from(browsing_context_handle),
        );
        params.insert("nodeHandle".into(), Value::from(node_handle));
        if let Some(frame) = frame_handle {
            if !is_valid_handle(frame) {
                return None;
            }
            params.insert("frameHandle".into(), Value::from(frame));
        }
        Self::from_parts(id, method, Some(params), RequestKind::Other)
    }

    /// Builds `Automation.performMouseInteraction`.
    pub(crate) fn perform_mouse_interaction(
        id: u64,
        handle: &str,
        x: f64,
        y: f64,
        button: MouseButton,
        interaction: MouseInteraction,
        modifiers: &[KeyModifier],
    ) -> Option<Self> {
        if !is_valid_handle(handle) || !x.is_finite() || !y.is_finite() || modifiers.len() > 5 {
            return None;
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        params.insert("position".into(), json!({ "x": x, "y": y }));
        params.insert("button".into(), Value::from(button.as_wire()));
        params.insert("interaction".into(), Value::from(interaction.as_wire()));
        params.insert(
            "modifiers".into(),
            Value::Array(
                modifiers
                    .iter()
                    .map(|modifier| Value::from(modifier.as_wire()))
                    .collect(),
            ),
        );
        Self::from_parts(
            id,
            "Automation.performMouseInteraction",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.performKeyboardInteractions`.
    pub(crate) fn perform_keyboard_interactions(
        id: u64,
        handle: &str,
        interactions: &[KeyboardInteraction],
    ) -> Option<Self> {
        if !is_valid_handle(handle)
            || interactions.is_empty()
            || interactions.len() > MAX_INTERACTIONS
        {
            return None;
        }
        let wire_steps: Option<Vec<Value>> = interactions
            .iter()
            .map(KeyboardInteraction::to_wire)
            .collect();
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        params.insert("interactions".into(), Value::Array(wire_steps?));
        Self::from_parts(
            id,
            "Automation.performKeyboardInteractions",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.cancelInteractionSequence`.
    pub(crate) fn cancel_interaction_sequence(id: u64, handle: &str) -> Option<Self> {
        if !is_valid_handle(handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        Self::from_parts(
            id,
            "Automation.cancelInteractionSequence",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.performInteractionSequence` for a bounded trusted wheel scroll.
    pub(crate) fn perform_wheel_scroll(
        id: u64,
        handle: &str,
        x: f64,
        y: f64,
        delta_x: i32,
        delta_y: i32,
    ) -> Option<Self> {
        if !is_valid_handle(handle)
            || !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || (delta_x == 0 && delta_y == 0)
        {
            return None;
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        params.insert(
            "inputSources".into(),
            json!([{ "sourceId": "wheel-0", "sourceType": "Wheel" }]),
        );
        params.insert(
            "steps".into(),
            json!([{
                "states": [{
                    "sourceId": "wheel-0",
                    "origin": "Viewport",
                    "location": { "x": x, "y": y },
                    "delta": { "width": f64::from(delta_x), "height": f64::from(delta_y) }
                }]
            }]),
        );
        Self::from_parts(
            id,
            "Automation.performInteractionSequence",
            Some(params),
            RequestKind::Other,
        )
    }

    /// Builds `Automation.takeScreenshot`.
    pub(crate) fn take_screenshot(
        id: u64,
        handle: &str,
        frame_handle: Option<&str>,
        node_handle: Option<&str>,
        scroll_into_view: bool,
        clip_to_viewport: bool,
    ) -> Option<Self> {
        if !is_valid_handle(handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert("handle".into(), Value::from(handle));
        params.insert(
            "scrollIntoViewIfNeeded".into(),
            Value::from(scroll_into_view),
        );
        params.insert("clipToViewport".into(), Value::from(clip_to_viewport));
        if let Some(frame) = frame_handle {
            if !is_valid_handle(frame) {
                return None;
            }
            params.insert("frameHandle".into(), Value::from(frame));
        }
        if let Some(node) = node_handle {
            if !is_valid_handle(node) {
                return None;
            }
            params.insert("nodeHandle".into(), Value::from(node));
        }
        Self::from_parts(
            id,
            "Automation.takeScreenshot",
            Some(params),
            RequestKind::Screenshot,
        )
    }

    /// Builds `Automation.sendInspectorMessage` wrapping a bounded read-only `InspectorCommand`.
    pub(crate) fn send_inspector_message(
        id: u64,
        browsing_context_handle: &str,
        command: &InspectorCommand,
    ) -> Option<Self> {
        if !is_valid_handle(browsing_context_handle) {
            return None;
        }
        let mut params = Map::new();
        params.insert(
            "browsingContextHandle".into(),
            Value::from(browsing_context_handle),
        );
        params.insert("message".into(), Value::from(command.as_json()));
        Self::from_parts(
            id,
            "Automation.sendInspectorMessage",
            Some(params),
            RequestKind::Other,
        )
    }

    pub(crate) fn as_json(&self) -> &str {
        &self.payload
    }

    pub(super) const fn kind(&self) -> &RequestKind {
        &self.kind
    }
}

pub(super) fn is_valid_handle(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_HANDLE_BYTES
        && !value
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
}
