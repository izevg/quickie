use quickie_domain_browsing::{Focus, JarChoice, Profile, Session, TabId, Timestamp};
use quickie_domain_commands::{Catalog, Keymap, Platform};
use quickie_domain_library::{Bookmarks, History, SearchShortcuts};
use url::Url;

use super::{AddressSuggestions, InlineCompletion, ROWS, Suggestion, SuggestionKind};
use crate::fakes::FakeClock;
use crate::palette::{Palette, Sources, Target};

const NOW: Timestamp = Timestamp::from_millis(0);

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn search(query: &str) -> Target {
    Target::Search(url(&format!("https://duckduckgo.com/?q={query}")))
}

/// Everything the overlay reads, empty until a test fills it.
struct World {
    session: Session,
    history: History,
    bookmarks: Bookmarks,
    shortcuts: SearchShortcuts,
    keymap: Keymap,
}

impl World {
    fn new() -> Self {
        let mut session = Session::new(Profile::Main);
        session.open_window();
        Self {
            session,
            history: History::new(),
            bookmarks: Bookmarks::new(),
            shortcuts: SearchShortcuts::defaults(),
            keymap: Keymap::defaults(Platform::MacOs),
        }
    }

    fn tab(&mut self, address: &str, title: &str) -> TabId {
        let window = self.session.windows().next().unwrap();
        let tab = self
            .session
            .open(window, url(address), JarChoice::Main, Focus::Foreground)
            .unwrap();
        self.session
            .navigated(tab, url(address), title.to_owned())
            .unwrap();
        tab
    }

    fn visit(&mut self, address: &str, title: &str, times: usize) {
        for _ in 0..times {
            self.history.record_visit(&url(address), title, NOW);
        }
    }

    fn bookmark(&mut self, address: &str, title: &str) {
        self.bookmarks.save(&url(address), title.to_owned(), NOW);
    }

    fn suggest(&self, typed: &str, complete: bool) -> AddressSuggestions {
        let palette = Palette::new(Catalog::builtin(), FakeClock::default());
        palette
            .open(&Sources {
                session: &self.session,
                history: &self.history,
                bookmarks: &self.bookmarks,
                shortcuts: &self.shortcuts,
                keymap: &self.keymap,
                extensions: &[],
            })
            .suggest(typed, complete)
    }
}

fn targets(rows: &[Suggestion]) -> Vec<Target> {
    rows.iter().map(|row| row.target.clone()).collect()
}

fn kinds(rows: &[Suggestion]) -> Vec<SuggestionKind> {
    rows.iter().map(|row| row.kind).collect()
}

mod completing {
    use super::*;

    #[test]
    fn blank_text_has_no_answer() {
        let mut world = World::new();
        world.visit("https://github.com/", "GitHub", 1);

        assert_eq!(world.suggest("", true), AddressSuggestions::default());
        assert_eq!(world.suggest("   ", true), AddressSuggestions::default());
    }

    #[test]
    fn a_host_is_completed_and_gone_to_first() {
        let mut world = World::new();
        let tab = world.tab("https://github.com/izevg/quickie/pulls", "Pull requests");

        let answer = world.suggest("git", true);

        assert_eq!(
            answer.completion,
            Some(InlineCompletion {
                typed: "git".into(),
                tail: "hub.com".into(),
                url: url("https://github.com/"),
            })
        );
        assert_eq!(
            answer.rows.first(),
            Some(&Suggestion {
                target: Target::Url(url("https://github.com/")),
                kind: SuggestionKind::GoTo,
                title: "Go to github.com".into(),
                url: "https://github.com/".into(),
                host: 8..18,
                title_matches: Vec::new(),
                url_matches: Vec::new(),
                field: "github.com".into(),
            })
        );
        assert_eq!(
            targets(&answer.rows),
            [
                Target::Url(url("https://github.com/")),
                Target::Tab(tab),
                search("git")
            ]
        );
    }

    #[test]
    fn the_best_page_completes_open_tabs_first() {
        let mut world = World::new();
        world.visit("https://gitlab.com/x", "GitLab", 2);
        world.tab("https://github.com/x", "GitHub");

        let answer = world.suggest("git", true);

        assert_eq!(
            answer.completion.map(|c| c.text()).as_deref(),
            Some("github.com")
        );
    }

    #[test]
    fn a_completion_that_is_a_known_page_is_that_page() {
        let mut world = World::new();
        world.visit("https://github.com/", "GitHub", 3);
        world.visit("https://github.com/izevg", "izevg", 1);

        let answer = world.suggest("gi", true);

        assert_eq!(
            kinds(&answer.rows),
            [
                SuggestionKind::History,
                SuggestionKind::History,
                SuggestionKind::Search
            ]
        );
        assert_eq!(
            targets(&answer.rows),
            [
                Target::Url(url("https://github.com/")),
                Target::Url(url("https://github.com/izevg")),
                search("gi")
            ]
        );
        assert_eq!(
            answer.rows.first().map(|row| row.field.as_str()),
            Some("https://github.com/")
        );
    }

    #[test]
    fn nothing_completes_unless_asked() {
        let mut world = World::new();
        world.visit("https://github.com/x", "GitHub", 1);

        let answer = world.suggest("git", false);

        assert_eq!(answer.completion, None);
        assert_eq!(
            targets(&answer.rows),
            [search("git"), Target::Url(url("https://github.com/x"))]
        );
    }

    #[test]
    fn text_with_a_space_never_completes() {
        let mut world = World::new();
        world.visit("https://github.com/x", "GitHub", 1);

        assert_eq!(world.suggest("git ", true).completion, None);
        assert_eq!(world.suggest(" git", true).completion, None);
    }

    #[test]
    fn www_is_completed_past() {
        let mut world = World::new();
        world.visit("https://www.rust-lang.org/learn", "Learn Rust", 1);

        let answer = world.suggest("rust", true);

        assert_eq!(
            answer.completion.map(|c| (c.text(), c.url)),
            Some(("rust-lang.org".into(), url("https://www.rust-lang.org/")))
        );
        let first = answer.rows.first().unwrap();
        assert_eq!(first.title, "Go to rust-lang.org");
        assert_eq!(first.host, 8..25);
    }
}

mod rows {
    use super::*;

    #[test]
    fn a_typed_address_is_gone_to_first() {
        let world = World::new();

        let answer = world.suggest("example.com", true);

        assert_eq!(answer.completion, None);
        assert_eq!(
            answer.rows,
            [
                Suggestion {
                    target: Target::Url(url("https://example.com/")),
                    kind: SuggestionKind::GoTo,
                    title: "Go to example.com".into(),
                    url: "https://example.com/".into(),
                    host: 8..19,
                    title_matches: Vec::new(),
                    url_matches: Vec::new(),
                    field: "example.com".into(),
                },
                Suggestion {
                    target: search("example.com"),
                    kind: SuggestionKind::Search,
                    title: "Search DuckDuckGo for “example.com”".into(),
                    url: "https://duckduckgo.com/?q=example.com".into(),
                    host: 8..22,
                    title_matches: Vec::new(),
                    url_matches: Vec::new(),
                    field: "example.com".into(),
                }
            ]
        );
    }

    #[test]
    fn the_address_of_an_open_tab_switches_to_it() {
        let mut world = World::new();
        world.visit("https://github.com/izevg", "izevg", 1);
        let tab = world.tab("https://github.com/izevg", "izevg");

        let answer = world.suggest("https://github.com/izevg", true);

        assert_eq!(
            targets(&answer.rows),
            [Target::Tab(tab), search("https%3A%2F%2Fgithub.com%2Fizevg")]
        );
        assert_eq!(
            answer.rows.first().map(|row| row.kind),
            Some(SuggestionKind::OpenTab)
        );
    }

    #[test]
    fn plain_text_is_searched_first() {
        let mut world = World::new();
        world.visit("https://news.ycombinator.com/", "Hacker News", 1);

        let answer = world.suggest("hacker", true);

        assert_eq!(answer.completion, None);
        assert_eq!(
            targets(&answer.rows),
            [
                search("hacker"),
                Target::Url(url("https://news.ycombinator.com/"))
            ]
        );
        let page = answer.rows.get(1).unwrap();
        assert_eq!(page.title_matches, [0, 1, 2, 3, 4, 5]);
        assert_eq!(page.host, 8..28);
        assert_eq!(page.field, "https://news.ycombinator.com/");
    }

    #[test]
    fn addresses_that_start_with_the_text_come_before_fuzzy_matches() {
        let mut world = World::new();
        world.visit("https://example.com/digits", "Digits", 9);
        world.visit("https://gitlab.com/", "GitLab", 1);
        world.visit("https://github.com/", "GitHub", 3);

        let answer = world.suggest("git", false);

        assert_eq!(
            targets(&answer.rows),
            [
                search("git"),
                Target::Url(url("https://github.com/")),
                Target::Url(url("https://gitlab.com/")),
                Target::Url(url("https://example.com/digits")),
            ]
        );
        assert_eq!(
            answer.rows.get(1).map(|row| row.url_matches.clone()),
            Some(vec![8, 9, 10])
        );
    }

    #[test]
    fn each_kind_is_named() {
        let mut world = World::new();
        world.visit("https://a.example/one", "One", 1);
        world.visit("https://a.example/two", "Two", 1);
        world.bookmark("https://a.example/two", "Two");
        world.visit("https://a.example/three", "Three", 1);
        world.tab("https://a.example/three", "Three");

        let answer = world.suggest("a.ex", false);

        assert_eq!(
            kinds(&answer.rows),
            [
                SuggestionKind::GoTo,
                SuggestionKind::OpenTab,
                SuggestionKind::Bookmark,
                SuggestionKind::History,
                SuggestionKind::Search,
            ]
        );
    }

    #[test]
    fn at_most_eight_rows_with_the_search_last() {
        let mut world = World::new();
        for n in 0..20 {
            world.visit(&format!("https://git{n}.example/"), "Git", 1);
        }

        let completed = world.suggest("git", true);
        let searched = world.suggest("git", false);

        assert_eq!(completed.rows.len(), ROWS);
        assert_eq!(
            completed.rows.last().map(|row| row.kind),
            Some(SuggestionKind::Search)
        );
        assert_eq!(searched.rows.len(), ROWS);
        assert_eq!(
            searched.rows.first().map(|row| row.kind),
            Some(SuggestionKind::Search)
        );
        assert_eq!(
            searched
                .rows
                .iter()
                .filter(|row| row.kind == SuggestionKind::Search)
                .count(),
            1
        );
    }

    #[test]
    fn a_page_is_offered_once() {
        let mut world = World::new();
        world.visit("https://github.com/izevg", "github izevg", 1);

        let answer = world.suggest("github", false);

        assert_eq!(answer.rows.len(), 2);
    }
}

mod page_actions {
    use quickie_domain_commands::CommandId;

    use super::*;

    #[test]
    fn the_page_offers_copying_and_its_shields_with_their_keys() {
        let world = World::new();
        let palette = Palette::new(Catalog::builtin(), FakeClock::default());
        let index = palette.open(&Sources {
            session: &world.session,
            history: &world.history,
            bookmarks: &world.bookmarks,
            shortcuts: &world.shortcuts,
            keymap: &world.keymap,
            extensions: &[],
        });

        let actions = index.page_actions();

        let command = |id: &str| Target::Command(CommandId::try_from(id).unwrap());
        assert_eq!(
            actions
                .iter()
                .map(|item| (
                    item.target.clone(),
                    item.title.as_str(),
                    item.glyphs.clone()
                ))
                .collect::<Vec<_>>(),
            [
                (
                    command("quickie.page.copyUrl"),
                    "Copy URL",
                    vec!["⇧⌘C".to_owned()]
                ),
                (
                    command("quickie.page.copyAsMarkdownLink"),
                    "Copy as Markdown Link",
                    vec!["⌥⌘C".to_owned()]
                ),
                (
                    command("quickie.shields.open"),
                    "Show Shields",
                    vec!["⌥⌘S".to_owned()]
                ),
            ]
        );
    }
}

mod shortcuts {
    use super::*;

    #[test]
    fn a_keyword_searches_its_site() {
        let world = World::new();

        let answer = world.suggest("!gh  quickie app", true);

        assert_eq!(
            answer.rows,
            [Suggestion {
                target: Target::Search(url("https://github.com/search?q=quickie+app")),
                kind: SuggestionKind::Search,
                title: "Search github.com for “quickie app”".into(),
                url: "https://github.com/search?q=quickie+app".into(),
                host: 8..18,
                title_matches: Vec::new(),
                url_matches: Vec::new(),
                field: "!gh  quickie app".into(),
            }]
        );
    }

    #[test]
    fn www_is_dropped_from_the_site() {
        let mut world = World::new();
        world.shortcuts.set(
            "x".try_into().unwrap(),
            "https://www.example.com/?q=%s".try_into().unwrap(),
        );

        let answer = world.suggest("!x y", true);

        assert_eq!(
            answer.rows.first().map(|row| row.title.as_str()),
            Some("Search example.com for “y”")
        );
    }

    #[test]
    fn a_keyword_without_terms_or_unknown_is_searched_as_text() {
        let world = World::new();

        assert_eq!(
            targets(&world.suggest("!gh ", true).rows),
            [search("%21gh")]
        );
        assert_eq!(targets(&world.suggest("!gh", true).rows), [search("%21gh")]);
        assert_eq!(
            targets(&world.suggest("!zz foo", true).rows),
            [search("%21zz+foo")]
        );
    }
}
