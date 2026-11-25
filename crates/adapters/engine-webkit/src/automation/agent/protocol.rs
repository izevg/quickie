//! Pure classic `WebKit` Automation protocol codec and command validation.
//!
//! All method and parameter names match pinned `WebKit`
//! `Source/WebKit/UIProcess/Automation/Automation.json`. No raw Inspector, JavaScript, `BiDi`, or
//! arbitrary protocol commands are accepted or emitted.

use std::collections::HashSet;

use serde_json::{Map, Value, json};
use url::{Host, Origin, Url};

use crate::automation::{AutomationError, BrowsingContext, PAGE_TIMEOUT_MS, reply, validate_url};

/// Maximum UTF-8 byte length of a browsing context, frame, node, or input source handle.
pub const MAX_HANDLE_BYTES: usize = 256;

/// Maximum number of browsing contexts accepted from a single `getBrowsingContexts` reply.
pub const MAX_BROWSING_CONTEXTS: usize = 64;

/// Maximum page load timeout accepted for `navigateBrowsingContext` (milliseconds).
pub const MAX_PAGE_LOAD_TIMEOUT_MS: u32 = 30_000;

/// Maximum absolute viewport coordinate accepted for pointer interactions.
pub const MAX_VIEWPORT_COORD: f64 = 16_384.0;

/// Maximum number of key modifiers attached to a mouse interaction.
pub const MAX_KEY_MODIFIERS: usize = 5;

/// Maximum number of keyboard interaction steps in a single `performKeyboardInteractions` call.
pub const MAX_KEYBOARD_INTERACTIONS: usize = 128;

/// Maximum total UTF-8 bytes of typed text in a single `performKeyboardInteractions` call.
pub const MAX_KEYBOARD_TEXT_BYTES: usize = 4_096;

/// Maximum number of input sources in a single `performInteractionSequence` call.
pub const MAX_INPUT_SOURCES: usize = 8;

/// Maximum number of steps in a single `performInteractionSequence` call.
pub const MAX_INTERACTION_STEPS: usize = 64;

/// Maximum duration of a single interaction step (milliseconds).
pub const MAX_STEP_DURATION_MS: u32 = 5_000;

/// Maximum total duration across all steps in an interaction sequence (milliseconds).
pub const MAX_SEQUENCE_DURATION_MS: u32 = 10_000;

/// Maximum base64 payload size accepted for a bounded screenshot (4 MiB).
pub const MAX_SCREENSHOT_BASE64_BYTES: usize = 4 * 1024 * 1024;

/// Maximum inbound JSON message size accepted from the native session (8 MiB).
pub const MAX_INBOUND_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// A validated HTTP or HTTPS tuple origin approved for isolated Agent navigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovedOrigin {
    origin: Origin,
}

impl ApprovedOrigin {
    /// Derives a provenance-approved origin from a validated HTTP(S) URL without credentials.
    ///
    /// # Errors
    /// Returns [`AutomationError::InvalidUrl`] if `url` is not an HTTP(S) URL with a valid host
    /// and no embedded credentials.
    pub fn from_url(url: &Url) -> Result<Self, AutomationError> {
        validate_url(url)?;
        let origin = url.origin();
        match &origin {
            Origin::Tuple(scheme, host, _)
                if matches!(scheme.as_str(), "http" | "https")
                    && !matches!(host, Host::Domain(domain) if domain.is_empty()) =>
            {
                Ok(Self { origin })
            }
            _ => Err(AutomationError::InvalidUrl),
        }
    }

    /// The underlying HTTP(S) tuple origin.
    #[must_use]
    pub const fn origin(&self) -> &Origin {
        &self.origin
    }

    /// Checks that `url` is a valid credential-free HTTP(S) URL matching this approved origin.
    ///
    /// # Errors
    /// Returns [`AutomationError::InvalidUrl`] if `url` is not a valid HTTP(S) URL without
    /// credentials, or [`AutomationError::OriginMismatch`] if its origin differs.
    pub fn verify_same_origin(&self, url: &Url) -> Result<(), AutomationError> {
        validate_url(url)?;
        if url.origin() == self.origin {
            Ok(())
        } else {
            Err(AutomationError::OriginMismatch)
        }
    }
}

/// `Automation.PageLoadStrategy` enum from `Automation.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageLoadStrategy {
    /// Return immediately without waiting for page load.
    None,
    /// Wait for `DOMContentLoaded`.
    Eager,
    /// Wait for the full document `load` event.
    #[default]
    Normal,
}

impl PageLoadStrategy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Eager => "Eager",
            Self::Normal => "Normal",
        }
    }
}

/// Typed navigation command for `Automation.navigateBrowsingContext`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationCommand {
    /// Target browsing context handle.
    pub handle: String,
    /// Destination HTTP(S) URL without credentials.
    pub url: Url,
    /// Provenance-approved origin that `url` must match.
    pub approved_origin: ApprovedOrigin,
    /// Page load completion strategy.
    pub page_load_strategy: PageLoadStrategy,
    /// Navigation timeout in milliseconds (`1..=MAX_PAGE_LOAD_TIMEOUT_MS`).
    pub page_load_timeout_ms: u32,
}

impl NavigationCommand {
    /// Creates a navigation command with `PageLoadStrategy::Normal` and the default timeout.
    #[must_use]
    pub fn new(handle: impl Into<String>, url: Url, approved_origin: ApprovedOrigin) -> Self {
        Self {
            handle: handle.into(),
            url,
            approved_origin,
            page_load_strategy: PageLoadStrategy::Normal,
            page_load_timeout_ms: PAGE_TIMEOUT_MS,
        }
    }
}

/// Viewport coordinate pair (`Automation.Point` in `Automation.json`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportPoint {
    /// Horizontal viewport coordinate in CSS pixels.
    pub x: f64,
    /// Vertical viewport coordinate in CSS pixels.
    pub y: f64,
}

/// 2D delta size (`Automation.Size` in `Automation.json`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionDelta {
    /// Horizontal delta.
    pub width: f64,
    /// Vertical delta.
    pub height: f64,
}

/// `Automation.MouseButton` enum from `Automation.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    /// No mouse button.
    None,
    /// Primary (left) button.
    Left,
    /// Auxiliary (middle) button.
    Middle,
    /// Secondary (right) button.
    Right,
    /// Browser back button.
    Back,
    /// Browser forward button.
    Forward,
}

impl MouseButton {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Left => "Left",
            Self::Middle => "Middle",
            Self::Right => "Right",
            Self::Back => "Back",
            Self::Forward => "Forward",
        }
    }
}

/// `Automation.MouseInteraction` enum from `Automation.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseInteraction {
    /// Move pointer without pressing.
    Move,
    /// Press button down.
    Down,
    /// Release button up.
    Up,
    /// Press and release once.
    SingleClick,
    /// Double-click.
    DoubleClick,
}

impl MouseInteraction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Down => "Down",
            Self::Up => "Up",
            Self::SingleClick => "SingleClick",
            Self::DoubleClick => "DoubleClick",
        }
    }
}

/// `Automation.KeyModifier` enum from `Automation.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyModifier {
    /// Caps Lock modifier.
    CapsLock,
    /// Control modifier.
    Control,
    /// Shift modifier.
    Shift,
    /// Meta (Command) modifier.
    Meta,
    /// Alt (Option) modifier.
    Alt,
}

impl KeyModifier {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CapsLock => "CapsLock",
            Self::Control => "Control",
            Self::Shift => "Shift",
            Self::Meta => "Meta",
            Self::Alt => "Alt",
        }
    }
}

/// Typed mouse command for `Automation.performMouseInteraction`.
#[derive(Clone, Debug, PartialEq)]
pub struct MouseCommand {
    /// Target browsing context handle.
    pub handle: String,
    /// Viewport coordinates for the interaction.
    pub position: ViewportPoint,
    /// Mouse button to use.
    pub button: MouseButton,
    /// Interaction kind.
    pub interaction: MouseInteraction,
    /// Active key modifiers during the interaction.
    pub modifiers: Vec<KeyModifier>,
}

impl MouseCommand {
    /// Builds a left single-click command with no modifiers.
    #[must_use]
    pub fn single_click(handle: impl Into<String>, position: ViewportPoint) -> Self {
        Self {
            handle: handle.into(),
            position,
            button: MouseButton::Left,
            interaction: MouseInteraction::SingleClick,
            modifiers: Vec::new(),
        }
    }
}

/// Subset of `Automation.VirtualKey` values from `Automation.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VirtualKey {
    /// Shift key.
    Shift,
    /// Control key.
    Control,
    /// Alternate (Option/Alt) key.
    Alternate,
    /// Meta key.
    Meta,
    /// Command key.
    Command,
    /// Backspace key.
    Backspace,
    /// Tab key.
    Tab,
    /// Enter key.
    Enter,
    /// Return key.
    Return,
    /// Escape key.
    Escape,
    /// Space key.
    Space,
    /// Page Up key.
    PageUp,
    /// Page Down key.
    PageDown,
    /// End key.
    End,
    /// Home key.
    Home,
    /// Left Arrow key.
    LeftArrow,
    /// Up Arrow key.
    UpArrow,
    /// Right Arrow key.
    RightArrow,
    /// Down Arrow key.
    DownArrow,
    /// Delete key.
    Delete,
}

impl VirtualKey {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Shift => "Shift",
            Self::Control => "Control",
            Self::Alternate => "Alternate",
            Self::Meta => "Meta",
            Self::Command => "Command",
            Self::Backspace => "Backspace",
            Self::Tab => "Tab",
            Self::Enter => "Enter",
            Self::Return => "Return",
            Self::Escape => "Escape",
            Self::Space => "Space",
            Self::PageUp => "PageUp",
            Self::PageDown => "PageDown",
            Self::End => "End",
            Self::Home => "Home",
            Self::LeftArrow => "LeftArrow",
            Self::UpArrow => "UpArrow",
            Self::RightArrow => "RightArrow",
            Self::DownArrow => "DownArrow",
            Self::Delete => "Delete",
        }
    }
}

/// One typed keyboard step (`Automation.KeyboardInteraction` in `Automation.json`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyboardStep {
    /// `KeyPress` on a virtual key.
    PressKey(VirtualKey),
    /// `KeyRelease` on a virtual key.
    ReleaseKey(VirtualKey),
    /// `InsertByKey` for a virtual key.
    InsertKey(VirtualKey),
    /// `InsertByKey` for a non-empty Unicode text sequence.
    InsertText(String),
}

/// Typed keyboard command for `Automation.performKeyboardInteractions`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardCommand {
    /// Target browsing context handle.
    pub handle: String,
    /// Ordered keyboard steps (`1..=MAX_KEYBOARD_INTERACTIONS`).
    pub interactions: Vec<KeyboardStep>,
}

/// `Automation.InputSourceType` enum from `Automation.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputSourceType {
    /// Pause-only null source.
    Null,
    /// Mouse pointer source.
    Mouse,
    /// Keyboard source.
    Keyboard,
    /// Touch pointer source.
    Touch,
    /// Scroll wheel source.
    Wheel,
    /// Pen pointer source.
    Pen,
}

impl InputSourceType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "Null",
            Self::Mouse => "Mouse",
            Self::Keyboard => "Keyboard",
            Self::Touch => "Touch",
            Self::Wheel => "Wheel",
            Self::Pen => "Pen",
        }
    }
}

/// `Automation.InputSource` from `Automation.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSource {
    /// Unique identifier for this input source within the sequence.
    pub source_id: String,
    /// Input device type.
    pub source_type: InputSourceType,
}

/// `Automation.MouseMoveOrigin` enum from `Automation.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseMoveOrigin {
    /// Relative to the visual viewport.
    Viewport,
    /// Relative to the current pointer location.
    Pointer,
    /// Relative to a referenced element (`nodeHandle`).
    Element,
}

impl MouseMoveOrigin {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Viewport => "Viewport",
            Self::Pointer => "Pointer",
            Self::Element => "Element",
        }
    }
}

/// `Automation.InputSourceState` from `Automation.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct InputSourceState {
    /// Identifier matching one declared [`InputSource::source_id`].
    pub source_id: String,
    /// Character keys held down for a keyboard source.
    pub pressed_char_keys: Option<Vec<String>>,
    /// Virtual keys held down for a keyboard source.
    pub pressed_virtual_keys: Option<Vec<VirtualKey>>,
    /// Button held down for a mouse/touch source.
    pub pressed_button: Option<MouseButton>,
    /// Mouse interaction kind for a mouse source.
    pub mouse_interaction: Option<MouseInteraction>,
    /// Coordinate origin for a mouse or wheel move.
    pub origin: Option<MouseMoveOrigin>,
    /// Element handle when `origin == Some(MouseMoveOrigin::Element)`.
    pub node_handle: Option<String>,
    /// Pointer location in view coordinates.
    pub location: Option<ViewportPoint>,
    /// Wheel scroll delta.
    pub delta: Option<InteractionDelta>,
    /// Minimum tick duration in milliseconds (`<= MAX_STEP_DURATION_MS`).
    pub duration_ms: Option<u32>,
}

impl InputSourceState {
    /// Creates an empty state transition for `source_id`.
    #[must_use]
    pub fn new(source_id: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            pressed_char_keys: None,
            pressed_virtual_keys: None,
            pressed_button: None,
            mouse_interaction: None,
            origin: None,
            node_handle: None,
            location: None,
            delta: None,
            duration_ms: None,
        }
    }
}

/// `Automation.InteractionStep` from `Automation.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct InteractionStep {
    /// Input source states for this tick.
    pub states: Vec<InputSourceState>,
}

/// Typed interaction sequence command for `Automation.performInteractionSequence`.
#[derive(Clone, Debug, PartialEq)]
pub struct InteractionSequenceCommand {
    /// Target browsing context handle.
    pub handle: String,
    /// Optional target frame handle (main frame when `None`).
    pub frame_handle: Option<String>,
    /// Declared input sources (`1..=MAX_INPUT_SOURCES`).
    pub input_sources: Vec<InputSource>,
    /// Ordered interaction ticks (`1..=MAX_INTERACTION_STEPS`).
    pub steps: Vec<InteractionStep>,
}

/// Typed cancel command for `Automation.cancelInteractionSequence`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelInteractionSequenceCommand {
    /// Target browsing context handle.
    pub handle: String,
    /// Optional frame handle passed to `performInteractionSequence`.
    pub frame_handle: Option<String>,
}

/// Typed screenshot command for `Automation.takeScreenshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenshotCommand {
    /// Target browsing context handle.
    pub handle: String,
    /// Optional frame handle (main frame when `None`).
    pub frame_handle: Option<String>,
    /// Optional node handle (document when `None`).
    pub node_handle: Option<String>,
    /// Whether to scroll an element into view before capturing.
    pub scroll_into_view_if_needed: Option<bool>,
    /// Whether to clip the capture to the visual viewport.
    pub clip_to_viewport: bool,
    /// Upper bound in bytes on the returned base64 PNG payload (`1..=MAX_SCREENSHOT_BASE64_BYTES`).
    pub max_base64_bytes: usize,
}

impl ScreenshotCommand {
    /// Builds a viewport-clipped screenshot command with the hard ceiling.
    #[must_use]
    pub fn viewport(handle: impl Into<String>) -> Self {
        Self {
            handle: handle.into(),
            frame_handle: None,
            node_handle: None,
            scroll_into_view_if_needed: None,
            clip_to_viewport: true,
            max_base64_bytes: MAX_SCREENSHOT_BASE64_BYTES,
        }
    }
}

/// Bounded base64 PNG screenshot payload returned by `Automation.takeScreenshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenshotData {
    /// Base64-encoded PNG bytes within the requested bound.
    pub png_base64: String,
}

/// Unsolicited classic `WebKit` Automation events routed to an observer.
///
/// Raw Inspector (`receiveInspectorMessage`) and `BiDi` (`bidiMessageSent`) passthrough payloads are
/// intentionally excluded so no raw debugging protocol messages cross the adapter boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutomationEvent {
    /// `Automation.browsingContextCleared`: the context navigated or closed.
    BrowsingContextCleared {
        /// Handle of the cleared browsing context.
        browsing_context_handle: String,
    },
    /// `Automation.fileChooserDismissed`: a file input chooser finished.
    ///
    /// Carries only the count of selected files rather than local filesystem paths.
    FileChooserDismissed {
        /// Handle of the browsing context.
        browsing_context_handle: String,
        /// Whether file selection was cancelled.
        selection_cancelled: bool,
        /// Number of selected files reported by `WebKit`.
        selected_files_count: usize,
    },
}

/// Typed command output parsed from a matched `Automation.*` response.
#[derive(Debug, PartialEq)]
pub(crate) enum CommandResponse {
    Contexts(Vec<BrowsingContext>),
    Unit,
    Position(ViewportPoint),
    Screenshot(ScreenshotData),
}

/// Expected response shape for an in-flight classic Automation request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResponseSpec {
    Contexts,
    Unit,
    Position,
    Screenshot { max_base64_bytes: usize },
}

/// Encoded allowlisted `Automation.*` command ready to wrap with a unique request ID.
#[derive(Debug)]
pub(crate) struct EncodedCommand {
    pub(crate) method: &'static str,
    pub(crate) params: Value,
    pub(crate) response: ResponseSpec,
}

pub(crate) fn encode_get_browsing_contexts() -> EncodedCommand {
    EncodedCommand {
        method: "Automation.getBrowsingContexts",
        params: json!({}),
        response: ResponseSpec::Contexts,
    }
}

pub(crate) fn encode_navigate(command: &NavigationCommand) -> Result<EncodedCommand, AutomationError> {
    validate_handle(&command.handle)?;
    command.approved_origin.verify_same_origin(&command.url)?;
    if command.page_load_timeout_ms == 0 || command.page_load_timeout_ms > MAX_PAGE_LOAD_TIMEOUT_MS
    {
        return Err(AutomationError::InvalidCommand);
    }
    Ok(EncodedCommand {
        method: "Automation.navigateBrowsingContext",
        params: json!({
            "handle": command.handle,
            "url": command.url.as_str(),
            "pageLoadStrategy": command.page_load_strategy.as_str(),
            "pageLoadTimeout": command.page_load_timeout_ms,
        }),
        response: ResponseSpec::Unit,
    })
}

pub(crate) fn encode_mouse_interaction(
    command: &MouseCommand,
) -> Result<EncodedCommand, AutomationError> {
    validate_handle(&command.handle)?;
    validate_point(command.position)?;
    if command.modifiers.len() > MAX_KEY_MODIFIERS {
        return Err(AutomationError::InvalidCommand);
    }
    let mut seen = HashSet::with_capacity(command.modifiers.len());
    let mut modifiers = Vec::with_capacity(command.modifiers.len());
    for &modifier in &command.modifiers {
        if !seen.insert(modifier) {
            return Err(AutomationError::InvalidCommand);
        }
        modifiers.push(modifier.as_str());
    }
    Ok(EncodedCommand {
        method: "Automation.performMouseInteraction",
        params: json!({
            "handle": command.handle,
            "position": {
                "x": command.position.x,
                "y": command.position.y,
            },
            "button": command.button.as_str(),
            "interaction": command.interaction.as_str(),
            "modifiers": modifiers,
        }),
        response: ResponseSpec::Position,
    })
}

pub(crate) fn encode_keyboard_interactions(
    command: &KeyboardCommand,
) -> Result<EncodedCommand, AutomationError> {
    validate_handle(&command.handle)?;
    if command.interactions.is_empty() || command.interactions.len() > MAX_KEYBOARD_INTERACTIONS {
        return Err(AutomationError::InvalidCommand);
    }
    let mut total_text_bytes = 0_usize;
    let mut encoded = Vec::with_capacity(command.interactions.len());
    for step in &command.interactions {
        match step {
            KeyboardStep::PressKey(key) => {
                encoded.push(json!({
                    "type": "KeyPress",
                    "key": key.as_str(),
                }));
            }
            KeyboardStep::ReleaseKey(key) => {
                encoded.push(json!({
                    "type": "KeyRelease",
                    "key": key.as_str(),
                }));
            }
            KeyboardStep::InsertKey(key) => {
                encoded.push(json!({
                    "type": "InsertByKey",
                    "key": key.as_str(),
                }));
            }
            KeyboardStep::InsertText(text) => {
                if text.is_empty() || text.contains('\0') {
                    return Err(AutomationError::InvalidCommand);
                }
                total_text_bytes = total_text_bytes.saturating_add(text.len());
                if total_text_bytes > MAX_KEYBOARD_TEXT_BYTES {
                    return Err(AutomationError::InvalidCommand);
                }
                encoded.push(json!({
                    "type": "InsertByKey",
                    "text": text,
                }));
            }
        }
    }
    Ok(EncodedCommand {
        method: "Automation.performKeyboardInteractions",
        params: json!({
            "handle": command.handle,
            "interactions": encoded,
        }),
        response: ResponseSpec::Unit,
    })
}

pub(crate) fn encode_interaction_sequence(
    command: &InteractionSequenceCommand,
) -> Result<EncodedCommand, AutomationError> {
    validate_handle(&command.handle)?;
    if let Some(frame) = &command.frame_handle {
        validate_handle(frame)?;
    }
    if command.input_sources.is_empty()
        || command.input_sources.len() > MAX_INPUT_SOURCES
        || command.steps.is_empty()
        || command.steps.len() > MAX_INTERACTION_STEPS
    {
        return Err(AutomationError::InvalidCommand);
    }

    let mut known_sources = HashSet::with_capacity(command.input_sources.len());
    let mut sources_json = Vec::with_capacity(command.input_sources.len());
    for source in &command.input_sources {
        validate_handle(&source.source_id)?;
        if !known_sources.insert(source.source_id.as_str()) {
            return Err(AutomationError::InvalidCommand);
        }
        sources_json.push(json!({
            "sourceId": source.source_id,
            "sourceType": source.source_type.as_str(),
        }));
    }

    let mut total_duration_ms = 0_u32;
    let mut steps_json = Vec::with_capacity(command.steps.len());
    for step in &command.steps {
        if step.states.len() > command.input_sources.len() {
            return Err(AutomationError::InvalidCommand);
        }
        let mut step_max_duration = 0_u32;
        let mut states_json = Vec::with_capacity(step.states.len());
        for state in &step.states {
            if !known_sources.contains(state.source_id.as_str()) {
                return Err(AutomationError::InvalidCommand);
            }
            let (encoded_state, duration) = encode_input_source_state(state)?;
            step_max_duration = step_max_duration.max(duration);
            states_json.push(encoded_state);
        }
        total_duration_ms = total_duration_ms
            .checked_add(step_max_duration)
            .filter(|&sum| sum <= MAX_SEQUENCE_DURATION_MS)
            .ok_or(AutomationError::InvalidCommand)?;
        steps_json.push(json!({ "states": states_json }));
    }

    let mut params = Map::new();
    params.insert("handle".to_owned(), Value::String(command.handle.clone()));
    if let Some(frame) = &command.frame_handle {
        params.insert("frameHandle".to_owned(), Value::String(frame.clone()));
    }
    params.insert("inputSources".to_owned(), Value::Array(sources_json));
    params.insert("steps".to_owned(), Value::Array(steps_json));

    Ok(EncodedCommand {
        method: "Automation.performInteractionSequence",
        params: Value::Object(params),
        response: ResponseSpec::Unit,
    })
}

pub(crate) fn encode_cancel_interaction_sequence(
    command: &CancelInteractionSequenceCommand,
) -> Result<EncodedCommand, AutomationError> {
    validate_handle(&command.handle)?;
    let mut params = Map::new();
    params.insert("handle".to_owned(), Value::String(command.handle.clone()));
    if let Some(frame) = &command.frame_handle {
        validate_handle(frame)?;
        params.insert("frameHandle".to_owned(), Value::String(frame.clone()));
    }
    Ok(EncodedCommand {
        method: "Automation.cancelInteractionSequence",
        params: Value::Object(params),
        response: ResponseSpec::Unit,
    })
}

pub(crate) fn encode_take_screenshot(
    command: &ScreenshotCommand,
) -> Result<EncodedCommand, AutomationError> {
    validate_handle(&command.handle)?;
    if command.max_base64_bytes == 0 || command.max_base64_bytes > MAX_SCREENSHOT_BASE64_BYTES {
        return Err(AutomationError::InvalidCommand);
    }
    let mut params = Map::new();
    params.insert("handle".to_owned(), Value::String(command.handle.clone()));
    if let Some(frame) = &command.frame_handle {
        validate_handle(frame)?;
        params.insert("frameHandle".to_owned(), Value::String(frame.clone()));
    }
    if let Some(node) = &command.node_handle {
        validate_handle(node)?;
        params.insert("nodeHandle".to_owned(), Value::String(node.clone()));
    }
    if let Some(scroll) = command.scroll_into_view_if_needed {
        params.insert("scrollIntoViewIfNeeded".to_owned(), Value::Bool(scroll));
    }
    params.insert(
        "clipToViewport".to_owned(),
        Value::Bool(command.clip_to_viewport),
    );
    Ok(EncodedCommand {
        method: "Automation.takeScreenshot",
        params: Value::Object(params),
        response: ResponseSpec::Screenshot {
            max_base64_bytes: command.max_base64_bytes,
        },
    })
}

pub(crate) fn parse_response(
    spec: ResponseSpec,
    result: &Value,
) -> Result<CommandResponse, AutomationError> {
    match spec {
        ResponseSpec::Contexts => {
            let contexts = reply::contexts(result)?;
            if contexts.len() > MAX_BROWSING_CONTEXTS {
                return Err(AutomationError::ResponseTooLarge);
            }
            Ok(CommandResponse::Contexts(contexts))
        }
        ResponseSpec::Unit => Ok(CommandResponse::Unit),
        ResponseSpec::Position => {
            let position = result
                .get("position")
                .and_then(Value::as_object)
                .ok_or(AutomationError::InvalidReply)?;
            let x = position
                .get("x")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or(AutomationError::InvalidReply)?;
            let y = position
                .get("y")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or(AutomationError::InvalidReply)?;
            Ok(CommandResponse::Position(ViewportPoint { x, y }))
        }
        ResponseSpec::Screenshot { max_base64_bytes } => {
            let data = result
                .get("data")
                .and_then(Value::as_str)
                .filter(|data| !data.is_empty())
                .ok_or(AutomationError::InvalidReply)?;
            if data.len() > max_base64_bytes {
                return Err(AutomationError::ResponseTooLarge);
            }
            if !is_valid_base64_chars(data) {
                return Err(AutomationError::InvalidReply);
            }
            Ok(CommandResponse::Screenshot(ScreenshotData {
                png_base64: data.to_owned(),
            }))
        }
    }
}

pub(crate) fn decode_event(message: &Value) -> Option<AutomationEvent> {
    let method = message.get("method")?.as_str()?;
    let params = message.get("params").and_then(Value::as_object);
    match method {
        "Automation.browsingContextCleared" => {
            let handle = params?
                .get("browsingContextHandle")?
                .as_str()
                .filter(|handle| validate_handle(handle).is_ok())?;
            Some(AutomationEvent::BrowsingContextCleared {
                browsing_context_handle: handle.to_owned(),
            })
        }
        "Automation.fileChooserDismissed" => {
            let params = params?;
            let handle = params
                .get("browsingContextHandle")?
                .as_str()
                .filter(|handle| validate_handle(handle).is_ok())?;
            let selection_cancelled = params.get("selectionCancelled")?.as_bool()?;
            let selected_files_count = params
                .get("selectedFiles")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Some(AutomationEvent::FileChooserDismissed {
                browsing_context_handle: handle.to_owned(),
                selection_cancelled,
                selected_files_count,
            })
        }
        // Raw Inspector and BiDi passthrough events are never surfaced outside the adapter.
        _ => None,
    }
}

fn encode_input_source_state(
    state: &InputSourceState,
) -> Result<(Value, u32), AutomationError> {
    let mut map = Map::new();
    map.insert(
        "sourceId".to_owned(),
        Value::String(state.source_id.clone()),
    );
    if let Some(keys) = &state.pressed_char_keys {
        if keys.len() > MAX_KEYBOARD_INTERACTIONS {
            return Err(AutomationError::InvalidCommand);
        }
        let mut encoded_keys = Vec::with_capacity(keys.len());
        for key in keys {
            if key.is_empty() || key.len() > 16 || key.contains('\0') {
                return Err(AutomationError::InvalidCommand);
            }
            encoded_keys.push(Value::String(key.clone()));
        }
        map.insert("pressedCharKeys".to_owned(), Value::Array(encoded_keys));
    }
    if let Some(keys) = &state.pressed_virtual_keys {
        if keys.len() > MAX_KEYBOARD_INTERACTIONS {
            return Err(AutomationError::InvalidCommand);
        }
        map.insert(
            "pressedVirtualKeys".to_owned(),
            Value::Array(
                keys.iter()
                    .map(|key| Value::String(key.as_str().to_owned()))
                    .collect(),
            ),
        );
    }
    if let Some(button) = state.pressed_button {
        map.insert(
            "pressedButton".to_owned(),
            Value::String(button.as_str().to_owned()),
        );
    }
    if let Some(interaction) = state.mouse_interaction {
        map.insert(
            "mouseInteraction".to_owned(),
            Value::String(interaction.as_str().to_owned()),
        );
    }
    if let Some(origin) = state.origin {
        map.insert(
            "origin".to_owned(),
            Value::String(origin.as_str().to_owned()),
        );
    }
    if let Some(node) = &state.node_handle {
        validate_handle(node)?;
        map.insert("nodeHandle".to_owned(), Value::String(node.clone()));
    }
    if let Some(location) = state.location {
        validate_point(location)?;
        map.insert(
            "location".to_owned(),
            json!({ "x": location.x, "y": location.y }),
        );
    }
    if let Some(delta) = state.delta {
        if !delta.width.is_finite()
            || !delta.height.is_finite()
            || delta.width.abs() > MAX_VIEWPORT_COORD
            || delta.height.abs() > MAX_VIEWPORT_COORD
        {
            return Err(AutomationError::InvalidCommand);
        }
        map.insert(
            "delta".to_owned(),
            json!({ "width": delta.width, "height": delta.height }),
        );
    }
    let duration = state.duration_ms.unwrap_or(0);
    if duration > MAX_STEP_DURATION_MS {
        return Err(AutomationError::InvalidCommand);
    }
    if let Some(duration_ms) = state.duration_ms {
        map.insert("duration".to_owned(), Value::from(duration_ms));
    }
    Ok((Value::Object(map), duration))
}

fn validate_handle(handle: &str) -> Result<(), AutomationError> {
    if handle.is_empty() || handle.len() > MAX_HANDLE_BYTES || handle.contains('\0') {
        Err(AutomationError::InvalidCommand)
    } else {
        Ok(())
    }
}

fn validate_point(point: ViewportPoint) -> Result<(), AutomationError> {
    if point.x.is_finite()
        && point.y.is_finite()
        && point.x >= 0.0
        && point.y >= 0.0
        && point.x <= MAX_VIEWPORT_COORD
        && point.y <= MAX_VIEWPORT_COORD
    {
        Ok(())
    } else {
        Err(AutomationError::InvalidCommand)
    }
}

fn is_valid_base64_chars(data: &str) -> bool {
    data.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'+' | b'/' | b'=' | b'\r' | b'\n')
    })
}
