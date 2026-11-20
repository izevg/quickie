//! Closed tabs, kept for 30 days so that closing never loses a tab.

use std::collections::VecDeque;

use quickie_domain_kernel::Timestamp;

use crate::Tab;

/// How long a closed tab stays in the trash: 30 days. A tab closed exactly 30 days ago is purged.
pub const RETENTION_MS: u64 = 30 * Timestamp::DAY_MS;

/// A tab in the trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedTab {
    /// The tab as it was when closed, id included.
    pub tab: Tab,
    /// When it was closed.
    pub closed_at: Timestamp,
}

/// Closed tabs, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Trash(VecDeque<ClosedTab>);

impl Trash {
    pub(crate) fn from_oldest_first(entries: Vec<ClosedTab>) -> Self {
        Self(entries.into())
    }

    pub(crate) fn push(&mut self, entry: ClosedTab) {
        self.0.push_back(entry);
    }

    pub(crate) fn pop_latest(&mut self) -> Option<ClosedTab> {
        self.0.pop_back()
    }

    /// Removes and returns the entries whose retention is over. Checks every entry, so a clock
    /// that jumped backwards between closes cannot shield an old one.
    pub(crate) fn purge(&mut self, now: Timestamp) -> Vec<ClosedTab> {
        let (expired, kept) = std::mem::take(&mut self.0)
            .into_iter()
            .partition(|entry| now.millis_since(entry.closed_at) >= RETENTION_MS);
        self.0 = kept;
        expired.into()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &ClosedTab> {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::{ClosedTab, RETENTION_MS, Trash};
    use crate::{Jar, Profile, Tab, TabId, Timestamp};

    fn closed(id: u64, at: u64) -> ClosedTab {
        ClosedTab {
            tab: Tab {
                id: TabId::new(Profile::Main, id),
                url: "https://example.test/".parse().unwrap(),
                title: String::new(),
                jar: Jar::Main,
            },
            closed_at: Timestamp::from_millis(at),
        }
    }

    #[test]
    fn retention_is_thirty_days() {
        assert_eq!(RETENTION_MS, 2_592_000_000);
    }

    #[test]
    fn pop_latest_returns_the_last_closed() {
        let mut trash = Trash::default();
        trash.push(closed(1, 10));
        trash.push(closed(2, 20));

        assert_eq!(trash.pop_latest(), Some(closed(2, 20)));
        assert_eq!(trash.pop_latest(), Some(closed(1, 10)));
        assert_eq!(trash.pop_latest(), None);
    }

    #[test]
    fn purge_keeps_a_tab_until_the_last_millisecond() {
        let mut trash = Trash::from_oldest_first(vec![closed(1, 0)]);

        let purged = trash.purge(Timestamp::from_millis(RETENTION_MS - 1));

        assert_eq!(purged, Vec::new());
        assert_eq!(trash.iter().count(), 1);
    }

    #[test]
    fn purge_drops_a_tab_after_exactly_thirty_days() {
        let mut trash = Trash::from_oldest_first(vec![closed(1, 0)]);

        let purged = trash.purge(Timestamp::from_millis(RETENTION_MS));

        assert_eq!(purged, vec![closed(1, 0)]);
        assert_eq!(trash.iter().count(), 0);
    }

    #[test]
    fn purge_checks_every_entry_whatever_the_order() {
        let now = RETENTION_MS + 100;
        let mut trash =
            Trash::from_oldest_first(vec![closed(1, 150), closed(2, 0), closed(3, 200)]);

        let purged = trash.purge(Timestamp::from_millis(now));

        assert_eq!(purged, vec![closed(2, 0)]);
        assert_eq!(
            trash.iter().cloned().collect::<Vec<_>>(),
            vec![closed(1, 150), closed(3, 200)]
        );
    }
}
