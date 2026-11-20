use quickie_application::{Clock, Library, LibraryRepo, LibrarySnapshot, StoreError};
use quickie_domain_browsing::Jar;
use quickie_domain_library::{
    Bookmarks, History, Keyword, SearchShortcuts, Tag, Timestamp, UrlTemplate,
};

use super::url;
use crate::SqliteStore;

fn at(millis: u64) -> Timestamp {
    Timestamp::from_millis(millis)
}

/// Three pages: one visited twelve times (only ten times kept), one with a fragment and a
/// non-Latin title, one without a title.
fn history() -> History {
    let mut history = History::new();
    for n in 0..12 {
        history.record_visit(&url("often"), "Often", at(1_700_000_000_000 + n));
    }
    history.record_visit(&url("ru#part"), "Привет, «мир»", at(5));
    history.record_visit(&url("untitled?q=1"), "", at(6));
    history
}

/// Two bookmarks: one with two tags, one without any.
fn bookmarks() -> Bookmarks {
    let mut bookmarks = Bookmarks::new();
    bookmarks.save(&url("a"), "A".to_owned(), at(1));
    bookmarks.save(&url("b"), String::new(), at(2));
    bookmarks
        .tag(&url("a"), Tag::try_from("rust").unwrap())
        .unwrap();
    bookmarks.tag(&url("a"), Tag::later()).unwrap();
    bookmarks
}

fn saved(store: &mut SqliteStore) {
    store.save_history(&history().snapshot()).unwrap();
    store.save_bookmarks(&bookmarks().snapshot()).unwrap();
    store
        .save_shortcuts(&SearchShortcuts::defaults().snapshot())
        .unwrap();
}

#[test]
fn a_new_store_has_no_library() {
    let store = SqliteStore::in_memory().unwrap();

    assert_eq!(store.load_library(), Ok(LibrarySnapshot::default()));
}

#[test]
fn a_saved_library_loads_back_equal_and_restorable() {
    let mut store = SqliteStore::in_memory().unwrap();

    saved(&mut store);
    let loaded = store.load_library().unwrap();

    assert_eq!(
        loaded,
        LibrarySnapshot {
            history: Some(history().snapshot()),
            bookmarks: Some(bookmarks().snapshot()),
            shortcuts: Some(SearchShortcuts::defaults().snapshot()),
            downloads: None,
        }
    );
    assert_eq!(History::restore(loaded.history.unwrap()), Ok(history()));
    assert_eq!(
        Bookmarks::restore(loaded.bookmarks.unwrap()),
        Ok(bookmarks())
    );
}

#[test]
fn each_part_saves_on_its_own() {
    let mut store = SqliteStore::in_memory().unwrap();

    store.save_bookmarks(&bookmarks().snapshot()).unwrap();

    let loaded = store.load_library().unwrap();
    assert_eq!((loaded.history, loaded.shortcuts), (None, None));
    assert_eq!(loaded.bookmarks, Some(bookmarks().snapshot()));
}

#[test]
fn a_part_saved_empty_loads_as_empty_not_as_unsaved() {
    let mut store = SqliteStore::in_memory().unwrap();
    saved(&mut store);

    store
        .save_shortcuts(&SearchShortcuts::new().snapshot())
        .unwrap();
    store.save_history(&History::new().snapshot()).unwrap();
    store.save_bookmarks(&Bookmarks::new().snapshot()).unwrap();

    assert_eq!(
        store.load_library(),
        Ok(LibrarySnapshot {
            history: Some(History::new().snapshot()),
            bookmarks: Some(Bookmarks::new().snapshot()),
            shortcuts: Some(SearchShortcuts::new().snapshot()),
            downloads: None,
        })
    );
}

#[test]
fn a_save_replaces_the_previous_one() {
    let mut store = SqliteStore::in_memory().unwrap();
    saved(&mut store);
    let mut fewer = History::new();
    fewer.record_visit(&url("only"), "Only", at(9));
    let mut shortcuts = SearchShortcuts::new();
    shortcuts.set(
        Keyword::try_from("x").unwrap(),
        UrlTemplate::try_from("https://x.test/?q=%s").unwrap(),
    );

    store.save_history(&fewer.snapshot()).unwrap();
    store.save_shortcuts(&shortcuts.snapshot()).unwrap();

    let loaded = store.load_library().unwrap();
    assert_eq!(loaded.history, Some(fewer.snapshot()));
    assert_eq!(loaded.shortcuts, Some(shortcuts.snapshot()));
}

#[test]
fn the_library_survives_reopening_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    saved(&mut SqliteStore::open(&path).unwrap());

    let reopened = SqliteStore::open(&path).unwrap();

    assert_eq!(
        reopened.load_library().unwrap().history,
        Some(history().snapshot())
    );
}

fn corrupt(statement: &str) -> Result<LibrarySnapshot, StoreError> {
    let mut store = SqliteStore::in_memory().unwrap();
    saved(&mut store);
    store.db.execute(statement, []).unwrap();
    store.load_library()
}

fn fails_with(result: Result<LibrarySnapshot, StoreError>, words: &str) -> bool {
    matches!(result, Err(StoreError(message)) if message.contains(words))
}

#[test]
fn a_corrupt_history_url_is_an_error_not_a_panic() {
    let result = corrupt("UPDATE history SET url = 'not a url' WHERE title = 'Often'");

    assert!(fails_with(result, "bad URL"));
}

#[test]
fn a_negative_visit_time_is_an_error() {
    let result = corrupt("UPDATE history SET recent_visits = '5,-3'");

    assert!(fails_with(result, "negative number -3"));
}

#[test]
fn a_garbled_visit_time_is_an_error() {
    let result = corrupt("UPDATE history SET recent_visits = '5,x'");

    assert!(fails_with(result, "bad visit time \"x\""));
}

#[test]
fn a_negative_last_visit_is_an_error() {
    let result = corrupt("UPDATE history SET last_visit = -2 WHERE title = 'Often'");

    assert!(fails_with(result, "negative number -2"));
}

#[test]
fn a_negative_save_time_is_an_error() {
    let result = corrupt("UPDATE bookmarks SET saved_at = -4");

    assert!(fails_with(result, "negative number -4"));
}

#[test]
fn a_corrupt_bookmark_url_is_an_error() {
    let result = corrupt("UPDATE bookmarks SET url = 'nope' WHERE position = 0");

    assert!(fails_with(result, "bad URL"));
}

#[test]
fn a_corrupt_tag_is_an_error() {
    let result = corrupt("UPDATE bookmark_tags SET tag = 'Rust' WHERE tag = 'rust'");

    assert!(fails_with(result, "bad tag \"Rust\""));
}

#[test]
fn a_corrupt_keyword_is_an_error() {
    let result = corrupt("UPDATE search_shortcuts SET keyword = 'G H' WHERE keyword = 'gh'");

    assert!(fails_with(result, "bad keyword \"G H\""));
}

#[test]
fn a_corrupt_template_is_an_error() {
    let result =
        corrupt("UPDATE search_shortcuts SET template = 'ftp://x/%s' WHERE keyword = 'gh'");

    assert!(fails_with(result, "bad template \"ftp://x/%s\""));
}

#[test]
fn a_negative_visit_count_is_an_error() {
    let result = corrupt("UPDATE history SET visit_count = -1");

    assert!(fails_with(result, "negative number -1"));
}

#[test]
fn visits_outside_the_domain_rules_load_for_the_domain_to_judge() {
    let mut store = SqliteStore::in_memory().unwrap();
    saved(&mut store);
    store
        .db
        .execute("UPDATE history SET recent_visits = ''", [])
        .unwrap();

    let loaded = store.load_library().unwrap().history.unwrap();

    assert!(History::restore(loaded).is_err());
}

#[test]
fn an_entry_saves_over_its_page_alone() {
    let mut store = SqliteStore::in_memory().unwrap();
    saved(&mut store);
    let mut history = history();
    history.record_visit(&url("ru"), "Привет", at(7));

    store
        .upsert_entry(history.get(&url("ru")).unwrap())
        .unwrap();

    let loaded = store.load_library().unwrap().history.unwrap();
    assert_eq!(loaded, history.snapshot());
}

#[test]
fn an_entry_of_a_new_page_joins_a_history_never_saved() {
    let mut store = SqliteStore::in_memory().unwrap();
    let mut history = History::new();
    history.record_visit(&url("new"), "New", at(1));

    store
        .upsert_entry(history.get(&url("new")).unwrap())
        .unwrap();

    let loaded = store.load_library().unwrap().history;
    assert_eq!(loaded, Some(history.snapshot()));
}

#[test]
fn history_loads_by_last_visit_whatever_order_it_was_written_in() {
    let mut store = SqliteStore::in_memory().unwrap();
    let history = history();

    for entry in history.entries() {
        store.upsert_entry(entry).unwrap();
    }

    let loaded = store.load_library().unwrap().history;
    assert_eq!(loaded, Some(history.snapshot()));
}

#[test]
fn removing_entries_leaves_the_other_pages() {
    let mut store = SqliteStore::in_memory().unwrap();
    saved(&mut store);
    let mut history = history();
    history.remove(&url("often"));
    history.remove(&url("untitled?q=1"));

    store
        .remove_entries(&[url("often"), url("untitled?q=1"), url("never")])
        .unwrap();

    let loaded = store.load_library().unwrap().history;
    assert_eq!(loaded, Some(history.snapshot()));
}

#[test]
fn an_entry_that_does_not_fit_a_column_is_refused() {
    let mut store = SqliteStore::in_memory().unwrap();
    let mut history = History::new();
    history.record_visit(&url("a"), "A", at(1));
    let mut entry = history.get(&url("a")).unwrap().clone();
    entry.visit_count = u64::MAX;

    let result = store.upsert_entry(&entry);

    assert!(matches!(result, Err(StoreError(message)) if message.contains("does not fit")));
    assert_eq!(store.load_library().unwrap().history, None);
}

/// The clock of the test: a fixed time.
pub(super) struct Fixed(pub(super) u64);

impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        at(self.0)
    }
}

#[test]
fn the_library_use_cases_persist_through_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    let store = SqliteStore::open(&path).unwrap();
    let mut library = Library::start(store, Fixed(77)).unwrap();
    library.record_visit(&url("a"), "A", Jar::Main).unwrap();
    library.save_bookmark(&url("a"), "A".to_owned()).unwrap();
    library.remove_shortcut("gh").unwrap();
    drop(library);

    let reopened = Library::start(SqliteStore::open(&path).unwrap(), Fixed(78)).unwrap();

    let entry = reopened.history().get(&url("a")).unwrap();
    assert_eq!(entry.recent_visits, [at(77)]);
    assert!(reopened.bookmarks().get(&url("a")).is_some());
    assert_eq!(reopened.shortcuts().get("gh"), None);
    assert_eq!(reopened.corrupt_on_start(), []);
}
