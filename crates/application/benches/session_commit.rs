//! What one browsing command costs on the main thread: the session change, its commit, and the
//! hand-over of the snapshot to the store.
//!
//! A heavy profile: 300 live tabs in 4 windows and 3000 tabs in the trash. The store keeps what
//! it is handed, as the write-behind store does; the engine does nothing. Run with
//! `cargo bench -p quickie-application`.

#![expect(clippy::unwrap_used, reason = "a broken fixture should stop the bench")]

use std::hint::black_box;

use divan::Bencher;
use quickie_application::{
    Browsing, Clock, EngineError, EnginePort, Navigation, SessionRepo, StoreError,
};
use quickie_domain_browsing::{
    Focus, Jar, JarChoice, JarId, Profile, Session, SessionSnapshot, TabId, Timestamp, WindowId,
};
use url::Url;

const WINDOWS: usize = 4;
const TABS_PER_WINDOW: usize = 75;
const TRASHED: usize = 3000;

fn main() {
    divan::main();
}

/// Keeps the latest snapshot it is handed, like the write-behind store's pending batch.
#[derive(Default)]
struct Store {
    seed: Option<SessionSnapshot>,
    pending: Option<SessionSnapshot>,
}

impl SessionRepo for Store {
    fn load(&self) -> Result<Option<SessionSnapshot>, StoreError> {
        Ok(self.seed.clone())
    }

    fn save(&mut self, snapshot: SessionSnapshot) -> Result<(), StoreError> {
        self.pending = Some(snapshot);
        Ok(())
    }
}

/// An engine that accepts everything.
struct Engine;

impl EnginePort for Engine {
    type Page = ();

    fn adopt(&mut self, (): (), _: TabId) -> Result<(), EngineError> {
        Ok(())
    }

    fn create(&mut self, _: TabId, _: &Url, _: Jar) -> Result<(), EngineError> {
        Ok(())
    }

    fn show(&mut self, _: WindowId, _: TabId) -> Result<(), EngineError> {
        Ok(())
    }

    fn navigate(&mut self, _: TabId, _: &Navigation) -> Result<(), EngineError> {
        Ok(())
    }

    fn close(&mut self, _: TabId) -> Result<(), EngineError> {
        Ok(())
    }

    fn destroy_jar(&mut self, _: JarId) -> Result<(), EngineError> {
        Ok(())
    }
}

/// A clock stopped at one moment, inside the trash's retention.
struct Stopped;

impl Clock for Stopped {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(1_000)
    }
}

type Bench = Browsing<Engine, Store, Stopped>;

fn page(n: usize) -> Url {
    Url::parse(&format!("https://example.com/articles/{n}?ref=bench")).unwrap()
}

/// The heavy profile, as the store would load it.
fn saved() -> SessionSnapshot {
    let mut session = Session::new(Profile::Main);
    let windows: Vec<WindowId> = (0..WINDOWS).map(|_| session.open_window()).collect();
    let closed_at = Timestamp::from_millis(500);
    for n in 0..TRASHED {
        let tab = session
            .open(windows[0], page(n), JarChoice::Main, Focus::Background)
            .unwrap();
        session.close(tab, closed_at).unwrap();
    }
    for (index, &window) in windows.iter().enumerate() {
        for n in 0..TABS_PER_WINDOW {
            let url = page(TRASHED + index * TABS_PER_WINDOW + n);
            let tab = session
                .open(window, url, JarChoice::Main, Focus::Foreground)
                .unwrap();
            session
                .navigated(tab, page(n), format!("Article {n}"))
                .unwrap();
        }
    }
    session.snapshot().unwrap()
}

fn browsing() -> Bench {
    let store = Store {
        seed: Some(saved()),
        pending: None,
    };
    let home = page(0);
    Browsing::start(Profile::Main, Engine, store, Stopped, &home).unwrap()
}

/// The least recent tab of the last window: the farthest from the front everywhere.
fn far_tab(browsing: &Bench) -> TabId {
    let session = browsing.session();
    let window = session.windows().last().unwrap();
    *session.mru(window).unwrap().last().unwrap()
}

/// `⌃⇥` back and forth between two tabs of the last window.
#[divan::bench(sample_count = 200)]
fn activate(bencher: Bencher) {
    let mut browsing = browsing();
    let session = browsing.session();
    let window = session.windows().last().unwrap();
    let mru = session.mru(window).unwrap();
    let (a, b) = (mru[0], mru[1]);
    let mut flip = false;
    bencher.bench_local(|| {
        flip = !flip;
        black_box(browsing.activate(if flip { b } else { a })).unwrap();
    });
}

/// The engine reports a navigation to a new page.
#[divan::bench(sample_count = 200)]
fn navigated(bencher: Bencher) {
    let mut browsing = browsing();
    let tab = far_tab(&browsing);
    let (here, there) = (page(1), page(2));
    let mut flip = false;
    bencher.bench_local(|| {
        flip = !flip;
        let url = if flip { there.clone() } else { here.clone() };
        black_box(browsing.navigated(tab, url, "Article".to_owned())).unwrap();
    });
}

/// The engine reports the page the tab already shows, as a title or same-document change can.
#[divan::bench(sample_count = 200)]
fn navigated_unchanged(bencher: Bencher) {
    let mut browsing = browsing();
    let tab = far_tab(&browsing);
    let url = page(1);
    browsing
        .navigated(tab, url.clone(), "Article".to_owned())
        .unwrap();
    bencher.bench_local(|| {
        black_box(browsing.navigated(tab, url.clone(), "Article".to_owned())).unwrap();
    });
}

/// `⌘W` on a tab; every sample starts from a fresh profile.
#[divan::bench(sample_count = 50, sample_size = 1)]
fn close(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let browsing = browsing();
            let tab = far_tab(&browsing);
            (browsing, tab)
        })
        .bench_local_values(|(mut browsing, tab)| {
            black_box(browsing.close(tab)).unwrap();
            browsing
        });
}
