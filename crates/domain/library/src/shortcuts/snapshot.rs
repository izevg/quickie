//! Search shortcuts as plain data: the only door between the store and the aggregate.

use indexmap::IndexMap;

use super::{Keyword, SearchShortcuts, UrlTemplate};

/// Every search shortcut as plain data, for the store to write and read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchShortcutsSnapshot {
    /// Shortcuts in the order they were added.
    pub shortcuts: Vec<(Keyword, UrlTemplate)>,
}

/// A snapshot that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptShortcuts {
    /// Two shortcuts share a keyword.
    #[error("{0} appears twice")]
    DuplicateKeyword(Keyword),
}

impl SearchShortcuts {
    /// Rebuilds the shortcuts from a snapshot, checking every invariant once, here.
    ///
    /// # Errors
    ///
    /// The first [`CorruptShortcuts`] found.
    pub fn restore(snapshot: SearchShortcutsSnapshot) -> Result<Self, CorruptShortcuts> {
        let mut shortcuts = IndexMap::with_capacity(snapshot.shortcuts.len());
        for (keyword, template) in snapshot.shortcuts {
            if shortcuts.contains_key(&keyword) {
                return Err(CorruptShortcuts::DuplicateKeyword(keyword));
            }
            shortcuts.insert(keyword, template);
        }
        Ok(Self { shortcuts })
    }

    /// The shortcuts as plain data; `restore(snapshot())` gives back equal shortcuts.
    #[must_use]
    pub fn snapshot(&self) -> SearchShortcutsSnapshot {
        SearchShortcutsSnapshot {
            shortcuts: self
                .shortcuts
                .iter()
                .map(|(keyword, template)| (keyword.clone(), template.clone()))
                .collect(),
        }
    }
}
