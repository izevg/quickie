//! What the user sets up in the Settings window and that no other context owns: the keymap
//! they edit, and the web extensions they install.
//!
//! - [`keymap`]: the default bindings plus `keybindings.json`, and the keymap editor's changes.
//! - [`extensions`]: installing, approving, running, and updating web extensions.

pub mod extensions;
pub mod keymap;
