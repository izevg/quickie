//! Page zoom, remembered per site.
//!
//! A site here is a page's registrable domain (`docs.example.com` and `example.com` share a
//! zoom), as the browsing context reckons it, so a site keeps its zoom across its subdomains.
//!
//! The same transaction rule as the other use cases: a command changes a copy, the copy is
//! saved, and only then does it replace the zooms, so a failed save changes nothing.
//!
//! **Throwaway jars.** A throwaway tab starts at 100 % whatever the main jar chose, and what it
//! zooms is kept for its jar while Quickie runs, never saved: nothing persists from a throwaway
//! jar ([`Zoom::forget_jar`] drops it with the jar).

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use quickie_domain_browsing::{Jar, JarId, registrable_domain};
use quickie_domain_settings::{CorruptZooms, Site, SiteError, SiteZooms, ZoomLevel, ZoomStep};
use url::Url;

use crate::{AppError, ZoomRepo};

/// The zoom use cases over one profile's site zooms.
#[derive(Debug)]
pub struct Zoom<R> {
    /// The main jar's, saved.
    sites: SiteZooms,
    /// Each throwaway jar's, in memory only.
    throwaway: BTreeMap<JarId, SiteZooms>,
    corrupt_on_start: Option<CorruptZooms>,
    repo: R,
}

impl<R: ZoomRepo> Zoom<R> {
    /// Loads the saved zooms. Corrupt ones start fresh and are reported by
    /// [`Self::corrupt_on_start`].
    ///
    /// # Errors
    ///
    /// The store could not be read.
    pub fn start(repo: R) -> Result<Self, AppError> {
        let saved = repo.load_site_zooms()?;
        let (sites, corrupt_on_start) = match SiteZooms::restore(saved) {
            Ok(sites) => (sites, None),
            Err(corrupt) => (SiteZooms::new(), Some(corrupt)),
        };
        Ok(Self {
            sites,
            throwaway: BTreeMap::new(),
            corrupt_on_start,
            repo,
        })
    }

    /// The saved zooms that were discarded at start, if they were.
    #[must_use]
    pub fn corrupt_on_start(&self) -> Option<&CorruptZooms> {
        self.corrupt_on_start.as_ref()
    }

    /// The main jar's zooms, for reading.
    #[must_use]
    pub fn sites(&self) -> &SiteZooms {
        &self.sites
    }

    /// The zoom a page at `url` shows at in `jar`: its site's, or 100 % off the web.
    #[must_use]
    pub fn level(&self, url: &Url, jar: Jar) -> ZoomLevel {
        let Ok(site) = site_of(url) else {
            return ZoomLevel::DEFAULT;
        };
        match jar {
            Jar::Main => self.sites.level(&site),
            Jar::Throwaway(id) => self
                .throwaway
                .get(&id)
                .map_or(ZoomLevel::DEFAULT, |zooms| zooms.level(&site)),
        }
    }

    /// Zooms the site of the page at `url` one step, or back to 100 %; returns its new zoom.
    /// The main jar's zoom is saved; a throwaway jar's is only kept.
    ///
    /// # Errors
    ///
    /// The page is not on the web, or the store failed; nothing changed.
    pub fn step(&mut self, url: &Url, jar: Jar, step: ZoomStep) -> Result<ZoomLevel, AppError> {
        let site = site_of(url)?;
        match jar {
            Jar::Main => {
                let mut next = self.sites.clone();
                let level = next.step(site, step);
                if next != self.sites {
                    self.repo.save_site_zooms(next.snapshot())?;
                    self.sites = next;
                }
                Ok(level)
            }
            Jar::Throwaway(id) => Ok(self.throwaway.entry(id).or_default().step(site, step)),
        }
    }

    /// A throwaway jar is gone: its zooms go with it.
    pub fn forget_jar(&mut self, jar: JarId) {
        self.throwaway.remove(&jar);
    }
}

/// The site a zoom is kept for: the page's registrable domain.
fn site_of(url: &Url) -> Result<Site, SiteError> {
    let host = Site::try_from(url)?;
    Site::try_from(registrable_domain(host.as_str()))
}
