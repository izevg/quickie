//! The filter lists as plain data: the only door between the store and the aggregate.

use super::{FilterList, FilterListId, FilterListSet, Refresh};

/// The filter lists as plain data, for the store to write and read back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterListSetSnapshot {
    /// The lists' states; a source missing here is off and never fetched.
    pub lists: Vec<FilterList>,
    /// When lists are fetched again.
    pub refresh: Refresh,
}

/// A snapshot that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptLists {
    /// Two states name one list.
    #[error("the filter list {0} appears twice")]
    DuplicateList(FilterListId),
}

impl FilterListSet {
    /// Rebuilds the lists from a snapshot, checking every invariant once, here.
    ///
    /// # Errors
    ///
    /// The first [`CorruptLists`] found.
    pub fn restore(snapshot: FilterListSetSnapshot) -> Result<Self, CorruptLists> {
        let mut set = Self::new();
        let mut seen = [false; FilterListId::ALL.len()];
        for list in snapshot.lists {
            let index = list.id.index();
            if seen[index] {
                return Err(CorruptLists::DuplicateList(list.id));
            }
            seen[index] = true;
            set.lists[index] = list;
        }
        set.refresh = snapshot.refresh;
        Ok(set)
    }

    /// The lists as plain data; `restore(snapshot())` gives back equal lists.
    #[must_use]
    pub fn snapshot(&self) -> FilterListSetSnapshot {
        FilterListSetSnapshot {
            lists: self.lists.to_vec(),
            refresh: self.refresh,
        }
    }
}
