//! The built-in agent on screen (`docs/design/mocks/agent/`): the answer panel, the badge and
//! the window frame that show while it works, and its settings.
//!
//! Everything here treats the answer as untrusted text from an endpoint that read an untrusted
//! page: it is shown, never followed. No image loads, no link opens without a click, and an
//! endpoint's error message is plain text.

pub mod agent_settings;
pub mod answer_panel;
pub mod session;
