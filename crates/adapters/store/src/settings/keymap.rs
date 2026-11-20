//! The `keybindings` table: the user's `keybindings.json` entries as text, one per row. The
//! keymap use case parses them, so a row that no longer parses is reported there, not lost here.

use quickie_application::{KeybindingText, StoreError};
use rusqlite::Connection;

use crate::failed;

/// The user's entries, in the order they were written.
pub(crate) fn load(db: &Connection) -> Result<Vec<KeybindingText>, StoreError> {
    db.prepare("SELECT key, command, when_clause FROM keybindings ORDER BY position")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(KeybindingText {
                    key: row.get(0)?,
                    command: row.get(1)?,
                    when: row.get(2)?,
                })
            })?
            .collect()
        })
        .map_err(failed)
}
