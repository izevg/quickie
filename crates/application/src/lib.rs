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
    Agent, AnswerEvent, AskError, Asking, LocalFound, PageReader, detect_local, list_models,
};
pub use browsing::Browsing;
#[cfg(any(test, feature = "conformance"))]
pub use engine::conformance;
pub use engine::{
    Access, Answer, Ask, AskKind, Capabilities, Dialog, DownloadEvent, DownloadToken, EngineEvent,
    EventQueue, Expected, ExtensionCommand, ExtensionEvent, ExtensionHost, ExtensionInfo,
    ExtensionPackages, FindStep, Followed, Found, LinkTarget, PackageSource, PageContent, PageRect,
    ReplyToken, Ticket, Unpacked, WindowTabs,
};
pub use error::{
    AppError, BlockerError, EngineError, HttpError, LlmError, PackageError, SecretError, StoreError,
};
pub use library::{CorruptLibrary, Library};
pub use palette::{PageFacts, Palette, PaletteIndex, PaletteItem, Sources, Target};
pub use ports::{
    AgentRepo, BlockerRules, ChatEvent, ChatRequest, Clock, ContentBlockerPort, Ended, EnginePort,
    Entropy, ExtensionsRepo, Flow, HttpEvent, HttpMethod, HttpPort, HttpRequest, KeybindingText,
    KeymapFile, KeymapRepo, LibraryRepo, LibrarySnapshot, ListRules, LlmPort, Navigation,
    SecretStore, SessionRepo, ShieldsRepo, ShieldsSnapshot, StopSignal, ZoomRepo,
};
pub use settings::extensions::{
    Approval, ExtensionOffer, ExtensionRun, ExtensionTrigger, Extensions, Fetched, ReadOutcome,
    UpdateCheck, UpdateOffer, fetch_packages, parse_answer,
};
pub use settings::keymap::{
    BindingLine, BindingSource, LoadedKeymap, RebindError, RejectedEntry, Rejection, check_binding,
    keymap_lines, load_keymap, rebind, reset_binding,
};
pub use shields::{CorruptShields, FetchedList, ListFetch, Refreshed, Shields, fetch_lists};
pub use site::{DialogGate, Permission, PermissionAnswer, PermissionGate, Prompts, Zoom};
