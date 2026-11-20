//! Keymap loading: the default bindings plus the user's `keybindings.json` entries.
//!
//! A bad entry must never cost the user their keyboard, so entries are taken one at a time: an
//! entry that does not parse, or that would make the keymap invalid, is set aside and reported,
//! and the keymap keeps the defaults and every entry before it that was fine.
//!
//! - [`listing`]: the keymap editor's lines, one per binding, with where each came from.
//! - [`edit`]: what the editor changes: a command bound to a recorded chord, or back to its
//!   defaults.

mod edit;
mod listing;
#[cfg(test)]
mod tests;

use quickie_domain_commands::{BindingError, Catalog, Keymap, KeymapEntry, KeymapError, Platform};

pub use edit::{RebindError, check_binding, rebind, reset_binding};
pub use listing::{BindingLine, BindingSource, keymap_lines};

use crate::{AppError, KeybindingText, KeymapRepo};

/// The keymap in effect after loading, and the entries it had to leave out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedKeymap {
    /// The defaults with every valid user entry applied.
    pub keymap: Keymap,
    /// The entries that were left out, in file order.
    pub rejected: Vec<RejectedEntry>,
}

/// A user entry the keymap left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedEntry {
    /// Its zero-based position among the user's entries.
    pub position: usize,
    /// The entry as written.
    pub entry: KeybindingText,
    /// Why it was left out.
    pub reason: Rejection,
}

/// Why a user entry was left out.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    /// A field does not parse.
    #[error(transparent)]
    Malformed(#[from] BindingError),
    /// It names an unknown command or conflicts with a binding already in effect.
    #[error(transparent)]
    Invalid(#[from] KeymapError),
}

/// Loads the user's entries from `repo` and applies the valid ones to the default bindings of
/// `platform`, reading `mod` in every entry as that platform's primary modifier.
///
/// # Errors
///
/// The store could not be read; the shell then runs on the defaults alone.
pub fn load_keymap(
    repo: &impl KeymapRepo,
    catalog: &Catalog,
    platform: Platform,
) -> Result<LoadedKeymap, AppError> {
    Ok(apply(repo.load_keybindings()?, catalog, platform))
}

/// The default bindings of `platform` with the valid `entries` applied, one at a time.
fn apply(entries: Vec<KeybindingText>, catalog: &Catalog, platform: Platform) -> LoadedKeymap {
    let mut keymap = Keymap::defaults(platform);
    let mut rejected = Vec::new();
    for (position, text) in entries.into_iter().enumerate() {
        let outcome = KeymapEntry::parse(&text.key, &text.command, text.when.as_deref(), platform)
            .map_err(Rejection::from)
            .and_then(|entry| {
                let mut entries = keymap.user_entries().to_vec();
                entries.push(entry);
                Ok(keymap.with_user_entries(catalog, entries)?)
            });
        match outcome {
            Ok(next) => keymap = next,
            Err(reason) => rejected.push(RejectedEntry {
                position,
                entry: text,
                reason,
            }),
        }
    }
    LoadedKeymap { keymap, rejected }
}

/// `entry` as `keybindings.json` writes it: the chord's keys, the command (`-<id>` for a
/// removal), and the clause.
fn text_of(entry: &KeymapEntry) -> KeybindingText {
    match entry {
        KeymapEntry::Bind(binding) => KeybindingText {
            key: binding.chord.keybinding_text(),
            command: binding.command.to_string(),
            when: binding.when.as_ref().map(ToString::to_string),
        },
        KeymapEntry::Unbind(unbinding) => KeybindingText {
            key: unbinding.chord.keybinding_text(),
            command: format!("-{}", unbinding.command),
            when: unbinding.when.as_ref().map(ToString::to_string),
        },
    }
}
