//! Use cases of Quickie and the ports they need, as traits. Depends on the domain only; adapters
//! implement the ports and the shell wires them together.
//!
//! Ports are synchronous: an adapter that talks to something slow does it off the main thread
//! and reports back through the use cases, the way engine events do.

mod agent;
mod browsing;
mod engine;
mod error;
#[cfg(test)]
mod fakes;
mod library;
mod palette;
mod ports;
mod settings;
mod shields;
mod site;

pub use agent::{
    Agent, AgentLink, AgentLinks, AnswerEvent, AskError, Asking, LocalFound, PageReader,
    SelectingLink, SelectionError, detect_local, list_models,
};
pub use agent::{
    AgentPage, AgentTool, AgentToolCall, Conversation, ToolChatMessage, ToolChatReply,
    ToolChatRequest, complete_workspace_step,
};
pub use browsing::{Browsing, Playback};
#[cfg(any(test, feature = "conformance"))]
pub use engine::conformance;
pub use engine::{
    Access, Answer, Ask, AskKind, Capabilities, DevTools, DevToolsPanel, Dialog, DownloadEvent,
    DownloadToken, Downloads, Engine, EngineEvent, EventQueue, Expected, ExtensionCommand,
    ExtensionEvent, ExtensionHost, ExtensionInfo, ExtensionPackages, FindStep, Followed, Found,
    Fullscreen, Inspector, LinkTarget, NoExtensions, PackageSource, PageAsks, PageContent,
    PageRect, PageSurface, PageVideo, PageZoom, PortsOf, ReplyToken, Ticket, Unpacked, WindowTabs,
};
pub use error::{
    AppError, BlockerError, EngineError, HttpError, LlmError, PackageError, SecretError, StoreError,
};
pub use library::{CorruptLibrary, DownloadMoved, Library};
pub use palette::{
    AddressSuggestions, InlineCompletion, PageFacts, Palette, PaletteIndex, PaletteItem, Sources,
    Suggestion, SuggestionKind, Target,
};
pub use ports::{
    ActiveRules, AgentRepo, AppearanceRepo, BlockerRules, ChatEvent, ChatRequest, Clock,
    ContentBlockerPort, Ended, EnginePort, Entropy, ExtensionsRepo, Flow, HttpEvent, HttpMethod,
    HttpPort, HttpRequest, KeybindingText, KeymapFile, KeymapRepo, LibraryRepo, LibrarySnapshot,
    ListRules, LlmPort, Navigation, RulesStatus, SecretStore, SessionRepo, ShieldsRepo,
    ShieldsSnapshot, StopSignal, ZoomRepo,
};
pub use settings::appearance::AppearanceSettings;
pub use settings::extensions::{
    Approval, ExtensionOffer, ExtensionRun, ExtensionTrigger, Extensions, Fetched, ReadOutcome,
    UpdateCheck, UpdateOffer, fetch_packages, parse_answer,
};
pub use settings::keymap::{
    BindingLine, BindingSource, LoadedKeymap, RebindError, RejectedEntry, Rejection, check_binding,
    keymap_lines, load_keymap, rebind, reset_binding,
};
pub use shields::{CorruptShields, FetchedList, ListFetch, Refreshed, Shields, fetch_lists};
pub use site::{
    DialogAnswer, DialogGate, Permission, PermissionAnswer, PermissionGate, Prompts, Waiting, Zoom,
};
