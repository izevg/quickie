//! The `bookmarks` and `bookmark_tags` tables.

use std::collections::{BTreeSet, HashMap};

use quickie_application::StoreError;
use quickie_domain_library::{Bookmark, BookmarksSnapshot, Tag};
use rusqlite::{Connection, Transaction, params};

use crate::failed;
use crate::schema::columns::{ordinal, time_from_sql, time_to_sql, url};

/// The saved bookmarks, in the order they were first saved, each with its tags.
pub(super) fn load(db: &Connection) -> Result<BookmarksSnapshot, StoreError> {
    let mut tags: HashMap<i64, BTreeSet<Tag>> = HashMap::new();
    let rows: Vec<(i64, String)> = db
        .prepare("SELECT bookmark, tag FROM bookmark_tags")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(failed)?;
    for (bookmark, raw) in rows {
        let tag = Tag::try_from(raw.as_str())
            .map_err(|error| StoreError(format!("bad tag {raw:?}: {error}")))?;
        tags.entry(bookmark).or_default().insert(tag);
    }
    let rows: Vec<(i64, String, String, i64)> = db
        .prepare("SELECT position, url, title, saved_at FROM bookmarks ORDER BY position")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect()
        })
        .map_err(failed)?;
    let bookmarks = rows
        .into_iter()
        .map(|(position, raw_url, title, saved_at)| {
            Ok(Bookmark {
                url: url(&raw_url)?,
                title,
                tags: tags.remove(&position).unwrap_or_default(),
                saved_at: time_from_sql(saved_at)?,
            })
        })
        .collect::<Result<_, StoreError>>()?;
    Ok(BookmarksSnapshot { bookmarks })
}

/// Replaces every row with the snapshot's.
pub(super) fn save(tx: &Transaction<'_>, snapshot: &BookmarksSnapshot) -> Result<(), StoreError> {
    tx.execute_batch("DELETE FROM bookmark_tags; DELETE FROM bookmarks;")
        .map_err(failed)?;
    let mut bookmarks = tx
        .prepare("INSERT INTO bookmarks (position, url, title, saved_at) VALUES (?1, ?2, ?3, ?4)")
        .map_err(failed)?;
    let mut tags = tx
        .prepare("INSERT INTO bookmark_tags (bookmark, tag) VALUES (?1, ?2)")
        .map_err(failed)?;
    for (position, bookmark) in snapshot.bookmarks.iter().enumerate() {
        let position = ordinal(position)?;
        bookmarks
            .execute(params![
                position,
                bookmark.url.as_str(),
                bookmark.title,
                time_to_sql(bookmark.saved_at)?
            ])
            .map_err(failed)?;
        for tag in &bookmark.tags {
            tags.execute(params![position, tag.as_str()])
                .map_err(failed)?;
        }
    }
    Ok(())
}
