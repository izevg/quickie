//! History as plain data: the only door between the store and the aggregate.

use indexmap::IndexMap;
use url::Url;

use super::{History, HistoryEntry, RECENT_VISITS};
use crate::is_web;
use crate::pages::is_page_url;

/// A whole history as plain data, for the store to write and read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistorySnapshot {
    /// Entries, least recently visited first.
    pub entries: Vec<HistoryEntry>,
}

/// A snapshot that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptHistory {
    /// Two entries name the same page.
    #[error("{0} appears twice")]
    DuplicateEntry(Url),
    /// An entry is not an http or https page.
    #[error("{0} is not a web page")]
    NotWeb(Url),
    /// An entry's URL keeps its fragment.
    #[error("{0} keeps its fragment")]
    HasFragment(Url),
    /// An entry has no visit time.
    #[error("{0} has no visit")]
    NoVisits(Url),
    /// An entry keeps more visit times than [`RECENT_VISITS`].
    #[error("{0} keeps more than {RECENT_VISITS} visit times")]
    TooManyVisits(Url),
    /// An entry counts fewer visits than it keeps times for.
    #[error("{0} counts fewer visits than it keeps")]
    CountBelowVisits(Url),
}

impl History {
    /// Rebuilds a history from a snapshot, checking every invariant once, here.
    ///
    /// # Errors
    ///
    /// The first [`CorruptHistory`] found.
    pub fn restore(snapshot: HistorySnapshot) -> Result<Self, CorruptHistory> {
        let mut entries = IndexMap::with_capacity(snapshot.entries.len());
        for entry in snapshot.entries {
            check_entry(&entry)?;
            let url = entry.url.clone();
            if entries.insert(url.clone(), entry).is_some() {
                return Err(CorruptHistory::DuplicateEntry(url));
            }
        }
        Ok(Self { entries })
    }

    /// The history as plain data; `restore(snapshot())` gives back an equal history.
    #[must_use]
    pub fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            entries: self.entries.values().cloned().collect(),
        }
    }
}

fn check_entry(entry: &HistoryEntry) -> Result<(), CorruptHistory> {
    let url = || entry.url.clone();
    let kept = entry.recent_visits.len();
    if !is_web(&entry.url) {
        Err(CorruptHistory::NotWeb(url()))
    } else if !is_page_url(&entry.url) {
        Err(CorruptHistory::HasFragment(url()))
    } else if kept == 0 {
        Err(CorruptHistory::NoVisits(url()))
    } else if kept > RECENT_VISITS {
        Err(CorruptHistory::TooManyVisits(url()))
    } else if entry.visit_count < kept as u64 {
        Err(CorruptHistory::CountBelowVisits(url()))
    } else {
        Ok(())
    }
}
