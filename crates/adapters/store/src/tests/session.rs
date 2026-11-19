use quickie_application::{SessionRepo, StoreError};
use quickie_domain_browsing::{Focus, JarChoice, PinnedSlot, Session, SessionSnapshot, Timestamp};

use super::url;
use crate::SqliteStore;

/// Two windows, a throwaway pair, a titled tab, a pinned tab, one tab in the trash.
pub(super) fn busy() -> SessionSnapshot {
    let mut session = Session::new();
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
    session.snapshot()
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

    store.save(&snapshot).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded.as_ref(), Some(&snapshot));
    assert!(Session::restore(loaded.unwrap()).is_ok());
}

#[test]
fn a_save_replaces_the_previous_one() {
    let mut store = SqliteStore::in_memory().unwrap();
    store.save(&busy()).unwrap();
    let mut session = Session::new();
    let window = session.open_window();
    session
        .open(window, url("only"), JarChoice::Main, Focus::Foreground)
        .unwrap();

    store.save(&session.snapshot()).unwrap();

    assert_eq!(store.load(), Ok(Some(session.snapshot())));
}

#[test]
fn an_empty_session_still_counts_as_saved() {
    let mut store = SqliteStore::in_memory().unwrap();

    store.save(&Session::new().snapshot()).unwrap();

    assert_eq!(store.load(), Ok(Some(Session::new().snapshot())));
}

#[test]
fn the_session_survives_reopening_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    let snapshot = busy();
    SqliteStore::open(&path).unwrap().save(&snapshot).unwrap();

    let reopened = SqliteStore::open(&path).unwrap();

    assert_eq!(reopened.load(), Ok(Some(snapshot)));
}

#[test]
fn a_corrupt_url_is_an_error_not_a_panic() {
    let mut store = SqliteStore::in_memory().unwrap();
    store.save(&busy()).unwrap();
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
    store.save(&busy()).unwrap();
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
    store.save(&busy()).unwrap();
    store.db.execute("UPDATE slots SET slot = 12", []).unwrap();

    assert!(matches!(store.load(), Err(StoreError(message)) if message.contains("bad slot 12")));
}
