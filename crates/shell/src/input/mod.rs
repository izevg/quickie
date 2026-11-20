//! What the user typed, before the window dispatches it: key strokes by key code, and `⌃⇥` as
//! it is held. Where typed text goes is the browsing context's rule (`Destination`).
//!
//! The hold timing is an interim home: it goes with the `tab_switcher` widget.

pub mod cycle;
pub mod keystroke;
