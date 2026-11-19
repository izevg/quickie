use quickie_domain_browsing::{Focus, JarChoice, Session, TabId, Timestamp};
use quickie_domain_commands::{Catalog, CommandId, Keymap, KeymapEntry};
use quickie_domain_library::{Bookmarks, History, Keyword, SearchShortcuts, Tag, UrlTemplate};
use quickie_domain_palette::Kind;
use url::Url;

use super::{Palette, PaletteItem, Sources, Target};
use crate::fakes::FakeClock;

fn url(path: &str) -> Url {
    Url::parse(&format!("https://example.test/{path}")).unwrap()
}

fn tag(text: &str) -> Tag {
    Tag::try_from(text).unwrap()
}

fn command(id: &str) -> CommandId {
    CommandId::try_from(id).unwrap()
}

const NOW: Timestamp = Timestamp::from_millis(0);

/// Everything the palette reads, empty until a test fills it.
struct World {
    session: Session,
    history: History,
    bookmarks: Bookmarks,
    shortcuts: SearchShortcuts,
    keymap: Keymap,
}

impl World {
    fn new() -> Self {
        let mut session = Session::new();
        session.open_window();
        Self {
            session,
            history: History::new(),
            bookmarks: Bookmarks::new(),
            shortcuts: SearchShortcuts::defaults(),
            keymap: Keymap::default(),
        }
    }

    fn tab(&mut self, path: &str, title: &str) -> TabId {
        let window = self.session.windows().next().unwrap();
        let tab = self
            .session
            .open(window, url(path), JarChoice::Main, Focus::Foreground)
            .unwrap();
        self.session
            .navigated(tab, url(path), title.to_owned())
            .unwrap();
        tab
    }

    fn visit(&mut self, path: &str, title: &str, times: usize) {
        for _ in 0..times {
            self.history.record_visit(&url(path), title, NOW);
        }
    }

    fn bookmark(&mut self, path: &str, title: &str) {
        self.bookmarks.save(&url(path), title.to_owned(), NOW);
    }

    fn query(&self, text: &str, limit: usize) -> Vec<PaletteItem> {
        let palette = Palette::new(Catalog::builtin(), FakeClock::default());
        palette.query(
            text,
            limit,
            &Sources {
                session: &self.session,
                history: &self.history,
                bookmarks: &self.bookmarks,
                shortcuts: &self.shortcuts,
                keymap: &self.keymap,
            },
        )
    }
}

fn targets(items: &[PaletteItem]) -> Vec<Target> {
    items.iter().map(|item| item.target.clone()).collect()
}

mod going_to {
    use super::*;

    #[test]
    fn at_equal_frecency_tabs_beat_bookmarks_beat_history() {
        let mut world = World::new();
        world.visit("h", "History", 1);
        world.visit("b", "Bookmark", 1);
        world.visit("t", "Tab", 1);
        world.bookmark("b", "Bookmark");
        let tab = world.tab("t", "Tab");

        let items = world.query("", 10);

        assert_eq!(
            targets(&items),
            [
                Target::Tab(tab),
                Target::Url(url("b")),
                Target::Url(url("h"))
            ]
        );
    }

    #[test]
    fn a_row_carries_its_kind_title_and_url() {
        let mut world = World::new();
        world.bookmark("b", "Bookmark");

        let items = world.query("", 10);

        assert_eq!(
            items,
            [PaletteItem {
                target: Target::Url(url("b")),
                kind: Kind::Bookmark,
                title: "Bookmark".to_owned(),
                detail: "https://example.test/b".to_owned(),
                glyphs: Vec::new(),
                title_matches: Vec::new(),
                detail_matches: Vec::new(),
            }]
        );
    }

    #[test]
    fn a_page_without_a_title_shows_its_url() {
        let mut world = World::new();
        for path in ["h", "b", "t"] {
            world.visit(path, "", 1);
        }
        world.bookmark("b", "");
        world.tab("t", "");

        let titles: Vec<String> = world.query("", 10).into_iter().map(|i| i.title).collect();

        assert_eq!(
            titles,
            [
                "https://example.test/t",
                "https://example.test/b",
                "https://example.test/h"
            ]
        );
    }

    #[test]
    fn an_open_page_is_offered_only_as_its_tab() {
        let mut world = World::new();
        world.visit("p", "Page", 1);
        world.bookmark("p", "Page");
        let tab = world.tab("p#section", "Page");

        let items = world.query("", 10);

        assert_eq!(targets(&items), [Target::Tab(tab)]);
    }

    #[test]
    fn a_saved_page_is_offered_only_as_its_bookmark() {
        let mut world = World::new();
        world.visit("p", "Page", 1);
        world.bookmark("p", "Page");

        let items = world.query("", 10);

        assert_eq!(
            items.iter().map(|item| item.kind).collect::<Vec<_>>(),
            [Kind::Bookmark]
        );
    }

    #[test]
    fn tabs_of_every_window_are_offered() {
        let mut world = World::new();
        let first = world.tab("a", "A");
        world.session.open_window();
        let window = world.session.windows().last().unwrap();
        let second = world
            .session
            .open(window, url("b"), JarChoice::Main, Focus::Foreground)
            .unwrap();

        let items = world.query("", 10);

        assert_eq!(targets(&items), [Target::Tab(first), Target::Tab(second)]);
    }

    #[test]
    fn history_ranks_by_frecency() {
        let mut world = World::new();
        world.visit("rare", "Rare", 1);
        world.visit("often", "Often", 9);

        let items = world.query("", 10);

        assert_eq!(
            targets(&items),
            [Target::Url(url("often")), Target::Url(url("rare"))]
        );
    }

    #[test]
    fn a_tab_takes_the_frecency_of_its_page() {
        let mut world = World::new();
        world.tab("rare", "Rare");
        let often = world.tab("often", "Often");
        world.visit("often", "Often", 9);

        let items = world.query("", 1);

        assert_eq!(targets(&items), [Target::Tab(often)]);
    }

    #[test]
    fn a_bookmark_takes_the_frecency_of_its_page() {
        let mut world = World::new();
        world.bookmark("rare", "Rare");
        world.bookmark("often", "Often");
        world.visit("often", "Often", 9);

        let items = world.query("", 1);

        assert_eq!(targets(&items), [Target::Url(url("often"))]);
    }

    #[test]
    fn the_text_filters_and_marks_what_it_matched() {
        let mut world = World::new();
        world.visit("a", "Rust book", 1);
        world.visit("b", "Swift book", 1);

        let items = world.query("rust", 10);

        assert_eq!(targets(&items), [Target::Url(url("a"))]);
        assert_eq!(items[0].title_matches, [0, 1, 2, 3]);
    }

    #[test]
    fn the_url_matches_too() {
        let mut world = World::new();
        world.visit("zebra", "Animals", 1);

        let items = world.query("zebra", 10);

        assert_eq!(items[0].detail_matches, [21, 22, 23, 24, 25]);
    }

    #[test]
    fn the_limit_keeps_the_best() {
        let mut world = World::new();
        world.visit("a", "A", 1);
        world.visit("b", "B", 2);
        world.visit("c", "C", 3);

        let items = world.query("", 2);

        assert_eq!(
            targets(&items),
            [Target::Url(url("c")), Target::Url(url("b"))]
        );
    }

    #[test]
    fn ten_thousand_pages_find_the_one_asked_for() {
        let mut world = World::new();
        for n in 0..10_000 {
            world.visit(&format!("page/{n}"), &format!("Page {n}"), 1);
        }

        let best = world.query("page 9999", 20);
        let all = world.query("", 50);

        assert_eq!(best[0].target, Target::Url(url("page/9999")));
        assert_eq!(all.len(), 50);
    }
}

mod commands {
    use super::*;

    fn item<'a>(items: &'a [PaletteItem], id: &str) -> &'a PaletteItem {
        let target = Target::Command(command(id));
        items.iter().find(|item| item.target == target).unwrap()
    }

    #[test]
    fn a_command_shows_its_category_and_keys() {
        let world = World::new();

        let items = world.query(">reopen closed", 10);

        let reopen = item(&items, "quickie.tab.reopenClosed");
        assert_eq!(
            (reopen.kind, reopen.title.as_str(), reopen.detail.as_str()),
            (Kind::Command, "Reopen Closed Tab", "Tabs")
        );
        assert_eq!(reopen.glyphs, ["⇧⌘T"]);
    }

    #[test]
    fn every_key_of_a_command_is_shown() {
        let world = World::new();

        let items = world.query(">show command palette", 10);

        assert_eq!(
            item(&items, "quickie.palette.openCommands").glyphs,
            ["⇧⌘P", "⌘K"]
        );
    }

    #[test]
    fn a_command_without_a_key_shows_none() {
        let world = World::new();

        let items = world.query(">inspect element", 10);

        assert_eq!(
            item(&items, "quickie.devtools.inspectElement").glyphs,
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_keys_follow_the_users_keymap() {
        let mut world = World::new();
        let entry = KeymapEntry::parse("cmd+e", "quickie.devtools.inspectElement", None).unwrap();
        world.keymap = world
            .keymap
            .with_user_entries(&Catalog::builtin(), vec![entry])
            .unwrap();

        let items = world.query(">inspect element", 10);

        assert_eq!(
            item(&items, "quickie.devtools.inspectElement").glyphs,
            ["⌘E"]
        );
    }

    #[test]
    fn the_empty_command_query_lists_the_whole_catalog() {
        let world = World::new();

        let items = world.query(">", 100);

        assert_eq!(items.len(), Catalog::builtin().iter().count());
    }
}

mod tags {
    use super::*;

    #[test]
    fn every_tag_is_listed_with_its_page_count() {
        let mut world = World::new();
        world.bookmark("a", "A");
        world.bookmark("b", "B");
        world.bookmarks.tag(&url("a"), tag("rust")).unwrap();
        world.bookmarks.tag(&url("b"), tag("rust")).unwrap();
        world.bookmarks.tag(&url("a"), tag("later")).unwrap();

        let items = world.query("#", 10);

        let rows: Vec<(Target, Kind, &str, &str)> = items
            .iter()
            .map(|i| {
                (
                    i.target.clone(),
                    i.kind,
                    i.title.as_str(),
                    i.detail.as_str(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (Target::Tag(tag("later")), Kind::Tag, "later", "1 page"),
                (Target::Tag(tag("rust")), Kind::Tag, "rust", "2 pages"),
            ]
        );
    }
}

mod searching {
    use super::*;

    #[test]
    fn a_keyword_aims_at_its_search_for_the_terms() {
        let world = World::new();

        let items = world.query("!gh quickie browser", 1);

        let search = Url::parse("https://github.com/search?q=quickie+browser").unwrap();
        assert_eq!(
            items,
            [PaletteItem {
                target: Target::Search(search),
                kind: Kind::SearchShortcut,
                title: "gh".to_owned(),
                detail: "https://github.com/search?q=%s".to_owned(),
                glyphs: Vec::new(),
                title_matches: vec![0, 1],
                detail_matches: vec![8, 11],
            }]
        );
    }

    #[test]
    fn a_template_the_terms_break_is_left_out() {
        let mut world = World::new();
        world.shortcuts = SearchShortcuts::new();
        world.shortcuts.set(
            Keyword::try_from("host").unwrap(),
            UrlTemplate::try_from("https://%s.example.test/").unwrap(),
        );

        let broken = world.query("!host #", 10);
        let fine = world.query("!host docs", 10);

        assert_eq!(broken, []);
        assert_eq!(
            targets(&fine),
            [Target::Search(
                Url::parse("https://docs.example.test/").unwrap()
            )]
        );
    }
}

mod unanswered {
    use super::*;

    /// A world where every kind of candidate exists and matches "a".
    fn full() -> World {
        let mut world = World::new();
        world.tab("a", "a");
        world.bookmark("a", "a");
        world.bookmarks.tag(&url("a"), tag("a")).unwrap();
        world
    }

    #[test]
    fn an_agent_question_lists_nothing() {
        assert_eq!(full().query("?a", 10), []);
    }

    #[test]
    fn headings_list_nothing_yet() {
        assert_eq!(full().query("@a", 10), []);
    }
}
