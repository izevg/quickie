//! Use cases of Quickie and the ports they need, as traits. Depends on the domain only; adapters
//! implement the ports and the shell wires them together.
//!
//! Ports are synchronous: an adapter that talks to something slow does it off the main thread
//! and reports back through the use cases, the way engine events do.

mod agent;
mod browsing;
mod error;
#[cfg(test)]
mod fakes;
mod keymap;
mod library;
mod palette;
mod ports;
mod prompts;
mod settings;
mod shields;

pub use agent::{Agent, AnswerEvent, AskError, Asking, LocalFound, detect_local, list_models};
pub use browsing::Browsing;
pub use error::{
    AppError, BlockerError, EngineError, HttpError, LlmError, SecretError, StoreError,
};
pub use keymap::{LoadedKeymap, RejectedEntry, Rejection, load_keymap};
pub use library::{CorruptLibrary, Library};
pub use palette::{Palette, PaletteItem, Sources, Target};
pub use ports::{
    AgentRepo, BlockerRules, ChatEvent, ChatRequest, Clock, ContentBlockerPort, Ended, EnginePort,
    Entropy, Flow, HttpEvent, HttpMethod, HttpPort, HttpRequest, KeybindingText, KeymapRepo,
    LibraryRepo, LibrarySnapshot, ListRules, LlmPort, Navigation, SecretStore, SessionRepo,
    ShieldsRepo, ShieldsSnapshot, StopSignal, ZoomRepo,
};
pub use prompts::{DialogGate, Permission, PermissionAnswer, PermissionGate, Prompts};
pub use settings::Zoom;
pub use shields::{CorruptShields, FetchedList, ListFetch, Refreshed, Shields, fetch_lists};
