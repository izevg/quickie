//! The one queue an engine adapter records its [`EngineEvent`]s in, until the shell drains it.
//! Pure, so every engine shares it and its folding is tested without one.
//!
//! Most events wait as they came. Navigation and loading fold per tab, so a page that commits,
//! retitles itself, and loads in many steps between two drains is reported once, where it first
//! changed: one visit (or one rename) with its latest title, and its latest progress.

use std::collections::HashMap;

use quickie_domain_browsing::TabId;
use url::Url;

use super::EngineEvent;

/// Engine events waiting for the shell, oldest first.
#[derive(Debug)]
pub struct EventQueue<P> {
    events: Vec<EngineEvent<P>>,
    /// The last committed URL and title of every live page: a title change is reported with it.
    current: HashMap<TabId, (Url, String)>,
}

impl<P> Default for EventQueue<P> {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            current: HashMap::new(),
        }
    }
}

impl<P> EventQueue<P> {
    /// Queues an event as it is. A navigation or loading report goes through
    /// [`Self::committed`], [`Self::titled`], or [`Self::loading`] instead, to fold.
    pub fn push(&mut self, event: EngineEvent<P>) {
        self.events.push(event);
    }

    /// A page committed a navigation to `url`. `title` is its title now; `None` keeps the one it
    /// had, for an engine that reports the title only once the page sets it. A commit not yet
    /// drained stays a commit, so a later title cannot turn the visit into a rename.
    pub fn committed(&mut self, tab: TabId, url: Url, title: Option<String>) {
        let title = match title {
            Some(title) => title,
            None => self
                .current
                .remove(&tab)
                .map_or_else(String::new, |(_, title)| title),
        };
        self.current.insert(tab, (url.clone(), title.clone()));
        let committed = EngineEvent::Committed { tab, url, title };
        match self.navigation_of(tab) {
            Some(waiting) => *waiting = committed,
            None => self.events.push(committed),
        }
    }

    /// A page's title changed. Before its first commit there is no visit to rename, and a title
    /// it already had is no news.
    pub fn titled(&mut self, tab: TabId, title: String) {
        let Some((url, current)) = self.current.get_mut(&tab) else {
            return;
        };
        if *current == title {
            return;
        }
        current.clone_from(&title);
        let url = url.clone();
        match self.navigation_of(tab) {
            Some(
                EngineEvent::Committed { title: waiting, .. }
                | EngineEvent::Titled { title: waiting, .. },
            ) => *waiting = title,
            _ => self.events.push(EngineEvent::Titled { tab, url, title }),
        }
    }

    /// A page's loading moved: `progress` is how much has loaded, `None` once it stopped.
    pub fn loading(&mut self, tab: TabId, progress: Option<f64>) {
        let waiting = self.events.iter_mut().find_map(|event| match event {
            EngineEvent::Loading {
                tab: loading,
                progress,
            } if *loading == tab => Some(progress),
            _ => None,
        });
        match waiting {
            Some(waiting) => *waiting = progress,
            None => self.events.push(EngineEvent::Loading { tab, progress }),
        }
    }

    /// The content blocker kept one of a page's loads from happening. Counts fold into the
    /// tab's waiting report, unless the tab committed a navigation since: those belong to the
    /// page before.
    pub fn blocked(&mut self, tab: TabId) {
        let waiting = self.events.iter_mut().rev().find_map(|event| match event {
            EngineEvent::Committed { tab: about, .. } if *about == tab => Some(None),
            EngineEvent::Blocked {
                tab: about,
                requests,
            } if *about == tab => Some(Some(requests)),
            _ => None,
        });
        match waiting.flatten() {
            Some(requests) => *requests = requests.saturating_add(1),
            None => self.events.push(EngineEvent::Blocked { tab, requests: 1 }),
        }
    }

    /// The tab's page is gone: its navigations, loading, and asks are dropped, and nothing it
    /// reports from now on is a visit. What answers a request, a download, or a page it opened
    /// still comes.
    pub fn forget(&mut self, tab: TabId) {
        self.current.remove(&tab);
        self.events.retain(|event| match event {
            EngineEvent::Committed { tab: about, .. }
            | EngineEvent::Titled { tab: about, .. }
            | EngineEvent::Loading { tab: about, .. }
            | EngineEvent::Blocked { tab: about, .. } => *about != tab,
            EngineEvent::Asked(ask) => ask.tab != tab,
            _ => true,
        });
    }

    /// True when nothing waits for the shell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Moves every waiting event to the end of `into`, oldest first.
    pub fn drain_into(&mut self, into: &mut Vec<EngineEvent<P>>) {
        into.append(&mut self.events);
    }

    /// The tab's navigation report that waits, if one does.
    fn navigation_of(&mut self, tab: TabId) -> Option<&mut EngineEvent<P>> {
        self.events.iter_mut().find(|event| {
            matches!(
                event,
                EngineEvent::Committed { tab: about, .. } | EngineEvent::Titled { tab: about, .. }
                    if *about == tab
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::{Profile, TabId};
    use url::Url;

    use super::EventQueue;
    use crate::ReplyToken;
    use crate::engine::{Ask, AskKind, Dialog, DownloadEvent, DownloadToken, EngineEvent};

    type Event = EngineEvent<u8>;

    fn tab(serial: u64) -> TabId {
        TabId::new(Profile::Main, serial)
    }

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    fn drained(queue: &mut EventQueue<u8>) -> Vec<Event> {
        let mut events = Vec::new();
        queue.drain_into(&mut events);
        events
    }

    fn committed(serial: u64, at: &str, title: &str) -> Event {
        EngineEvent::Committed {
            tab: tab(serial),
            url: url(at),
            title: title.to_owned(),
        }
    }

    fn titled(serial: u64, at: &str, title: &str) -> Event {
        EngineEvent::Titled {
            tab: tab(serial),
            url: url(at),
            title: title.to_owned(),
        }
    }

    fn ask(serial: u64) -> Event {
        EngineEvent::Asked(Ask {
            token: ReplyToken::new(serial),
            tab: tab(serial),
            origin: url("https://a.example/"),
            kind: AskKind::Dialog(Dialog::Alert {
                message: String::new(),
            }),
        })
    }

    #[test]
    fn events_wait_in_order_until_drained_into_what_the_shell_holds() {
        let mut queue = EventQueue::default();
        queue.push(EngineEvent::ClosedByPage { tab: tab(2) });
        queue.push(EngineEvent::Crashed { tab: tab(1) });
        let mut events = vec![EngineEvent::Pictured { tab: tab(9) }];

        assert!(!queue.is_empty());
        queue.drain_into(&mut events);

        assert!(queue.is_empty());
        assert_eq!(
            events,
            [
                EngineEvent::Pictured { tab: tab(9) },
                EngineEvent::ClosedByPage { tab: tab(2) },
                EngineEvent::Crashed { tab: tab(1) },
            ]
        );
        assert_eq!(drained(&mut queue), []);
    }

    #[test]
    fn a_commit_is_reported_once_with_its_title() {
        let mut queue = EventQueue::default();

        queue.committed(tab(1), url("https://a.example/"), Some("A".into()));

        assert_eq!(
            drained(&mut queue),
            [committed(1, "https://a.example/", "A")]
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn a_later_title_comes_with_the_waiting_commit() {
        let mut queue = EventQueue::default();
        queue.committed(tab(1), url("https://a.example/"), Some(String::new()));

        queue.titled(tab(1), "Loaded".into());

        assert_eq!(
            drained(&mut queue),
            [committed(1, "https://a.example/", "Loaded")]
        );
    }

    #[test]
    fn a_title_after_the_commit_was_drained_renames_the_visit() {
        let mut queue = EventQueue::default();
        queue.committed(tab(1), url("https://a.example/"), Some(String::new()));
        drained(&mut queue);

        queue.titled(tab(1), "Loaded".into());
        queue.titled(tab(1), "Later".into());

        assert_eq!(
            drained(&mut queue),
            [titled(1, "https://a.example/", "Later")]
        );
    }

    #[test]
    fn a_commit_after_a_waiting_rename_is_a_visit() {
        let mut queue = EventQueue::default();
        queue.committed(tab(1), url("https://a.example/"), Some(String::new()));
        drained(&mut queue);
        queue.titled(tab(1), "A".into());

        queue.committed(tab(1), url("https://b.example/"), Some("B".into()));

        assert_eq!(
            drained(&mut queue),
            [committed(1, "https://b.example/", "B")]
        );
    }

    #[test]
    fn a_commit_without_a_title_keeps_the_one_the_page_had() {
        let mut queue = EventQueue::default();
        queue.committed(tab(1), url("https://a.example/"), None);
        queue.titled(tab(1), "A".into());
        drained(&mut queue);

        queue.committed(tab(1), url("https://a.example/#part"), None);

        assert_eq!(
            drained(&mut queue),
            [committed(1, "https://a.example/#part", "A")]
        );
    }

    #[test]
    fn navigations_fold_into_one_per_tab_where_the_tab_first_changed() {
        let mut queue = EventQueue::default();
        queue.committed(tab(2), url("https://b.example/"), Some(String::new()));
        queue.push(EngineEvent::ClosedByPage { tab: tab(5) });
        queue.committed(tab(1), url("https://a.example/"), Some(String::new()));
        queue.committed(tab(2), url("https://c.example/"), Some("C".into()));

        assert_eq!(
            drained(&mut queue),
            [
                committed(2, "https://c.example/", "C"),
                EngineEvent::ClosedByPage { tab: tab(5) },
                committed(1, "https://a.example/", ""),
            ]
        );
    }

    #[test]
    fn a_title_before_any_commit_or_unchanged_is_not_news() {
        let mut queue = EventQueue::default();
        queue.titled(tab(1), "Early".into());
        assert!(queue.is_empty());

        queue.committed(tab(1), url("https://a.example/"), Some("A".into()));
        drained(&mut queue);
        queue.titled(tab(1), "A".into());

        assert!(queue.is_empty());
    }

    #[test]
    fn loading_folds_into_the_latest_progress_per_tab() {
        let mut queue = EventQueue::default();
        queue.loading(tab(1), Some(0.1));
        queue.loading(tab(2), Some(0.5));
        queue.loading(tab(1), Some(0.7));
        queue.loading(tab(1), None);

        assert_eq!(
            drained(&mut queue),
            [
                EngineEvent::Loading {
                    tab: tab(1),
                    progress: None
                },
                EngineEvent::Loading {
                    tab: tab(2),
                    progress: Some(0.5)
                },
            ]
        );
    }

    #[test]
    fn a_forgotten_tab_reports_no_navigation_loading_or_ask_and_no_later_title() {
        let mut queue = EventQueue::default();
        queue.committed(tab(1), url("https://a.example/"), Some(String::new()));
        queue.committed(tab(2), url("https://b.example/"), Some(String::new()));
        queue.titled(tab(2), "B".into());
        drained(&mut queue);
        queue.loading(tab(1), Some(0.5));
        queue.push(ask(1));
        queue.titled(tab(2), "B2".into());
        queue.loading(tab(2), None);
        queue.push(ask(2));
        queue.committed(tab(1), url("https://a.example/"), Some(String::new()));
        queue.push(EngineEvent::ClosedByPage { tab: tab(1) });

        queue.forget(tab(1));
        queue.titled(tab(1), "Late".into());

        assert_eq!(
            drained(&mut queue),
            [
                titled(2, "https://b.example/", "B2"),
                EngineEvent::Loading {
                    tab: tab(2),
                    progress: None
                },
                ask(2),
                EngineEvent::ClosedByPage { tab: tab(1) },
            ]
        );
    }

    fn blocked(serial: u64, requests: u32) -> Event {
        EngineEvent::Blocked {
            tab: tab(serial),
            requests,
        }
    }

    #[test]
    fn blocked_loads_fold_into_one_count_per_tab() {
        let mut queue = EventQueue::default();
        queue.blocked(tab(1));
        queue.blocked(tab(2));
        queue.push(ask(3));

        queue.blocked(tab(1));
        queue.blocked(tab(1));

        assert_eq!(drained(&mut queue), [blocked(1, 3), blocked(2, 1), ask(3)]);
    }

    #[test]
    fn loads_blocked_after_a_commit_count_for_the_new_page() {
        let mut queue = EventQueue::default();
        queue.blocked(tab(1));
        queue.committed(tab(1), url("https://a.example/"), Some(String::new()));
        queue.committed(tab(2), url("https://b.example/"), Some(String::new()));

        queue.blocked(tab(1));
        queue.blocked(tab(1));

        assert_eq!(
            drained(&mut queue),
            [
                blocked(1, 1),
                committed(1, "https://a.example/", ""),
                committed(2, "https://b.example/", ""),
                blocked(1, 2),
            ]
        );
    }

    #[test]
    fn a_blocked_count_stops_at_the_largest_number() {
        let mut queue = EventQueue::default();
        queue.push(blocked(1, u32::MAX));

        queue.blocked(tab(1));

        assert_eq!(drained(&mut queue), [blocked(1, u32::MAX)]);
    }

    #[test]
    fn a_forgotten_tab_reports_nothing_blocked() {
        let mut queue = EventQueue::default();
        queue.blocked(tab(1));
        queue.blocked(tab(2));

        queue.forget(tab(1));

        assert_eq!(drained(&mut queue), [blocked(2, 1)]);
    }

    #[test]
    fn forgetting_a_tab_keeps_what_answers_requests_and_downloads() {
        let mut queue = EventQueue::default();
        let download = EngineEvent::Download(DownloadEvent::Finished {
            token: DownloadToken::new(1),
        });
        queue.push(EngineEvent::ClosedByPage { tab: tab(1) });
        queue.push(EngineEvent::Download(DownloadEvent::Finished {
            token: DownloadToken::new(1),
        }));

        queue.forget(tab(1));

        assert_eq!(
            drained(&mut queue),
            [EngineEvent::ClosedByPage { tab: tab(1) }, download]
        );
    }
}
