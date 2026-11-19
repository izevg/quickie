//! Use cases of Quickie and the ports they need, as traits. Depends on the domain only; adapters
//! implement the ports and the shell wires them together.
//!
//! Ports are synchronous: an adapter that talks to something slow does it off the main thread
//! and reports back through the use cases, the way engine events do.

mod browsing;
mod error;
#[cfg(test)]
mod fakes;
mod keymap;
mod library;
mod palette;
mod ports;
mod prompts;
mod shields;

pub use browsing::Browsing;
pub use error::{AppError, BlockerError, EngineError, HttpError, StoreError};
pub use keymap::{LoadedKeymap, RejectedEntry, Rejection, load_keymap};
pub use library::{CorruptLibrary, Library};
pub use palette::{Palette, PaletteItem, Sources, Target};
pub use ports::{
    BlockerRules, Clock, ContentBlockerPort, EnginePort, HttpPort, KeybindingText, KeymapRepo,
    LibraryRepo, LibrarySnapshot, ListRules, Navigation, SessionRepo, ShieldsRepo, ShieldsSnapshot,
};
pub use prompts::{DialogGate, Permission, PermissionAnswer, PermissionGate, Prompts};
pub use shields::{CorruptShields, FetchedList, ListFetch, Refreshed, Shields, fetch_lists};
