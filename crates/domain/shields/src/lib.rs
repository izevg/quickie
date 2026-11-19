//! Shields context: what Quickie blocks and where it holds back.
//!
//! - [`SitePolicies`]: per-site exceptions. Every site starts with shields on, HTTPS-only on,
//!   and JavaScript on ([`SitePolicy::DEFAULT`]); only the sites the user changed are kept.
//! - [`FilterListSet`]: the filter lists, only from the allowed sources of [`FilterListId`],
//!   each with when it was last fetched and the hash of what came back.
//! - [`BlockTally`]: how many requests each tab's page had blocked, for the origin chip.
//!
//! **Trackerless.** Nothing here fetches anything, and nothing asks for a fetch on its own. A
//! fresh profile has every list disabled, so it needs no connection. A list the user enables is
//! fetched once, because enabling it is the request. After that a list is fetched again only on
//! an explicit refresh, or, when the user turned on [`Refresh::KeepFresh`], once it is stale. A
//! scheduled refresh *is* a connection made on the browser's own initiative, so it is off until
//! the user turns it on (`docs/privacy.md`).
//!
//! Both [`SitePolicies`] and [`FilterListSet`] are aggregates the store saves in one
//! transaction each, through a plain snapshot that comes back through a validated `restore`.
//! Pure: no I/O, no clock (the caller passes [`Timestamp`]s), no async.

mod lists;
mod sites;
mod tally;

pub use lists::{
    ContentHash, CorruptLists, Fetch, FilterList, FilterListId, FilterListSet,
    FilterListSetSnapshot, Refresh, STALE_AFTER_MS, UnknownList,
};
pub use quickie_domain_kernel::{TabId, Timestamp};
pub use sites::{CorruptSites, Site, SiteError, SitePolicies, SitePoliciesSnapshot, SitePolicy};
pub use tally::BlockTally;
