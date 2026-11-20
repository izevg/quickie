//! What a page load costs the main thread in history: a visit to a new page, a revisit, and the
//! title that arrives after the commit, in a history of 50 000 pages. The repository does what
//! the write-behind store does on the main thread: it copies what it is handed and returns.

use std::cell::Cell;
use std::hint::black_box;

use divan::Bencher;
use quickie_application::{Clock, Library, LibraryRepo, LibrarySnapshot, StoreError};
use quickie_domain_browsing::{Jar, Timestamp};
use quickie_domain_library::{
    BookmarksSnapshot, DownloadsSnapshot, History, HistoryEntry, HistorySnapshot,
    SearchShortcutsSnapshot,
};
use url::Url;

/// How many pages the history holds before the measured visit.
const PAGES: usize = 50_000;

fn main() {
    divan::main();
}

/// A clock that moves a second forward each time it is read.
#[derive(Debug, Default)]
struct Ticking(Cell<u64>);

impl Clock for Ticking {
    fn now(&self) -> Timestamp {
        let now = self.0.get() + 1_000;
        self.0.set(now);
        Timestamp::from_millis(now)
    }
}

/// Copies what it is handed, as the write-behind store does before its writer takes over.
#[derive(Debug, Default)]
struct HandOver {
    saved: LibrarySnapshot,
}

impl LibraryRepo for HandOver {
    fn load_library(&self) -> Result<LibrarySnapshot, StoreError> {
        Ok(self.saved.clone())
    }

    fn save_history(&mut self, history: &HistorySnapshot) -> Result<(), StoreError> {
        black_box(history.clone());
        Ok(())
    }

    fn upsert_entry(&mut self, entry: &HistoryEntry) -> Result<(), StoreError> {
        black_box(entry.clone());
        Ok(())
    }

    fn remove_entries(&mut self, pages: &[Url]) -> Result<(), StoreError> {
        black_box(pages.to_vec());
        Ok(())
    }

    fn save_bookmarks(&mut self, bookmarks: &BookmarksSnapshot) -> Result<(), StoreError> {
        black_box(bookmarks.clone());
        Ok(())
    }

    fn save_shortcuts(&mut self, shortcuts: &SearchShortcutsSnapshot) -> Result<(), StoreError> {
        black_box(shortcuts.clone());
        Ok(())
    }

    fn save_downloads(&mut self, downloads: &DownloadsSnapshot) -> Result<(), StoreError> {
        black_box(downloads.clone());
        Ok(())
    }
}

/// The `n`th page of the generated web.
#[expect(
    clippy::unwrap_used,
    reason = "benchmark setup; the URL is well formed"
)]
fn page(n: usize) -> Url {
    Url::parse(&format!("https://site{}.example/articles/{n}", n % 97)).unwrap()
}

/// The library over a history of [`PAGES`] pages, each visited once.
#[expect(
    clippy::unwrap_used,
    reason = "benchmark setup; the in-memory repository never fails"
)]
fn library() -> Library<HandOver, Ticking> {
    let clock = Ticking::default();
    let mut history = History::new();
    for n in 0..PAGES {
        history.record_visit(&page(n), &format!("Article {n}"), clock.now());
    }
    let repo = HandOver {
        saved: LibrarySnapshot {
            history: Some(history.snapshot()),
            ..LibrarySnapshot::default()
        },
    };
    Library::start(repo, clock).unwrap()
}

/// A page never visited before.
#[divan::bench]
fn new_visit(bencher: Bencher<'_, '_>) {
    let mut library = library();
    let mut next = PAGES;
    bencher
        .with_inputs(|| {
            next += 1;
            page(next)
        })
        .bench_local_refs(|url| library.record_visit(url, "Fresh", Jar::Main));
}

/// A page already in history, from the oldest end, so it moves the whole way.
#[divan::bench]
fn revisit(bencher: Bencher<'_, '_>) {
    let mut library = library();
    let mut next = 0;
    bencher
        .with_inputs(|| {
            next = (next + 1) % PAGES;
            page(next)
        })
        .bench_local_refs(|url| library.record_visit(url, "", Jar::Main));
}

/// The title a page reports after its commit, different each time so that it is saved.
#[divan::bench]
fn retitle(bencher: Bencher<'_, '_>) {
    let mut library = library();
    let url = page(PAGES / 2);
    let mut flip = false;
    bencher
        .with_inputs(|| {
            flip = !flip;
            if flip { "Loaded" } else { "Reloaded" }
        })
        .bench_local_refs(|title| library.retitle_visit(&url, title, Jar::Main));
}
