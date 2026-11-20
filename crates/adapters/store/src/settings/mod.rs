//! The user's settings: key bindings, each site's page zoom, the agent's endpoint, and the
//! appearance ([`appearance`], here: one row of one column).

pub(crate) mod agent;
pub(crate) mod keymap;
pub(crate) mod zoom;

/// The `appearance` table: the appearance choices in its one row; no row is the defaults.
pub(crate) mod appearance {
    use quickie_application::StoreError;
    use quickie_domain_settings::AppearanceSnapshot;
    use rusqlite::{Connection, OptionalExtension, params};

    use crate::failed;

    /// The saved appearance, or the defaults.
    pub(crate) fn load(db: &Connection) -> Result<AppearanceSnapshot, StoreError> {
        let inset_page = db
            .query_row(
                "SELECT inset_page FROM appearance WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(failed)?;
        Ok(AppearanceSnapshot {
            inset_page: inset_page.unwrap_or_default(),
        })
    }

    /// Replaces the saved appearance.
    pub(crate) fn save(db: &Connection, appearance: AppearanceSnapshot) -> Result<(), StoreError> {
        db.execute(
            "INSERT OR REPLACE INTO appearance (id, inset_page) VALUES (1, ?1)",
            params![appearance.inset_page],
        )
        .map(drop)
        .map_err(failed)
    }
}
