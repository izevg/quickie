//! Integration tests against real SQLite, in memory or in a temporary file; this module holds
//! the schema's own.

mod library;
mod session;
mod settings;

use quickie_application::{
    AgentRepo, KeymapRepo, LibraryRepo, LibrarySnapshot, SessionRepo, StoreError, ZoomRepo,
};
use quickie_domain_settings::SiteZoomsSnapshot;
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

/// A database at schema 9, the last before history went row by row, holding three pages the
/// way that schema saved them: by position, least recently visited first, each visit time in a
/// row of its own. The middle page keeps all ten times; their rows are not in rank order.
fn ninth_version_history(path: &std::path::Path) {
    let mut db = Connection::open(path).unwrap();
    for migration in &MIGRATIONS[..9] {
        db.execute_batch(migration).unwrap();
    }
    db.pragma_update(None, "user_version", 9).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch(
        "INSERT INTO library_saved (part) VALUES ('history');
         INSERT INTO history (position, url, title, visit_count) VALUES
             (0, 'https://example.test/old', 'Old', 1),
             (1, 'https://example.test/often', 'Привет, «мир»', 14),
             (2, 'https://example.test/new?q=1', '', 2);
         INSERT INTO history_visits (entry, rank, visited_at) VALUES
             (0, 0, 1000),
             (2, 1, 9000), (2, 0, 8000);",
    )
    .unwrap();
    for rank in (0..10).rev() {
        tx.execute(
            "INSERT INTO history_visits (entry, rank, visited_at) VALUES (1, ?1, ?2)",
            [rank, 2_000 + rank],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn a_ninth_version_history_keeps_its_pages_order_and_visits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    ninth_version_history(&path);

    let store = SqliteStore::open(&path).unwrap();

    let history = store.load_library().unwrap().history.unwrap();
    let rows: Vec<(String, &str, u64, Vec<u64>, u64)> = history
        .entries
        .iter()
        .map(|entry| {
            (
                entry.url.to_string(),
                entry.title.as_str(),
                entry.visit_count,
                entry
                    .recent_visits
                    .iter()
                    .map(|at| at.as_millis())
                    .collect(),
                entry.last_visit.get(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                "https://example.test/old".to_owned(),
                "Old",
                1,
                vec![1_000],
                1
            ),
            (
                "https://example.test/often".to_owned(),
                "Привет, «мир»",
                14,
                (2_000..2_010).collect(),
                2
            ),
            (
                "https://example.test/new?q=1".to_owned(),
                "",
                2,
                vec![8_000, 9_000],
                3
            ),
        ]
    );
    assert!(quickie_domain_library::History::restore(history).is_ok());
}

#[test]
fn a_migrated_history_takes_new_visits_after_its_last() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    ninth_version_history(&path);
    let store = SqliteStore::open(&path).unwrap();
    let mut library = quickie_application::Library::start(store, library::Fixed(10_000)).unwrap();

    library
        .record_visit(&url("old"), "", quickie_domain_browsing::Jar::Main)
        .unwrap();
    drop(library);

    let reopened = SqliteStore::open(&path).unwrap();
    let history = reopened.load_library().unwrap().history.unwrap();
    let order: Vec<(String, u64)> = history
        .entries
        .iter()
        .map(|entry| (entry.url.path().to_owned(), entry.last_visit.get()))
        .collect();
    assert_eq!(
        order,
        [
            ("/often".to_owned(), 2),
            ("/new".to_owned(), 3),
            ("/old".to_owned(), 4)
        ]
    );
}

#[test]
fn the_migrated_history_is_one_table_with_an_index_on_the_last_visit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shell.sqlite");
    ninth_version_history(&path);

    let store = SqliteStore::open(&path).unwrap();

    let objects: Vec<(String, String)> = store
        .db
        .prepare(
            "SELECT type, name FROM sqlite_master \
             WHERE tbl_name LIKE 'history%' ORDER BY type, name",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let named = |kind: &str, name: &str| (kind.to_owned(), name.to_owned());
    assert_eq!(
        objects,
        [
            named("index", "history_by_last_visit"),
            named("table", "history"),
        ]
    );
}

#[test]
fn a_seventh_version_database_gains_an_empty_agent_endpoint_and_no_site_zooms() {
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

    assert_eq!(user_version(&store.db), i64::try_from(VERSION).unwrap());
    assert_eq!(store.load(), Ok(Some(session)));
    assert_eq!(store.load_endpoint(), Ok(None));
    assert_eq!(store.load_site_zooms(), Ok(SiteZoomsSnapshot::default()));
}
