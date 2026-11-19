//! The schema and its migrations, numbered by `PRAGMA user_version`.

use quickie_application::StoreError;
use rusqlite::Connection;

use crate::failed;

/// Migration `n` takes the schema from version `n` to `n + 1`. Never edit a shipped one; add
/// the next.
const MIGRATIONS: &[&str] = &["
    CREATE TABLE meta (
        key TEXT PRIMARY KEY,
        value INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE tabs (
        id INTEGER PRIMARY KEY,
        position INTEGER NOT NULL,
        url TEXT NOT NULL,
        title TEXT NOT NULL,
        jar INTEGER
    ) STRICT;
    CREATE TABLE windows (
        id INTEGER PRIMARY KEY,
        position INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE window_tabs (
        window INTEGER NOT NULL REFERENCES windows(id) ON DELETE CASCADE,
        rank INTEGER NOT NULL,
        tab INTEGER NOT NULL,
        PRIMARY KEY (window, rank)
    ) STRICT;
    CREATE TABLE slots (
        slot INTEGER PRIMARY KEY,
        tab INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE trash (
        position INTEGER PRIMARY KEY,
        id INTEGER NOT NULL,
        url TEXT NOT NULL,
        title TEXT NOT NULL,
        jar INTEGER,
        closed_at INTEGER NOT NULL
    ) STRICT;
"];

/// The schema version this build writes.
pub(crate) const VERSION: usize = MIGRATIONS.len();

/// Applies the migrations the database has not seen yet, each in its own transaction.
pub(crate) fn migrate(db: &mut Connection) -> Result<(), StoreError> {
    let current: i64 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(failed)?;
    let current = usize::try_from(current)
        .ok()
        .filter(|&version| version <= VERSION)
        .ok_or_else(|| {
            StoreError(format!(
                "the database has schema {current}; this build knows {VERSION}"
            ))
        })?;
    for (version, migration) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = db.transaction().map_err(failed)?;
        tx.execute_batch(migration).map_err(failed)?;
        tx.pragma_update(None, "user_version", crate::session::ordinal(version + 1)?)
            .map_err(failed)?;
        tx.commit().map_err(failed)?;
    }
    Ok(())
}
