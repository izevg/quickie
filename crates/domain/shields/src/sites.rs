//! Per-site policies: the sites where the user changed what shields do.

mod policy;
mod snapshot;
#[cfg(test)]
mod tests;

use indexmap::IndexMap;

pub use policy::SitePolicy;
pub use quickie_domain_kernel::{Site, SiteError};
pub use snapshot::{CorruptSites, SitePoliciesSnapshot};

/// The per-site exceptions of one profile. A site not listed has [`SitePolicy::DEFAULT`]; a
/// site set back to the default leaves the list, so the list is exactly the user's exceptions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SitePolicies {
    /// In the order they were first set.
    exceptions: IndexMap<Site, SitePolicy>,
}

impl SitePolicies {
    /// No exceptions: every site has the default policy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The policy of a site.
    #[must_use]
    pub fn policy(&self, site: &Site) -> SitePolicy {
        self.exceptions.get(site).copied().unwrap_or_default()
    }

    /// Gives a site a policy; returns whether that changed anything.
    pub fn set(&mut self, site: Site, policy: SitePolicy) -> bool {
        if self.policy(&site) == policy {
            return false;
        }
        if policy == SitePolicy::DEFAULT {
            self.exceptions.shift_remove(&site);
        } else {
            self.exceptions.insert(site, policy);
        }
        true
    }

    /// Turns a site's shields off if they were on, or on if they were off; returns the site's
    /// new policy.
    pub fn toggle_shields(&mut self, site: Site) -> SitePolicy {
        let mut policy = self.policy(&site);
        policy.shields = !policy.shields;
        self.set(site, policy);
        policy
    }

    /// The sites whose shields are off, in the order they were first set: the engine lets
    /// their pages load everything.
    pub fn unshielded(&self) -> impl Iterator<Item = &Site> {
        self.exceptions
            .iter()
            .filter(|(_, policy)| !policy.shields)
            .map(|(site, _)| site)
    }

    /// Every exception, in the order it was first set.
    pub fn iter(&self) -> impl Iterator<Item = (&Site, SitePolicy)> {
        self.exceptions.iter().map(|(site, &policy)| (site, policy))
    }
}
