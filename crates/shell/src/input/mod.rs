//! What the user typed, before the window dispatches it: key strokes by key code, `⌃⇥` as it is
//! held, and the address overlay's text as a destination.
//!
//! These are interim homes: the destination rule moves to `browsing`, and the hold timing goes
//! with the `tab_switcher` widget.

pub mod cycle;
pub mod destination;
pub mod keystroke;
