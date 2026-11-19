//! Commands context: everything Quickie can do on request, and the keys that ask for it.
//!
//! A [`Command`] is a named action in the [`Catalog`], reachable from the palette. A [`Keymap`]
//! binds [`Chord`]s of [`KeyStroke`]s to commands, guarded by [`WhenExpr`] clauses, with VS Code
//! semantics: user bindings come after the defaults and win, a user entry `-<id>` removes a
//! default, and the first stroke of a two-stroke chord waits for the second.
//!
//! Keys are layout independent: the shell maps the ASCII-capable layout to a [`Key`], so `⌘⇧C`
//! works on a Russian layout. Pure: no I/O, no clock, no async.

mod command;
mod key;
mod keymap;

pub use command::{Catalog, Category, Command};
pub use key::{
    Chord, ChordError, Digit, FunctionKey, Key, KeyStroke, KeyStrokeError, Letter, Modifiers,
};
pub use keymap::{
    BindingError, Conflict, KeyBinding, Keymap, KeymapEntry, KeymapError, Resolution, Unbinding,
    WhenContext, WhenError, WhenExpr,
};
pub use quickie_domain_kernel::{CommandId, CommandIdError};
