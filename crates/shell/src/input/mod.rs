//! What the user typed, before the window dispatches it: browser shortcuts by key code, and the address
//! overlay's text as a destination.
//!
//! Both are interim homes: the keymap moves to the `commands` context and the destination rule
//! to `browsing` once the lead wires the domain.

pub mod destination;
pub mod shortcut;
