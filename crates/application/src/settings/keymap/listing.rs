//! The keymap editor's lines: every command in palette order, one line per binding in effect
//! (or one without keys when nothing is bound), where each binding came from, and the user's
//! entries the keymap refused because their keys already run another command.

use quickie_domain_commands::{Catalog, CommandId, Keymap, KeymapEntry, KeymapError, Platform};

use super::{RejectedEntry, Rejection};

/// Where a binding comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingSource {
    /// Quickie's own default.
    Default,
    /// The user's `keybindings.json`.
    User,
}

/// One line of the keymap editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingLine {
    /// The command the keys run.
    pub command: CommandId,
    /// What the palette calls it: `Reopen Closed Tab`.
    pub title: &'static str,
    /// The keys in the platform's glyphs (`⇧⌘T`); `None` when nothing runs the command.
    pub keys: Option<String>,
    /// The `when` clause, as `keybindings.json` writes it.
    pub when: Option<String>,
    /// Where the binding comes from. A command without keys is `User` when the user removed
    /// its defaults.
    pub source: BindingSource,
    /// For a user entry the keymap refused: the title of the command its keys already run.
    pub conflict: Option<String>,
}

impl BindingLine {
    /// Whether the line answers `query`: every word of it appears, ignoring case, in the title,
    /// the command id, the keys, or the clause. An empty query keeps every line.
    #[must_use]
    pub fn matches(&self, query: &str) -> bool {
        let fields = [
            Some(self.title),
            Some(self.command.as_str()),
            self.keys.as_deref(),
            self.when.as_deref(),
        ];
        let fields: Vec<String> = fields
            .into_iter()
            .flatten()
            .map(str::to_lowercase)
            .collect();
        query.split_whitespace().all(|word| {
            let word = word.to_lowercase();
            fields.iter().any(|field| field.contains(&word))
        })
    }
}

/// The editor's lines for `keymap`: each command of `catalog` in order with its bindings, then
/// the `rejected` entries that conflict with a binding in effect, marked as such.
#[must_use]
pub fn keymap_lines(
    keymap: &Keymap,
    catalog: &Catalog,
    rejected: &[RejectedEntry],
) -> Vec<BindingLine> {
    let platform = keymap.platform();
    let mut lines = Vec::new();
    for command in catalog.iter() {
        let line = |keys, when, source, conflict| BindingLine {
            command: command.id.clone(),
            title: command.title,
            keys,
            when,
            source,
            conflict,
        };
        let mut bound = false;
        for binding in keymap.bindings_for(&command.id) {
            bound = true;
            let source = if keymap
                .user_entries()
                .contains(&KeymapEntry::Bind(binding.clone()))
            {
                BindingSource::User
            } else {
                BindingSource::Default
            };
            lines.push(line(
                Some(binding.chord.glyphs(platform)),
                binding.when.as_ref().map(ToString::to_string),
                source,
                None,
            ));
        }
        if !bound {
            let removed = keymap.user_entries().iter().any(|entry| {
                matches!(entry, KeymapEntry::Unbind(unbinding) if unbinding.command == command.id)
            });
            let source = if removed {
                BindingSource::User
            } else {
                BindingSource::Default
            };
            lines.push(line(None, None, source, None));
        }
        for refused in rejected {
            if let Some((keys, when, other)) = conflict(refused, &command.id, catalog, platform) {
                lines.push(line(Some(keys), when, BindingSource::User, Some(other)));
            }
        }
    }
    lines
}

/// The keys, clause, and the other command's title of `refused`, when it is an entry for
/// `command` whose keys already run another command.
fn conflict(
    refused: &RejectedEntry,
    command: &CommandId,
    catalog: &Catalog,
    platform: Platform,
) -> Option<(String, Option<String>, String)> {
    let Rejection::Invalid(KeymapError::Conflict(conflict)) = &refused.reason else {
        return None;
    };
    if conflict.second != *command {
        return None;
    }
    let other = catalog.get(&conflict.first).map_or_else(
        || conflict.first.to_string(),
        |other| other.title.to_owned(),
    );
    Some((
        conflict.chord.glyphs(platform),
        conflict.when.as_ref().map(ToString::to_string),
        other,
    ))
}
