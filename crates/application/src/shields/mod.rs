//! Shields use cases: per-site shields, the filter lists, fetching them on request, and
//! compiling them into the engine's content blocker.
//!
//! The same transaction rule as browsing: a command changes a copy of one aggregate, the copy
//! is saved, and only then does it replace the aggregate, so a failed save changes nothing. The
//! content blocker follows after the commit, like the engine does for browsing.
//!
//! Nothing here reaches the network. The shell asks [`Shields::due_fetches`] or
//! [`Shields::requested_fetches`] what to fetch, runs [`fetch_lists`] on a worker thread, and
//! hands the result to [`Shields::lists_fetched`] on the main thread. A fresh profile has no
//! list enabled, so nothing is ever due on it.

#[cfg(test)]
mod fakes;
mod fetch;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use quickie_domain_browsing::TabId;
use quickie_domain_shields::{
    BlockTally, ContentHash, CorruptLists, CorruptSites, FilterListId, FilterListSet, Refresh,
    Site, SitePolicies, SitePolicy,
};
use url::Url;

pub use fetch::{FetchedList, ListFetch, Refreshed, fetch_lists};

use crate::{AppError, BlockerRules, Clock, ContentBlockerPort, ListRules, ShieldsRepo};

/// A saved part of the shields that broke an invariant at start; it was replaced by a fresh
/// one, which the next change of that part saves over it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptShields {
    /// The saved per-site exceptions.
    #[error("saved site shields are corrupt: {0}")]
    Sites(#[from] CorruptSites),
    /// The saved filter list states.
    #[error("saved filter lists are corrupt: {0}")]
    Lists(#[from] CorruptLists),
    /// A list's saved content is not what its last fetch recorded; the list blocks nothing until
    /// it is fetched again.
    #[error("the saved content of {0} does not match its last fetch")]
    Body(FilterListId),
}

/// The shields use cases over one profile's site policies and filter lists.
#[derive(Debug)]
pub struct Shields<R, C, B> {
    sites: SitePolicies,
    lists: FilterListSet,
    /// The content of each list as last fetched, whether enabled or not.
    bodies: BTreeMap<FilterListId, Arc<[u8]>>,
    tally: BlockTally,
    corrupt_on_start: Vec<CorruptShields>,
    repo: R,
    clock: C,
    blocker: B,
}

impl<R: ShieldsRepo, C: Clock, B: ContentBlockerPort> Shields<R, C, B> {
    /// Loads the saved shields and, when an enabled list has content, installs the rules. Never
    /// fetches anything. A corrupt part starts fresh and is reported by
    /// [`Self::corrupt_on_start`].
    ///
    /// # Errors
    ///
    /// The store could not be read, or the content blocker refused the rules.
    pub fn start(repo: R, clock: C, blocker: B) -> Result<Self, AppError> {
        let saved = repo.load_shields()?;
        let mut corrupt_on_start = Vec::new();
        let sites = SitePolicies::restore(saved.sites).unwrap_or_else(|error| {
            corrupt_on_start.push(error.into());
            SitePolicies::new()
        });
        let lists = FilterListSet::restore(saved.lists).unwrap_or_else(|error| {
            corrupt_on_start.push(error.into());
            FilterListSet::new()
        });
        let mut bodies = BTreeMap::new();
        for (list, body) in saved.bodies {
            let recorded = lists.get(list).fetched.map(|fetch| fetch.hash);
            if recorded == Some(ContentHash::of(&body)) {
                bodies.insert(list, Arc::from(body));
            } else {
                corrupt_on_start.push(CorruptShields::Body(list));
            }
        }
        let mut shields = Self {
            sites,
            lists,
            bodies,
            tally: BlockTally::new(),
            corrupt_on_start,
            repo,
            clock,
            blocker,
        };
        if !shields.rules().lists.is_empty() {
            shields.install()?;
        }
        Ok(shields)
    }

    /// The per-site exceptions, for reading.
    #[must_use]
    pub fn sites(&self) -> &SitePolicies {
        &self.sites
    }

    /// The filter lists, for reading.
    #[must_use]
    pub fn lists(&self) -> &FilterListSet {
        &self.lists
    }

    /// The saved parts that were discarded at start, if any were.
    #[must_use]
    pub fn corrupt_on_start(&self) -> &[CorruptShields] {
        &self.corrupt_on_start
    }

    /// The content blocker adapter, for the shell to hear when it finished compiling.
    pub fn blocker_mut(&mut self) -> &mut B {
        &mut self.blocker
    }

    /// The policy of the site a page is on; `None` for a page off the web.
    #[must_use]
    pub fn policy(&self, url: &Url) -> Option<SitePolicy> {
        Site::try_from(url)
            .ok()
            .map(|site| self.sites.policy(&site))
    }

    /// Turns shields off on the site of the page at `url`, or back on; returns the site's new
    /// policy. Its pages load with the new rules once the blocker compiled them.
    ///
    /// # Errors
    ///
    /// The page is off the web, the store failed, or the blocker refused the rules (the change
    /// is kept).
    pub fn toggle_site(&mut self, url: &Url) -> Result<SitePolicy, AppError> {
        let site = Site::try_from(url)?;
        let mut next = self.sites.clone();
        let policy = next.toggle_shields(site);
        self.commit_sites(next)?;
        self.install()?;
        Ok(policy)
    }

    /// Gives the site of the page at `url` a policy (HTTPS-only, JavaScript); returns whether
    /// that changed anything. The rules are compiled again only when its shields changed.
    ///
    /// # Errors
    ///
    /// The page is off the web, the store failed, or the blocker refused the rules (the change
    /// is kept).
    pub fn set_policy(&mut self, url: &Url, policy: SitePolicy) -> Result<bool, AppError> {
        let site = Site::try_from(url)?;
        let shields_before = self.sites.policy(&site).shields;
        let mut next = self.sites.clone();
        if !next.set(site, policy) {
            return Ok(false);
        }
        self.commit_sites(next)?;
        if shields_before != policy.shields {
            self.install()?;
        }
        Ok(true)
    }

    /// Turns lists on; returns those that were off. A list that has content blocks at once;
    /// one never fetched is now due, and blocks once [`Self::lists_fetched`] has it.
    ///
    /// # Errors
    ///
    /// The store failed, or the blocker refused the rules (the change is kept).
    pub fn enable_lists(&mut self, lists: &[FilterListId]) -> Result<Vec<FilterListId>, AppError> {
        self.change_lists(lists, FilterListSet::enable)
    }

    /// Turns lists off, keeping their content for when they come back; returns those that were
    /// on.
    ///
    /// # Errors
    ///
    /// The store failed, or the blocker refused the rules (the change is kept).
    pub fn disable_lists(&mut self, lists: &[FilterListId]) -> Result<Vec<FilterListId>, AppError> {
        self.change_lists(lists, FilterListSet::disable)
    }

    /// Sets whether lists are refreshed once stale ("keep lists fresh") or only on request;
    /// returns whether that changed anything.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn set_refresh(&mut self, refresh: Refresh) -> Result<bool, AppError> {
        let mut next = self.lists.clone();
        if !next.set_refresh(refresh) {
            return Ok(false);
        }
        self.repo.save_lists(&next.snapshot())?;
        self.lists = next;
        Ok(true)
    }

    /// The fetches due now without the user asking again: enabled lists never fetched, and,
    /// when the user keeps lists fresh, stale ones. None on a fresh profile.
    #[must_use]
    pub fn due_fetches(&self) -> Vec<ListFetch> {
        ListFetch::of(self.lists.due(self.clock.now()))
    }

    /// The fetches of an explicit refresh: every enabled list.
    #[must_use]
    pub fn requested_fetches(&self) -> Vec<ListFetch> {
        ListFetch::of(self.lists.enabled().map(|list| list.id))
    }

    /// Records what [`fetch_lists`] brought. New content is saved with its fetch, and when it
    /// belongs to an enabled list the rules are compiled again; a failed fetch keeps the
    /// previous content.
    ///
    /// # Errors
    ///
    /// The store failed (nothing changed), or the blocker refused the rules (the content is
    /// kept).
    pub fn lists_fetched(&mut self, fetched: Vec<FetchedList>) -> Result<Refreshed, AppError> {
        let now = self.clock.now();
        let mut next = self.lists.clone();
        let mut refreshed = Refreshed::default();
        let mut new_bodies = Vec::new();
        for FetchedList { list, body } in fetched {
            match body {
                Err(error) => refreshed.failed.push((list, error)),
                Ok(body) => {
                    if next.record_fetch(list, now, ContentHash::of(&body)) {
                        refreshed.updated.push(list);
                        new_bodies.push((list, Arc::<[u8]>::from(body)));
                    } else {
                        refreshed.unchanged.push(list);
                    }
                }
            }
        }
        if next == self.lists {
            return Ok(refreshed);
        }
        for (list, body) in &new_bodies {
            self.repo.save_list_body(*list, body)?;
        }
        self.repo.save_lists(&next.snapshot())?;
        self.lists = next;
        self.bodies.extend(new_bodies);
        if refreshed
            .updated
            .iter()
            .any(|&list| self.lists.get(list).enabled)
        {
            self.install()?;
        }
        Ok(refreshed)
    }

    /// Counts requests the engine blocked on a tab's page; returns the page's count so far.
    pub fn record_blocked(&mut self, tab: TabId, blocked: u32) -> u32 {
        self.tally.record(tab, blocked)
    }

    /// How many requests the page a tab shows had blocked, for the origin chip.
    #[must_use]
    pub fn blocked_count(&self, tab: TabId) -> u32 {
        self.tally.count(tab)
    }

    /// The page a tab showed is gone: a navigation committed, or the tab closed. Its count
    /// starts again at zero.
    pub fn page_left(&mut self, tab: TabId) {
        self.tally.forget(tab);
    }

    /// The rules for the content blocker: every enabled list that has content, and the sites
    /// whose shields are off.
    fn rules(&self) -> BlockerRules {
        let lists = self
            .lists
            .enabled()
            .filter_map(|list| {
                Some(ListRules {
                    list: list.id,
                    hash: list.fetched?.hash,
                    body: Arc::clone(self.bodies.get(&list.id)?),
                })
            })
            .collect();
        BlockerRules {
            lists,
            unshielded: self.sites.unshielded().cloned().collect(),
        }
    }

    fn install(&mut self) -> Result<(), AppError> {
        let rules = self.rules();
        self.blocker.install(&rules)?;
        Ok(())
    }

    fn commit_sites(&mut self, next: SitePolicies) -> Result<(), AppError> {
        self.repo.save_sites(&next.snapshot())?;
        self.sites = next;
        Ok(())
    }

    /// Applies `change` to each list, and saves and installs once if any changed.
    fn change_lists(
        &mut self,
        lists: &[FilterListId],
        change: impl Fn(&mut FilterListSet, FilterListId) -> bool,
    ) -> Result<Vec<FilterListId>, AppError> {
        let mut next = self.lists.clone();
        let changed: Vec<FilterListId> = lists
            .iter()
            .copied()
            .filter(|&list| change(&mut next, list))
            .collect();
        if changed.is_empty() {
            return Ok(changed);
        }
        self.repo.save_lists(&next.snapshot())?;
        self.lists = next;
        if changed.iter().any(|list| self.bodies.contains_key(list)) {
            self.install()?;
        }
        Ok(changed)
    }
}
