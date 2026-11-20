//! What pages reported since the shell last asked: committed navigations and title changes,
//! folded into one [`Navigated`] per tab.

use std::collections::{HashMap, HashSet};

use quickie_application::Navigated;
use quickie_domain_browsing::TabId;
use url::Url;

/// Page reports waiting for the shell, in the order tabs first changed.
#[derive(Debug, Default)]
pub(crate) struct Reports {
    /// The last committed URL and title of every page that committed.
    current: HashMap<TabId, (Url, String)>,
    /// Tabs that changed since the last [`Reports::take`].
    changed: Vec<TabId>,
    /// Tabs that committed a navigation since the last [`Reports::take`].
    committed: HashSet<TabId>,
}

impl Reports {
    /// A page committed a navigation. A URL that does not parse is not one the session can
    /// keep, so it is dropped. The title stays until the page reports a new one, because a
    /// same-document navigation (a fragment, `history.pushState`) keeps it.
    ///
    /// TODO(verify): that Chromium reports `TitleWasSet` after every cross-document commit, even
    /// for a page without a `<title>` (`content/browser/web_contents/web_contents_impl.cc`);
    /// otherwise the previous page's title lingers until the next one.
    pub(crate) fn committed(&mut self, tab: TabId, url: &str) {
        let Ok(url) = Url::parse(url) else {
            return;
        };
        let title = self
            .current
            .remove(&tab)
            .map_or_else(String::new, |(_, title)| title);
        self.current.insert(tab, (url, title));
        self.committed.insert(tab);
        self.mark(tab);
    }

    /// A page's title changed. Before its first commit there is no URL to report it with.
    pub(crate) fn titled(&mut self, tab: TabId, title: String) {
        let Some((_, current)) = self.current.get_mut(&tab) else {
            return;
        };
        if *current != title {
            *current = title;
            self.mark(tab);
        }
    }

    /// The tab's page is gone; nothing more is reported for it.
    pub(crate) fn forget(&mut self, tab: TabId) {
        self.current.remove(&tab);
        self.committed.remove(&tab);
        self.changed.retain(|&changed| changed != tab);
    }

    /// True when nothing waits for [`Reports::take`].
    pub(crate) fn is_empty(&self) -> bool {
        self.changed.is_empty()
    }

    /// Every changed tab's latest state, once.
    pub(crate) fn take(&mut self) -> Vec<Navigated> {
        let changed = std::mem::take(&mut self.changed);
        changed
            .into_iter()
            .filter_map(|tab| {
                let (url, title) = self.current.get(&tab)?;
                Some(Navigated {
                    tab,
                    url: url.clone(),
                    title: title.clone(),
                    committed: self.committed.remove(&tab),
                })
            })
            .collect()
    }

    fn mark(&mut self, tab: TabId) {
        if !self.changed.contains(&tab) {
            self.changed.push(tab);
        }
    }
}
