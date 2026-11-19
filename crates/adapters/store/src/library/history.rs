//! The `history` and `history_visits` tables.

use std::collections::HashMap;

use quickie_application::StoreError;
use quickie_domain_library::{HistoryEntry, HistorySnapshot, Timestamp};
use rusqlite::{Connection, Transaction, params};

use crate::failed;
use crate::schema::columns::{from_sql, ordinal, time_from_sql, time_to_sql, to_sql, url};

/// The saved history, least recently visited first. Visit times come in one query, not one per
/// entry, so a long history loads in a single pass.
pub(super) fn load(db: &Connection) -> Result<HistorySnapshot, StoreError> {
    let mut visits: HashMap<i64, Vec<Timestamp>> = HashMap::new();
    let rows: Vec<(i64, i64)> = db
        .prepare("SELECT entry, visited_at FROM history_visits ORDER BY entry, rank")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(failed)?;
    for (entry, visited_at) in rows {
        visits
            .entry(entry)
            .or_default()
            .push(time_from_sql(visited_at)?);
    }
    let rows: Vec<(i64, String, String, i64)> = db
        .prepare("SELECT position, url, title, visit_count FROM history ORDER BY position")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect()
        })
        .map_err(failed)?;
    let entries = rows
        .into_iter()
        .map(|(position, raw_url, title, visit_count)| {
            Ok(HistoryEntry {
                url: url(&raw_url)?,
                title,
                visit_count: from_sql(visit_count)?,
                recent_visits: visits.remove(&position).unwrap_or_default(),
            })
        })
        .collect::<Result<_, StoreError>>()?;
    Ok(HistorySnapshot { entries })
}

/// Replaces every row with the snapshot's.
pub(super) fn save(tx: &Transaction<'_>, snapshot: &HistorySnapshot) -> Result<(), StoreError> {
    tx.execute_batch("DELETE FROM history_visits; DELETE FROM history;")
        .map_err(failed)?;
    let mut entries = tx
        .prepare("INSERT INTO history (position, url, title, visit_count) VALUES (?1, ?2, ?3, ?4)")
        .map_err(failed)?;
    let mut visits = tx
        .prepare("INSERT INTO history_visits (entry, rank, visited_at) VALUES (?1, ?2, ?3)")
        .map_err(failed)?;
    for (position, entry) in snapshot.entries.iter().enumerate() {
        let position = ordinal(position)?;
        entries
            .execute(params![
                position,
                entry.url.as_str(),
                entry.title,
                to_sql(entry.visit_count)?
            ])
            .map_err(failed)?;
        for (rank, &visited_at) in entry.recent_visits.iter().enumerate() {
            visits
                .execute(params![position, ordinal(rank)?, time_to_sql(visited_at)?])
                .map_err(failed)?;
        }
    }
    Ok(())
}
