//! What the keymap editor changes in `keybindings.json`.
//!
//! Binding a command to a recorded chord replaces whatever it had: the user's own entries for
//! it go, every default binding of it is removed with a `-<id>` entry, and one entry binds the
//! chord, under the clause the command was bound with, so a key rebound for the page does not
//! start firing in text fields. Resetting a command drops the user's entries for it, which
//! brings its defaults back. Every other entry, a bad one included, stays as written.

use quickie_domain_commands::{Catalog, Chord, CommandId, Keymap, Platform};

use super::{LoadedKeymap, Rejection, apply, text_of};
use crate::{AppError, KeybindingText, KeymapFile};

/// Why a command was not bound to a chord.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RebindError {
    /// The keymap refuses the chord: it already runs another command under the same clause.
    #[error(transparent)]
    Refused(#[from] Rejection),
    /// The file could not be read or written; it is unchanged.
    #[error(transparent)]
    Store(#[from] AppError),
}

/// Whether binding `command` to `chord` would be accepted by `keymap`, before anything is
/// written: the editor shows a conflict while the user is still recording.
///
/// # Errors
///
/// Why the keymap would refuse the chord.
pub fn check_binding(
    keymap: &Keymap,
    catalog: &Catalog,
    command: &CommandId,
    chord: Chord,
) -> Result<(), Rejection> {
    let entries = keymap.user_entries().iter().map(text_of).collect();
    let edited = rebound(entries, keymap, command, chord);
    refusal(edited, catalog, keymap.platform()).map(|_| ())
}

/// Binds `command` to `chord` in `file` in place of what it had, and returns the keymap that
/// is in effect then.
///
/// # Errors
///
/// [`RebindError::Refused`] when the keymap refuses the chord, and [`RebindError::Store`]
/// when the file could not be read or written; the file is unchanged either way.
pub fn rebind(
    file: &impl KeymapFile,
    catalog: &Catalog,
    platform: Platform,
    command: &CommandId,
    chord: Chord,
) -> Result<LoadedKeymap, RebindError> {
    let entries = file.load_keybindings().map_err(AppError::from)?;
    let current = apply(entries.clone(), catalog, platform);
    let edited = rebound(entries, &current.keymap, command, chord);
    let loaded = refusal(edited.clone(), catalog, platform)?;
    file.save_keybindings(&edited).map_err(AppError::from)?;
    Ok(loaded)
}

/// Gives `command` its default bindings back: the user's entries for it leave `file`.
///
/// # Errors
///
/// The file could not be read or written; it is unchanged.
pub fn reset_binding(
    file: &impl KeymapFile,
    catalog: &Catalog,
    platform: Platform,
    command: &CommandId,
) -> Result<LoadedKeymap, AppError> {
    let entries = without(file.load_keybindings()?, command);
    file.save_keybindings(&entries)?;
    Ok(apply(entries, catalog, platform))
}

/// The keymap of `edited`, or why it refuses the last entry, the one that binds the chord.
fn refusal(
    edited: Vec<KeybindingText>,
    catalog: &Catalog,
    platform: Platform,
) -> Result<LoadedKeymap, Rejection> {
    let added = edited.len() - 1;
    let loaded = apply(edited, catalog, platform);
    match loaded
        .rejected
        .iter()
        .find(|refused| refused.position == added)
    {
        Some(refused) => Err(refused.reason.clone()),
        None => Ok(loaded),
    }
}

/// `entries` with `command` bound to `chord` instead, as the module describes; `current` is
/// the keymap `entries` make, for the clause the command is bound with now.
fn rebound(
    entries: Vec<KeybindingText>,
    current: &Keymap,
    command: &CommandId,
    chord: Chord,
) -> Vec<KeybindingText> {
    let defaults = Keymap::defaults(current.platform());
    let when = current
        .bindings_for(command)
        .next()
        .or_else(|| defaults.bindings_for(command).next())
        .and_then(|binding| binding.when.as_ref())
        .map(ToString::to_string);
    let mut edited = without(entries, command);
    edited.extend(
        defaults
            .bindings_for(command)
            .map(|binding| KeybindingText {
                key: binding.chord.keybinding_text(),
                command: format!("-{command}"),
                when: binding.when.as_ref().map(ToString::to_string),
            }),
    );
    edited.push(KeybindingText {
        key: chord.keybinding_text(),
        command: command.to_string(),
        when,
    });
    edited
}

/// `entries` without those that bind or remove `command`.
fn without(entries: Vec<KeybindingText>, command: &CommandId) -> Vec<KeybindingText> {
    let removal = format!("-{command}");
    entries
        .into_iter()
        .filter(|entry| entry.command != command.as_str() && entry.command != removal)
        .collect()
}
