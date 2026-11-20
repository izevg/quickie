//! Library context: what a profile keeps about the web beyond its open tabs. The pages it
//! visited ([`History`]), the pages it saved ([`Bookmarks`], tagged instead of filed in folders),
//! the `!` keyword searches of the palette ([`SearchShortcuts`]), and its [`Downloads`].
//!
//! Local only: nothing here syncs or leaves the Mac. Each of the four is an aggregate the store
//! saves in one transaction, through a plain snapshot that comes back through a validated
//! `restore`. Pure: no I/O, no clock (the caller passes [`Timestamp`]s), no async.

mod downloads;
mod pages;
mod shortcuts;

pub use downloads::{
    CorruptDownloads, Download, DownloadError, DownloadId, DownloadState, Downloads,
    DownloadsSnapshot,
};
pub use pages::{
    Bookmark, Bookmarks, BookmarksSnapshot, CorruptBookmarks, CorruptHistory, Forgotten,
    HISTORY_CAP, History, HistoryEntry, HistorySnapshot, NotSaved, RECENT_VISITS, Recorded, Saved,
    Tag, TagError, VisitNumber, Visited,
};
pub use quickie_domain_kernel::Timestamp;
pub use shortcuts::{
    CorruptShortcuts, Keyword, KeywordError, SearchShortcuts, SearchShortcutsSnapshot,
    TemplateError, UrlTemplate,
};

use url::Url;

/// Whether a URL is a web page: http or https.
fn is_web(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}
