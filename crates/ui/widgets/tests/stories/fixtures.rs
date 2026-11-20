//! Fixtures the stories share: a page of text, URLs, and the browsing state the palette and
//! the tab overlays read. English and Russian titles, and one far too long.

use quickie_application::{Clock, Palette, PaletteIndex, PaletteItem, Sources};
use quickie_domain_browsing::{Focus, JarChoice, Profile, Session, TabId, Timestamp, WindowId};
use quickie_domain_commands::{Catalog, Keymap, Platform};
use quickie_domain_library::{Bookmarks, History, SearchShortcuts, Tag};
use url::Url;

/// The text of the page a find runs on: "sovereign" four times, "суверен" twice.
pub const PAGE_TEXT: &str = "A sovereign state rests on its people. Sovereign power is \
    limited by law; the sovereign answers to courts, and no sovereign stands above them. \
    Суверенитет — это верховенство власти; суверен подчиняется праву.";

/// A title that never fits.
pub const LONG_TITLE: &str = "The Remarkable and Thoroughly Documented History of Every \
    Keyboard Shortcut Ever Proposed for a Web Browser, With Commentary, Footnotes, and an \
    Index of Rejected Ideas";

/// Open tabs: (URL, title), English and Russian.
pub const TABS: [(&str, &str); 6] = [
    (
        "https://developer.apple.com/documentation/appkit",
        "AppKit | Apple Developer Documentation",
    ),
    ("https://news.ycombinator.com/", "Hacker News"),
    ("https://lenta.ru/news/", "Новости — Лента.ру"),
    (
        "https://github.com/izevg/quickie/pulls",
        "Pull requests · quickie",
    ),
    (
        "https://ru.wikipedia.org/wiki/Суверенитет",
        "Суверенитет — Википедия",
    ),
    ("https://doc.rust-lang.org/std/", "std - Rust"),
];

/// Now, for every story: 2026-01-15 12:00 UTC.
pub const NOW: Timestamp = Timestamp::from_millis(1_768_478_400_000);

/// How often `query` occurs in `text`, ignoring case.
pub fn count(text: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    text.to_lowercase().matches(&query.to_lowercase()).count()
}

/// A fixture URL.
#[expect(
    clippy::expect_used,
    reason = "fixture URLs are literals in a test; a typo should stop the run"
)]
pub fn url(text: &str) -> Url {
    Url::parse(text).expect("a fixture URL parses")
}

/// The host of `url`, as the overlays show it.
pub fn host(text: &str) -> String {
    url(text).host_str().unwrap_or_default().to_owned()
}

/// A clock stopped at [`NOW`].
pub struct Stopped;

impl Clock for Stopped {
    fn now(&self) -> Timestamp {
        NOW
    }
}

/// Everything the palette reads: a window of [`TABS`], a history, bookmarks with a tag, the
/// default search shortcuts, and the default keymap.
pub struct World {
    pub session: Session,
    pub window: WindowId,
    pub tabs: Vec<TabId>,
    pub history: History,
    pub bookmarks: Bookmarks,
    pub shortcuts: SearchShortcuts,
    pub keymap: Keymap,
    palette: Palette<Stopped>,
}

impl World {
    pub fn new(tabs: &[(&str, &str)]) -> Self {
        let mut session = Session::new(Profile::Main);
        let window = session.open_window();
        let mut opened = Vec::new();
        let mut history = History::new();
        for &(address, title) in tabs {
            let page = url(address);
            if let Ok(tab) = session.open(window, page.clone(), JarChoice::Main, Focus::Foreground)
            {
                let _ = session.navigated(tab, page.clone(), title.to_owned());
                opened.push(tab);
            }
            history.record_visit(&page, title, NOW);
        }
        history.record_visit(
            &url("https://www.rust-lang.org/learn"),
            "Learn Rust - Rust Programming Language",
            NOW,
        );
        let mut bookmarks = Bookmarks::new();
        let docs = url("https://docs.rs/objc2/latest/objc2/");
        bookmarks.save(&docs, "objc2 - Rust".to_owned(), NOW);
        if let Ok(tag) = Tag::try_from("rust") {
            let _ = bookmarks.tag(&docs, tag);
        }
        Self {
            session,
            window,
            tabs: opened,
            history,
            bookmarks,
            shortcuts: SearchShortcuts::defaults(),
            keymap: Keymap::defaults(Platform::MacOs),
            palette: Palette::new(Catalog::builtin(), Stopped),
        }
    }

    /// The palette as the browser opens it: everything gathered once.
    pub fn index(&self) -> PaletteIndex {
        self.palette.open(&Sources {
            session: &self.session,
            history: &self.history,
            bookmarks: &self.bookmarks,
            shortcuts: &self.shortcuts,
            keymap: &self.keymap,
        })
    }

    /// The palette use case's answer to `text`, as the browser asks it.
    pub fn answer(&self, text: &str) -> Vec<PaletteItem> {
        self.index().query(text, 50)
    }

    /// The titles of the window's tabs, in opening order.
    pub fn titles(&self) -> Vec<String> {
        self.tabs
            .iter()
            .filter_map(|&tab| self.session.tab(tab))
            .map(|tab| tab.title.clone())
            .collect()
    }
}
