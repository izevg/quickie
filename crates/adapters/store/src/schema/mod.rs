//! The schema, its migrations numbered by `PRAGMA user_version`, and how values map to columns.

pub(crate) mod columns;
mod migrations;

use quickie_application::StoreError;
use rusqlite::{Connection, TransactionBehavior};

use crate::failed;
pub(crate) use migrations::MIGRATIONS;

/// The schema version this build writes.
pub(crate) const VERSION: usize = MIGRATIONS.len();

/// Applies the migrations the database has not seen yet, all in one transaction.
///
/// The transaction takes the write lock before it reads the version, so of two connections
/// migrating the same file at once, the second waits for the first and then finds nothing left
/// to do (#76).
pub(crate) fn migrate(db: &mut Connection) -> Result<(), StoreError> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(failed)?;
    let current: i64 = tx
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(failed)?;
    let current = usize::try_from(current)
        .ok()
        .filter(|&version| version <= VERSION)
        .ok_or_else(|| {
            StoreError(format!(
                "the database has schema {current}; this build knows {VERSION}"
            ))
        })?;
    for (version, migration) in MIGRATIONS.iter().enumerate().skip(current) {
        tx.execute_batch(migration).map_err(failed)?;
        tx.pragma_update(None, "user_version", columns::ordinal(version + 1)?)
            .map_err(failed)?;
    }
    tx.commit().map_err(failed)
}
