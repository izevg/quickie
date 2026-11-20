//! Installed extensions as plain data: the only door between the store and the aggregate.

use indexmap::IndexMap;
use quickie_domain_kernel::{Site, Timestamp};

use super::{
    ExtensionError, ExtensionId, ExtensionSource, ExtensionVersion, Grants, InstalledExtension,
    InstalledExtensions, Package, SiteAccess, check,
};

/// One installed extension as plain data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionSnapshot {
    pub id: String,
    pub name: String,
    pub version: String,
    pub source: ExtensionSource,
    pub location: String,
    pub enabled: bool,
    pub granted: Grants,
    /// The sites decided for it, in the order they were first decided.
    pub sites: Vec<(Site, SiteAccess)>,
}

/// Every installed extension as plain data, for the store to write and read back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstalledExtensionsSnapshot {
    /// In the order they were installed.
    pub extensions: Vec<ExtensionSnapshot>,
    pub auto_update: bool,
    pub last_checked: Option<Timestamp>,
}

/// A snapshot that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptExtensions {
    /// An id that is not one.
    #[error("{0:?} is not an extension id")]
    Id(String),
    /// A version that is not one.
    #[error("the extension {0} has the version {1:?}")]
    Version(ExtensionId, String),
    /// Two entries name one extension.
    #[error("the extension {0} appears twice")]
    Duplicate(ExtensionId),
    /// Two decisions name one site for one extension.
    #[error("the extension {0} has two decisions for {1}")]
    DuplicateSite(ExtensionId, Site),
    /// An entry the list would refuse.
    #[error(transparent)]
    Entry(#[from] ExtensionError),
}

impl InstalledExtensions {
    /// Rebuilds the list from a snapshot, checking every invariant once, here.
    ///
    /// # Errors
    ///
    /// The first [`CorruptExtensions`] found.
    pub fn restore(snapshot: InstalledExtensionsSnapshot) -> Result<Self, CorruptExtensions> {
        let mut installed = IndexMap::with_capacity(snapshot.extensions.len());
        for entry in snapshot.extensions {
            let id = ExtensionId::try_from(entry.id.as_str())
                .map_err(|_| CorruptExtensions::Id(entry.id.clone()))?;
            let version = ExtensionVersion::try_from(entry.version.as_str())
                .map_err(|_| CorruptExtensions::Version(id.clone(), entry.version.clone()))?;
            if installed.contains_key(&id) {
                return Err(CorruptExtensions::Duplicate(id));
            }
            let package = Package {
                id: id.clone(),
                name: entry.name,
                version,
                source: entry.source,
                location: entry.location,
            };
            check(&package, &entry.granted)?;
            let mut sites = IndexMap::with_capacity(entry.sites.len());
            for (site, access) in entry.sites {
                if sites.contains_key(&site) {
                    return Err(CorruptExtensions::DuplicateSite(id, site));
                }
                sites.insert(site, access);
            }
            installed.insert(
                id,
                InstalledExtension {
                    package,
                    enabled: entry.enabled,
                    granted: entry.granted,
                    sites,
                },
            );
        }
        Ok(Self {
            installed,
            auto_update: snapshot.auto_update,
            last_checked: snapshot.last_checked,
        })
    }

    /// The list as plain data; `restore(snapshot())` gives back an equal list.
    #[must_use]
    pub fn snapshot(&self) -> InstalledExtensionsSnapshot {
        InstalledExtensionsSnapshot {
            extensions: self
                .iter()
                .map(|extension| ExtensionSnapshot {
                    id: extension.id().as_str().to_owned(),
                    name: extension.name().to_owned(),
                    version: extension.version().as_str().to_owned(),
                    source: extension.source(),
                    location: extension.location().to_owned(),
                    enabled: extension.enabled(),
                    granted: extension.granted().clone(),
                    sites: extension
                        .sites()
                        .map(|(site, access)| (site.clone(), access))
                        .collect(),
                })
                .collect(),
            auto_update: self.auto_update,
            last_checked: self.last_checked,
        }
    }
}
