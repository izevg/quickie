use std::sync::Arc;

use quickie_application::{SessionRepo, StoreError};
use quickie_domain_browsing::{
    ClosedTab, Focus, Jar, JarChoice, JarId, NextIds, PinnedSlot, Profile, Session,
    SessionSnapshot, Tab, TabId, Timestamp, WindowId,
};
use rusqlite::Connection;

use super::url;
use crate::SqliteStore;
use crate::schema::MIGRATIONS;

/// Two windows, a throwaway pair, a titled tab, a pinned tab, one tab in the trash.
pub(super) fn busy() -> SessionSnapshot {
    let mut session = Session::new(Profile::Main);
    let first = session.open_window();
    let second = session.open_window();
    let a = session
        .open(first, url("a"), JarChoice::Main, Focus::Foreground)
        .unwrap();
    let b = session
        .open(first, url("b"), JarChoice::NewThrowaway, Focus::Background)
        .unwrap();
    session
        .open(second, url("c"), JarChoice::SameAs(b), Focus::Foreground)
        .unwrap();
    let d = session
        .open(
            second,
            url("d?q=1#x"),
            JarChoice::NewThrowaway,
            Focus::Foreground,
        )
        .unwrap();
    session
        .navigated(a, url("a2"), "Привет, «мир»".to_owned())
        .unwrap();
    session.pin(a, PinnedSlot::try_from(9).unwrap()).unwrap();
    session
        .close(d, Timestamp::from_millis(1_700_000_000_000))
        .unwrap();
    session.snapshot().unwrap()
}

#[test]
fn a_new_store_has_no_session() {
    let store = SqliteStore::in_memory().unwrap();

    assert_eq!(store.load(), Ok(None));
}

#[test]
fn a_saved_session_loads_back_equal_and_restorable() {
    let mut store = SqliteStore::in_memory().unwrap();
    let snapshot = busy();

    store.save(snapshot.clone()).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded.as_ref(), Some(&snapshot));
    assert!(Session::restore(loaded.unwrap()).is_ok());
}

#[test]
fn a_save_replaces_the_previous_one() {
    let mut store = SqliteStore::in_memory().unwrap();
    store.save(busy()).unwrap();
    let mut session = Session::new(Profile::Main);
    let window = session.open_window();
    session
        .open(window, url("only"), JarChoice::Main, Focus::Foreground)
        .unwrap();

    store.save(session.snapshot().unwrap()).unwrap();

    assert_eq!(store.load(), Ok(Some(session.snapshot().unwrap())));
}

#[test]
fn an_empty_session_still_counts_as_saved() {
    let mut store = SqliteStore::in_memory().unwrap();

    store
        .save(Session::new(Profile::Main).snapshot().unwrap())
        .unwrap();

    assert_eq!(
        store.load(),
        Ok(Some(Session::new(Profile::Main).snapshot().unwrap()))
    );
}

#[test]
fn the_session_survives_reopening_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    let snapshot = busy();
    SqliteStore::open(&path)
        .unwrap()
        .save(snapshot.clone())
        .unwrap();

    let reopened = SqliteStore::open(&path).unwrap();

    assert_eq!(reopened.load(), Ok(Some(snapshot)));
}

#[test]
fn a_corrupt_url_is_an_error_not_a_panic() {
    let mut store = SqliteStore::in_memory().unwrap();
    store.save(busy()).unwrap();
    store
        .db
        .execute("UPDATE tabs SET url = 'not a url' WHERE id = 0", [])
        .unwrap();

    let result = store.load();

    assert!(matches!(result, Err(StoreError(message)) if message.contains("bad URL")));
}

#[test]
fn a_negative_id_is_an_error() {
    let mut store = SqliteStore::in_memory().unwrap();
    store.save(busy()).unwrap();
    store
        .db
        .execute("UPDATE meta SET value = -1 WHERE key = 'next_tab'", [])
        .unwrap();

    assert!(
        matches!(store.load(), Err(StoreError(message)) if message.contains("negative number -1"))
    );
}

#[test]
fn a_bad_slot_is_an_error() {
    let mut store = SqliteStore::in_memory().unwrap();
    store.save(busy()).unwrap();
    store.db.execute("UPDATE slots SET slot = 12", []).unwrap();

    assert!(matches!(store.load(), Err(StoreError(message)) if message.contains("bad slot 12")));
}

/// Schema 9 is the last one written while ids were bare numbers. These rows are what that build
/// saved for one window with an active throwaway tab, a pinned tab, and a closed tab, written as
/// literal SQL so they do not depend on today's `save`.
const SAVED_BEFORE_PROFILES: &str = "
    INSERT INTO meta (key, value) VALUES ('next_tab', 3), ('next_window', 1), ('next_jar', 1);
    INSERT INTO tabs (id, position, url, title, jar) VALUES
        (0, 0, 'https://example.test/a', 'A', NULL),
        (2, 1, 'https://example.test/c', '', 0);
    INSERT INTO windows (id, position) VALUES (0, 0);
    INSERT INTO window_tabs (window, rank, tab) VALUES (0, 0, 2), (0, 1, 0);
    INSERT INTO slots (slot, tab) VALUES (3, 0);
    INSERT INTO trash (position, id, url, title, jar, closed_at) VALUES
        (0, 1, 'https://example.test/b', 'B', NULL, 1700000000000);
";

#[test]
fn a_session_saved_before_ids_named_their_profile_comes_back_as_the_main_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    let db = Connection::open(&path).unwrap();
    for migration in &MIGRATIONS[..9] {
        db.execute_batch(migration).unwrap();
    }
    db.pragma_update(None, "user_version", 9).unwrap();
    db.execute_batch(SAVED_BEFORE_PROFILES).unwrap();
    drop(db);

    let loaded = SqliteStore::open(&path).unwrap().load().unwrap().unwrap();

    let tab = |serial| TabId::new(Profile::Main, serial);
    let page = |id, path: &str, title: &str, jar| Tab {
        id: tab(id),
        url: url(path),
        title: title.to_owned(),
        jar,
    };
    let mut slots = [None; PinnedSlot::COUNT];
    slots[2] = Some(tab(0));
    let expected = SessionSnapshot {
        tabs: vec![
            Arc::new(page(0, "a", "A", Jar::Main)),
            Arc::new(page(2, "c", "", Jar::Throwaway(JarId::new(0)))),
        ],
        windows: vec![(WindowId::new(Profile::Main, 0), vec![tab(2), tab(0)])],
        slots,
        trash: vec![Arc::new(ClosedTab {
            tab: page(1, "b", "B", Jar::Main),
            closed_at: Timestamp::from_millis(1_700_000_000_000),
        })],
        next: NextIds {
            tab: 3,
            window: 1,
            jar: 1,
        },
    };
    assert_eq!(loaded, expected);
    assert_eq!(
        Session::restore(loaded).map(|session| session.profile()),
        Ok(Profile::Main)
    );
}
