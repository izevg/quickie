//! Site zooms as plain data: the only door between the store and the aggregate.

use indexmap::IndexMap;
use quickie_domain_kernel::Site;

use super::{SiteZooms, ZoomLevel};

/// Every zoomed site as plain data, for the store to write and read back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteZoomsSnapshot {
    /// Each site and its zoom in percent, in the order they were first zoomed.
    pub sites: Vec<(Site, u16)>,
}

/// A snapshot that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptZooms {
    /// Two entries name one site.
    #[error("the site {0} appears twice")]
    DuplicateSite(Site),
    /// A site's zoom is not one of the steps.
    #[error("{0} is zoomed to {1} %, which is not a zoom step")]
    NotAStep(Site, u16),
}

impl SiteZooms {
    /// Rebuilds the zooms from a snapshot, checking every invariant once, here. An entry at
    /// 100 % is no choice and is left out.
    ///
    /// # Errors
    ///
    /// The first [`CorruptZooms`] found.
    pub fn restore(snapshot: SiteZoomsSnapshot) -> Result<Self, CorruptZooms> {
        let mut levels = IndexMap::with_capacity(snapshot.sites.len());
        for (site, percent) in snapshot.sites {
            if levels.contains_key(&site) {
                return Err(CorruptZooms::DuplicateSite(site));
            }
            let Some(level) = ZoomLevel::from_percent(percent) else {
                return Err(CorruptZooms::NotAStep(site, percent));
            };
            levels.insert(site, level);
        }
        levels.retain(|_, level| *level != ZoomLevel::DEFAULT);
        Ok(Self { levels })
    }

    /// The zooms as plain data; `restore(snapshot())` gives back equal zooms.
    #[must_use]
    pub fn snapshot(&self) -> SiteZoomsSnapshot {
        SiteZoomsSnapshot {
            sites: self
                .iter()
                .map(|(site, level)| (site.clone(), level.percent()))
                .collect(),
        }
    }
}
