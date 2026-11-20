//! History: one entry per visited page, holding what the palette's frecency reads.
//!
//! A visit changes one entry in place and never fails, so history is never copied to record
//! one. Entries are ordered by the number of their last visit, which only grows: a revisit
//! renumbers its entry instead of moving it through a list, whatever the clock says.

mod snapshot;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;

use indexmap::set::MutableValues;
use indexmap::{Equivalent, IndexSet};
use quickie_domain_kernel::Timestamp;
use url::Url;

use super::page_url;
use crate::is_web;
pub use snapshot::{CorruptHistory, HistorySnapshot};

/// How many visit times an entry keeps. The palette's frecency samples the 10 most recent visits,
/// so an older time would never be read again.
pub const RECENT_VISITS: usize = 10;

/// How many pages a history keeps unless told otherwise: years of browsing for most people, and
/// still a palette that opens within its frame budget. Past it, a new page pushes out the page
/// visited longest ago.
pub const HISTORY_CAP: NonZeroUsize = NonZeroUsize::MIN.saturating_add(99_999);

/// Where a visit stands among all the visits a history recorded: each visit takes the next
/// number, so a higher number is a later visit even when the clock jumped back in between.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VisitNumber(u64);

impl VisitNumber {
    /// The number `raw`, as the store keeps it.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The number as the store keeps it.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The number after this one.
    const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// One visited page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The page: http or https, without a fragment.
    pub url: Url,
    /// The title of the last visit that reported one; empty while none did.
    pub title: String,
    /// Every visit recorded, including those whose times are no longer kept.
    pub visit_count: u64,
    /// The times of the last [`RECENT_VISITS`] visits at most, in the order they were recorded.
    /// Never empty: an entry without a visit is not in history.
    pub recent_visits: Vec<Timestamp>,
    /// The number of the page's last visit, which orders history; no two entries share one.
    pub last_visit: VisitNumber,
}

impl HistoryEntry {
    /// Adds a visit at `now`, numbered `number`.
    fn visit(&mut self, title: &str, now: Timestamp, number: VisitNumber) {
        if !title.is_empty() {
            title.clone_into(&mut self.title);
        }
        self.visit_count = self.visit_count.saturating_add(1);
        self.recent_visits.push(now);
        if self.recent_visits.len() > RECENT_VISITS {
            self.recent_visits.remove(0);
        }
        self.last_visit = number;
    }
}

/// Whether recording a visit made an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// The page's first visit made a new entry.
    NewEntry,
    /// The page was already in history; the visit was added to it.
    Revisit,
    /// Not a web page: nothing was recorded.
    Ignored,
}

/// What recording a visit did to history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visited {
    /// Whether the visit made an entry.
    pub recorded: Recorded,
    /// The entries a new one pushed out of a full history, the longest unvisited first; empty
    /// while history is below its cap.
    pub dropped: Vec<HistoryEntry>,
}

impl Visited {
    const fn only(recorded: Recorded) -> Self {
        Self {
            recorded,
            dropped: Vec::new(),
        }
    }
}

/// What forgetting recent history removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Forgotten {
    /// Visits removed, across all entries.
    pub visits: usize,
    /// Entries removed because none of their known visits was left.
    pub entries: usize,
}

/// An entry in the set under its page's URL, so the URL is kept once: found by the URL alone,
/// hashed as the URL alone.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Keyed(HistoryEntry);

impl Hash for Keyed {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.url.hash(state);
    }
}

impl Equivalent<Keyed> for Url {
    fn equivalent(&self, key: &Keyed) -> bool {
        *self == key.0.url
    }
}

/// The visited pages of one profile, kept on this Mac only.
#[derive(Clone, Debug)]
pub struct History {
    entries: IndexSet<Keyed>,
    /// Where each entry sits in `entries`, by the number of its last visit: the first is the
    /// page visited longest ago.
    recency: BTreeMap<VisitNumber, usize>,
    /// The number the next visit takes.
    next: VisitNumber,
    cap: NonZeroUsize,
}

impl History {
    /// An empty history that keeps up to [`HISTORY_CAP`] pages.
    #[must_use]
    pub fn new() -> Self {
        Self::with_cap(HISTORY_CAP)
    }

    /// An empty history that keeps up to `cap` pages.
    #[must_use]
    pub fn with_cap(cap: NonZeroUsize) -> Self {
        Self {
            entries: IndexSet::new(),
            recency: BTreeMap::new(),
            next: VisitNumber(1),
            cap,
        }
    }

    /// Keeps up to `cap` pages from now on. A history already past it shrinks back at its next
    /// new page, which pushes out every page beyond the cap at once.
    pub fn set_cap(&mut self, cap: NonZeroUsize) {
        self.cap = cap;
    }

    /// Records a committed navigation to `url` at `now`. An empty `title` keeps the title already
    /// known: the engine often commits a page before it reports the title. A page new to a full
    /// history pushes out the page visited longest ago.
    ///
    /// Only http and https pages are recorded. `about:` pages belong to the browser and are
    /// commands, not places; a `data:` URL carries its whole content, often megabytes and
    /// sometimes a secret, in the URL itself; a `file:` URL exposes the disk's layout and points
    /// at something that may be gone. None of them is a place the palette should offer again.
    pub fn record_visit(&mut self, url: &Url, title: &str, now: Timestamp) -> Visited {
        if !is_web(url) {
            return Visited::only(Recorded::Ignored);
        }
        let url = page_url(url);
        let number = self.next;
        self.next = number.next();
        if let Some((index, Keyed(entry))) = self.entries.get_full_mut2(&url) {
            self.recency.remove(&entry.last_visit);
            entry.visit(title, now, number);
            self.recency.insert(number, index);
            return Visited::only(Recorded::Revisit);
        }
        let mut entry = HistoryEntry {
            url,
            title: String::new(),
            visit_count: 0,
            recent_visits: Vec::new(),
            last_visit: number,
        };
        entry.visit(title, now, number);
        let (index, _) = self.entries.insert_full(Keyed(entry));
        self.recency.insert(number, index);
        Visited {
            recorded: Recorded::NewEntry,
            dropped: self.make_room(),
        }
    }

    /// Gives a visited page the title it reported after its visit was recorded; pages usually
    /// name themselves only once they load. An empty title keeps the one it had, and a page not
    /// in history stays out: a title is not a visit. Returns whether the title changed.
    pub fn retitle(&mut self, url: &Url, title: &str) -> bool {
        match self.entries.get_full_mut2(&page_url(url)) {
            Some((_, Keyed(entry))) if !title.is_empty() && entry.title != title => {
                title.clone_into(&mut entry.title);
                true
            }
            _ => false,
        }
    }

    /// Removes a page from history, whatever fragment `url` carries, and returns its entry.
    pub fn remove(&mut self, url: &Url) -> Option<HistoryEntry> {
        let index = self.entries.get_index_of(&page_url(url))?;
        self.remove_at(index)
    }

    /// Forgets every visit at or after `since` ("forget the last hour"). An entry whose kept
    /// visits all go is removed as a whole, even if older visits were counted: with no time left
    /// to date it, keeping it would rank a page the person asked to forget. Every visit time is
    /// checked, so a clock that jumped backwards cannot shield one.
    pub fn clear_since(&mut self, since: Timestamp) -> Forgotten {
        let mut forgotten = Forgotten::default();
        self.entries.retain2(|Keyed(entry)| {
            let before = entry.recent_visits.len();
            entry.recent_visits.retain(|&visit| visit < since);
            let removed = before - entry.recent_visits.len();
            forgotten.visits += removed;
            entry.visit_count = entry.visit_count.saturating_sub(removed as u64);
            let keep = !entry.recent_visits.is_empty();
            if !keep {
                forgotten.entries += 1;
            }
            keep
        });
        self.reindex();
        forgotten
    }

    /// Every entry, most recently visited first.
    pub fn entries(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.oldest_first().rev()
    }

    /// The entry of the page `url` names, whatever fragment it carries.
    #[must_use]
    pub fn get(&self, url: &Url) -> Option<&HistoryEntry> {
        self.entries.get(&page_url(url)).map(|Keyed(entry)| entry)
    }

    /// How many pages history holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether history holds no page.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every entry, the page visited longest ago first.
    fn oldest_first(&self) -> impl DoubleEndedIterator<Item = &HistoryEntry> {
        self.recency
            .values()
            .filter_map(|&index| self.entries.get_index(index))
            .map(|Keyed(entry)| entry)
    }

    /// Takes the entry at `index` out. The last entry moves into its place, so its place in
    /// the recency order moves with it.
    fn remove_at(&mut self, index: usize) -> Option<HistoryEntry> {
        let Keyed(entry) = self.entries.swap_remove_index(index)?;
        self.recency.remove(&entry.last_visit);
        if let Some(Keyed(moved)) = self.entries.get_index(index) {
            self.recency.insert(moved.last_visit, index);
        }
        Some(entry)
    }

    /// Pushes out the pages visited longest ago until history fits its cap.
    fn make_room(&mut self) -> Vec<HistoryEntry> {
        let mut dropped = Vec::new();
        while self.entries.len() > self.cap.get() {
            let Some(&oldest) = self.recency.values().next() else {
                break;
            };
            let Some(entry) = self.remove_at(oldest) else {
                break;
            };
            dropped.push(entry);
        }
        dropped
    }

    /// Rebuilds the recency order after entries left from anywhere in the set.
    fn reindex(&mut self) {
        self.recency = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, Keyed(entry))| (entry.last_visit, index))
            .collect();
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

/// Two histories are equal when they hold the same entries, number the next visit alike, and
/// keep as many pages; where an entry sits in memory does not matter.
impl PartialEq for History {
    fn eq(&self, other: &Self) -> bool {
        self.cap == other.cap
            && self.next == other.next
            && self.oldest_first().eq(other.oldest_first())
    }
}

impl Eq for History {}
