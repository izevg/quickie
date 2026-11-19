//! History: one entry per visited page, holding what the palette's frecency reads.

mod snapshot;
#[cfg(test)]
mod tests;

use indexmap::IndexMap;
use quickie_domain_kernel::Timestamp;
use url::Url;

use super::page_url;
use crate::is_web;
pub use snapshot::{CorruptHistory, HistorySnapshot};

/// How many visit times an entry keeps. The palette's frecency samples the 10 most recent visits,
/// so an older time would never be read again.
pub const RECENT_VISITS: usize = 10;

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
}

/// What recording a visit did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// The page's first visit made a new entry.
    NewEntry,
    /// The page was already in history; the visit was added to it.
    Revisit,
    /// Not a web page: nothing was recorded.
    Ignored,
}

/// What forgetting recent history removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Forgotten {
    /// Visits removed, across all entries.
    pub visits: usize,
    /// Entries removed because none of their known visits was left.
    pub entries: usize,
}

/// The visited pages of one profile, kept on this Mac only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// Least recently visited first: a visit moves its entry to the back.
    entries: IndexMap<Url, HistoryEntry>,
}

impl History {
    /// An empty history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a committed navigation to `url` at `now`. An empty `title` keeps the title already
    /// known: the engine often commits a page before it reports the title.
    ///
    /// Only http and https pages are recorded. `about:` pages belong to the browser and are
    /// commands, not places; a `data:` URL carries its whole content, often megabytes and
    /// sometimes a secret, in the URL itself; a `file:` URL exposes the disk's layout and points
    /// at something that may be gone. None of them is a place the palette should offer again.
    pub fn record_visit(&mut self, url: &Url, title: &str, now: Timestamp) -> Recorded {
        if !is_web(url) {
            return Recorded::Ignored;
        }
        let url = page_url(url);
        let (mut entry, recorded) = match self.entries.shift_remove(&url) {
            Some(entry) => (entry, Recorded::Revisit),
            None => (
                HistoryEntry {
                    url: url.clone(),
                    title: String::new(),
                    visit_count: 0,
                    recent_visits: Vec::new(),
                },
                Recorded::NewEntry,
            ),
        };
        if !title.is_empty() {
            title.clone_into(&mut entry.title);
        }
        entry.visit_count = entry.visit_count.saturating_add(1);
        entry.recent_visits.push(now);
        if entry.recent_visits.len() > RECENT_VISITS {
            entry.recent_visits.remove(0);
        }
        self.entries.insert(url, entry);
        recorded
    }

    /// Removes a page from history, whatever fragment `url` carries, and returns its entry.
    pub fn remove(&mut self, url: &Url) -> Option<HistoryEntry> {
        self.entries.shift_remove(&page_url(url))
    }

    /// Forgets every visit at or after `since` ("forget the last hour"). An entry whose kept
    /// visits all go is removed as a whole, even if older visits were counted: with no time left
    /// to date it, keeping it would rank a page the person asked to forget. Every visit time is
    /// checked, so a clock that jumped backwards cannot shield one.
    pub fn clear_since(&mut self, since: Timestamp) -> Forgotten {
        let mut forgotten = Forgotten::default();
        self.entries.retain(|_, entry| {
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
        forgotten
    }

    /// Every entry, most recently visited first.
    pub fn entries(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.values().rev()
    }

    /// The entry of the page `url` names, whatever fragment it carries.
    #[must_use]
    pub fn get(&self, url: &Url) -> Option<&HistoryEntry> {
        self.entries.get(&page_url(url))
    }
}
