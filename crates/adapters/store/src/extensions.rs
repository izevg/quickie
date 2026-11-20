//! The extension tables: one `extensions` row per installed extension in install order, its
//! grants and its per-site decisions beside it, and whether updates come on their own in the
//! one row of `extension_updates`. The settings context judges every value on restore, so a
//! row that no longer makes sense is reported there, not lost here.

use quickie_application::StoreError;
use quickie_domain_settings::{
    ExtensionSnapshot, ExtensionSource, Grants, InstalledExtensionsSnapshot, Site, SiteAccess,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::failed;
use crate::schema::columns::{ordinal, time_from_sql, time_to_sql};

/// What a grant row holds.
const PERMISSION: &str = "permission";
const PATTERN: &str = "pattern";

/// The saved extensions; none on a profile that never saved them.
pub(crate) fn load(db: &Connection) -> Result<InstalledExtensionsSnapshot, StoreError> {
    let rows: Vec<(String, String, String, String, String, bool)> = db
        .prepare(
            "SELECT id, name, version, source, location, enabled FROM extensions \
             ORDER BY position",
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
                ))
            })?
            .collect()
        })
        .map_err(failed)?;
    let mut extensions = Vec::with_capacity(rows.len());
    for (id, name, version, source, location, enabled) in rows {
        let source = ExtensionSource::parse(&source)
            .ok_or_else(|| StoreError(format!("unknown extension source {source:?}")))?;
        extensions.push(ExtensionSnapshot {
            granted: grants(db, &id)?,
            sites: sites(db, &id)?,
            id,
            name,
            version,
            source,
            location,
            enabled,
        });
    }
    let updates: Option<(bool, Option<i64>)> = db
        .query_row(
            "SELECT auto_update, last_checked FROM extension_updates WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(failed)?;
    let (auto_update, last_checked) = updates.unwrap_or_default();
    Ok(InstalledExtensionsSnapshot {
        extensions,
        auto_update,
        last_checked: last_checked.map(time_from_sql).transpose()?,
    })
}

fn grants(db: &Connection, id: &str) -> Result<Grants, StoreError> {
    let rows: Vec<(String, String)> = db
        .prepare_cached("SELECT kind, name FROM extension_grants WHERE extension = ?1")
        .and_then(|mut stmt| {
            stmt.query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(failed)?;
    let mut grants = Grants::new();
    for (kind, name) in rows {
        match kind.as_str() {
            PERMISSION => grants.permissions.insert(name),
            PATTERN => grants.patterns.insert(name),
            _ => return Err(StoreError(format!("unknown grant kind {kind:?}"))),
        };
    }
    Ok(grants)
}

fn sites(db: &Connection, id: &str) -> Result<Vec<(Site, SiteAccess)>, StoreError> {
    let rows: Vec<(String, String)> = db
        .prepare_cached(
            "SELECT site, access FROM extension_sites WHERE extension = ?1 ORDER BY rank",
        )
        .and_then(|mut stmt| {
            stmt.query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(failed)?;
    rows.into_iter()
        .map(|(site, access)| {
            let site =
                Site::try_from(site.as_str()).map_err(|error| StoreError(error.to_string()))?;
            let access = SiteAccess::parse(&access)
                .ok_or_else(|| StoreError(format!("unknown site access {access:?}")))?;
            Ok((site, access))
        })
        .collect()
}

/// Replaces every row with the snapshot's, in one transaction.
pub(crate) fn save(
    db: &mut Connection,
    snapshot: &InstalledExtensionsSnapshot,
) -> Result<(), StoreError> {
    let tx = db.transaction().map_err(failed)?;
    // The grants and sites go with their extension (ON DELETE CASCADE).
    tx.execute("DELETE FROM extensions", []).map_err(failed)?;
    tx.execute("DELETE FROM extension_updates", [])
        .map_err(failed)?;
    {
        let mut extension = tx
            .prepare_cached(
                "INSERT INTO extensions (position, id, name, version, source, location, enabled) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(failed)?;
        let mut grant = tx
            .prepare_cached(
                "INSERT INTO extension_grants (extension, kind, name) VALUES (?1, ?2, ?3)",
            )
            .map_err(failed)?;
        let mut site = tx
            .prepare_cached(
                "INSERT INTO extension_sites (extension, rank, site, access) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(failed)?;
        for (position, entry) in snapshot.extensions.iter().enumerate() {
            extension
                .execute(params![
                    ordinal(position)?,
                    entry.id,
                    entry.name,
                    entry.version,
                    entry.source.as_str(),
                    entry.location,
                    entry.enabled,
                ])
                .map_err(failed)?;
            let named = entry
                .granted
                .permissions
                .iter()
                .map(|name| (PERMISSION, name))
                .chain(entry.granted.patterns.iter().map(|name| (PATTERN, name)));
            for (kind, name) in named {
                grant
                    .execute(params![entry.id, kind, name])
                    .map_err(failed)?;
            }
            for (rank, (on, access)) in entry.sites.iter().enumerate() {
                site.execute(params![
                    entry.id,
                    ordinal(rank)?,
                    on.as_str(),
                    access.as_str()
                ])
                .map_err(failed)?;
            }
        }
    }
    let last_checked = snapshot.last_checked.map(time_to_sql).transpose()?;
    tx.execute(
        "INSERT INTO extension_updates (id, auto_update, last_checked) VALUES (1, ?1, ?2)",
        params![snapshot.auto_update, last_checked],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)
}

#[cfg(test)]
mod tests {
    use quickie_application::ExtensionsRepo;
    use quickie_domain_settings::{
        ExtensionSnapshot, ExtensionSource, Grants, InstalledExtensionsSnapshot, Site, SiteAccess,
        Timestamp,
    };

    use crate::SqliteStore;

    fn entry(id: &str, enabled: bool) -> ExtensionSnapshot {
        ExtensionSnapshot {
            id: id.to_owned(),
            name: "Bitwarden".to_owned(),
            version: "2025.1.0".to_owned(),
            source: ExtensionSource::ChromeWebStore,
            location: format!("/kept/{id}.zip"),
            enabled,
            granted: Grants {
                permissions: ["storage".to_owned(), "tabs".to_owned()].into(),
                patterns: ["<all_urls>".to_owned()].into(),
            },
            sites: vec![
                (Site::try_from("b.com").unwrap(), SiteAccess::Blocked),
                (Site::try_from("a.com").unwrap(), SiteAccess::Allowed),
            ],
        }
    }

    #[test]
    fn a_fresh_profile_has_no_extensions_and_no_auto_update() {
        let store = SqliteStore::in_memory().unwrap();

        assert_eq!(
            store.load_extensions(),
            Ok(InstalledExtensionsSnapshot::default())
        );
    }

    #[test]
    fn extensions_round_trip_in_order_with_their_grants_and_sites() {
        let mut store = SqliteStore::in_memory().unwrap();
        let mut second = entry("ddkjiahejlhfcafbddmgiahcphecmpfh", false);
        second.source = ExtensionSource::Folder;
        second.granted = Grants::new();
        second.sites = Vec::new();
        let snapshot = InstalledExtensionsSnapshot {
            extensions: vec![entry("nngceckbapebfimnlniiiahkandclblb", true), second],
            auto_update: true,
            last_checked: Some(Timestamp::from_millis(1_700)),
        };

        store.save_extensions(snapshot.clone()).unwrap();

        assert_eq!(store.load_extensions(), Ok(snapshot));
    }

    #[test]
    fn a_save_replaces_everything_saved_before() {
        let mut store = SqliteStore::in_memory().unwrap();
        store
            .save_extensions(InstalledExtensionsSnapshot {
                extensions: vec![entry("nngceckbapebfimnlniiiahkandclblb", true)],
                auto_update: true,
                last_checked: None,
            })
            .unwrap();

        store
            .save_extensions(InstalledExtensionsSnapshot::default())
            .unwrap();

        assert_eq!(
            store.load_extensions(),
            Ok(InstalledExtensionsSnapshot::default())
        );
        let grants: i64 = store
            .db
            .query_row("SELECT count(*) FROM extension_grants", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(grants, 0);
    }
}
