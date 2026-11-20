//! Keymap loading: the default bindings plus the user's `keybindings.json` entries.
//!
//! A bad entry must never cost the user their keyboard, so entries are taken one at a time: an
//! entry that does not parse, or that would make the keymap invalid, is set aside and reported,
//! and the keymap keeps the defaults and every entry before it that was fine.

#[cfg(test)]
mod tests;

use quickie_domain_commands::{BindingError, Catalog, Keymap, KeymapEntry, KeymapError, Platform};

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
    let mut keymap = Keymap::defaults(platform);
    let mut rejected = Vec::new();
    for (position, text) in repo.load_keybindings()?.into_iter().enumerate() {
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
    Ok(LoadedKeymap { keymap, rejected })
}
