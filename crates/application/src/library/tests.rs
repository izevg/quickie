use std::time::Duration;

use quickie_domain_browsing::{Jar, JarId, Timestamp};
use quickie_domain_library::{
    Bookmark, Bookmarks, BookmarksSnapshot, CorruptBookmarks, CorruptHistory, CorruptShortcuts,
    Forgotten, History, HistoryEntry, HistorySnapshot, Keyword, NotSaved, Recorded, Saved,
    SearchShortcuts, SearchShortcutsSnapshot, Tag, UrlTemplate,
};
use url::Url;

use super::{CorruptLibrary, Library};
use crate::fakes::{FakeClock, FakeLibraryRepo};
use crate::{AppError, LibrarySnapshot, StoreError};

type Subject = Library<FakeLibraryRepo, FakeClock>;

fn url(path: &str) -> Url {
    Url::parse(&format!("https://example.test/{path}")).unwrap()
}

fn tag(text: &str) -> Tag {
    Tag::try_from(text).unwrap()
}

fn keyword(text: &str) -> Keyword {
    Keyword::try_from(text).unwrap()
}

fn template(text: &str) -> UrlTemplate {
    UrlTemplate::try_from(text).unwrap()
}

fn start_with(saved: LibrarySnapshot) -> Subject {
    let repo = FakeLibraryRepo {
        saved,
        ..FakeLibraryRepo::default()
    };
    Library::start(repo, FakeClock::default()).unwrap()
}

fn started() -> Subject {
    start_with(LibrarySnapshot::default())
}

fn at(library: &Subject, millis: u64) {
    library.clock.0.set(millis);
}

fn visit(library: &mut Subject, path: &str, millis: u64) {
    at(library, millis);
    library.record_visit(&url(path), "", Jar::Main).unwrap();
}

fn bookmarked(path: &str) -> Subject {
    let mut library = started();
    library
        .save_bookmark(&url(path), "Page".to_owned())
        .unwrap();
    library
}

fn fail_saves(library: &mut Subject) {
    library.repo.save_fails = true;
}

fn store_failure() -> AppError {
    AppError::Store(StoreError("save".to_owned()))
}

mod starting {
    use super::*;

    #[test]
    fn a_first_launch_has_an_empty_history_and_the_default_shortcuts() {
        let library = started();

        assert_eq!(library.history(), &History::new());
        assert_eq!(library.bookmarks(), &Bookmarks::new());
        assert_eq!(library.shortcuts(), &SearchShortcuts::defaults());
        assert_eq!(library.corrupt_on_start(), []);
    }

    #[test]
    fn starting_saves_nothing() {
        let library = started();

        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn every_saved_part_is_restored() {
        let mut history = History::new();
        history.record_visit(&url("a"), "A", Timestamp::from_millis(5));
        let mut bookmarks = Bookmarks::new();
        bookmarks.save(&url("b"), "B".to_owned(), Timestamp::from_millis(6));
        let mut shortcuts = SearchShortcuts::new();
        shortcuts.set(keyword("x"), template("https://x.test/?q=%s"));

        let library = start_with(LibrarySnapshot {
            history: Some(history.snapshot()),
            bookmarks: Some(bookmarks.snapshot()),
            shortcuts: Some(shortcuts.snapshot()),
            downloads: None,
        });

        assert_eq!(library.history(), &history);
        assert_eq!(library.bookmarks(), &bookmarks);
        assert_eq!(library.shortcuts(), &shortcuts);
    }

    #[test]
    fn shortcuts_the_user_deleted_stay_deleted() {
        let library = start_with(LibrarySnapshot {
            shortcuts: Some(SearchShortcuts::new().snapshot()),
            ..LibrarySnapshot::default()
        });

        assert_eq!(library.shortcuts(), &SearchShortcuts::new());
    }

    #[test]
    fn corrupt_parts_start_fresh_and_say_why() {
        let with_fragment = Url::parse("https://example.test/a#x").unwrap();
        let bookmark = Bookmark {
            url: url("b"),
            title: String::new(),
            tags: std::collections::BTreeSet::new(),
            saved_at: Timestamp::from_millis(0),
        };
        let shortcut = (keyword("x"), template("https://x.test/?q=%s"));

        let library = start_with(LibrarySnapshot {
            history: Some(HistorySnapshot {
                entries: vec![HistoryEntry {
                    url: with_fragment.clone(),
                    title: String::new(),
                    visit_count: 1,
                    recent_visits: vec![Timestamp::from_millis(0)],
                }],
            }),
            bookmarks: Some(BookmarksSnapshot {
                bookmarks: vec![bookmark.clone(), bookmark],
            }),
            shortcuts: Some(SearchShortcutsSnapshot {
                shortcuts: vec![shortcut.clone(), shortcut],
            }),
            downloads: None,
        });

        assert_eq!(
            library.corrupt_on_start(),
            [
                CorruptLibrary::History(CorruptHistory::HasFragment(with_fragment)),
                CorruptLibrary::Bookmarks(CorruptBookmarks::DuplicateBookmark(url("b"))),
                CorruptLibrary::Shortcuts(CorruptShortcuts::DuplicateKeyword(keyword("x"))),
            ]
        );
        assert_eq!(library.history(), &History::new());
        assert_eq!(library.bookmarks(), &Bookmarks::new());
        assert_eq!(library.shortcuts(), &SearchShortcuts::defaults());
    }

    #[test]
    fn a_failed_load_stops_the_start() {
        let repo = FakeLibraryRepo {
            load_fails: true,
            ..FakeLibraryRepo::default()
        };

        let result = Library::start(repo, FakeClock::default());

        assert_eq!(
            result.err(),
            Some(AppError::Store(StoreError("load".to_owned())))
        );
    }
}

mod visiting {
    use super::*;

    #[test]
    fn a_visit_is_recorded_at_the_clock_and_saved() {
        let mut library = started();
        at(&library, 42);

        let recorded = library.record_visit(&url("a"), "A", Jar::Main);

        assert_eq!(recorded, Ok(Recorded::NewEntry));
        let entry = library.history().get(&url("a")).unwrap();
        assert_eq!(
            (entry.title.as_str(), entry.recent_visits.clone()),
            ("A", vec![Timestamp::from_millis(42)])
        );
        assert_eq!(
            library.repo.saved.history,
            Some(library.history().snapshot())
        );
    }

    #[test]
    fn a_second_visit_is_a_revisit() {
        let mut library = started();
        visit(&mut library, "a", 1);

        let recorded = library.record_visit(&url("a"), "", Jar::Main);

        assert_eq!(recorded, Ok(Recorded::Revisit));
        assert_eq!(library.repo.saves, 2);
    }

    #[test]
    fn a_throwaway_jar_leaves_no_trace() {
        let mut library = started();

        let recorded = library.record_visit(&url("a"), "A", Jar::Throwaway(JarId::new(0)));

        assert_eq!(recorded, Ok(Recorded::Ignored));
        assert_eq!(library.history(), &History::new());
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn a_page_outside_the_web_saves_nothing() {
        let mut library = started();
        let blank = Url::parse("about:blank").unwrap();

        let recorded = library.record_visit(&blank, "", Jar::Main);

        assert_eq!(recorded, Ok(Recorded::Ignored));
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let mut library = started();
        fail_saves(&mut library);

        let result = library.record_visit(&url("a"), "A", Jar::Main);

        assert_eq!(result, Err(store_failure()));
        assert_eq!(library.history(), &History::new());
    }

    #[test]
    fn a_later_title_renames_the_visit_and_saves() {
        let mut library = started();
        visit(&mut library, "a", 1);

        let changed = library.retitle_visit(&url("a"), "Loaded", Jar::Main);

        assert_eq!(changed, Ok(true));
        let entry = library.history().get(&url("a")).unwrap();
        assert_eq!((entry.title.as_str(), entry.visit_count), ("Loaded", 1));
        assert_eq!(library.repo.saves, 2);
        assert_eq!(
            library.repo.saved.history,
            Some(library.history().snapshot())
        );
    }

    #[test]
    fn a_title_that_changes_nothing_saves_nothing() {
        let mut library = started();
        visit(&mut library, "a", 1);

        let unvisited = library.retitle_visit(&url("b"), "B", Jar::Main);
        let throwaway = library.retitle_visit(&url("a"), "A", Jar::Throwaway(JarId::new(0)));

        assert_eq!((unvisited, throwaway), (Ok(false), Ok(false)));
        assert_eq!(library.history().get(&url("a")).unwrap().title, "");
        assert_eq!(library.repo.saves, 1);
    }

    #[test]
    fn a_failed_retitle_keeps_the_old_title() {
        let mut library = started();
        visit(&mut library, "a", 1);
        fail_saves(&mut library);

        let result = library.retitle_visit(&url("a"), "Loaded", Jar::Main);

        assert_eq!(result, Err(store_failure()));
        assert_eq!(library.history().get(&url("a")).unwrap().title, "");
    }
}

mod forgetting {
    use super::*;

    /// Visits to `old` at 1 s and to `new` at 8 s and 9 s; the clock reads 10 s.
    fn visited() -> Subject {
        let mut library = started();
        visit(&mut library, "old", 1_000);
        visit(&mut library, "new", 8_000);
        visit(&mut library, "new", 9_000);
        at(&library, 10_000);
        library
    }

    #[test]
    fn forgets_the_visits_of_the_span_up_to_its_first_millisecond() {
        let mut library = visited();

        let forgotten = library.forget_since(Duration::from_secs(2));

        assert_eq!(
            forgotten,
            Ok(Forgotten {
                visits: 2,
                entries: 1
            })
        );
        assert_eq!(library.history().get(&url("new")), None);
        assert!(library.history().get(&url("old")).is_some());
        assert_eq!(
            library.repo.saved.history,
            Some(library.history().snapshot())
        );
    }

    #[test]
    fn a_visit_just_before_the_span_is_kept() {
        let mut library = visited();

        let forgotten = library.forget_since(Duration::from_millis(1_999));

        assert_eq!(
            forgotten,
            Ok(Forgotten {
                visits: 1,
                entries: 0
            })
        );
    }

    #[test]
    fn a_span_without_visits_saves_nothing() {
        let mut library = visited();
        let saves = library.repo.saves;

        let forgotten = library.forget_since(Duration::from_millis(999));

        assert_eq!(forgotten, Ok(Forgotten::default()));
        assert_eq!(library.repo.saves, saves);
    }

    #[test]
    fn a_span_longer_than_time_itself_forgets_everything() {
        let mut library = visited();

        library.forget_since(Duration::MAX).unwrap();

        assert_eq!(library.history(), &History::new());
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let mut library = visited();
        let before = library.history().clone();
        fail_saves(&mut library);

        let result = library.forget_since(Duration::from_secs(2));

        assert_eq!(result, Err(store_failure()));
        assert_eq!(library.history(), &before);
    }
}

mod bookmarking {
    use super::*;

    #[test]
    fn a_new_bookmark_is_dated_by_the_clock_and_saved() {
        let mut library = started();
        at(&library, 7);

        let saved = library.save_bookmark(&url("a"), "A".to_owned());

        assert_eq!(saved, Ok(Saved { created: true }));
        let bookmark = library.bookmarks().get(&url("a")).unwrap();
        assert_eq!(bookmark.saved_at, Timestamp::from_millis(7));
        assert_eq!(
            library.repo.saved.bookmarks,
            Some(library.bookmarks().snapshot())
        );
    }

    #[test]
    fn saving_a_saved_page_again_saves_nothing() {
        let mut library = bookmarked("a");

        let saved = library.save_bookmark(&url("a"), "Other".to_owned());

        assert_eq!(saved, Ok(Saved { created: false }));
        assert_eq!(library.repo.saves, 1);
    }

    #[test]
    fn a_failed_save_keeps_the_page_unsaved() {
        let mut library = started();
        fail_saves(&mut library);

        let result = library.save_bookmark(&url("a"), "A".to_owned());

        assert_eq!(result, Err(store_failure()));
        assert_eq!(library.bookmarks(), &Bookmarks::new());
    }

    #[test]
    fn removing_a_bookmark_returns_it_and_saves() {
        let mut library = bookmarked("a");

        let removed = library.remove_bookmark(&url("a"));

        assert_eq!(
            removed.unwrap().map(|bookmark| bookmark.url),
            Some(url("a"))
        );
        assert_eq!(
            library.repo.saved.bookmarks,
            Some(Bookmarks::new().snapshot())
        );
    }

    #[test]
    fn removing_an_unsaved_page_saves_nothing() {
        let mut library = bookmarked("a");

        let removed = library.remove_bookmark(&url("b"));

        assert_eq!(removed, Ok(None));
        assert_eq!(library.repo.saves, 1);
    }

    #[test]
    fn a_failed_removal_keeps_the_bookmark() {
        let mut library = bookmarked("a");
        fail_saves(&mut library);

        let result = library.remove_bookmark(&url("a"));

        assert_eq!(result, Err(store_failure()));
        assert!(library.bookmarks().get(&url("a")).is_some());
    }
}

mod tagging {
    use super::*;

    fn tags_of(library: &Subject, path: &str) -> Vec<Tag> {
        let bookmark = library.bookmarks().get(&url(path)).unwrap();
        bookmark.tags.iter().cloned().collect()
    }

    #[test]
    fn a_tag_is_added_and_saved() {
        let mut library = bookmarked("a");

        library.tag(&url("a"), tag("rust")).unwrap();

        assert_eq!(tags_of(&library, "a"), [tag("rust")]);
        assert_eq!(
            library.repo.saved.bookmarks,
            Some(library.bookmarks().snapshot())
        );
    }

    #[test]
    fn a_tag_is_taken_off_and_saved() {
        let mut library = bookmarked("a");
        library.tag(&url("a"), tag("rust")).unwrap();

        library.untag(&url("a"), &tag("rust")).unwrap();

        assert_eq!(tags_of(&library, "a"), []);
        assert_eq!(library.repo.saves, 3);
    }

    #[test]
    fn an_unsaved_page_cannot_be_tagged_or_untagged() {
        let mut library = started();

        let tagged = library.tag(&url("a"), tag("rust"));
        let untagged = library.untag(&url("a"), &tag("rust"));

        let not_saved = Err(AppError::NotSaved(NotSaved(url("a"))));
        assert_eq!((tagged, untagged), (not_saved.clone(), not_saved));
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn a_failed_save_keeps_the_tags() {
        let mut library = bookmarked("a");
        library.tag(&url("a"), tag("old")).unwrap();
        fail_saves(&mut library);

        let tagged = library.tag(&url("a"), tag("new"));
        let untagged = library.untag(&url("a"), &tag("old"));

        assert_eq!(
            (tagged, untagged),
            (Err(store_failure()), Err(store_failure()))
        );
        assert_eq!(tags_of(&library, "a"), [tag("old")]);
    }
}

mod shortcuts {
    use super::*;

    #[test]
    fn a_default_keyword_expands_its_terms() {
        let library = started();

        let search = library.expand_shortcut("gh", "quickie browser");

        assert_eq!(
            search.as_ref().map(Url::as_str),
            Some("https://github.com/search?q=quickie+browser")
        );
    }

    #[test]
    fn an_unknown_keyword_expands_to_nothing() {
        assert_eq!(started().expand_shortcut("zz", "x"), None);
    }

    #[test]
    fn setting_a_shortcut_returns_the_replaced_template_and_saves() {
        let mut library = started();
        let before = library.shortcuts().get("gh").cloned();

        let replaced = library.set_shortcut(keyword("gh"), template("https://gh.test/?q=%s"));

        assert_eq!(replaced, Ok(before));
        assert_eq!(
            library.repo.saved.shortcuts,
            Some(library.shortcuts().snapshot())
        );
        assert_eq!(
            library.expand_shortcut("gh", "x").map(String::from),
            Some("https://gh.test/?q=x".to_owned())
        );
    }

    #[test]
    fn removing_a_shortcut_returns_its_template_and_saves() {
        let mut library = started();

        let removed = library.remove_shortcut("gh");

        assert!(matches!(removed, Ok(Some(_))));
        assert_eq!(library.shortcuts().get("gh"), None);
        assert_eq!(
            library.repo.saved.shortcuts,
            Some(library.shortcuts().snapshot())
        );
    }

    #[test]
    fn removing_an_unknown_keyword_saves_nothing() {
        let mut library = started();

        let removed = library.remove_shortcut("zz");

        assert_eq!(removed, Ok(None));
        assert_eq!(library.repo.saves, 0);
    }

    #[test]
    fn a_failed_save_keeps_the_shortcuts() {
        let mut library = started();
        fail_saves(&mut library);

        let set = library.set_shortcut(keyword("x"), template("https://x.test/?q=%s"));
        let removed = library.remove_shortcut("gh");

        assert_eq!((set, removed), (Err(store_failure()), Err(store_failure())));
        assert_eq!(library.shortcuts(), &SearchShortcuts::defaults());
    }
}
