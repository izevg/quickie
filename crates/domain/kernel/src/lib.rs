//! Shared kernel of the Quickie domain: identifiers and primitives that every bounded context
//! uses. Pure: no I/O, no async, no platform code.

mod id;
mod site;
mod time;

pub use id::{CommandId, CommandIdError, TabId, WindowId};
pub use site::{Site, SiteError};
pub use time::Timestamp;
