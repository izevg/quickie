//! The `filter_lists`, `filter_refresh` and `filter_list_bodies` tables.

use quickie_application::StoreError;
use quickie_domain_shields::{
    ContentHash, Fetch, FilterList, FilterListId, FilterListSetSnapshot, Refresh,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::list_id;
use crate::failed;
use crate::schema::columns::{time_from_sql, time_to_sql};

/// One `filter_lists` row as SQLite holds it.
type Row = (String, bool, Option<i64>, Option<i64>);

/// The saved list states and when they are refreshed.
pub(super) fn load(db: &Connection) -> Result<FilterListSetSnapshot, StoreError> {
    let rows: Vec<Row> = db
        .prepare("SELECT id, enabled, fetched_at, content_hash FROM filter_lists ORDER BY rowid")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect()
        })
        .map_err(failed)?;
    let lists = rows
        .into_iter()
        .map(|(id, enabled, at, hash)| {
            let fetched = match (at, hash) {
                (Some(at), Some(hash)) => Some(Fetch {
                    at: time_from_sql(at)?,
                    hash: hash_from_sql(hash),
                }),
                _ => None,
            };
            Ok(FilterList {
                id: list_id(&id)?,
                enabled,
                fetched,
            })
        })
        .collect::<Result<_, StoreError>>()?;
    let keep_fresh: Option<bool> = db
        .query_row("SELECT keep_fresh FROM filter_refresh", [], |row| {
            row.get(0)
        })
        .optional()
        .map_err(failed)?;
    let refresh = if keep_fresh == Some(true) {
        Refresh::KeepFresh
    } else {
        Refresh::OnRequest
    };
    Ok(FilterListSetSnapshot { lists, refresh })
}

/// Replaces every list state, and the refresh setting, with the snapshot's.
pub(super) fn save(
    tx: &Transaction<'_>,
    snapshot: &FilterListSetSnapshot,
) -> Result<(), StoreError> {
    tx.execute_batch("DELETE FROM filter_lists; DELETE FROM filter_refresh;")
        .map_err(failed)?;
    let mut insert = tx
        .prepare(
            "INSERT INTO filter_lists (id, enabled, fetched_at, content_hash)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(failed)?;
    for list in &snapshot.lists {
        let at = list
            .fetched
            .map(|fetch| time_to_sql(fetch.at))
            .transpose()?;
        let hash = list.fetched.map(|fetch| hash_to_sql(fetch.hash));
        insert
            .execute(params![list.id.as_str(), list.enabled, at, hash])
            .map_err(failed)?;
    }
    tx.execute(
        "INSERT INTO filter_refresh (keep_fresh) VALUES (?1)",
        [snapshot.refresh == Refresh::KeepFresh],
    )
    .map_err(failed)?;
    Ok(())
}

/// Every list's saved content.
pub(super) fn load_bodies(db: &Connection) -> Result<Vec<(FilterListId, Vec<u8>)>, StoreError> {
    let rows: Vec<(String, Vec<u8>)> = db
        .prepare("SELECT id, body FROM filter_list_bodies ORDER BY id")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(failed)?;
    rows.into_iter()
        .map(|(id, body)| Ok((list_id(&id)?, body)))
        .collect()
}

/// A hash uses all 64 bits, more than SQLite's signed `INTEGER` holds as a positive number, so
/// it is stored bit for bit.
fn hash_to_sql(hash: ContentHash) -> i64 {
    i64::from_ne_bytes(hash.get().to_ne_bytes())
}

/// The way back from [`hash_to_sql`].
fn hash_from_sql(raw: i64) -> ContentHash {
    ContentHash::from_raw(u64::from_ne_bytes(raw.to_ne_bytes()))
}
