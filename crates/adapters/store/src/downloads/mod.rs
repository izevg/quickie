//! The `downloads` and `download_counter` tables: rows ↔ a [`DownloadsSnapshot`]. A part of
//! the library, saved and recorded as saved like the others (`crate::library`); it lives beside
//! them only to keep that folder at four files.

#[cfg(test)]
mod tests;

use quickie_application::StoreError;
use quickie_domain_library::{Download, DownloadId, DownloadState, DownloadsSnapshot};
use rusqlite::{Connection, Transaction, params};

use crate::failed;
use crate::schema::columns::{from_sql, ordinal, time_from_sql, time_to_sql, to_sql, url};

const IN_PROGRESS: &str = "in_progress";
const FINISHED: &str = "finished";
const FAILED: &str = "failed";
const CANCELLED: &str = "cancelled";

/// One row as read: id, URL, file name, state, received, total, reason, started at.
type Row = (
    i64,
    String,
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<String>,
    i64,
);

/// The saved downloads, in the order they started, and the id counter.
pub(crate) fn load(db: &Connection) -> Result<DownloadsSnapshot, StoreError> {
    let rows: Vec<Row> = db
        .prepare(
            "SELECT id, url, file_name, state, received, total, reason, started_at
             FROM downloads ORDER BY position",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            })?
            .collect()
        })
        .map_err(failed)?;
    let downloads = rows
        .into_iter()
        .map(download)
        .collect::<Result<_, StoreError>>()?;
    let next_id: Option<i64> = db
        .query_row("SELECT next_id FROM download_counter", [], |row| row.get(0))
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(failed)?;
    Ok(DownloadsSnapshot {
        downloads,
        next_id: next_id.map(from_sql).transpose()?.unwrap_or_default(),
    })
}

/// Replaces every row and the counter with the snapshot's.
pub(crate) fn save(tx: &Transaction<'_>, snapshot: &DownloadsSnapshot) -> Result<(), StoreError> {
    tx.execute_batch("DELETE FROM downloads; DELETE FROM download_counter;")
        .map_err(failed)?;
    let mut insert = tx
        .prepare(
            "INSERT INTO downloads
             (position, id, url, file_name, state, received, total, reason, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(failed)?;
    for (position, download) in snapshot.downloads.iter().enumerate() {
        let (state, received, total, reason) = match &download.state {
            DownloadState::InProgress { received, total } => (
                IN_PROGRESS,
                Some(to_sql(*received)?),
                total.map(to_sql).transpose()?,
                None,
            ),
            DownloadState::Finished => (FINISHED, None, None, None),
            DownloadState::Failed(reason) => (FAILED, None, None, Some(reason.as_str())),
            DownloadState::Cancelled => (CANCELLED, None, None, None),
        };
        insert
            .execute(params![
                ordinal(position)?,
                to_sql(download.id.get())?,
                download.url.as_str(),
                download.file_name,
                state,
                received,
                total,
                reason,
                time_to_sql(download.started_at)?,
            ])
            .map_err(failed)?;
    }
    tx.execute(
        "INSERT INTO download_counter (next_id) VALUES (?1)",
        [to_sql(snapshot.next_id)?],
    )
    .map_err(failed)?;
    Ok(())
}

fn download(row: Row) -> Result<Download, StoreError> {
    let (id, raw_url, file_name, state, received, total, reason, started_at) = row;
    let state = match (state.as_str(), received, reason) {
        (IN_PROGRESS, Some(received), None) => DownloadState::InProgress {
            received: from_sql(received)?,
            total: total.map(from_sql).transpose()?,
        },
        (FINISHED, None, None) => DownloadState::Finished,
        (FAILED, None, Some(reason)) => DownloadState::Failed(reason),
        (CANCELLED, None, None) => DownloadState::Cancelled,
        (other, ..) => return Err(StoreError(format!("bad download state {other:?}"))),
    };
    Ok(Download {
        id: DownloadId::new(from_sql(id)?),
        url: url(&raw_url)?,
        file_name,
        state,
        started_at: time_from_sql(started_at)?,
    })
}
