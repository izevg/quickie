//! `⌃⇥` switching: a cursor over a snapshot of the most-recently-used order.
//!
//! The snapshot is taken when `⌃` goes down and stays fixed while it is held, so fast presses
//! never see a half-updated order (the Vivaldi bug). Releasing `⌃` commits the selection through
//! [`crate::Session::activate`]; a quick tap is `begin` followed by that commit.

use quickie_domain_kernel::TabId;

/// Which way `⇥` moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// `⌃⇥`: towards less recently used tabs.
    Forward,
    /// `⌃⇧⇥`: towards more recently used tabs, wrapping to the least recent.
    Backward,
}

/// A `⌃⇥` cycle in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabCycle {
    order: Vec<TabId>,
    cursor: usize,
}

impl TabCycle {
    /// Starts a cycle over a window's MRU order, front = active. Forward selects the previous
    /// tab, backward the least recent. `None` when there is no other tab to go to.
    #[must_use]
    pub fn begin(mru: &[TabId], direction: Direction) -> Option<Self> {
        if mru.len() < 2 {
            return None;
        }
        let cursor = match direction {
            Direction::Forward => 1,
            Direction::Backward => mru.len() - 1,
        };
        Some(Self {
            order: mru.to_vec(),
            cursor,
        })
    }

    /// Moves the selection one tab, wrapping at both ends.
    pub fn step(&mut self, direction: Direction) {
        let len = self.order.len();
        self.cursor = match direction {
            Direction::Forward => (self.cursor + 1) % len,
            Direction::Backward => (self.cursor + len - 1) % len,
        };
    }

    /// The selected tab.
    #[must_use]
    pub fn selected(&self) -> TabId {
        self.order[self.cursor]
    }

    /// The order the switcher shows, fixed for the whole cycle.
    #[must_use]
    pub fn order(&self) -> &[TabId] {
        &self.order
    }
}

#[cfg(test)]
mod tests {
    use super::{Direction, TabCycle};
    use quickie_domain_kernel::{Profile, TabId};

    fn id(serial: u64) -> TabId {
        TabId::new(Profile::Main, serial)
    }

    fn ids(raw: &[u64]) -> Vec<TabId> {
        raw.iter().copied().map(id).collect()
    }

    #[test]
    fn needs_another_tab() {
        assert_eq!(TabCycle::begin(&[], Direction::Forward), None);
        assert_eq!(TabCycle::begin(&ids(&[1]), Direction::Forward), None);
    }

    #[test]
    fn two_tabs_are_enough() {
        let cycle = TabCycle::begin(&ids(&[2, 1]), Direction::Forward).unwrap();

        assert_eq!(cycle.selected(), id(1));
    }

    #[test]
    fn a_quick_tap_selects_the_previous_tab() {
        let cycle = TabCycle::begin(&ids(&[3, 1, 2]), Direction::Forward).unwrap();

        assert_eq!(cycle.selected(), id(1));
        assert_eq!(cycle.order(), ids(&[3, 1, 2]).as_slice());
    }

    #[test]
    fn backward_starts_at_the_least_recent() {
        let cycle = TabCycle::begin(&ids(&[3, 1, 2]), Direction::Backward).unwrap();

        assert_eq!(cycle.selected(), id(2));
    }

    #[test]
    fn forward_wraps_to_the_active_tab() {
        let mut cycle = TabCycle::begin(&ids(&[3, 1, 2]), Direction::Forward).unwrap();

        cycle.step(Direction::Forward);
        let at_end = cycle.selected();
        cycle.step(Direction::Forward);

        assert_eq!(at_end, id(2));
        assert_eq!(cycle.selected(), id(3));
    }

    #[test]
    fn backward_wraps_to_the_least_recent() {
        let mut cycle = TabCycle::begin(&ids(&[3, 1, 2]), Direction::Forward).unwrap();

        cycle.step(Direction::Backward);
        let at_start = cycle.selected();
        cycle.step(Direction::Backward);

        assert_eq!(at_start, id(3));
        assert_eq!(cycle.selected(), id(2));
    }
}
