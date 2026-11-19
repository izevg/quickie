//! Integration tests against real SQLite, in memory or in a temporary file; this module holds
//! the schema's own.

mod library;
mod session;
mod settings;

use quickie_application::{
    AgentRepo, KeymapRepo, LibraryRepo, LibrarySnapshot, SessionRepo, StoreError,
};
use rusqlite::Connection;
use url::Url;

use crate::SqliteStore;
use crate::schema::{MIGRATIONS, VERSION};

fn url(path: &str) -> Url {
    Url::parse(&format!("https://example.test/{path}")).unwrap()
}

fn user_version(db: &Connection) -> i64 {
    db.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

#[test]
fn a_new_database_gets_the_whole_schema() {
    let store = SqliteStore::in_memory().unwrap();

    assert_eq!(user_version(&store.db), i64::try_from(VERSION).unwrap());
}

#[test]
fn a_database_from_a_newer_build_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "user_version", i64::try_from(VERSION + 1).unwrap())
        .unwrap();
    drop(db);

    let result = SqliteStore::open(&path);

    let expected = format!("schema {}", VERSION + 1);
    assert!(matches!(result, Err(StoreError(message)) if message.contains(&expected)));
}

#[test]
fn a_first_version_database_keeps_its_session_and_gains_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch(MIGRATIONS[0]).unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
    let session = session::busy();
    crate::session::save(&mut db, &session).unwrap();
    drop(db);

    let store = SqliteStore::open(&path).unwrap();

    assert_eq!(user_version(&store.db), i64::try_from(VERSION).unwrap());
    assert_eq!(store.load(), Ok(Some(session)));
    assert_eq!(store.load_library(), Ok(LibrarySnapshot::default()));
    assert_eq!(store.load_keybindings(), Ok(Vec::new()));
    assert_eq!(store.load_endpoint(), Ok(None));
}

#[test]
fn a_seventh_version_database_gains_an_empty_agent_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    let mut db = Connection::open(&path).unwrap();
    for migration in &MIGRATIONS[..7] {
        db.execute_batch(migration).unwrap();
    }
    db.pragma_update(None, "user_version", 7).unwrap();
    let session = session::busy();
    crate::session::save(&mut db, &session).unwrap();
    drop(db);

    let store = SqliteStore::open(&path).unwrap();

    assert_eq!(user_version(&store.db), 8);
    assert_eq!(store.load(), Ok(Some(session)));
    assert_eq!(store.load_endpoint(), Ok(None));
}
