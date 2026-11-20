//! `⌃⇥` switching: a cursor over a snapshot of the most-recently-used order.
//!
//! The snapshot is taken when `⌃` goes down and stays fixed while it is held, so fast presses
//! never see a half-updated order (the Vivaldi bug). Releasing `⌃` commits the selection through
//! [`crate::Session::activate`]; a quick tap is `begin` followed by that commit.
//!
//! While `⌃` is held, typing narrows the tabs `⇥` visits to those a filter keeps
//! ([`TabCycle::narrow`]), and a tab closed from the switcher leaves the snapshot
//! ([`TabCycle::remove`]). Neither reorders it.

use std::cmp::Ordering;

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
    /// The positions in `order` that `⇥` visits, ascending: all of them, or those the filter
    /// keeps.
    shown: Vec<usize>,
    /// The selection, a position in `order` that is in `shown`; `None` when nothing is shown.
    cursor: Option<usize>,
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
            shown: (0..mru.len()).collect(),
            cursor: Some(cursor),
        })
    }

    /// Moves the selection one shown tab, wrapping at both ends.
    pub fn step(&mut self, direction: Direction) {
        let Some(at) = self
            .cursor
            .and_then(|cursor| self.shown.iter().position(|&shown| shown == cursor))
        else {
            return;
        };
        let len = self.shown.len();
        let next = match direction {
            Direction::Forward => (at + 1) % len,
            Direction::Backward => (at + len - 1) % len,
        };
        self.cursor = Some(self.shown[next]);
    }

    /// Lets `⇥` visit only the positions in `kept`, as a filter decided; positions past the
    /// order are ignored. A selection that is kept stays; otherwise the first kept tab behind
    /// the active one is selected, or the active one when it is all that is kept.
    pub fn narrow(&mut self, kept: &[usize]) {
        let mut shown: Vec<usize> = kept
            .iter()
            .copied()
            .filter(|&position| position < self.order.len())
            .collect();
        shown.sort_unstable();
        shown.dedup();
        if !self.cursor.is_some_and(|cursor| shown.contains(&cursor)) {
            self.cursor = shown
                .iter()
                .copied()
                .find(|&position| position > 0)
                .or_else(|| shown.first().copied());
        }
        self.shown = shown;
    }

    /// Takes `tab`, closed while the cycle runs, out of the order. When it was selected, the
    /// next shown tab is, wrapping to the first.
    pub fn remove(&mut self, tab: TabId) {
        let Some(gone) = self.order.iter().position(|&id| id == tab) else {
            return;
        };
        self.order.remove(gone);
        let at = self.shown.iter().position(|&shown| shown == gone);
        self.shown = self
            .shown
            .iter()
            .filter_map(|&shown| shift(shown, gone))
            .collect();
        self.cursor = self.cursor.and_then(|cursor| match shift(cursor, gone) {
            Some(cursor) => Some(cursor),
            None => self.shown.get(at?).or_else(|| self.shown.first()).copied(),
        });
    }

    /// The selected tab; `None` when the filter keeps nothing.
    #[must_use]
    pub fn selected(&self) -> Option<TabId> {
        self.cursor.map(|cursor| self.order[cursor])
    }

    /// The position of the selected tab in [`Self::order`], for the switcher to highlight.
    #[must_use]
    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    /// The order the switcher shows, fixed for the whole cycle but for tabs closed in it.
    #[must_use]
    pub fn order(&self) -> &[TabId] {
        &self.order
    }

    /// The positions in [`Self::order`] that `⇥` visits, ascending.
    #[must_use]
    pub fn shown(&self) -> &[usize] {
        &self.shown
    }
}

/// Where `position` is once the tab at `gone` left the order; `None` for the tab that left.
fn shift(position: usize, gone: usize) -> Option<usize> {
    match position.cmp(&gone) {
        Ordering::Less => Some(position),
        Ordering::Equal => None,
        Ordering::Greater => Some(position - 1),
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

    /// A forward cycle over five tabs, `[5, 4, 3, 2, 1]`, on `4`.
    fn five() -> TabCycle {
        TabCycle::begin(&ids(&[5, 4, 3, 2, 1]), Direction::Forward).unwrap()
    }

    #[test]
    fn needs_another_tab() {
        assert_eq!(TabCycle::begin(&[], Direction::Forward), None);
        assert_eq!(TabCycle::begin(&ids(&[1]), Direction::Forward), None);
    }

    #[test]
    fn two_tabs_are_enough() {
        let cycle = TabCycle::begin(&ids(&[2, 1]), Direction::Forward).unwrap();

        assert_eq!(cycle.selected(), Some(id(1)));
    }

    #[test]
    fn a_quick_tap_selects_the_previous_tab() {
        let cycle = TabCycle::begin(&ids(&[3, 1, 2]), Direction::Forward).unwrap();

        assert_eq!(cycle.selected(), Some(id(1)));
        assert_eq!(cycle.order(), ids(&[3, 1, 2]).as_slice());
        assert_eq!(cycle.shown(), [0, 1, 2]);
    }

    #[test]
    fn backward_starts_at_the_least_recent() {
        let cycle = TabCycle::begin(&ids(&[3, 1, 2]), Direction::Backward).unwrap();

        assert_eq!(cycle.selected(), Some(id(2)));
    }

    #[test]
    fn the_cursor_is_the_selections_position_in_the_order() {
        let mut cycle = TabCycle::begin(&ids(&[3, 1, 2]), Direction::Forward).unwrap();
        let first = cycle.cursor();

        cycle.step(Direction::Forward);

        assert_eq!(first, Some(1));
        assert_eq!(cycle.cursor(), Some(2));
        assert_eq!(cycle.selected(), Some(id(2)));
    }

    #[test]
    fn forward_wraps_to_the_active_tab() {
        let mut cycle = TabCycle::begin(&ids(&[3, 1, 2]), Direction::Forward).unwrap();

        cycle.step(Direction::Forward);
        let at_end = cycle.selected();
        cycle.step(Direction::Forward);

        assert_eq!(at_end, Some(id(2)));
        assert_eq!(cycle.selected(), Some(id(3)));
    }

    #[test]
    fn backward_wraps_to_the_least_recent() {
        let mut cycle = TabCycle::begin(&ids(&[3, 1, 2]), Direction::Forward).unwrap();

        cycle.step(Direction::Backward);
        let at_start = cycle.selected();
        cycle.step(Direction::Backward);

        assert_eq!(at_start, Some(id(3)));
        assert_eq!(cycle.selected(), Some(id(2)));
    }

    #[test]
    fn a_kept_selection_stays_and_steps_skip_what_the_filter_hides() {
        let mut cycle = five();

        cycle.narrow(&[3, 1]);

        assert_eq!(cycle.shown(), [1, 3]);
        assert_eq!(cycle.cursor(), Some(1));
        cycle.step(Direction::Forward);
        assert_eq!(cycle.cursor(), Some(3));
        cycle.step(Direction::Forward);
        assert_eq!(cycle.cursor(), Some(1));
        cycle.step(Direction::Backward);
        assert_eq!(cycle.cursor(), Some(3));
        assert_eq!(cycle.order(), ids(&[5, 4, 3, 2, 1]).as_slice());
    }

    #[test]
    fn a_hidden_selection_moves_to_the_first_kept_tab_behind_the_active_one() {
        let mut cycle = five();

        cycle.narrow(&[0, 4, 2]);

        assert_eq!(cycle.selected(), Some(id(3)));
    }

    #[test]
    fn the_active_tab_is_selected_when_it_alone_is_kept() {
        let mut cycle = five();

        cycle.narrow(&[0]);

        assert_eq!(cycle.selected(), Some(id(5)));
        cycle.step(Direction::Forward);
        assert_eq!(cycle.selected(), Some(id(5)));
    }

    #[test]
    fn nothing_kept_selects_nothing_and_steps_do_nothing() {
        let mut cycle = five();

        cycle.narrow(&[]);
        cycle.step(Direction::Forward);
        cycle.step(Direction::Backward);

        assert_eq!(cycle.selected(), None);
        assert_eq!(cycle.cursor(), None);
        assert_eq!(cycle.shown(), Vec::<usize>::new());
    }

    #[test]
    fn widening_again_keeps_the_selection() {
        let mut cycle = five();
        cycle.narrow(&[3]);

        cycle.narrow(&[0, 1, 2, 3, 4]);
        cycle.step(Direction::Forward);

        assert_eq!(cycle.selected(), Some(id(1)));
    }

    #[test]
    fn kept_positions_are_read_in_order_once_and_only_inside_the_order() {
        let mut cycle = five();

        cycle.narrow(&[4, 2, 4, 9, 5]);

        assert_eq!(cycle.shown(), [2, 4]);
        assert_eq!(cycle.cursor(), Some(2));
        cycle.step(Direction::Forward);
        assert_eq!(cycle.cursor(), Some(4));
        cycle.step(Direction::Forward);
        assert_eq!(cycle.cursor(), Some(2));
    }

    #[test]
    fn closing_the_selection_selects_the_next_tab() {
        let mut cycle = five();

        cycle.remove(id(4));

        assert_eq!(cycle.order(), ids(&[5, 3, 2, 1]).as_slice());
        assert_eq!(cycle.shown(), [0, 1, 2, 3]);
        assert_eq!(cycle.selected(), Some(id(3)));
    }

    #[test]
    fn closing_the_last_selected_tab_wraps_to_the_first_shown() {
        let mut cycle = five();
        cycle.narrow(&[1, 4]);
        cycle.step(Direction::Forward);

        cycle.remove(id(1));

        assert_eq!(cycle.shown(), [1]);
        assert_eq!(cycle.selected(), Some(id(4)));
    }

    #[test]
    fn closing_a_tab_before_the_selection_keeps_it() {
        let mut cycle = five();
        cycle.step(Direction::Forward);

        cycle.remove(id(5));

        assert_eq!(cycle.selected(), Some(id(3)));
        assert_eq!(cycle.cursor(), Some(1));
    }

    #[test]
    fn closing_a_tab_after_the_selection_keeps_it() {
        let mut cycle = five();

        cycle.remove(id(2));

        assert_eq!(cycle.selected(), Some(id(4)));
        assert_eq!(cycle.cursor(), Some(1));
        assert_eq!(cycle.shown(), [0, 1, 2, 3]);
    }

    #[test]
    fn closing_a_hidden_tab_shifts_the_shown_ones() {
        let mut cycle = five();
        cycle.narrow(&[1, 3]);

        cycle.remove(id(3));

        assert_eq!(cycle.shown(), [1, 2]);
        assert_eq!(cycle.selected(), Some(id(4)));
        cycle.step(Direction::Forward);
        assert_eq!(cycle.selected(), Some(id(2)));
    }

    #[test]
    fn closing_the_only_shown_tab_selects_nothing() {
        let mut cycle = five();
        cycle.narrow(&[1]);

        cycle.remove(id(4));

        assert_eq!(cycle.selected(), None);
        assert_eq!(cycle.order().len(), 4);
    }

    #[test]
    fn closing_a_tab_outside_the_cycle_changes_nothing() {
        let mut cycle = five();
        let before = cycle.clone();

        cycle.remove(id(9));

        assert_eq!(cycle, before);
    }
}
