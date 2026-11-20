//! The write-behind store: a save hands its snapshot to one writer thread and returns at once,
//! so a command never waits for SQLite on the main thread.
//!
//! The writer owns the only connection. It coalesces: a snapshot replaces the one of the same
//! kind still waiting, so a burst of commands costs a write or two, and only the latest state
//! reaches the disk. History comes entry by entry: the writer keeps the latest change of each
//! page (its entry, or its removal) and writes only those pages. Loads go through the same thread and wait for it, so a load sees every save
//! made before it. A failed write is kept for [`PendingWrites::take_error`] instead of being lost
//! with the thread that saw it.

#[cfg(test)]
mod tests;
mod writer;

use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use quickie_application::{
    AgentRepo, ExtensionsRepo, KeybindingText, KeymapRepo, LibraryRepo, LibrarySnapshot,
    SessionRepo, ShieldsRepo, ShieldsSnapshot, StoreError, ZoomRepo,
};
use quickie_domain_agents::EndpointSnapshot;
use quickie_domain_browsing::SessionSnapshot;
use quickie_domain_library::{
    BookmarksSnapshot, DownloadsSnapshot, HistoryEntry, HistorySnapshot, SearchShortcutsSnapshot,
};
use quickie_domain_settings::{InstalledExtensionsSnapshot, SiteZoomsSnapshot};
use quickie_domain_shields::{FilterListId, FilterListSetSnapshot, SitePoliciesSnapshot};
use url::Url;

use crate::SqliteStore;
use writer::{Barrier, Job, Save};

/// How long the writer waits after the first save of a batch for more to arrive. A key held
/// down repeats faster than this, so its commands share a write; a crash loses at most this much.
pub const SETTLE: Duration = Duration::from_millis(100);

/// The last write that failed, kept until someone takes it.
type Failure = Arc<Mutex<Option<StoreError>>>;

/// A store whose saves return at once and reach the disk on a writer thread. Loads wait for the
/// writer. Dropping it writes everything still pending first.
#[derive(Debug)]
pub struct WriteBehindStore {
    pending: PendingWrites,
    writer: Option<JoinHandle<()>>,
}

/// The writes a [`WriteBehindStore`] has accepted but may not have finished: lets the shell wait
/// for them and learn of a failure while the store itself belongs to the use cases.
#[derive(Clone, Debug)]
pub struct PendingWrites {
    jobs: Sender<Job>,
    failure: Failure,
}

impl WriteBehindStore {
    /// Opens the database file and puts a writer thread in front of it.
    ///
    /// # Errors
    ///
    /// The file could not be opened, its schema is newer than this build, or the thread could
    /// not start.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::spawn(SqliteStore::open(path)?, SETTLE)
    }

    /// Moves `store` onto a new writer thread that waits `settle` after a batch's first save
    /// before writing it.
    ///
    /// # Errors
    ///
    /// The thread could not start.
    pub fn spawn<S>(store: S, settle: Duration) -> Result<Self, StoreError>
    where
        S: SessionRepo
            + LibraryRepo
            + KeymapRepo
            + ShieldsRepo
            + AgentRepo
            + ZoomRepo
            + ExtensionsRepo
            + Send
            + 'static,
    {
        let (jobs, inbox) = mpsc::channel();
        let failure = Failure::default();
        let failed = Arc::clone(&failure);
        let writer = thread::Builder::new()
            .name("quickie-store-writer".to_owned())
            .spawn(move || writer::run(store, &inbox, settle, &failed))
            .map_err(|error| StoreError(format!("cannot start the writer: {error}")))?;
        Ok(Self {
            pending: PendingWrites { jobs, failure },
            writer: Some(writer),
        })
    }

    /// A handle on this store's pending writes, for the shell to flush on quit and to poll for
    /// failures.
    #[must_use]
    pub fn pending(&self) -> PendingWrites {
        self.pending.clone()
    }

    /// Waits until every save made so far is on disk; see [`PendingWrites::flush`].
    ///
    /// # Errors
    ///
    /// A write failed since the last error was taken.
    pub fn flush(&self) -> Result<(), StoreError> {
        self.pending.flush()
    }

    /// The last write that failed, if any, now cleared; see [`PendingWrites::take_error`].
    #[must_use]
    pub fn take_error(&self) -> Option<StoreError> {
        self.pending.take_error()
    }

    fn send(&self, job: Job) -> Result<(), StoreError> {
        self.pending.jobs.send(job).map_err(|_| stopped())
    }

    /// Hands a snapshot to the writer.
    fn hand_over(&self, save: Save) -> Result<(), StoreError> {
        self.send(Job::Save(save))
    }

    /// Asks the writer for something it serves after the pending saves, and waits for it.
    fn ask<T>(&self, barrier: impl FnOnce(Sender<T>) -> Barrier) -> Result<T, StoreError> {
        let (reply, answer) = mpsc::channel();
        self.send(Job::Barrier(barrier(reply)))?;
        answer.recv().map_err(|_| stopped())
    }
}

impl Drop for WriteBehindStore {
    fn drop(&mut self) {
        // A writer that is already gone panicked: it has nothing left to write, and there is no
        // one left to tell.
        let _ = self.pending.jobs.send(Job::Stop);
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

impl PendingWrites {
    /// Waits until every save accepted before this call is on disk, or failed. Returns at once
    /// if the writer has already stopped, since it writes everything before it stops.
    ///
    /// # Errors
    ///
    /// A write failed since the last error was taken; the error is taken.
    pub fn flush(&self) -> Result<(), StoreError> {
        let (done, wait) = mpsc::channel();
        if self.jobs.send(Job::Barrier(Barrier::Flush(done))).is_ok() {
            // The writer answers once the batch is written; a closed channel means it stopped.
            let _ = wait.recv();
        }
        self.take_error().map_or(Ok(()), Err)
    }

    /// The last write that failed, if any, now cleared, so each failure is reported once.
    #[must_use]
    pub fn take_error(&self) -> Option<StoreError> {
        self.failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl SessionRepo for WriteBehindStore {
    fn load(&self) -> Result<Option<SessionSnapshot>, StoreError> {
        self.ask(Barrier::LoadSession)?
    }

    fn save(&mut self, snapshot: SessionSnapshot) -> Result<(), StoreError> {
        self.hand_over(Save::Session(Box::new(snapshot)))
    }
}

impl LibraryRepo for WriteBehindStore {
    fn load_library(&self) -> Result<LibrarySnapshot, StoreError> {
        self.ask(Barrier::LoadLibrary)?
    }

    fn save_history(&mut self, history: &HistorySnapshot) -> Result<(), StoreError> {
        self.hand_over(Save::History(history.clone()))
    }

    fn upsert_entry(&mut self, entry: &HistoryEntry) -> Result<(), StoreError> {
        self.hand_over(Save::HistoryEntry(Box::new(entry.clone())))
    }

    fn remove_entries(&mut self, pages: &[Url]) -> Result<(), StoreError> {
        self.hand_over(Save::HistoryRemoved(pages.to_vec()))
    }

    fn save_bookmarks(&mut self, bookmarks: &BookmarksSnapshot) -> Result<(), StoreError> {
        self.hand_over(Save::Bookmarks(bookmarks.clone()))
    }

    fn save_shortcuts(&mut self, shortcuts: &SearchShortcutsSnapshot) -> Result<(), StoreError> {
        self.hand_over(Save::Shortcuts(shortcuts.clone()))
    }

    fn save_downloads(&mut self, downloads: &DownloadsSnapshot) -> Result<(), StoreError> {
        self.hand_over(Save::Downloads(downloads.clone()))
    }
}

impl KeymapRepo for WriteBehindStore {
    fn load_keybindings(&self) -> Result<Vec<KeybindingText>, StoreError> {
        self.ask(Barrier::LoadKeybindings)?
    }
}

impl ZoomRepo for WriteBehindStore {
    fn load_site_zooms(&self) -> Result<SiteZoomsSnapshot, StoreError> {
        self.ask(Barrier::LoadZooms)?
    }

    fn save_site_zooms(&mut self, zooms: SiteZoomsSnapshot) -> Result<(), StoreError> {
        self.hand_over(Save::Zooms(zooms))
    }
}

impl ExtensionsRepo for WriteBehindStore {
    fn load_extensions(&self) -> Result<InstalledExtensionsSnapshot, StoreError> {
        self.ask(Barrier::LoadExtensions)?
    }

    fn save_extensions(
        &mut self,
        extensions: InstalledExtensionsSnapshot,
    ) -> Result<(), StoreError> {
        self.hand_over(Save::Extensions(extensions))
    }
}

impl ShieldsRepo for WriteBehindStore {
    fn load_shields(&self) -> Result<ShieldsSnapshot, StoreError> {
        self.ask(Barrier::LoadShields)?
    }

    fn save_sites(&mut self, sites: SitePoliciesSnapshot) -> Result<(), StoreError> {
        self.hand_over(Save::Sites(sites))
    }

    fn save_lists(&mut self, lists: FilterListSetSnapshot) -> Result<(), StoreError> {
        self.hand_over(Save::Lists(lists))
    }

    fn save_list_body(&mut self, list: FilterListId, body: &[u8]) -> Result<(), StoreError> {
        self.hand_over(Save::Body(list, body.to_vec()))
    }
}

impl AgentRepo for WriteBehindStore {
    fn load_endpoint(&self) -> Result<Option<EndpointSnapshot>, StoreError> {
        self.ask(Barrier::LoadEndpoint)?
    }

    fn save_endpoint(&mut self, endpoint: Option<EndpointSnapshot>) -> Result<(), StoreError> {
        self.hand_over(Save::Endpoint(endpoint))
    }
}

/// The writer is gone: it panicked, so nothing more reaches the disk.
fn stopped() -> StoreError {
    StoreError("the store writer stopped".to_owned())
}
