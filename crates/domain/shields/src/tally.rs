//! How many requests each tab's page had blocked.

use std::collections::HashMap;

use quickie_domain_kernel::TabId;

/// Blocked requests per tab, for the origin chip. A count belongs to the page a tab shows: it
/// starts again at zero with each committed navigation. Kept in memory only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockTally {
    counts: HashMap<TabId, u32>,
}

impl BlockTally {
    /// No tab has blocked anything.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `blocked` requests to a tab's page; returns its new count, which stops at
    /// `u32::MAX`.
    pub fn record(&mut self, tab: TabId, blocked: u32) -> u32 {
        let count = self.counts.entry(tab).or_default();
        *count = count.saturating_add(blocked);
        *count
    }

    /// How many requests the page a tab shows had blocked.
    #[must_use]
    pub fn count(&self, tab: TabId) -> u32 {
        self.counts.get(&tab).copied().unwrap_or_default()
    }

    /// Forgets a tab's count: its page is gone, replaced by a new one or closed.
    pub fn forget(&mut self, tab: TabId) {
        self.counts.remove(&tab);
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_kernel::TabId;

    use super::BlockTally;

    #[test]
    fn a_tab_that_blocked_nothing_counts_zero() {
        let tally = BlockTally::new();

        assert_eq!(tally.count(TabId::new(1)), 0);
    }

    #[test]
    fn blocked_requests_add_up_per_tab() {
        let mut tally = BlockTally::new();
        tally.record(TabId::new(1), 2);

        let count = tally.record(TabId::new(1), 3);

        assert_eq!(count, 5);
        assert_eq!(tally.count(TabId::new(1)), 5);
        assert_eq!(tally.count(TabId::new(2)), 0);
    }

    #[test]
    fn a_count_stops_at_the_largest_number() {
        let mut tally = BlockTally::new();
        tally.record(TabId::new(1), u32::MAX);

        let count = tally.record(TabId::new(1), 1);

        assert_eq!(count, u32::MAX);
    }

    #[test]
    fn a_forgotten_tab_counts_from_zero_again() {
        let mut tally = BlockTally::new();
        tally.record(TabId::new(1), 4);
        tally.record(TabId::new(2), 1);

        tally.forget(TabId::new(1));

        assert_eq!(tally.count(TabId::new(1)), 0);
        assert_eq!(tally.count(TabId::new(2)), 1);
    }
}
