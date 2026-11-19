//! Bookmarks as plain data: the only door between the store and the aggregate.

use indexmap::IndexMap;
use url::Url;

use super::{Bookmark, Bookmarks};
use crate::pages::is_page_url;

/// Every bookmark as plain data, for the store to write and read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarksSnapshot {
    /// Bookmarks in the order they were first saved.
    pub bookmarks: Vec<Bookmark>,
}

/// A snapshot that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptBookmarks {
    /// Two bookmarks name the same page.
    #[error("{0} is saved twice")]
    DuplicateBookmark(Url),
    /// A bookmark's URL keeps its fragment.
    #[error("{0} keeps its fragment")]
    HasFragment(Url),
}

impl Bookmarks {
    /// Rebuilds the bookmarks from a snapshot, checking every invariant once, here.
    ///
    /// # Errors
    ///
    /// The first [`CorruptBookmarks`] found.
    pub fn restore(snapshot: BookmarksSnapshot) -> Result<Self, CorruptBookmarks> {
        let mut bookmarks = IndexMap::with_capacity(snapshot.bookmarks.len());
        for bookmark in snapshot.bookmarks {
            let url = bookmark.url.clone();
            if !is_page_url(&url) {
                return Err(CorruptBookmarks::HasFragment(url));
            }
            if bookmarks.insert(url.clone(), bookmark).is_some() {
                return Err(CorruptBookmarks::DuplicateBookmark(url));
            }
        }
        Ok(Self { bookmarks })
    }

    /// The bookmarks as plain data; `restore(snapshot())` gives back equal bookmarks.
    #[must_use]
    pub fn snapshot(&self) -> BookmarksSnapshot {
        BookmarksSnapshot {
            bookmarks: self.bookmarks.values().cloned().collect(),
        }
    }
}
