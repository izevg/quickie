//! What the user typed, before the window dispatches it: browser shortcuts by key code, `⌃⇥` as
//! it is held, and the address overlay's text as a destination.
//!
//! These are interim homes: the keymap moves to the `commands` context and the destination rule
//! to `browsing`; the hold timing goes with the `tab_switcher` widget once `ui/widgets` exists.

pub mod cycle;
pub mod destination;
pub mod shortcut;
