//! How domain values sit in SQLite columns. Validation is the domain's job (each aggregate's
//! `restore`); these only reject values that cannot be numbers, times, or URLs at all.

use quickie_application::StoreError;
use quickie_domain_library::Timestamp;
use url::Url;

/// Ids, counts and times are `u64` in the domain and `INTEGER` (`i64`) in SQLite; real values
/// never reach 2^63.
pub(crate) fn to_sql(raw: u64) -> Result<i64, StoreError> {
    i64::try_from(raw).map_err(|_| StoreError(format!("{raw} does not fit SQLite")))
}

/// The way back from [`to_sql`].
pub(crate) fn from_sql(raw: i64) -> Result<u64, StoreError> {
    u64::try_from(raw).map_err(|_| StoreError(format!("negative number {raw}")))
}

/// A position in an ordered list, as SQLite stores it.
pub(crate) fn ordinal(n: usize) -> Result<i64, StoreError> {
    i64::try_from(n).map_err(|_| StoreError(format!("position {n} does not fit SQLite")))
}

/// A point in time as milliseconds since the Unix epoch.
pub(crate) fn time_to_sql(at: Timestamp) -> Result<i64, StoreError> {
    to_sql(at.as_millis())
}

/// The way back from [`time_to_sql`].
pub(crate) fn time_from_sql(raw: i64) -> Result<Timestamp, StoreError> {
    from_sql(raw).map(Timestamp::from_millis)
}

/// A URL column.
pub(crate) fn url(raw: &str) -> Result<Url, StoreError> {
    Url::parse(raw).map_err(|error| StoreError(format!("bad URL {raw:?}: {error}")))
}
