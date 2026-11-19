//! The `site_policies` table: one row per site the user changed.

use quickie_application::StoreError;
use quickie_domain_shields::{Site, SitePoliciesSnapshot, SitePolicy};
use rusqlite::{Connection, Transaction, params};

use crate::failed;
use crate::schema::columns::ordinal;

/// The saved exceptions, in the order they were first set.
pub(super) fn load(db: &Connection) -> Result<SitePoliciesSnapshot, StoreError> {
    let rows: Vec<(String, SitePolicy)> = db
        .prepare(
            "SELECT site, shields, https_only, javascript FROM site_policies ORDER BY position",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((
                    row.get(0)?,
                    SitePolicy {
                        shields: row.get(1)?,
                        https_only: row.get(2)?,
                        javascript: row.get(3)?,
                    },
                ))
            })?
            .collect()
        })
        .map_err(failed)?;
    let sites = rows
        .into_iter()
        .map(|(site, policy)| {
            Site::try_from(site.as_str())
                .map(|site| (site, policy))
                .map_err(|error| StoreError(error.to_string()))
        })
        .collect::<Result<_, _>>()?;
    Ok(SitePoliciesSnapshot { sites })
}

/// Replaces every row with the snapshot's.
pub(super) fn save(
    tx: &Transaction<'_>,
    snapshot: &SitePoliciesSnapshot,
) -> Result<(), StoreError> {
    tx.execute("DELETE FROM site_policies", [])
        .map_err(failed)?;
    let mut insert = tx
        .prepare(
            "INSERT INTO site_policies (position, site, shields, https_only, javascript)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(failed)?;
    for (position, (site, policy)) in snapshot.sites.iter().enumerate() {
        insert
            .execute(params![
                ordinal(position)?,
                site.as_str(),
                policy.shields,
                policy.https_only,
                policy.javascript
            ])
            .map_err(failed)?;
    }
    Ok(())
}
