//! Commands context: everything Quickie can do on request, and the keys that ask for it.
//!
//! A [`Command`] is a named action in the [`Catalog`], reachable from the palette. A [`Keymap`]
//! binds [`Chord`]s of [`KeyStroke`]s to commands, guarded by [`WhenExpr`] clauses, with VS Code
//! semantics: user bindings come after the defaults and win, a user entry `-<id>` removes a
//! default, and the first stroke of a two-stroke chord waits for the second.
//!
//! A shortcut never takes a key that types: the first stroke of every binding holds `⌘`, `⌥`
//! or `⌃`, or is a key that types nothing (a function key, an arrow, `⎋`), so a text field
//! always gets its characters whatever the page says about its focus.
//!
//! Keys are layout independent: the shell maps the ASCII-capable layout to a [`Key`], so `⌘⇧C`
//! works on a Russian layout. Pure: no I/O, no clock, no async.
//!
//! Platform-neutral: the composition root hands in a [`Platform`] once. Bindings are written
//! with `mod` for its primary modifier (`⌘` on macOS, Ctrl on Linux), resolved when the keymap
//! is made, and keycaps are printed the way that platform prints them.

mod command;
mod key;
mod keymap;
pub mod link_hint;

pub use command::{Catalog, Category, Command};
pub use key::{
    Chord, ChordError, Digit, FunctionKey, Key, KeyStroke, KeyStrokeError, Letter, Modifiers,
    Platform, TypingStroke,
};
pub use keymap::{
    BindingError, Conflict, ContextFlag, FocusedView, KeyBinding, Keymap, KeymapEntry, KeymapError,
    Resolution, Unbinding, WhenContext, WhenError, WhenExpr,
};
pub use quickie_domain_kernel::{CommandId, CommandIdError};
