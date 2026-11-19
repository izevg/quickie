//! Pages the profile visited or saved. Both are keyed by the page's URL without its fragment:
//! `#install` and `#usage` are places on one page, so they share one history entry and one
//! bookmark, and the palette joins history and bookmarks by that key.

mod bookmarks;
mod history;

pub use bookmarks::{
    Bookmark, Bookmarks, BookmarksSnapshot, CorruptBookmarks, NotSaved, Saved, Tag, TagError,
};
pub use history::{
    CorruptHistory, Forgotten, History, HistoryEntry, HistorySnapshot, RECENT_VISITS, Recorded,
};

use url::Url;

/// The URL that names a page: the same URL without its fragment. The query stays, because it
/// often picks the content (`?q=rust`, `?page=2`).
fn page_url(url: &Url) -> Url {
    let mut page = url.clone();
    page.set_fragment(None);
    page
}

/// Whether a URL already names a page, as a restored snapshot must.
fn is_page_url(url: &Url) -> bool {
    url.fragment().is_none()
}
