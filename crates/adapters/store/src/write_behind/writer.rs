//! The writer thread: gathers saves into a batch, keeping the latest of each kind, and writes
//! the batch once it settles, or at once when someone waits for it.

use std::collections::BTreeMap;
use std::sync::PoisonError;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use quickie_application::{
    AgentRepo, AppearanceRepo, ExtensionsRepo, KeybindingText, KeymapRepo, LibraryRepo,
    LibrarySnapshot, SessionRepo, ShieldsRepo, ShieldsSnapshot, StoreError, ZoomRepo,
};
use quickie_domain_agents::EndpointSnapshot;
use quickie_domain_browsing::SessionSnapshot;
use quickie_domain_library::{
    BookmarksSnapshot, DownloadsSnapshot, HistoryEntry, HistorySnapshot, SearchShortcutsSnapshot,
};
use quickie_domain_settings::{AppearanceSnapshot, InstalledExtensionsSnapshot, SiteZoomsSnapshot};
use quickie_domain_shields::{FilterListId, FilterListSetSnapshot, SitePoliciesSnapshot};
use url::Url;

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
    /// The whole history.
    History(HistorySnapshot),
    /// One page's history entry; boxed, as it comes with every page load.
    HistoryEntry(Box<HistoryEntry>),
    /// Pages whose history entries go.
    HistoryRemoved(Vec<Url>),
    /// The bookmarks.
    Bookmarks(BookmarksSnapshot),
    /// The search shortcuts.
    Shortcuts(SearchShortcutsSnapshot),
    /// The downloads.
    Downloads(DownloadsSnapshot),
    /// The per-site shields exceptions.
    Sites(SitePoliciesSnapshot),
    /// The filter list states.
    Lists(FilterListSetSnapshot),
    /// One filter list's content; each list is its own kind.
    Body(FilterListId, Vec<u8>),
    /// The agent's endpoint, or that there is none.
    Endpoint(Option<EndpointSnapshot>),
    /// Each site's page zoom.
    Zooms(SiteZoomsSnapshot),
    /// The appearance choices.
    Appearance(AppearanceSnapshot),
    /// The installed extensions.
    Extensions(InstalledExtensionsSnapshot),
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
    /// Answer with the saved shields.
    LoadShields(Sender<Result<ShieldsSnapshot, StoreError>>),
    /// Answer with the saved agent endpoint.
    LoadEndpoint(Sender<Result<Option<EndpointSnapshot>, StoreError>>),
    /// Answer with the saved site zooms.
    LoadZooms(Sender<Result<SiteZoomsSnapshot, StoreError>>),
    /// Answer with the saved appearance.
    LoadAppearance(Sender<Result<AppearanceSnapshot, StoreError>>),
    /// Answer with the saved extensions.
    LoadExtensions(Sender<Result<InstalledExtensionsSnapshot, StoreError>>),
    /// Answer once written.
    Flush(Sender<()>),
}

/// What happens to one page of history.
enum PageChange {
    /// Its entry is written over the saved one.
    Upsert(Box<HistoryEntry>),
    /// Its entry goes.
    Remove,
}

/// The saves not written yet: at most one of each kind, the latest.
#[derive(Default)]
struct Batch {
    session: Option<Box<SessionSnapshot>>,
    history: Option<HistorySnapshot>,
    /// The latest change of each page since `history`, written after it.
    pages: BTreeMap<Url, PageChange>,
    bookmarks: Option<BookmarksSnapshot>,
    shortcuts: Option<SearchShortcutsSnapshot>,
    downloads: Option<DownloadsSnapshot>,
    sites: Option<SitePoliciesSnapshot>,
    lists: Option<FilterListSetSnapshot>,
    bodies: BTreeMap<FilterListId, Vec<u8>>,
    /// `Some(None)` is a pending save of no endpoint.
    #[expect(
        clippy::option_option,
        reason = "whether a save is pending, and whether it saves an endpoint or forgets it"
    )]
    endpoint: Option<Option<EndpointSnapshot>>,
    zooms: Option<SiteZoomsSnapshot>,
    appearance: Option<AppearanceSnapshot>,
    extensions: Option<InstalledExtensionsSnapshot>,
}

impl Batch {
    /// Keeps a save in place of the pending one of its kind.
    fn add(&mut self, save: Save) {
        match save {
            Save::Session(session) => self.session = Some(session),
            Save::History(history) => {
                // The snapshot was taken after every page change still waiting: it holds them.
                self.pages.clear();
                self.history = Some(history);
            }
            Save::HistoryEntry(entry) => {
                self.pages
                    .insert(entry.url.clone(), PageChange::Upsert(entry));
            }
            Save::HistoryRemoved(pages) => {
                for page in pages {
                    self.pages.insert(page, PageChange::Remove);
                }
            }
            Save::Bookmarks(bookmarks) => self.bookmarks = Some(bookmarks),
            Save::Shortcuts(shortcuts) => self.shortcuts = Some(shortcuts),
            Save::Downloads(downloads) => self.downloads = Some(downloads),
            Save::Sites(sites) => self.sites = Some(sites),
            Save::Lists(lists) => self.lists = Some(lists),
            Save::Body(list, body) => {
                self.bodies.insert(list, body);
            }
            Save::Endpoint(endpoint) => self.endpoint = Some(endpoint),
            Save::Zooms(zooms) => self.zooms = Some(zooms),
            Save::Appearance(appearance) => self.appearance = Some(appearance),
            Save::Extensions(extensions) => self.extensions = Some(extensions),
        }
    }

    /// Writes and empties the batch. A failure does not stop the other kinds; the last one is
    /// kept for the shell. A list's content goes before the list states that describe it, and a
    /// whole history before the page changes made after it.
    fn write<
        S: SessionRepo
            + LibraryRepo
            + ShieldsRepo
            + AgentRepo
            + ZoomRepo
            + AppearanceRepo
            + ExtensionsRepo,
    >(
        &mut self,
        store: &mut S,
        failure: &Failure,
    ) {
        let bodies: Vec<_> = std::mem::take(&mut self.bodies)
            .into_iter()
            .map(|(list, body)| store.save_list_body(list, &body))
            .collect();
        let history = self
            .history
            .take()
            .map(|history| store.save_history(&history));
        let pages = write_pages(std::mem::take(&mut self.pages), store);
        let results = [
            self.session.take().map(|session| store.save(*session)),
            history,
            self.bookmarks
                .take()
                .map(|bookmarks| store.save_bookmarks(&bookmarks)),
            self.shortcuts
                .take()
                .map(|shortcuts| store.save_shortcuts(&shortcuts)),
            self.downloads
                .take()
                .map(|downloads| store.save_downloads(&downloads)),
            self.sites.take().map(|sites| store.save_sites(sites)),
            self.lists.take().map(|lists| store.save_lists(lists)),
            self.endpoint
                .take()
                .map(|endpoint| store.save_endpoint(endpoint)),
            self.zooms.take().map(|zooms| store.save_site_zooms(zooms)),
            self.appearance
                .take()
                .map(|appearance| store.save_appearance(appearance)),
            self.extensions
                .take()
                .map(|extensions| store.save_extensions(extensions)),
        ];
        if let Some(error) = bodies
            .into_iter()
            .chain(pages)
            .chain(results.into_iter().flatten())
            .filter_map(Result::err)
            .next_back()
        {
            *failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        }
    }
}

/// Writes the pages' entries one by one and removes the removed pages' all at once.
fn write_pages<S: LibraryRepo>(
    pages: BTreeMap<Url, PageChange>,
    store: &mut S,
) -> Vec<Result<(), StoreError>> {
    let mut removed = Vec::new();
    let mut results: Vec<_> = pages
        .into_iter()
        .filter_map(|(page, change)| match change {
            PageChange::Upsert(entry) => Some(store.upsert_entry(&entry)),
            PageChange::Remove => {
                removed.push(page);
                None
            }
        })
        .collect();
    if !removed.is_empty() {
        results.push(store.remove_entries(&removed));
    }
    results
}

/// Serves jobs until [`Job::Stop`] or until every sender is gone, writing a batch `settle`
/// after its first save, or sooner when a job needs it on disk.
pub(super) fn run<S>(mut store: S, inbox: &Receiver<Job>, settle: Duration, failure: &Failure)
where
    S: SessionRepo
        + LibraryRepo
        + KeymapRepo
        + ShieldsRepo
        + AgentRepo
        + ZoomRepo
        + AppearanceRepo
        + ExtensionsRepo,
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
fn serve<S>(store: &S, barrier: Barrier)
where
    S: SessionRepo
        + LibraryRepo
        + KeymapRepo
        + ShieldsRepo
        + AgentRepo
        + ZoomRepo
        + AppearanceRepo
        + ExtensionsRepo,
{
    match barrier {
        Barrier::LoadExtensions(reply) => drop(reply.send(store.load_extensions())),
        Barrier::LoadEndpoint(reply) => drop(reply.send(store.load_endpoint())),
        Barrier::LoadSession(reply) => drop(reply.send(store.load())),
        Barrier::LoadLibrary(reply) => drop(reply.send(store.load_library())),
        Barrier::LoadKeybindings(reply) => drop(reply.send(store.load_keybindings())),
        Barrier::LoadShields(reply) => drop(reply.send(store.load_shields())),
        Barrier::LoadZooms(reply) => drop(reply.send(store.load_site_zooms())),
        Barrier::LoadAppearance(reply) => drop(reply.send(store.load_appearance())),
        Barrier::Flush(reply) => drop(reply.send(())),
    }
}
