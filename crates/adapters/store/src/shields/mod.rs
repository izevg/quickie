//! The shields tables: rows ↔ a [`ShieldsSnapshot`]. Each part saves in its own transaction.
//! Tables never written load as a fresh profile's shields: no exceptions, every list off.

mod lists;
mod sites;
#[cfg(test)]
mod tests;

use quickie_application::{ShieldsSnapshot, StoreError};
use quickie_domain_shields::{FilterListId, FilterListSetSnapshot, SitePoliciesSnapshot};
use rusqlite::{Connection, Transaction, params};

use crate::failed;

/// Everything saved of the shields.
pub(crate) fn load(db: &Connection) -> Result<ShieldsSnapshot, StoreError> {
    Ok(ShieldsSnapshot {
        sites: sites::load(db)?,
        lists: lists::load(db)?,
        bodies: lists::load_bodies(db)?,
    })
}

/// Replaces the saved per-site exceptions in one transaction.
pub(crate) fn save_sites(
    db: &mut Connection,
    snapshot: &SitePoliciesSnapshot,
) -> Result<(), StoreError> {
    save(db, |tx| sites::save(tx, snapshot))
}

/// Replaces the saved filter list states in one transaction.
pub(crate) fn save_lists(
    db: &mut Connection,
    snapshot: &FilterListSetSnapshot,
) -> Result<(), StoreError> {
    save(db, |tx| lists::save(tx, snapshot))
}

/// Replaces one list's saved content.
pub(crate) fn save_body(
    db: &Connection,
    list: FilterListId,
    body: &[u8],
) -> Result<(), StoreError> {
    db.execute(
        "INSERT OR REPLACE INTO filter_list_bodies (id, body) VALUES (?1, ?2)",
        params![list.as_str(), body],
    )
    .map(drop)
    .map_err(failed)
}

/// Runs `write` in a transaction.
fn save(
    db: &mut Connection,
    write: impl FnOnce(&Transaction<'_>) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    let tx = db.transaction().map_err(failed)?;
    write(&tx)?;
    tx.commit().map_err(failed)
}

/// A list id column.
fn list_id(raw: &str) -> Result<FilterListId, StoreError> {
    FilterListId::try_from(raw).map_err(|error| StoreError(error.to_string()))
}
