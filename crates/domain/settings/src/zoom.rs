//! Page zoom per site.

mod level;
mod snapshot;
#[cfg(test)]
mod tests;

use indexmap::IndexMap;
use quickie_domain_kernel::Site;

pub use level::{ZoomLevel, ZoomStep};
pub use snapshot::{CorruptZooms, SiteZoomsSnapshot};

/// The zoom of each site the user zoomed. A site not listed shows at [`ZoomLevel::DEFAULT`]; a
/// site zoomed back to it leaves the list, so the list is exactly the user's choices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteZooms {
    /// In the order they were first zoomed.
    levels: IndexMap<Site, ZoomLevel>,
}

impl SiteZooms {
    /// No site zoomed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The zoom of a site.
    #[must_use]
    pub fn level(&self, site: &Site) -> ZoomLevel {
        self.levels.get(site).copied().unwrap_or_default()
    }

    /// Gives a site a zoom; returns whether that changed anything.
    pub fn set(&mut self, site: Site, level: ZoomLevel) -> bool {
        if self.level(&site) == level {
            return false;
        }
        if level == ZoomLevel::DEFAULT {
            self.levels.shift_remove(&site);
        } else {
            self.levels.insert(site, level);
        }
        true
    }

    /// Zooms a site one step in or out, or back to 100 %; returns its new zoom. At either end
    /// of the steps it stays where it is.
    pub fn step(&mut self, site: Site, step: ZoomStep) -> ZoomLevel {
        let level = self.level(&site).step(step);
        self.set(site, level);
        level
    }

    /// Every zoomed site, in the order it was first zoomed.
    pub fn iter(&self) -> impl Iterator<Item = (&Site, ZoomLevel)> {
        self.levels.iter().map(|(site, &level)| (site, level))
    }
}
