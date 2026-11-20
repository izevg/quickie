//! The keymap: default bindings plus the user's entries, validated as a whole, and the lookup
//! that turns a key stroke into a command.

mod binding;
mod defaults;
#[cfg(test)]
mod tests;
mod when;

use std::collections::HashMap;

use quickie_domain_kernel::CommandId;

pub use binding::{BindingError, KeyBinding, KeymapEntry, Unbinding};
pub use when::{WhenContext, WhenError, WhenExpr};

use crate::command::Catalog;
use crate::key::{Chord, KeyStroke, Platform};

/// Every key binding in effect: the defaults, minus those the user removed, then the user's own.
///
/// It is the aggregate of the keymap editor: a change replaces the user entries and is accepted
/// only if the whole keymap stays valid. It belongs to one [`Platform`], fixed when it is made:
/// every `mod` is resolved by then, so a lookup only compares concrete strokes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    platform: Platform,
    reserved: Vec<KeyStroke>,
    defaults: Vec<KeyBinding>,
    user: Vec<KeymapEntry>,
    effective: Vec<KeyBinding>,
}

/// What a key stroke does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Run this command.
    Command(CommandId),
    /// The stroke starts a chord: wait for the next stroke and pass this one back as `pending`.
    Pending(KeyStroke),
    /// Nothing is bound: the stroke goes to the page.
    Unbound,
}

impl Keymap {
    /// Quickie's default bindings on `platform` and no user entries.
    #[must_use]
    pub fn defaults(platform: Platform) -> Self {
        Self::assemble(platform, defaults::bindings(platform), Vec::new())
    }

    /// A keymap on `platform` of `defaults` with the user's `entries` applied in order.
    ///
    /// # Errors
    ///
    /// [`KeymapError::UnknownCommand`] when a binding or entry names a command the catalog does
    /// not have; [`KeymapError::Conflict`] when two bindings in effect share a chord and a `when`
    /// clause but run different commands.
    pub fn new(
        catalog: &Catalog,
        platform: Platform,
        defaults: Vec<KeyBinding>,
        entries: Vec<KeymapEntry>,
    ) -> Result<Self, KeymapError> {
        let keymap = Self::assemble(platform, defaults, entries);
        keymap.validate(catalog)?;
        Ok(keymap)
    }

    /// The platform whose conventions the keymap follows, for parsing the user's entries and for
    /// printing keycaps.
    #[must_use]
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// The same defaults with `entries` as the user's entries instead of the current ones.
    ///
    /// # Errors
    ///
    /// As [`Keymap::new`].
    pub fn with_user_entries(
        &self,
        catalog: &Catalog,
        entries: Vec<KeymapEntry>,
    ) -> Result<Self, KeymapError> {
        Self::new(catalog, self.platform, self.defaults.clone(), entries)
    }

    /// Whether the shell handles `stroke` before the page ever sees it: the primary modifier
    /// with `Q`, `W`, `T`, `L`, `P`, or `⇧P` (`⌘Q` on macOS, Ctrl+Q on Linux), and `⌃⇥` on
    /// both. A page cannot take these keys away from the browser.
    #[must_use]
    pub fn is_reserved(&self, stroke: KeyStroke) -> bool {
        self.reserved.contains(&stroke)
    }

    /// What `stroke` does in `context`. `pending` is the first stroke of a chord, from an earlier
    /// [`Resolution::Pending`]. Among the bindings whose clause holds, the latest wins, so a user
    /// binding beats a default.
    #[must_use]
    pub fn resolve(
        &self,
        stroke: KeyStroke,
        pending: Option<KeyStroke>,
        context: &WhenContext,
    ) -> Resolution {
        let mut candidates = self
            .effective
            .iter()
            .rev()
            .filter(|binding| binding.applies(context));
        match pending {
            None => candidates
                .find(|binding| binding.chord.first == stroke)
                .map_or(Resolution::Unbound, |binding| match binding.chord.second {
                    Some(_) => Resolution::Pending(stroke),
                    None => Resolution::Command(binding.command.clone()),
                }),
            Some(first) => candidates
                .find(|binding| {
                    binding.chord.first == first && binding.chord.second == Some(stroke)
                })
                .map_or(Resolution::Unbound, |binding| {
                    Resolution::Command(binding.command.clone())
                }),
        }
    }

    /// The bindings in effect, defaults first, then the user's.
    pub fn bindings(&self) -> impl Iterator<Item = &KeyBinding> {
        self.effective.iter()
    }

    /// The bindings in effect that run `command`, for the keycaps beside it in the palette.
    pub fn bindings_for<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> impl Iterator<Item = &'a KeyBinding> {
        self.effective
            .iter()
            .filter(move |binding| &binding.command == command)
    }

    /// The user's entries, in order, as the keymap editor lists them.
    #[must_use]
    pub fn user_entries(&self) -> &[KeymapEntry] {
        &self.user
    }

    fn assemble(platform: Platform, defaults: Vec<KeyBinding>, user: Vec<KeymapEntry>) -> Self {
        let removals: Vec<&Unbinding> = user
            .iter()
            .filter_map(|entry| match entry {
                KeymapEntry::Unbind(unbinding) => Some(unbinding),
                KeymapEntry::Bind(_) => None,
            })
            .collect();
        let added = user.iter().filter_map(|entry| match entry {
            KeymapEntry::Bind(binding) => Some(binding),
            KeymapEntry::Unbind(_) => None,
        });
        let effective = defaults
            .iter()
            .filter(|binding| !removals.iter().any(|removal| removal.removes(binding)))
            .chain(added)
            .cloned()
            .collect();
        Self {
            platform,
            reserved: defaults::reserved(platform),
            defaults,
            user,
            effective,
        }
    }

    fn validate(&self, catalog: &Catalog) -> Result<(), KeymapError> {
        let commands = self
            .defaults
            .iter()
            .map(|binding| &binding.command)
            .chain(self.user.iter().map(KeymapEntry::command));
        for command in commands {
            if !catalog.contains(command) {
                return Err(KeymapError::UnknownCommand(command.clone()));
            }
        }
        let mut seen: HashMap<(&Chord, Option<&WhenExpr>), &CommandId> = HashMap::new();
        for binding in &self.effective {
            let key = (&binding.chord, binding.when.as_ref());
            if let Some(first) = seen.insert(key, &binding.command)
                && *first != binding.command
            {
                return Err(KeymapError::Conflict(Box::new(Conflict {
                    chord: binding.chord,
                    when: binding.when.clone(),
                    first: first.clone(),
                    second: binding.command.clone(),
                })));
            }
        }
        Ok(())
    }
}

/// A keymap that cannot take effect.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeymapError {
    /// A binding or entry names a command Quickie does not have.
    #[error("{0} is not a known command")]
    UnknownCommand(CommandId),
    /// Two bindings in effect share a chord and a `when` clause but run different commands.
    #[error(transparent)]
    Conflict(Box<Conflict>),
}

/// Two bindings in effect that share a chord and a `when` clause but run different commands; the
/// keymap editor marks the later one in red.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{}` runs both {first} and {second} under the same condition",
    chord.keybinding_text()
)]
pub struct Conflict {
    /// The shared chord.
    pub chord: Chord,
    /// The shared clause.
    pub when: Option<WhenExpr>,
    /// The command bound first.
    pub first: CommandId,
    /// The command bound later.
    pub second: CommandId,
}
