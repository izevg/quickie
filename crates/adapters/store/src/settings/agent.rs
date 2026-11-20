//! The `agent_endpoint` table: the agent's endpoint as text, in its one row. The key is never
//! here, only the reference the Keychain keeps it under; the agent use cases judge the row, so
//! one that no longer parses is reported there, not lost here.

use quickie_application::StoreError;
use quickie_domain_agents::EndpointSnapshot;
use rusqlite::{Connection, OptionalExtension, params};

use crate::failed;

/// The saved endpoint, if there is one.
pub(crate) fn load(db: &Connection) -> Result<Option<EndpointSnapshot>, StoreError> {
    db.query_row(
        "SELECT base_url, model, key_ref FROM agent_endpoint WHERE id = 1",
        [],
        |row| {
            Ok(EndpointSnapshot {
                base_url: row.get(0)?,
                model: row.get(1)?,
                key_ref: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(failed)
}

/// Replaces the saved endpoint in one transaction; `None` leaves the table empty.
pub(crate) fn save(
    db: &mut Connection,
    endpoint: Option<&EndpointSnapshot>,
) -> Result<(), StoreError> {
    let tx = db.transaction().map_err(failed)?;
    tx.execute("DELETE FROM agent_endpoint", [])
        .map_err(failed)?;
    if let Some(endpoint) = endpoint {
        tx.execute(
            "INSERT INTO agent_endpoint (id, base_url, model, key_ref) VALUES (1, ?1, ?2, ?3)",
            params![endpoint.base_url, endpoint.model, endpoint.key_ref],
        )
        .map_err(failed)?;
    }
    tx.commit().map_err(failed)
}
