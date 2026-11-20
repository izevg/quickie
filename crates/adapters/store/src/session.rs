//! The session tables: rows ↔ [`SessionSnapshot`]. Validation is the domain's job
//! (`Session::restore`); this only maps columns and rejects values that cannot be ids or URLs.
//!
//! The tables hold the main profile's session and nothing else: only it has a snapshot, so the
//! agent's tabs never reach them. A stored id is therefore a serial of [`Profile::Main`], and
//! needs no profile column; rows written before ids named their profile read back unchanged.

use std::sync::Arc;

use quickie_application::StoreError;
use quickie_domain_browsing::{
    ClosedTab, Jar, JarId, NextIds, PinnedSlot, Profile, SessionSnapshot, Tab, TabId, Timestamp,
    WindowId,
};
use rusqlite::{Connection, Row, params};

use crate::failed;
use crate::schema::columns::{from_sql, ordinal, to_sql, url};

fn tab_id(raw: i64) -> Result<TabId, StoreError> {
    Ok(TabId::new(Profile::Main, from_sql(raw)?))
}

fn window_id(raw: i64) -> Result<WindowId, StoreError> {
    Ok(WindowId::new(Profile::Main, from_sql(raw)?))
}

fn jar_to_sql(jar: Jar) -> Result<Option<i64>, StoreError> {
    match jar {
        Jar::Main => Ok(None),
        Jar::Throwaway(id) => to_sql(id.get()).map(Some),
    }
}

fn jar_from_sql(raw: Option<i64>) -> Result<Jar, StoreError> {
    match raw {
        None => Ok(Jar::Main),
        Some(id) => Ok(Jar::Throwaway(JarId::new(from_sql(id)?))),
    }
}

/// Raw columns of a tab row: id, url, title, jar.
type TabRow = (i64, String, String, Option<i64>);

fn tab_row(row: &Row<'_>) -> rusqlite::Result<TabRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn tab((id, raw_url, title, jar): TabRow) -> Result<Tab, StoreError> {
    Ok(Tab {
        id: tab_id(id)?,
        url: url(&raw_url)?,
        title,
        jar: jar_from_sql(jar)?,
    })
}

/// The saved session; `None` before the first save.
pub(crate) fn load(db: &Connection) -> Result<Option<SessionSnapshot>, StoreError> {
    let mut meta = db.prepare("SELECT key, value FROM meta").map_err(failed)?;
    let mut next = None::<NextIds>;
    let rows = meta
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(failed)?;
    for row in rows {
        let (key, value) = row.map_err(failed)?;
        let ids = next.get_or_insert_with(NextIds::default);
        let value = from_sql(value)?;
        match key.as_str() {
            "next_tab" => ids.tab = value,
            "next_window" => ids.window = value,
            "next_jar" => ids.jar = value,
            _ => {}
        }
    }
    let Some(next) = next else { return Ok(None) };

    let tabs = db
        .prepare("SELECT id, url, title, jar FROM tabs ORDER BY position")
        .and_then(|mut stmt| stmt.query_map([], tab_row)?.collect::<Result<Vec<_>, _>>())
        .map_err(failed)?
        .into_iter()
        .map(|row| tab(row).map(Arc::new))
        .collect::<Result<Vec<_>, _>>()?;

    let mut windows = Vec::new();
    let ids: Vec<i64> = db
        .prepare("SELECT id FROM windows ORDER BY position")
        .and_then(|mut stmt| stmt.query_map([], |row| row.get(0))?.collect())
        .map_err(failed)?;
    for id in ids {
        let order: Vec<i64> = db
            .prepare_cached("SELECT tab FROM window_tabs WHERE window = ?1 ORDER BY rank")
            .and_then(|mut stmt| stmt.query_map([id], |row| row.get(0))?.collect())
            .map_err(failed)?;
        let order = order.into_iter().map(tab_id).collect::<Result<_, _>>()?;
        windows.push((window_id(id)?, order));
    }

    let mut slots = [None; PinnedSlot::COUNT];
    let held: Vec<(i64, i64)> = db
        .prepare("SELECT slot, tab FROM slots")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(failed)?;
    for (slot, id) in held {
        let slot = u8::try_from(slot)
            .ok()
            .and_then(|n| PinnedSlot::try_from(n).ok())
            .ok_or_else(|| StoreError(format!("bad slot {slot}")))?;
        slots[slot.index()] = Some(tab_id(id)?);
    }

    let trash = db
        .prepare("SELECT id, url, title, jar, closed_at FROM trash ORDER BY position")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((tab_row(row)?, row.get::<_, i64>(4)?)))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(failed)?
        .into_iter()
        .map(|(row, closed_at)| {
            Ok(Arc::new(ClosedTab {
                tab: tab(row)?,
                closed_at: Timestamp::from_millis(from_sql(closed_at)?),
            }))
        })
        .collect::<Result<Vec<_>, StoreError>>()?;

    Ok(Some(SessionSnapshot {
        tabs,
        windows,
        slots,
        trash,
        next,
    }))
}

/// Replaces the saved session in one transaction. It runs after every burst of commands, so
/// its statements stay prepared on the connection between saves.
pub(crate) fn save(db: &mut Connection, snapshot: &SessionSnapshot) -> Result<(), StoreError> {
    let tx = db.transaction().map_err(failed)?;
    tx.execute_batch(
        "DELETE FROM window_tabs; DELETE FROM windows; DELETE FROM tabs;
         DELETE FROM slots; DELETE FROM trash; DELETE FROM meta;",
    )
    .map_err(failed)?;
    {
        let mut meta = tx
            .prepare_cached("INSERT INTO meta (key, value) VALUES (?1, ?2)")
            .map_err(failed)?;
        for (key, value) in [
            ("next_tab", snapshot.next.tab),
            ("next_window", snapshot.next.window),
            ("next_jar", snapshot.next.jar),
        ] {
            meta.execute(params![key, to_sql(value)?]).map_err(failed)?;
        }
        let mut tabs = tx
            .prepare_cached(
                "INSERT INTO tabs (id, position, url, title, jar) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(failed)?;
        for (position, tab) in snapshot.tabs.iter().enumerate() {
            tabs.execute(params![
                to_sql(tab.id.get())?,
                ordinal(position)?,
                tab.url.as_str(),
                tab.title,
                jar_to_sql(tab.jar)?
            ])
            .map_err(failed)?;
        }
        let mut windows = tx
            .prepare_cached("INSERT INTO windows (id, position) VALUES (?1, ?2)")
            .map_err(failed)?;
        let mut order = tx
            .prepare_cached("INSERT INTO window_tabs (window, rank, tab) VALUES (?1, ?2, ?3)")
            .map_err(failed)?;
        for (position, (window, tabs)) in snapshot.windows.iter().enumerate() {
            let window = to_sql(window.get())?;
            windows
                .execute(params![window, ordinal(position)?])
                .map_err(failed)?;
            for (rank, tab) in tabs.iter().enumerate() {
                order
                    .execute(params![window, ordinal(rank)?, to_sql(tab.get())?])
                    .map_err(failed)?;
            }
        }
        let mut slots = tx
            .prepare_cached("INSERT INTO slots (slot, tab) VALUES (?1, ?2)")
            .map_err(failed)?;
        for (index, held) in snapshot.slots.iter().enumerate() {
            if let Some(tab) = held {
                slots
                    .execute(params![ordinal(index + 1)?, to_sql(tab.get())?])
                    .map_err(failed)?;
            }
        }
        let mut trash = tx
            .prepare_cached(
                "INSERT INTO trash (position, id, url, title, jar, closed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(failed)?;
        for (position, closed) in snapshot.trash.iter().enumerate() {
            trash
                .execute(params![
                    ordinal(position)?,
                    to_sql(closed.tab.id.get())?,
                    closed.tab.url.as_str(),
                    closed.tab.title,
                    jar_to_sql(closed.tab.jar)?,
                    to_sql(closed.closed_at.as_millis())?
                ])
                .map_err(failed)?;
        }
    }
    tx.commit().map_err(failed)
}
