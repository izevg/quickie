//! Web extensions the user installed in the main profile: what each is, whether it runs, what
//! it may do, and on which sites the user decided otherwise.
//!
//! An extension joins only once the user approved what it asks for, so the list holds no
//! half-installed entries; the use case keeps a package waiting for approval in memory.

mod access;
mod identity;
mod snapshot;
#[cfg(test)]
mod tests;

use indexmap::IndexMap;
use quickie_domain_kernel::{Site, Timestamp};

pub use access::{Grants, SiteAccess, site_patterns};
pub use identity::{
    ExtensionId, ExtensionIdError, ExtensionSource, ExtensionVersion, VersionError,
};
pub use snapshot::{CorruptExtensions, ExtensionSnapshot, InstalledExtensionsSnapshot};

/// An extension's package as the engine read it, ready to join the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub id: ExtensionId,
    /// Its name as the manifest gives it, in the user's language.
    pub name: String,
    pub version: ExtensionVersion,
    pub source: ExtensionSource,
    /// Where the engine finds it again: opaque here, never empty.
    pub location: String,
}

/// A change the list refuses.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExtensionError {
    /// Its manifest gives no name.
    #[error("the extension {0} has no name")]
    NoName(ExtensionId),
    /// The engine gave no place to find it again.
    #[error("the extension {0} has no location")]
    NoLocation(ExtensionId),
    /// A permission or pattern granted to it is empty or holds white space.
    #[error("the extension {0} was granted {1:?}, which is no permission")]
    BadGrant(ExtensionId, String),
    /// The command named an extension that is not installed.
    #[error("no extension {0} is installed")]
    NotInstalled(ExtensionId),
}

/// An installed extension and the user's choices about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledExtension {
    package: Package,
    enabled: bool,
    granted: Grants,
    sites: IndexMap<Site, SiteAccess>,
}

impl InstalledExtension {
    #[must_use]
    pub fn id(&self) -> &ExtensionId {
        &self.package.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.package.name
    }

    #[must_use]
    pub fn version(&self) -> &ExtensionVersion {
        &self.package.version
    }

    #[must_use]
    pub fn source(&self) -> ExtensionSource {
        self.package.source
    }

    #[must_use]
    pub fn location(&self) -> &str {
        &self.package.location
    }

    /// Whether it runs.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// What the user let it do.
    #[must_use]
    pub fn granted(&self) -> &Grants {
        &self.granted
    }

    /// The sites the user allowed or blocked it on, in the order they were first decided.
    pub fn sites(&self) -> impl Iterator<Item = (&Site, SiteAccess)> {
        self.sites.iter().map(|(site, &access)| (site, access))
    }

    /// What the user decided for it on `site`, `None` for no decision.
    #[must_use]
    pub fn access(&self, site: &Site) -> Option<SiteAccess> {
        self.sites.get(site).copied()
    }
}

/// How an install changed the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    /// A new extension, enabled.
    New,
    /// An installed extension in another version; it keeps whether it runs and its sites.
    Replaced {
        /// The version it had.
        previous: ExtensionVersion,
    },
}

/// The extensions of the main profile, in the order they were installed, and whether updates
/// come on their own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstalledExtensions {
    installed: IndexMap<ExtensionId, InstalledExtension>,
    auto_update: bool,
    last_checked: Option<Timestamp>,
}

impl InstalledExtensions {
    /// No extension; updates only on request.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `package` with `granted` as what it may do, or replaces the installed extension
    /// with the same id: that one keeps whether it runs and its sites, and gets `granted`.
    ///
    /// # Errors
    ///
    /// The package has no name or location, or a grant is malformed; nothing changes.
    pub fn install(
        &mut self,
        package: Package,
        granted: Grants,
    ) -> Result<Installed, ExtensionError> {
        check(&package, &granted)?;
        if let Some(existing) = self.installed.get_mut(&package.id) {
            let previous = std::mem::replace(&mut existing.package, package).version;
            existing.granted = granted;
            return Ok(Installed::Replaced { previous });
        }
        self.installed.insert(
            package.id.clone(),
            InstalledExtension {
                package,
                enabled: true,
                granted,
                sites: IndexMap::new(),
            },
        );
        Ok(Installed::New)
    }

    /// Takes an extension out of the list; returns it, `None` when it was not installed.
    pub fn remove(&mut self, id: &ExtensionId) -> Option<InstalledExtension> {
        self.installed.shift_remove(id)
    }

    /// Lets an extension run or stops it; returns whether that changed anything.
    ///
    /// # Errors
    ///
    /// It is not installed.
    pub fn set_enabled(&mut self, id: &ExtensionId, enabled: bool) -> Result<bool, ExtensionError> {
        let extension = self.get_mut(id)?;
        Ok(std::mem::replace(&mut extension.enabled, enabled) != enabled)
    }

    /// Lets an extension do `more` as well; returns whether anything was new.
    ///
    /// # Errors
    ///
    /// It is not installed, or a grant is malformed (nothing changes).
    pub fn grant(&mut self, id: &ExtensionId, more: &Grants) -> Result<bool, ExtensionError> {
        let extension = self.get_mut(id)?;
        if let Some(bad) = more.malformed() {
            return Err(ExtensionError::BadGrant(id.clone(), bad.to_owned()));
        }
        Ok(extension.granted.add(more))
    }

    /// Takes `less` away from what an extension may do; returns whether it had any of it.
    ///
    /// # Errors
    ///
    /// It is not installed.
    pub fn revoke(&mut self, id: &ExtensionId, less: &Grants) -> Result<bool, ExtensionError> {
        Ok(self.get_mut(id)?.granted.remove(less))
    }

    /// Allows or blocks an extension on `site`, or forgets the decision (`None`); returns
    /// whether that changed anything.
    ///
    /// # Errors
    ///
    /// It is not installed.
    pub fn set_access(
        &mut self,
        id: &ExtensionId,
        site: Site,
        access: Option<SiteAccess>,
    ) -> Result<bool, ExtensionError> {
        let sites = &mut self.get_mut(id)?.sites;
        Ok(match access {
            Some(access) => sites.insert(site, access) != Some(access),
            None => sites.shift_remove(&site).is_some(),
        })
    }

    /// Whether updates come on their own or only on request.
    #[must_use]
    pub fn auto_update(&self) -> bool {
        self.auto_update
    }

    /// Lets updates come on their own, or only on request; returns whether that changed.
    pub fn set_auto_update(&mut self, on: bool) -> bool {
        std::mem::replace(&mut self.auto_update, on) != on
    }

    /// When updates were last looked for, `None` if never.
    #[must_use]
    pub fn last_checked(&self) -> Option<Timestamp> {
        self.last_checked
    }

    /// Records that updates were looked for at `now`.
    pub fn checked(&mut self, now: Timestamp) {
        self.last_checked = Some(now);
    }

    /// Whether looking for updates is due without the user asking: only when they let updates
    /// come on their own, and a day after the last look.
    #[must_use]
    pub fn update_due(&self, now: Timestamp) -> bool {
        self.auto_update
            && self
                .last_checked
                .is_none_or(|last| now.millis_since(last) >= Timestamp::DAY_MS)
    }

    /// Whether `version` of `id` would be new: not installed, or later than the installed one.
    #[must_use]
    pub fn is_newer(&self, id: &ExtensionId, version: &ExtensionVersion) -> bool {
        self.installed
            .get(id)
            .is_none_or(|installed| version > installed.version())
    }

    #[must_use]
    pub fn get(&self, id: &ExtensionId) -> Option<&InstalledExtension> {
        self.installed.get(id)
    }

    /// Every installed extension, in the order it was installed.
    pub fn iter(&self) -> impl Iterator<Item = &InstalledExtension> {
        self.installed.values()
    }

    /// The extensions the Chrome Web Store updates.
    pub fn from_store(&self) -> impl Iterator<Item = &InstalledExtension> {
        self.iter()
            .filter(|extension| extension.source() == ExtensionSource::ChromeWebStore)
    }

    fn get_mut(&mut self, id: &ExtensionId) -> Result<&mut InstalledExtension, ExtensionError> {
        self.installed
            .get_mut(id)
            .ok_or_else(|| ExtensionError::NotInstalled(id.clone()))
    }
}

/// The invariants of one entry, checked on install and on restore.
fn check(package: &Package, granted: &Grants) -> Result<(), ExtensionError> {
    if package.name.trim().is_empty() {
        return Err(ExtensionError::NoName(package.id.clone()));
    }
    if package.location.is_empty() {
        return Err(ExtensionError::NoLocation(package.id.clone()));
    }
    if let Some(bad) = granted.malformed() {
        return Err(ExtensionError::BadGrant(package.id.clone(), bad.to_owned()));
    }
    Ok(())
}
