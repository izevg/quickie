//! Library use cases: history, bookmarks and their tags, search shortcuts, downloads.
//!
//! The same transaction rule as browsing: a command changes a copy of one aggregate, the copy
//! is saved, and only then does it replace the aggregate, so a failed save changes nothing. A
//! command that changes nothing saves nothing.
//!
//! History is the exception, as it changes on every page load and may hold a hundred thousand
//! pages: copying it per visit would cost the main thread more than a frame. A visit cannot
//! fail, so history changes in place and saves only the entry it touched (and removes those a
//! full history pushed out). A failed save is reported and leaves the visit in memory; the store
//! catches up with that entry at its next visit.

mod downloads;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::time::Duration;

use quickie_domain_browsing::{Jar, Timestamp};
use quickie_domain_library::{
    Bookmark, Bookmarks, CorruptBookmarks, CorruptDownloads, CorruptHistory, CorruptShortcuts,
    DownloadId, Downloads, Forgotten, History, Keyword, Recorded, Saved, SearchShortcuts, Tag,
    UrlTemplate,
};
use url::Url;

use crate::{AppError, Clock, LibraryRepo, StoreError};

/// A saved part of the library that broke an invariant at start; it was replaced by a fresh
/// one, which the next change of that part saves over it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptLibrary {
    /// The saved history.
    #[error("saved history is corrupt: {0}")]
    History(#[from] CorruptHistory),
    /// The saved bookmarks.
    #[error("saved bookmarks are corrupt: {0}")]
    Bookmarks(#[from] CorruptBookmarks),
    /// The saved search shortcuts.
    #[error("saved search shortcuts are corrupt: {0}")]
    Shortcuts(#[from] CorruptShortcuts),
    /// The saved downloads.
    #[error("saved downloads are corrupt: {0}")]
    Downloads(#[from] CorruptDownloads),
}

/// The library use cases over one profile's history, bookmarks, search shortcuts and downloads.
#[derive(Debug)]
pub struct Library<R, C> {
    history: History,
    bookmarks: Bookmarks,
    shortcuts: SearchShortcuts,
    downloads: Downloads,
    /// Downloads of throwaway jars: listed while Quickie runs, never saved.
    session_only: HashSet<DownloadId>,
    corrupt_on_start: Vec<CorruptLibrary>,
    repo: R,
    clock: C,
}

impl<R: LibraryRepo, C: Clock> Library<R, C> {
    /// Loads the saved library. A part never saved starts empty, or with the default search
    /// shortcuts; a corrupt part starts the same way and is reported by
    /// [`Self::corrupt_on_start`]. A download still in progress when Quickie last quit failed
    /// with it. Nothing is saved until something changes.
    ///
    /// # Errors
    ///
    /// The store could not be read.
    pub fn start(repo: R, clock: C) -> Result<Self, AppError> {
        let saved = repo.load_library()?;
        let mut corrupt_on_start = Vec::new();
        let corrupt = &mut corrupt_on_start;
        let history = restore(saved.history, History::restore, History::new, corrupt);
        let bookmarks = restore(saved.bookmarks, Bookmarks::restore, Bookmarks::new, corrupt);
        let shortcuts = restore(
            saved.shortcuts,
            SearchShortcuts::restore,
            SearchShortcuts::defaults,
            corrupt,
        );
        let mut downloads = restore(saved.downloads, Downloads::restore, Downloads::new, corrupt);
        downloads::interrupt(&mut downloads);
        Ok(Self {
            history,
            bookmarks,
            shortcuts,
            downloads,
            session_only: HashSet::new(),
            corrupt_on_start,
            repo,
            clock,
        })
    }

    /// The visited pages, for reading.
    #[must_use]
    pub fn history(&self) -> &History {
        &self.history
    }

    /// The saved pages, for reading.
    #[must_use]
    pub fn bookmarks(&self) -> &Bookmarks {
        &self.bookmarks
    }

    /// The keyword searches, for reading.
    #[must_use]
    pub fn shortcuts(&self) -> &SearchShortcuts {
        &self.shortcuts
    }

    /// Keeps up to `cap` pages of history from now on, instead of
    /// [`HISTORY_CAP`](quickie_domain_library::HISTORY_CAP). A history
    /// already past it shrinks back at its next new page, which removes the pages visited longest
    /// ago from memory and from the store.
    pub fn set_history_cap(&mut self, cap: NonZeroUsize) {
        self.history.set_cap(cap);
    }

    /// The saved parts that were discarded at start, if any were.
    #[must_use]
    pub fn corrupt_on_start(&self) -> &[CorruptLibrary] {
        &self.corrupt_on_start
    }

    /// Records a committed navigation, after `Browsing::navigated`. `jar` is the tab's jar: a
    /// throwaway jar exists so that a visit leaves no trace once its tabs close, and a history
    /// entry would outlive them, so its visits are never recorded. Saves the page's entry, and
    /// removes the pages a full history pushed out.
    ///
    /// # Errors
    ///
    /// The store failed; the visit stays recorded in memory.
    pub fn record_visit(&mut self, url: &Url, title: &str, jar: Jar) -> Result<Recorded, AppError> {
        if let Jar::Throwaway(_) = jar {
            return Ok(Recorded::Ignored);
        }
        let now = self.clock.now();
        let visited = self.history.record_visit(url, title, now);
        if !visited.dropped.is_empty() {
            let pages: Vec<Url> = visited.dropped.into_iter().map(|entry| entry.url).collect();
            self.repo.remove_entries(&pages)?;
        }
        self.save_entry(url)?;
        Ok(visited.recorded)
    }

    /// Gives a visited page the title it reported after its visit was recorded, without counting
    /// another visit. A throwaway jar's pages are not in history, so nothing changes for them.
    /// Returns whether the title changed; only then is the page's entry saved.
    ///
    /// # Errors
    ///
    /// The store failed; the new title stays in memory.
    pub fn retitle_visit(&mut self, url: &Url, title: &str, jar: Jar) -> Result<bool, AppError> {
        if let Jar::Throwaway(_) = jar {
            return Ok(false);
        }
        if !self.history.retitle(url, title) {
            return Ok(false);
        }
        self.save_entry(url)?;
        Ok(true)
    }

    /// Forgets every visit of the last `span` ("forget the last hour"); an entry left without a
    /// known visit goes as a whole. Rare and wide, so it saves the whole history.
    ///
    /// # Errors
    ///
    /// The store failed; the visits stay forgotten in memory.
    pub fn forget_since(&mut self, span: Duration) -> Result<Forgotten, AppError> {
        let now = self.clock.now().as_millis();
        let span = u64::try_from(span.as_millis()).unwrap_or(u64::MAX);
        let since = Timestamp::from_millis(now.saturating_sub(span));
        let forgotten = self.history.clear_since(since);
        if forgotten.visits > 0 {
            self.repo.save_history(&self.history.snapshot())?;
        }
        Ok(forgotten)
    }

    /// Saves the history entry of the page `url` names, if it has one.
    fn save_entry(&mut self, url: &Url) -> Result<(), StoreError> {
        match self.history.get(url) {
            Some(entry) => self.repo.upsert_entry(entry),
            None => Ok(()),
        }
    }

    /// Bookmarks a page (`⌘D`). A page already saved stays as it was.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn save_bookmark(&mut self, url: &Url, title: String) -> Result<Saved, AppError> {
        let now = self.clock.now();
        let mut next = self.bookmarks.clone();
        let saved = next.save(url, title, now);
        if saved.created {
            self.commit_bookmarks(next)?;
        }
        Ok(saved)
    }

    /// Removes a bookmark and returns it; `None` when the page was not saved.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn remove_bookmark(&mut self, url: &Url) -> Result<Option<Bookmark>, AppError> {
        let mut next = self.bookmarks.clone();
        let removed = next.remove(url);
        if removed.is_some() {
            self.commit_bookmarks(next)?;
        }
        Ok(removed)
    }

    /// Tags a saved page; a tag it already has stays once.
    ///
    /// # Errors
    ///
    /// The page is not saved, or the store failed.
    pub fn tag(&mut self, url: &Url, tag: Tag) -> Result<(), AppError> {
        let mut next = self.bookmarks.clone();
        next.tag(url, tag)?;
        self.commit_bookmarks(next)
    }

    /// Takes a tag off a saved page.
    ///
    /// # Errors
    ///
    /// The page is not saved, or the store failed.
    pub fn untag(&mut self, url: &Url, tag: &Tag) -> Result<(), AppError> {
        let mut next = self.bookmarks.clone();
        next.untag(url, tag)?;
        self.commit_bookmarks(next)
    }

    /// Binds a keyword to a search template; returns the template it replaced.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn set_shortcut(
        &mut self,
        keyword: Keyword,
        template: UrlTemplate,
    ) -> Result<Option<UrlTemplate>, AppError> {
        let mut next = self.shortcuts.clone();
        let replaced = next.set(keyword, template);
        self.commit_shortcuts(next)?;
        Ok(replaced)
    }

    /// Unbinds a keyword; returns its template, `None` for a keyword that was not bound.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn remove_shortcut(&mut self, keyword: &str) -> Result<Option<UrlTemplate>, AppError> {
        let mut next = self.shortcuts.clone();
        let removed = next.remove(keyword);
        if removed.is_some() {
            self.commit_shortcuts(next)?;
        }
        Ok(removed)
    }

    /// The URL `!keyword terms` opens; `None` for an unknown keyword or terms that make no URL.
    #[must_use]
    pub fn expand_shortcut(&self, keyword: &str, terms: &str) -> Option<Url> {
        self.shortcuts.expand(keyword, terms)
    }

    fn commit_bookmarks(&mut self, next: Bookmarks) -> Result<(), AppError> {
        self.repo.save_bookmarks(&next.snapshot())?;
        self.bookmarks = next;
        Ok(())
    }

    fn commit_shortcuts(&mut self, next: SearchShortcuts) -> Result<(), StoreError> {
        self.repo.save_shortcuts(&next.snapshot())?;
        self.shortcuts = next;
        Ok(())
    }
}

/// The aggregate a saved snapshot restores to; a `fresh` one when none was saved, and also,
/// reported in `corrupt`, when the snapshot is corrupt.
fn restore<S, A, E: Into<CorruptLibrary>>(
    saved: Option<S>,
    restore: impl FnOnce(S) -> Result<A, E>,
    fresh: impl FnOnce() -> A,
    corrupt: &mut Vec<CorruptLibrary>,
) -> A {
    match saved.map(restore) {
        None => fresh(),
        Some(Ok(aggregate)) => aggregate,
        Some(Err(error)) => {
            corrupt.push(error.into());
            fresh()
        }
    }
}
