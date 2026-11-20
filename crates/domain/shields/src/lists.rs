//! The filter lists: which of the allowed sources are on, what was fetched from each and when,
//! and whether Quickie may refresh them on its own.

mod hash;
mod snapshot;
mod source;
#[cfg(test)]
mod tests;

use quickie_domain_kernel::Timestamp;

pub use hash::ContentHash;
pub use snapshot::{CorruptLists, FilterListSetSnapshot};
pub use source::{FilterListId, UnknownList};

/// How old a fetched list may get before [`Refresh::KeepFresh`] fetches it again: four days,
/// the expiry EasyList itself declares.
pub const STALE_AFTER_MS: u64 = 4 * Timestamp::DAY_MS;

/// The last successful fetch of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fetch {
    /// When the content arrived.
    pub at: Timestamp,
    /// The hash of the content.
    pub hash: ContentHash,
}

/// One filter list's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FilterList {
    /// Which list.
    pub id: FilterListId,
    /// Whether its rules block requests.
    pub enabled: bool,
    /// Its last successful fetch; `None` until the first.
    pub fetched: Option<Fetch>,
}

impl FilterList {
    /// A list as a fresh profile has it: off, never fetched.
    const fn fresh(id: FilterListId) -> Self {
        Self {
            id,
            enabled: false,
            fetched: None,
        }
    }
}

/// When Quickie fetches a list again after its first fetch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Refresh {
    /// Only when the user asks. The default: a scheduled fetch is a connection the user did not
    /// ask for at that moment.
    #[default]
    OnRequest,
    /// Also once a list is older than [`STALE_AFTER_MS`]; the user turns this on ("keep lists
    /// fresh").
    KeepFresh,
}

/// The filter lists of one profile: every allowed source, each on or off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterListSet {
    /// One per source, in [`FilterListId::ALL`] order.
    lists: [FilterList; FilterListId::ALL.len()],
    refresh: Refresh,
}

impl Default for FilterListSet {
    fn default() -> Self {
        Self {
            lists: FilterListId::ALL.map(FilterList::fresh),
            refresh: Refresh::default(),
        }
    }
}

impl FilterListSet {
    /// A fresh profile's lists: all off, none fetched, refreshed only on request.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// One list's state.
    #[must_use]
    pub fn get(&self, id: FilterListId) -> &FilterList {
        &self.lists[id.index()]
    }

    /// Every list, in [`FilterListId::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = &FilterList> {
        self.lists.iter()
    }

    /// The lists whose rules block requests.
    pub fn enabled(&self) -> impl Iterator<Item = &FilterList> {
        self.lists.iter().filter(|list| list.enabled)
    }

    /// When lists are fetched again.
    #[must_use]
    pub fn refresh(&self) -> Refresh {
        self.refresh
    }

    /// Sets when lists are fetched again; returns whether that changed anything.
    pub fn set_refresh(&mut self, refresh: Refresh) -> bool {
        let changed = self.refresh != refresh;
        self.refresh = refresh;
        changed
    }

    /// Turns a list on; returns whether it was off.
    pub fn enable(&mut self, id: FilterListId) -> bool {
        let list = self.list_mut(id);
        let changed = !list.enabled;
        list.enabled = true;
        changed
    }

    /// Turns a list off, keeping what was fetched; returns whether it was on.
    pub fn disable(&mut self, id: FilterListId) -> bool {
        let list = self.list_mut(id);
        let changed = list.enabled;
        list.enabled = false;
        changed
    }

    /// Records that a fetch of a list brought content with this hash at `at`; returns whether
    /// the content is new: the first fetch, or one that differs from the last.
    pub fn record_fetch(&mut self, id: FilterListId, at: Timestamp, hash: ContentHash) -> bool {
        let list = self.list_mut(id);
        let new = list.fetched.is_none_or(|last| last.hash != hash);
        list.fetched = Some(Fetch { at, hash });
        new
    }

    /// The lists to fetch at `now` without the user asking again: every enabled list never
    /// fetched (enabling it was the request), and, with [`Refresh::KeepFresh`], every enabled
    /// list at least [`STALE_AFTER_MS`] old. A fresh profile has none.
    #[must_use]
    pub fn due(&self, now: Timestamp) -> Vec<FilterListId> {
        self.enabled()
            .filter(|list| match list.fetched {
                None => true,
                Some(last) => {
                    self.refresh == Refresh::KeepFresh
                        && now.millis_since(last.at) >= STALE_AFTER_MS
                }
            })
            .map(|list| list.id)
            .collect()
    }

    fn list_mut(&mut self, id: FilterListId) -> &mut FilterList {
        &mut self.lists[id.index()]
    }
}
