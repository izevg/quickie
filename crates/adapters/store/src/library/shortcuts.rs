//! The `search_shortcuts` table.

use quickie_application::StoreError;
use quickie_domain_library::{Keyword, SearchShortcutsSnapshot, UrlTemplate};
use rusqlite::{Connection, Transaction, params};

use crate::failed;
use crate::schema::columns::ordinal;

/// The saved shortcuts, in the order they were added.
pub(super) fn load(db: &Connection) -> Result<SearchShortcutsSnapshot, StoreError> {
    let rows: Vec<(String, String)> = db
        .prepare("SELECT keyword, template FROM search_shortcuts ORDER BY position")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(failed)?;
    let shortcuts = rows
        .into_iter()
        .map(|(keyword, template)| {
            let keyword = Keyword::try_from(keyword.as_str())
                .map_err(|error| StoreError(format!("bad keyword {keyword:?}: {error}")))?;
            let template = UrlTemplate::try_from(template.as_str())
                .map_err(|error| StoreError(format!("bad template {template:?}: {error}")))?;
            Ok((keyword, template))
        })
        .collect::<Result<_, StoreError>>()?;
    Ok(SearchShortcutsSnapshot { shortcuts })
}

/// Replaces every row with the snapshot's.
pub(super) fn save(
    tx: &Transaction<'_>,
    snapshot: &SearchShortcutsSnapshot,
) -> Result<(), StoreError> {
    tx.execute("DELETE FROM search_shortcuts", [])
        .map_err(failed)?;
    let mut insert = tx
        .prepare("INSERT INTO search_shortcuts (position, keyword, template) VALUES (?1, ?2, ?3)")
        .map_err(failed)?;
    for (position, (keyword, template)) in snapshot.shortcuts.iter().enumerate() {
        insert
            .execute(params![
                ordinal(position)?,
                keyword.as_str(),
                template.as_str()
            ])
            .map_err(failed)?;
    }
    Ok(())
}
