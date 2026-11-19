//! Use cases of Quickie and the ports they need, as traits. Depends on the domain only; adapters
//! implement the ports and the shell wires them together.
//!
//! Ports are synchronous: an adapter that talks to something slow does it off the main thread
//! and reports back through the use cases, the way engine events do.

mod browsing;
mod error;
mod ports;

pub use browsing::Browsing;
pub use error::{AppError, EngineError, StoreError};
pub use ports::{Clock, EnginePort, Navigation, SessionRepo};
