//! What the user sets up in the Settings window and that no other context owns: how the
//! browser looks, the keymap they edit, and the web extensions they install.
//!
//! - [`appearance`]: the appearance choices, *Inset page* among them.
//! - [`keymap`]: the default bindings plus `keybindings.json`, and the keymap editor's changes.
//! - [`extensions`]: installing, approving, running, and updating web extensions.

pub mod appearance;
pub mod extensions;
pub mod keymap;
