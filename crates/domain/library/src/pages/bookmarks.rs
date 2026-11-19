//! Bookmarks: pages saved with `⌘D` and found again through the palette. Tags replace folders:
//! a page can carry several, `#tag` filters by one, and "read later" is just the tag `later`.

mod snapshot;
mod tag;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use indexmap::map::Entry;
use quickie_domain_kernel::Timestamp;
use url::Url;

use super::page_url;
pub use snapshot::{BookmarksSnapshot, CorruptBookmarks};
pub use tag::{Tag, TagError};

/// A saved page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    /// The page, without a fragment.
    pub url: Url,
    /// The title it was saved under, or the one it was given since.
    pub title: String,
    /// Its tags, in alphabetical order.
    pub tags: BTreeSet<Tag>,
    /// When it was first saved.
    pub saved_at: Timestamp,
}

/// What saving a page did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    /// `true` for a new bookmark; `false` when the page was already saved and was kept as it
    /// was, so a second `⌘D` opens the bookmark for editing instead of duplicating it.
    pub created: bool,
}

/// A command named a page that is not saved.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0} is not saved")]
pub struct NotSaved(pub Url);

/// The saved pages of one profile, kept on this Mac only. No folders.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bookmarks {
    /// In the order they were first saved.
    bookmarks: IndexMap<Url, Bookmark>,
}

impl Bookmarks {
    /// No saved pages.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Saves the page `url` names (`⌘D`). Saving a page that is already saved keeps its title,
    /// tags and save time.
    pub fn save(&mut self, url: &Url, title: String, now: Timestamp) -> Saved {
        match self.bookmarks.entry(page_url(url)) {
            Entry::Occupied(_) => Saved { created: false },
            Entry::Vacant(slot) => {
                let url = slot.key().clone();
                slot.insert(Bookmark {
                    url,
                    title,
                    tags: BTreeSet::new(),
                    saved_at: now,
                });
                Saved { created: true }
            }
        }
    }

    /// Removes a saved page and returns its bookmark.
    pub fn remove(&mut self, url: &Url) -> Option<Bookmark> {
        self.bookmarks.shift_remove(&page_url(url))
    }

    /// Adds a tag to a saved page; a tag it already has stays once.
    ///
    /// # Errors
    ///
    /// [`NotSaved`] for a page that is not saved.
    pub fn tag(&mut self, url: &Url, tag: Tag) -> Result<(), NotSaved> {
        self.saved(url)?.tags.insert(tag);
        Ok(())
    }

    /// Takes a tag off a saved page; a tag it does not have is no change.
    ///
    /// # Errors
    ///
    /// [`NotSaved`] for a page that is not saved.
    pub fn untag(&mut self, url: &Url, tag: &Tag) -> Result<(), NotSaved> {
        self.saved(url)?.tags.remove(tag);
        Ok(())
    }

    /// Gives a saved page a new title.
    ///
    /// # Errors
    ///
    /// [`NotSaved`] for a page that is not saved.
    pub fn retitle(&mut self, url: &Url, title: String) -> Result<(), NotSaved> {
        self.saved(url)?.title = title;
        Ok(())
    }

    /// The bookmark of the page `url` names, whatever fragment it carries.
    #[must_use]
    pub fn get(&self, url: &Url) -> Option<&Bookmark> {
        self.bookmarks.get(&page_url(url))
    }

    /// Every saved page, in the order it was first saved.
    pub fn all(&self) -> impl Iterator<Item = &Bookmark> {
        self.bookmarks.values()
    }

    /// The saved pages with a tag, in the order they were first saved: what `#tag` lists.
    pub fn by_tag(&self, tag: &Tag) -> impl Iterator<Item = &Bookmark> {
        self.all()
            .filter(move |bookmark| bookmark.tags.contains(tag))
    }

    /// Every tag in use, alphabetically, with the number of pages that carry it.
    #[must_use]
    pub fn tags(&self) -> BTreeMap<&Tag, usize> {
        let mut counts = BTreeMap::new();
        for tag in self.all().flat_map(|bookmark| &bookmark.tags) {
            *counts.entry(tag).or_insert(0) += 1;
        }
        counts
    }

    fn saved(&mut self, url: &Url) -> Result<&mut Bookmark, NotSaved> {
        let url = page_url(url);
        match self.bookmarks.get_mut(&url) {
            Some(bookmark) => Ok(bookmark),
            None => Err(NotSaved(url)),
        }
    }
}
