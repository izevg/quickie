//! The writer thread: gathers saves into a batch, keeping the latest of each kind, and writes
//! the batch once it settles, or at once when someone waits for it.

use std::sync::PoisonError;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use quickie_application::{
    KeybindingText, KeymapRepo, LibraryRepo, LibrarySnapshot, SessionRepo, StoreError,
};
use quickie_domain_browsing::SessionSnapshot;
use quickie_domain_library::{BookmarksSnapshot, HistorySnapshot, SearchShortcutsSnapshot};

use super::Failure;

/// What the store asks of its writer.
#[derive(Debug)]
pub(super) enum Job {
    /// A snapshot to write, in place of any pending one of its kind.
    Save(Save),
    /// Something that needs every pending save written first.
    Barrier(Barrier),
    /// Write what is pending, then end the thread.
    Stop,
}

/// A snapshot that replaces what is saved of its kind.
#[derive(Debug)]
pub(super) enum Save {
    /// The session; boxed, as it is by far the largest.
    Session(Box<SessionSnapshot>),
    /// The history.
    History(HistorySnapshot),
    /// The bookmarks.
    Bookmarks(BookmarksSnapshot),
    /// The search shortcuts.
    Shortcuts(SearchShortcutsSnapshot),
}

/// A job served only once the pending saves are written, so it sees them.
#[derive(Debug)]
pub(super) enum Barrier {
    /// Answer with the saved session.
    LoadSession(Sender<Result<Option<SessionSnapshot>, StoreError>>),
    /// Answer with the saved library.
    LoadLibrary(Sender<Result<LibrarySnapshot, StoreError>>),
    /// Answer with the saved key bindings.
    LoadKeybindings(Sender<Result<Vec<KeybindingText>, StoreError>>),
    /// Answer once written.
    Flush(Sender<()>),
}

/// The saves not written yet: at most one of each kind, the latest.
#[derive(Default)]
struct Batch {
    session: Option<Box<SessionSnapshot>>,
    history: Option<HistorySnapshot>,
    bookmarks: Option<BookmarksSnapshot>,
    shortcuts: Option<SearchShortcutsSnapshot>,
}

impl Batch {
    /// Keeps a save in place of the pending one of its kind.
    fn add(&mut self, save: Save) {
        match save {
            Save::Session(session) => self.session = Some(session),
            Save::History(history) => self.history = Some(history),
            Save::Bookmarks(bookmarks) => self.bookmarks = Some(bookmarks),
            Save::Shortcuts(shortcuts) => self.shortcuts = Some(shortcuts),
        }
    }

    /// Writes and empties the batch. A failure does not stop the other kinds; the last one is
    /// kept for the shell.
    fn write<S: SessionRepo + LibraryRepo>(&mut self, store: &mut S, failure: &Failure) {
        let results = [
            self.session.take().map(|session| store.save(&session)),
            self.history
                .take()
                .map(|history| store.save_history(&history)),
            self.bookmarks
                .take()
                .map(|bookmarks| store.save_bookmarks(&bookmarks)),
            self.shortcuts
                .take()
                .map(|shortcuts| store.save_shortcuts(&shortcuts)),
        ];
        if let Some(error) = results
            .into_iter()
            .flatten()
            .filter_map(Result::err)
            .next_back()
        {
            *failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        }
    }
}

/// Serves jobs until [`Job::Stop`] or until every sender is gone, writing a batch `settle`
/// after its first save, or sooner when a job needs it on disk.
pub(super) fn run<S>(mut store: S, inbox: &Receiver<Job>, settle: Duration, failure: &Failure)
where
    S: SessionRepo + LibraryRepo + KeymapRepo,
{
    let mut batch = Batch::default();
    // When the pending batch is written; `None` while nothing is pending.
    let mut due = None::<Instant>;
    loop {
        let job = match due {
            None => inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(due) => inbox.recv_timeout(due.saturating_duration_since(Instant::now())),
        };
        match job {
            Ok(Job::Save(save)) => {
                batch.add(save);
                due.get_or_insert_with(|| {
                    let now = Instant::now();
                    now.checked_add(settle).unwrap_or(now)
                });
            }
            Ok(Job::Barrier(barrier)) => {
                batch.write(&mut store, failure);
                due = None;
                serve(&store, barrier);
            }
            Err(RecvTimeoutError::Timeout) => {
                batch.write(&mut store, failure);
                due = None;
            }
            Ok(Job::Stop) | Err(RecvTimeoutError::Disconnected) => {
                batch.write(&mut store, failure);
                return;
            }
        }
    }
}

/// Answers a barrier once the batch is written. A caller that stopped waiting needs no answer,
/// so a failed reply is dropped.
fn serve<S: SessionRepo + LibraryRepo + KeymapRepo>(store: &S, barrier: Barrier) {
    match barrier {
        Barrier::LoadSession(reply) => drop(reply.send(store.load())),
        Barrier::LoadLibrary(reply) => drop(reply.send(store.load_library())),
        Barrier::LoadKeybindings(reply) => drop(reply.send(store.load_keybindings())),
        Barrier::Flush(reply) => drop(reply.send(())),
    }
}
