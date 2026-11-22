use quickie_domain_browsing::{Focus, JarChoice, PinnedSlot, Profile, Session, TabId, Timestamp};
use quickie_domain_commands::{Catalog, CommandId, Keymap, KeymapEntry, Platform};
use quickie_domain_library::{Bookmarks, History, Keyword, SearchShortcuts, Tag, UrlTemplate};
use quickie_domain_palette::Kind;
use url::Url;

use super::{PageFacts, Palette, PaletteIndex, PaletteItem, Sources, Target};
use crate::ExtensionOffer;
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
    extensions: Vec<ExtensionOffer>,
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
            extensions: Vec::new(),
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

    fn open(&self) -> PaletteIndex {
        let palette = Palette::new(Catalog::builtin(), FakeClock::default());
        palette.open(&Sources {
            session: &self.session,
            history: &self.history,
            bookmarks: &self.bookmarks,
            shortcuts: &self.shortcuts,
            keymap: &self.keymap,
            extensions: &self.extensions,
        })
    }

    /// The answer of a palette opened for this one query.
    fn query(&self, text: &str, limit: usize) -> Vec<PaletteItem> {
        self.open().query(text, limit)
    }
}

mod extensions {
    use quickie_domain_commands::KeyStroke;
    use quickie_domain_settings::ExtensionId;

    use super::*;
    use crate::{ExtensionRun, ExtensionTrigger};

    const BITWARDEN: &str = "nngceckbapebfimnlniiiahkandclblb";

    fn trigger(run: ExtensionRun) -> ExtensionTrigger {
        ExtensionTrigger {
            id: ExtensionId::try_from(BITWARDEN).unwrap(),
            run,
        }
    }

    fn world() -> World {
        let mut world = World::new();
        world.extensions = vec![
            ExtensionOffer {
                trigger: trigger(ExtensionRun::Action),
                title: "Bitwarden: Open".to_owned(),
                stroke: Some(KeyStroke::parse("cmd+shift+y", Platform::MacOs).unwrap()),
            },
            ExtensionOffer {
                trigger: trigger(ExtensionRun::Command("autofill_login".to_owned())),
                title: "Bitwarden: Autofill".to_owned(),
                stroke: None,
            },
        ];
        world
    }

    #[test]
    fn running_extensions_are_commands_with_their_keys() {
        let mut items = world().query(">bitwarden", 10);
        items.sort_by(|a, b| a.title.cmp(&b.title));

        assert_eq!(
            targets(&items),
            [
                Target::Extension(trigger(ExtensionRun::Command("autofill_login".to_owned()))),
                Target::Extension(trigger(ExtensionRun::Action)),
            ]
        );
        assert_eq!(items[1].title, "Bitwarden: Open");
        assert_eq!(items[1].detail, "Extension");
        assert_eq!(items[1].kind, Kind::Command);
        assert_eq!(items[1].glyphs, ["⇧⌘Y"]);
        assert_eq!(items[0].glyphs, Vec::<String>::new());
    }

    #[test]
    fn a_pasted_store_page_offers_to_install_first() {
        let mut world = world();
        let page = format!("https://chromewebstore.google.com/detail/bitwarden/{BITWARDEN}");
        world.visit("x", &page, 1);

        let items = world.query(&format!("  {page} "), 10);

        assert_eq!(
            items[0].target,
            Target::InstallExtension(Url::parse(&page).unwrap())
        );
        assert_eq!(items[0].title, "Install Extension from Chrome Web Store");
        assert_eq!(items[0].detail, page);
        assert_eq!(items[0].kind, Kind::Command);
        assert_eq!(items.len(), 2);
        assert_eq!(world.query(&page, 1).len(), 1);
        assert_eq!(world.query(&page, 0), []);
    }

    #[test]
    fn other_pages_offer_no_install() {
        let world = world();

        let items = world.query("https://example.test/x", 10);

        assert!(
            items
                .iter()
                .all(|item| !matches!(item.target, Target::InstallExtension(_)))
        );
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
                facts: PageFacts::default(),
            }]
        );
    }

    #[test]
    fn a_page_row_carries_what_history_knows() {
        let mut world = World::new();
        for at in [1_000, 3_000, 2_000] {
            world
                .history
                .record_visit(&url("h"), "History", Timestamp::from_millis(at));
        }

        let items = world.query("", 10);

        assert_eq!(
            items[0].facts,
            PageFacts {
                visits: 3,
                last_visit: Some(Timestamp::from_millis(3_000)),
                slot: None,
            }
        );
    }

    #[test]
    fn a_bookmark_and_a_tab_carry_the_history_of_their_page() {
        let mut world = World::new();
        world.visit("b", "Bookmark", 2);
        world.visit("t", "Tab", 4);
        world.bookmark("b", "Bookmark");
        world.bookmark("never", "Never visited");
        world.tab("t#part", "Tab");

        let items = world.query("", 10);

        let visits: Vec<(String, u64, Option<Timestamp>)> = items
            .iter()
            .map(|item| (item.title.clone(), item.facts.visits, item.facts.last_visit))
            .collect();
        assert_eq!(
            visits,
            [
                ("Tab".to_owned(), 4, Some(NOW)),
                ("Bookmark".to_owned(), 2, Some(NOW)),
                ("Never visited".to_owned(), 0, None),
            ]
        );
    }

    #[test]
    fn a_pinned_tab_carries_its_slot() {
        let mut world = World::new();
        let pinned = world.tab("p", "Pinned");
        world.tab("q", "Loose");
        let slot = PinnedSlot::try_from(3).unwrap();
        world.session.pin(pinned, slot).unwrap();

        let items = world.query("", 10);

        let mut slots: Vec<(String, Option<PinnedSlot>)> = items
            .iter()
            .map(|item| (item.title.clone(), item.facts.slot))
            .collect();
        slots.sort();
        assert_eq!(
            slots,
            [
                ("Loose".to_owned(), None),
                ("Pinned".to_owned(), Some(slot))
            ]
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
    fn a_page_never_visited_adds_no_frecency() {
        let mut world = World::new();
        world.bookmark("a", "A");
        world.visit("b", "B", 1);

        let items = world.query("", 10);

        // The visit is worth 4 × log2(101) = 24 points, the bookmark 20 and nothing more.
        assert_eq!(
            targets(&items),
            [Target::Url(url("b")), Target::Url(url("a"))]
        );
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

mod open {
    use super::*;

    /// A world with something for every mode.
    fn full() -> World {
        let mut world = World::new();
        world.tab("t", "Tab · über");
        world.bookmark("b", "Bookmark");
        world.bookmarks.tag(&url("b"), tag("rust")).unwrap();
        world.visit("h", "History", 2);
        world
    }

    #[test]
    fn an_open_palette_answers_each_keystroke_as_a_fresh_one_would() {
        let world = full();
        let typed = [
            "", "b", "bo", "über", ">rel", "#", "#ru", "!gh x", "?summ", "@a", "h",
        ];
        let mut index = world.open();

        let typing: Vec<Vec<PaletteItem>> = typed.iter().map(|t| index.query(t, 10)).collect();

        let fresh: Vec<Vec<PaletteItem>> = typed.iter().map(|t| world.query(t, 10)).collect();
        assert_eq!(typing, fresh);
    }

    #[test]
    fn an_open_palette_answers_from_what_it_gathered() {
        let mut world = full();
        let mut index = world.open();

        world.visit("later", "Later", 1);

        let found = index.query("later", 10);
        assert_eq!(found, []);
        assert_eq!(
            targets(&world.query("later", 10)),
            [Target::Url(url("later"))]
        );
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
    fn the_keys_read_as_the_platform_writes_them() {
        let mut world = World::new();
        world.keymap = Keymap::defaults(Platform::Linux);

        let items = world.query(">show command palette", 10);

        assert_eq!(
            item(&items, "quickie.palette.openCommands").glyphs,
            ["Ctrl+Shift+P", "Ctrl+K"]
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
        let entry = KeymapEntry::parse(
            "cmd+e",
            "quickie.devtools.inspectElement",
            None,
            Platform::MacOs,
        )
        .unwrap();
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
                facts: PageFacts::default(),
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
    fn headings_list_nothing_yet() {
        assert_eq!(full().query("@a", 10), []);
    }
}

mod asking {
    use quickie_domain_agents::{Question, Skill, SkillId};

    use super::*;
    use crate::palette::{ASK_DETAIL, ASK_TITLE};

    fn skill(id: &str) -> Target {
        Target::Skill(SkillId::try_from(id).unwrap())
    }

    fn ask(text: &str) -> Target {
        Target::Ask(Question::try_from(text).unwrap())
    }

    fn act(text: &str) -> Target {
        Target::Act(Question::try_from(text).unwrap())
    }

    #[test]
    fn a_bare_question_mark_lists_every_skill_and_asks_nothing() {
        let items = World::new().query("?", 50);

        let mut listed = targets(&items);
        listed.sort_by_key(|target| format!("{target:?}"));
        let mut builtin: Vec<Target> = Skill::builtin()
            .iter()
            .map(|skill| Target::Skill(skill.id().clone()))
            .collect();
        builtin.sort_by_key(|target| format!("{target:?}"));
        assert_eq!(listed, builtin);
        assert!(items.iter().all(|item| item.kind == Kind::Skill));
    }

    #[test]
    fn a_skill_row_shows_its_name_and_what_it_does() {
        let items = World::new().query("?summ", 50);

        let first = &items[0];
        assert_eq!(first.target, skill("summarize"));
        assert_eq!(first.title, "Summarize");
        assert_eq!(
            first.detail,
            "Sums up the page: its main point first, then the key facts."
        );
        assert_eq!(first.title_matches, [0, 1, 2, 3]);
    }

    #[test]
    fn matching_skills_precede_the_typed_question_and_action() {
        let items = World::new().query("?summ", 50);

        let question = &items[1];
        assert_eq!(
            targets(&items),
            [skill("summarize"), ask("summ"), act("summ")]
        );
        assert_eq!(question.kind, Kind::Question);
        assert_eq!(question.title, format!("{ASK_TITLE}summ"));
        assert_eq!(question.detail, ASK_DETAIL);
        assert_eq!(question.glyphs, Vec::<String>::new());
        assert_eq!(question.title_matches, Vec::<u32>::new());
    }

    #[test]
    fn unmatched_questions_offer_ask_and_act() {
        let items = World::new().query("? What does it cost?", 50);

        assert_eq!(
            targets(&items),
            [ask("What does it cost?"), act("What does it cost?")]
        );
        assert_eq!(items[0].title, "Ask about this page: What does it cost?");
    }

    #[test]
    fn a_navigation_request_offers_an_explicit_act_row_after_ask() {
        let items = World::new().query("?open first video", 8);

        assert_eq!(items.len(), 2, "the user needs a separate Act choice");
        assert_eq!(items[0].target, ask("open first video"));
        assert_eq!(items[1].target, act("open first video"));
        assert_eq!(items[1].title, "Act on this page: open first video");
        assert_eq!(
            items[1].detail,
            "Choose a link and open it in an isolated Agent window"
        );
    }

    #[test]
    fn the_question_keeps_its_row_within_the_limit() {
        assert_eq!(targets(&World::new().query("?e", 1)), [ask("e")]);
        assert_eq!(targets(&World::new().query("?e", 2)).len(), 2);
        assert_eq!(World::new().query("?e", 0), []);
    }

    #[test]
    fn a_question_too_long_is_not_offered() {
        let long = format!("?{}", "x".repeat(Question::MAX_CHARS + 1));

        assert!(
            World::new()
                .query(&long, 50)
                .iter()
                .all(|item| item.kind != Kind::Question)
        );
    }

    #[test]
    fn the_agent_offers_no_page_tab_or_command() {
        let mut world = World::new();
        world.tab("summary", "Summary of things");
        world.bookmark("summary", "Summary of things");

        let items = world.query("?summ", 50);

        assert!(items.iter().all(|item| matches!(
            item.target,
            Target::Skill(_) | Target::Ask(_) | Target::Act(_)
        )));
    }
}
