//! Shared kernel of the Quickie domain: identifiers and primitives that every bounded context
//! uses. Pure: no I/O, no async, no platform code.

mod id;
mod time;

pub use id::{TabId, WindowId};
pub use time::Timestamp;
