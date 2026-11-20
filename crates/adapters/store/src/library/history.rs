//! The `history` table: one row per page, keyed by its URL, ordered by its last visit.

use quickie_application::StoreError;
use quickie_domain_library::{HistoryEntry, HistorySnapshot, Timestamp, VisitNumber};
use rusqlite::{Connection, Transaction, params};
use url::Url;

use crate::failed;
use crate::schema::columns::{from_sql, time_from_sql, time_to_sql, to_sql, url};

/// The saved history, least recently visited first, in one pass over the table.
pub(super) fn load(db: &Connection) -> Result<HistorySnapshot, StoreError> {
    let rows: Vec<(String, String, i64, i64, String)> = db
        .prepare(
            "SELECT url, title, visit_count, last_visit, recent_visits \
             FROM history ORDER BY last_visit",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect()
        })
        .map_err(failed)?;
    let entries = rows
        .into_iter()
        .map(|(raw_url, title, visit_count, last_visit, recent)| {
            Ok(HistoryEntry {
                url: url(&raw_url)?,
                title,
                visit_count: from_sql(visit_count)?,
                recent_visits: visits_from_sql(&recent)?,
                last_visit: VisitNumber::new(from_sql(last_visit)?),
            })
        })
        .collect::<Result<_, StoreError>>()?;
    Ok(HistorySnapshot { entries })
}

/// Replaces every row with the snapshot's.
pub(super) fn save(tx: &Transaction<'_>, snapshot: &HistorySnapshot) -> Result<(), StoreError> {
    tx.execute("DELETE FROM history", []).map_err(failed)?;
    snapshot
        .entries
        .iter()
        .try_for_each(|entry| upsert(tx, entry))
}

/// Writes one page's row over its saved one, or adds it.
pub(super) fn upsert(tx: &Transaction<'_>, entry: &HistoryEntry) -> Result<(), StoreError> {
    let visit_count = to_sql(entry.visit_count)?;
    let last_visit = to_sql(entry.last_visit.get())?;
    let recent_visits = visits_to_sql(&entry.recent_visits)?;
    tx.prepare_cached(
        "INSERT INTO history (url, title, visit_count, last_visit, recent_visits) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT (url) DO UPDATE SET title = excluded.title, \
         visit_count = excluded.visit_count, last_visit = excluded.last_visit, \
         recent_visits = excluded.recent_visits",
    )
    .and_then(|mut stmt| {
        stmt.execute(params![
            entry.url.as_str(),
            entry.title,
            visit_count,
            last_visit,
            recent_visits,
        ])
    })
    .map(drop)
    .map_err(failed)
}

/// Removes these pages' rows.
pub(super) fn remove(tx: &Transaction<'_>, pages: &[Url]) -> Result<(), StoreError> {
    let mut stmt = tx
        .prepare_cached("DELETE FROM history WHERE url = ?1")
        .map_err(failed)?;
    pages
        .iter()
        .try_for_each(|page| stmt.execute([page.as_str()]).map(drop).map_err(failed))
}

/// Visit times as their row keeps them: milliseconds, comma-separated, oldest first.
fn visits_to_sql(visits: &[Timestamp]) -> Result<String, StoreError> {
    let millis: Vec<String> = visits
        .iter()
        .map(|&at| time_to_sql(at).map(|millis| millis.to_string()))
        .collect::<Result<_, _>>()?;
    Ok(millis.join(","))
}

/// The way back from [`visits_to_sql`].
fn visits_from_sql(raw: &str) -> Result<Vec<Timestamp>, StoreError> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    raw.split(',')
        .map(|millis| {
            let millis = millis
                .parse()
                .map_err(|_| StoreError(format!("bad visit time {millis:?}")))?;
            time_from_sql(millis)
        })
        .collect()
}
