//! What the palette costs per keystroke: 300 open tabs, 2 000 bookmarks, and a history of
//! 10 000 or 50 000 pages, queried as the user types `github issues`. Gathering the candidates
//! is part of the measurement, as it is part of the answer the user waits for.

use divan::Bencher;
use quickie_application::{Clock, Palette, Sources};
use quickie_domain_browsing::{Focus, JarChoice, Profile, Session, Timestamp};
use quickie_domain_commands::{Catalog, Keymap, Platform};
use quickie_domain_library::{Bookmarks, History, SearchShortcuts};
use url::Url;

/// How many answers the shell asks for.
const ANSWERS: usize = 50;

/// What the user has typed after each keystroke.
const TYPED: [&str; 4] = ["", "g", "git", "github issues"];

fn main() {
    divan::main();
}

/// A clock stopped at a fixed day, so frecency does not drift between runs.
#[derive(Debug, Clone, Copy)]
struct Stopped;

impl Clock for Stopped {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(400 * Timestamp::DAY_MS)
    }
}

const HOSTS: [&str; 8] = [
    "github.com",
    "docs.rs",
    "en.wikipedia.org",
    "news.ycombinator.com",
    "developer.apple.com",
    "crates.io",
    "stackoverflow.com",
    "www.youtube.com",
];

const WORDS: [&str; 16] = [
    "Rust", "issues", "release", "notes", "Swift", "guide", "pull", "request", "browser", "tabs",
    "palette", "memory", "layout", "review", "design", "tokio",
];

/// A page and its title, different for each `n` and spread over a few hosts.
#[expect(
    clippy::unwrap_used,
    reason = "benchmark setup; the URL is well formed"
)]
fn page(n: usize) -> (Url, String) {
    let host = HOSTS[n % HOSTS.len()];
    let url = Url::parse(&format!("https://{host}/p{}/item/{n}", n % 211)).unwrap();
    let title = format!(
        "{} {} {} · {n}",
        WORDS[n % WORDS.len()],
        WORDS[(n / 3) % WORDS.len()],
        WORDS[(n / 7) % WORDS.len()],
    );
    (url, title)
}

/// Everything the palette reads.
struct World {
    session: Session,
    history: History,
    bookmarks: Bookmarks,
    shortcuts: SearchShortcuts,
    keymap: Keymap,
}

impl World {
    /// 300 tabs in three windows, 2 000 bookmarks, and `pages` pages of history visited one to
    /// five times over the last year.
    fn new(pages: usize) -> Self {
        let mut history = History::new();
        for n in 0..pages {
            let (url, title) = page(n);
            for visit in 0..=(n % 5) {
                let day = ((n * 7 + visit * 13) % 365) as u64;
                history.record_visit(
                    &url,
                    &title,
                    Timestamp::from_millis(day * Timestamp::DAY_MS),
                );
            }
        }
        let mut bookmarks = Bookmarks::new();
        for n in (0..pages).step_by(pages / 2_000) {
            let (url, title) = page(n);
            bookmarks.save(&url, title, Timestamp::from_millis(0));
        }
        let mut session = Session::new(Profile::Main);
        for window in 0..3 {
            let id = session.open_window();
            for n in 0..100 {
                let (url, title) = page(window * 1_000 + n * 3);
                if let Ok(tab) = session.open(id, url.clone(), JarChoice::Main, Focus::Background) {
                    let _ = session.navigated(tab, url, title);
                }
            }
        }
        Self {
            session,
            history,
            bookmarks,
            shortcuts: SearchShortcuts::defaults(),
            keymap: Keymap::defaults(Platform::MacOs),
        }
    }

    fn sources(&self) -> Sources<'_> {
        Sources {
            session: &self.session,
            history: &self.history,
            bookmarks: &self.bookmarks,
            shortcuts: &self.shortcuts,
            keymap: &self.keymap,
            extensions: &[],
        }
    }
}

/// The first answer of a palette just opened: gathering, then ranking.
#[divan::bench(consts = [10_000, 50_000], args = TYPED)]
fn keystroke<const PAGES: usize>(bencher: Bencher<'_, '_>, typed: &str) {
    let world = World::new(PAGES);
    let palette = Palette::new(Catalog::builtin(), Stopped);
    let sources = world.sources();
    bencher.bench_local(|| palette.open(&sources).query(typed, ANSWERS));
}

/// Opening the palette alone: gathering every candidate and scoring its frecency.
#[divan::bench(consts = [10_000, 50_000])]
fn open<const PAGES: usize>(bencher: Bencher<'_, '_>) {
    let world = World::new(PAGES);
    let palette = Palette::new(Catalog::builtin(), Stopped);
    let sources = world.sources();
    bencher.bench_local(|| palette.open(&sources));
}

/// A keystroke in a palette already open: ranking alone.
#[divan::bench(consts = [10_000, 50_000], args = TYPED)]
fn typing<const PAGES: usize>(bencher: Bencher<'_, '_>, typed: &str) {
    let world = World::new(PAGES);
    let palette = Palette::new(Catalog::builtin(), Stopped);
    let mut index = palette.open(&world.sources());
    bencher.bench_local(|| index.query(typed, ANSWERS));
}
