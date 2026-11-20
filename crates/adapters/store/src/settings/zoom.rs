//! The `site_zooms` table: one row per site the main jar zoomed away from 100 %.

use quickie_application::StoreError;
use quickie_domain_settings::{Site, SiteZoomsSnapshot};
use rusqlite::{Connection, params};

use crate::failed;
use crate::schema::columns::ordinal;

/// The saved zooms, in the order the sites were first zoomed. The use case checks the
/// percentages; a row that names no host is a store error.
pub(crate) fn load(db: &Connection) -> Result<SiteZoomsSnapshot, StoreError> {
    let rows: Vec<(String, u16)> = db
        .prepare("SELECT site, percent FROM site_zooms ORDER BY position")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(failed)?;
    let sites = rows
        .into_iter()
        .map(|(site, percent)| {
            Site::try_from(site.as_str())
                .map(|site| (site, percent))
                .map_err(|error| StoreError(error.to_string()))
        })
        .collect::<Result<_, _>>()?;
    Ok(SiteZoomsSnapshot { sites })
}

/// Replaces every row with the snapshot's, in one transaction.
pub(crate) fn save(db: &mut Connection, snapshot: &SiteZoomsSnapshot) -> Result<(), StoreError> {
    let tx = db.transaction().map_err(failed)?;
    tx.execute("DELETE FROM site_zooms", []).map_err(failed)?;
    {
        let mut insert = tx
            .prepare("INSERT INTO site_zooms (position, site, percent) VALUES (?1, ?2, ?3)")
            .map_err(failed)?;
        for (position, (site, percent)) in snapshot.sites.iter().enumerate() {
            insert
                .execute(params![ordinal(position)?, site.as_str(), percent])
                .map_err(failed)?;
        }
    }
    tx.commit().map_err(failed)
}
