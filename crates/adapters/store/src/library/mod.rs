//! The library tables: rows ↔ the parts of a [`LibrarySnapshot`]. Each part saves in its own
//! transaction, which also records that the part was saved, so a part saved empty loads back as
//! empty instead of as never saved.

mod bookmarks;
mod history;
mod shortcuts;

use std::collections::HashSet;

use quickie_application::{LibrarySnapshot, StoreError};
use quickie_domain_library::{BookmarksSnapshot, HistorySnapshot, SearchShortcutsSnapshot};
use rusqlite::{Connection, Transaction};

use crate::failed;

/// The name each part is recorded under in `library_saved`.
const HISTORY: &str = "history";
const BOOKMARKS: &str = "bookmarks";
const SHORTCUTS: &str = "shortcuts";

/// Every part of the library that was ever saved.
pub(crate) fn load(db: &Connection) -> Result<LibrarySnapshot, StoreError> {
    let saved: HashSet<String> = db
        .prepare("SELECT part FROM library_saved")
        .and_then(|mut stmt| stmt.query_map([], |row| row.get(0))?.collect())
        .map_err(failed)?;
    let was_saved = |part: &str| saved.contains(part);
    Ok(LibrarySnapshot {
        history: was_saved(HISTORY).then(|| history::load(db)).transpose()?,
        bookmarks: was_saved(BOOKMARKS)
            .then(|| bookmarks::load(db))
            .transpose()?,
        shortcuts: was_saved(SHORTCUTS)
            .then(|| shortcuts::load(db))
            .transpose()?,
    })
}

/// Replaces the saved history in one transaction.
pub(crate) fn save_history(
    db: &mut Connection,
    snapshot: &HistorySnapshot,
) -> Result<(), StoreError> {
    save(db, HISTORY, |tx| history::save(tx, snapshot))
}

/// Replaces the saved bookmarks in one transaction.
pub(crate) fn save_bookmarks(
    db: &mut Connection,
    snapshot: &BookmarksSnapshot,
) -> Result<(), StoreError> {
    save(db, BOOKMARKS, |tx| bookmarks::save(tx, snapshot))
}

/// Replaces the saved search shortcuts in one transaction.
pub(crate) fn save_shortcuts(
    db: &mut Connection,
    snapshot: &SearchShortcutsSnapshot,
) -> Result<(), StoreError> {
    save(db, SHORTCUTS, |tx| shortcuts::save(tx, snapshot))
}

/// Runs `write` and records `part` as saved, together or not at all.
fn save(
    db: &mut Connection,
    part: &str,
    write: impl FnOnce(&Transaction<'_>) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    let tx = db.transaction().map_err(failed)?;
    write(&tx)?;
    tx.execute(
        "INSERT OR IGNORE INTO library_saved (part) VALUES (?1)",
        [part],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)
}
