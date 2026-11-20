//! The panes that are forms: they scroll, and lay their sections out top down.
//!
//! - [`general`]: the default browser and the downloads folder.
//! - [`shields`]: protection, the filter lists, and keeping them fresh.
//! - [`agent`]: the agent's endpoint, model, and key.

pub mod agent;
pub mod general;
pub mod shields;
