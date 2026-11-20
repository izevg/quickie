//! What pages reported since the shell last asked: committed navigations and title changes,
//! folded into one [`Navigated`] per tab. Pure, so the folding is tested without `WebKit`.

use std::collections::{HashMap, HashSet};

use quickie_application::Navigated;
use quickie_domain_browsing::TabId;
use url::Url;

/// Page reports waiting for the shell, in the order tabs first changed.
#[derive(Debug, Default)]
pub(crate) struct Reports {
    /// The last committed URL and title of every live page.
    current: HashMap<TabId, (Url, String)>,
    /// Tabs that changed since the last [`Reports::take`].
    changed: Vec<TabId>,
    /// Tabs that committed a navigation since the last [`Reports::take`].
    committed: HashSet<TabId>,
}

impl Reports {
    /// A page committed a navigation. A URL that does not parse is not one the session can
    /// keep, so it is dropped.
    pub(crate) fn committed(&mut self, tab: TabId, url: Option<&str>, title: Option<String>) {
        let Some(url) = url.and_then(|url| Url::parse(url).ok()) else {
            return;
        };
        self.current.insert(tab, (url, title.unwrap_or_default()));
        self.committed.insert(tab);
        self.mark(tab);
    }

    /// A page's title changed. Before its first commit there is no URL to report it with.
    pub(crate) fn titled(&mut self, tab: TabId, title: Option<String>) {
        let Some((_, current)) = self.current.get_mut(&tab) else {
            return;
        };
        let title = title.unwrap_or_default();
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

    /// True when something waits for [`Reports::take`].
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

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::TabId;
    use url::Url;

    use super::{Navigated, Reports};

    fn navigated(tab: u64, url: &str, title: &str) -> Navigated {
        Navigated {
            tab: TabId::new(tab),
            url: Url::parse(url).unwrap(),
            title: title.to_owned(),
            committed: true,
        }
    }

    fn retitled(tab: u64, url: &str, title: &str) -> Navigated {
        Navigated {
            committed: false,
            ..navigated(tab, url, title)
        }
    }

    #[test]
    fn a_title_after_the_report_of_its_commit_is_no_visit() {
        let mut reports = Reports::default();
        reports.committed(TabId::new(1), Some("https://a.example/"), None);
        let _ = reports.take();

        reports.titled(TabId::new(1), Some("Loaded".into()));

        assert_eq!(
            reports.take(),
            [retitled(1, "https://a.example/", "Loaded")]
        );
    }

    #[test]
    fn a_commit_is_reported_once() {
        let mut reports = Reports::default();

        reports.committed(TabId::new(1), Some("https://a.example/"), Some("A".into()));

        assert!(!reports.is_empty());
        assert_eq!(reports.take(), [navigated(1, "https://a.example/", "A")]);
        assert!(reports.is_empty());
        assert_eq!(reports.take(), []);
    }

    #[test]
    fn a_later_title_comes_with_the_committed_url() {
        let mut reports = Reports::default();
        reports.committed(TabId::new(1), Some("https://a.example/"), None);

        reports.titled(TabId::new(1), Some("Loaded".into()));

        assert_eq!(
            reports.take(),
            [navigated(1, "https://a.example/", "Loaded")]
        );
    }

    #[test]
    fn changes_fold_into_one_report_per_tab_in_first_change_order() {
        let mut reports = Reports::default();
        reports.committed(TabId::new(2), Some("https://b.example/"), None);
        reports.committed(TabId::new(1), Some("https://a.example/"), None);
        reports.committed(TabId::new(2), Some("https://c.example/"), Some("C".into()));

        assert_eq!(
            reports.take(),
            [
                navigated(2, "https://c.example/", "C"),
                navigated(1, "https://a.example/", ""),
            ]
        );
    }

    #[test]
    fn a_title_before_any_commit_or_unchanged_is_not_news() {
        let mut reports = Reports::default();
        reports.titled(TabId::new(1), Some("Early".into()));
        assert!(reports.is_empty());

        reports.committed(TabId::new(1), Some("https://a.example/"), Some("A".into()));
        let _ = reports.take();
        reports.titled(TabId::new(1), Some("A".into()));

        assert!(reports.is_empty());
    }

    #[test]
    fn unparsable_or_missing_urls_are_dropped() {
        let mut reports = Reports::default();

        reports.committed(TabId::new(1), None, Some("A".into()));
        reports.committed(TabId::new(1), Some("not a url"), Some("A".into()));

        assert!(reports.is_empty());
    }

    #[test]
    fn a_forgotten_tab_reports_nothing_more() {
        let mut reports = Reports::default();
        reports.committed(TabId::new(1), Some("https://a.example/"), None);

        reports.forget(TabId::new(1));
        reports.titled(TabId::new(1), Some("Late".into()));

        assert!(reports.is_empty());
        assert_eq!(reports.take(), []);
    }
}
