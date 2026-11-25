//! Typed isolated `Profile::Agent` MCP tool contracts, schemas, and wire codecs.
//!
//! All operations in this module are scoped strictly to [`quickie_application::mcp::AgentContextId`]
//! and never accept `Profile::Main` diagnostic attachment IDs, raw engine commands, arbitrary
//! JavaScript, or caller-supplied approval claims.

pub mod catalog;
pub mod types;
pub(crate) mod wire;

pub use self::types::{
    AgentActResult, AgentActStatus, AgentAction, AgentContextCloseResult, AgentContextListResult,
    AgentContextOpenResult, AgentContextSummary, AgentElementRef, AgentKey, AgentKeyModifier,
    AgentLocateResult, AgentLocator, AgentLocatorKind, AgentMouseButton, AgentNavigateResult,
    AgentNavigateStatus, AgentObservedState, AgentOperation, AgentScreenshotResult,
    AgentScreenshotTarget, AgentViewport, AgentWaitCondition, AgentWaitResult, AgentWaitStage,
    AgentWaitStatus, DEFAULT_ACTION_TIMEOUT_MS, DEFAULT_LOCATE_RESULTS,
    DEFAULT_NAVIGATE_TIMEOUT_MS, DEFAULT_SCREENSHOT_MAX_BYTES, DEFAULT_VIEWPORT_HEIGHT,
    DEFAULT_VIEWPORT_WIDTH, ElementBounds, ElementProvenance, FrameScope, LocatedElement,
    MAX_ACTION_TIMEOUT_MS, MAX_BBOX_COORD, MAX_ELEMENT_ID_BYTES, MAX_INPUT_TEXT_BYTES,
    MAX_KEY_MODIFIERS, MAX_LOCATE_RESULTS, MAX_LOCATOR_TEXT_BYTES, MAX_REASON_BYTES,
    MAX_ROLE_BYTES, MAX_SCREENSHOT_BYTES, MAX_SCROLL_DELTA, MAX_URL_BYTES, MAX_VIEWPORT_HEIGHT,
    MAX_VIEWPORT_WIDTH, MIN_ACTION_TIMEOUT_MS, MIN_VIEWPORT_HEIGHT, MIN_VIEWPORT_WIDTH,
    SelectOptionBy,
};
