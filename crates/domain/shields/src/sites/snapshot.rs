//! Site policies as plain data: the only door between the store and the aggregate.

use indexmap::IndexMap;

use super::{Site, SitePolicies, SitePolicy};

/// Every per-site exception as plain data, for the store to write and read back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SitePoliciesSnapshot {
    /// Exceptions in the order they were first set.
    pub sites: Vec<(Site, SitePolicy)>,
}

/// A snapshot that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptSites {
    /// Two exceptions name one site.
    #[error("the site {0} appears twice")]
    DuplicateSite(Site),
}

impl SitePolicies {
    /// Rebuilds the exceptions from a snapshot, checking every invariant once, here. An entry
    /// with the default policy is no exception and is left out.
    ///
    /// # Errors
    ///
    /// The first [`CorruptSites`] found.
    pub fn restore(snapshot: SitePoliciesSnapshot) -> Result<Self, CorruptSites> {
        let mut exceptions = IndexMap::with_capacity(snapshot.sites.len());
        for (site, policy) in snapshot.sites {
            if exceptions.contains_key(&site) {
                return Err(CorruptSites::DuplicateSite(site));
            }
            exceptions.insert(site, policy);
        }
        exceptions.retain(|_, policy| *policy != SitePolicy::DEFAULT);
        Ok(Self { exceptions })
    }

    /// The exceptions as plain data; `restore(snapshot())` gives back equal policies.
    #[must_use]
    pub fn snapshot(&self) -> SitePoliciesSnapshot {
        SitePoliciesSnapshot {
            sites: self
                .iter()
                .map(|(site, policy)| (site.clone(), policy))
                .collect(),
        }
    }
}
