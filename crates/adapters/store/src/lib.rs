//! SQLite persistence (`shell.sqlite`): implements the application's [`SessionRepo`].
//!
//! A save replaces the whole session inside one transaction, so a crash mid-save leaves the
//! previous session intact. Write-ahead logging keeps saves cheap enough for every command.

mod schema;
mod session;
#[cfg(test)]
mod tests;

use std::path::Path;

use quickie_application::{SessionRepo, StoreError};
use quickie_domain_browsing::SessionSnapshot;
use rusqlite::Connection;

/// The SQLite store.
#[derive(Debug)]
pub struct SqliteStore {
    db: Connection,
}

impl SqliteStore {
    /// Opens or creates the database file and brings its schema up to date.
    ///
    /// # Errors
    ///
    /// The file could not be opened, or the schema is newer than this build.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let db = Connection::open(path).map_err(failed)?;
        db.pragma_update(None, "journal_mode", "WAL")
            .map_err(failed)?;
        Self::init(db)
    }

    /// An in-memory database, for tests and the throwaway case.
    ///
    /// # Errors
    ///
    /// SQLite could not allocate it.
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory().map_err(failed)?)
    }

    fn init(mut db: Connection) -> Result<Self, StoreError> {
        db.pragma_update(None, "foreign_keys", true)
            .map_err(failed)?;
        schema::migrate(&mut db)?;
        Ok(Self { db })
    }
}

impl SessionRepo for SqliteStore {
    fn load(&self) -> Result<Option<SessionSnapshot>, StoreError> {
        session::load(&self.db)
    }

    fn save(&mut self, snapshot: &SessionSnapshot) -> Result<(), StoreError> {
        session::save(&mut self.db, snapshot)
    }
}

/// Wraps a SQLite error.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape `map_err` needs: `.map_err(failed)`"
)]
fn failed(error: rusqlite::Error) -> StoreError {
    StoreError(error.to_string())
}
