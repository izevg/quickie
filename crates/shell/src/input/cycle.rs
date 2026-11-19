//! `⌃⇥` as the user holds it: a [`TabCycle`] plus when `⌃` went down. A quick tap commits with
//! no UI; only a hold longer than [`REVEAL_AFTER`] shows the switcher (`docs/architecture.md`,
//! hot-path budgets).

use std::time::{Duration, Instant};

use quickie_domain_browsing::{Direction, TabCycle, WindowId};

/// The switcher appears only after `⌃` is held longer than this.
pub const REVEAL_AFTER: Duration = Duration::from_millis(150);

/// A cycle in progress in one window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleHold {
    cycle: TabCycle,
    window: WindowId,
    began: Instant,
    revealed: bool,
}

impl CycleHold {
    /// `⌃⇥` went down at `now` in `window`.
    #[must_use]
    pub fn new(cycle: TabCycle, window: WindowId, now: Instant) -> Self {
        Self {
            cycle,
            window,
            began: now,
            revealed: false,
        }
    }

    /// How long to wait before asking [`Self::reveal`] again: just past the threshold.
    #[must_use]
    pub fn reveal_delay() -> Duration {
        REVEAL_AFTER + Duration::from_millis(1)
    }

    /// True once, the first time it is asked after `⌃` has been held longer than
    /// [`REVEAL_AFTER`]: the moment to show the switcher.
    pub fn reveal(&mut self, now: Instant) -> bool {
        if self.revealed || now.saturating_duration_since(self.began) <= REVEAL_AFTER {
            return false;
        }
        self.revealed = true;
        true
    }

    /// True when the switcher is on screen.
    #[must_use]
    pub fn revealed(&self) -> bool {
        self.revealed
    }

    /// A further `⇥` while `⌃` is held.
    pub fn step(&mut self, direction: Direction) {
        self.cycle.step(direction);
    }

    /// The cycle, to show or to commit.
    #[must_use]
    pub fn cycle(&self) -> &TabCycle {
        &self.cycle
    }

    /// The window the cycle runs in.
    #[must_use]
    pub fn window(&self) -> WindowId {
        self.window
    }

    /// Position of the selection in [`TabCycle::order`].
    #[must_use]
    pub fn selected_index(&self) -> usize {
        let selected = self.cycle.selected();
        self.cycle
            .order()
            .iter()
            .position(|&tab| tab == selected)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use quickie_domain_browsing::{Direction, TabCycle, TabId, WindowId};

    use super::{CycleHold, REVEAL_AFTER};

    fn hold(at: Instant) -> CycleHold {
        let order = [TabId::new(3), TabId::new(1), TabId::new(2)];
        let cycle = TabCycle::begin(&order, Direction::Forward).unwrap();
        CycleHold::new(cycle, WindowId::new(7), at)
    }

    #[test]
    fn a_quick_tap_reveals_nothing() {
        let start = Instant::now();
        let mut hold = hold(start);

        assert!(!hold.reveal(start));
        assert!(!hold.reveal(start + Duration::from_millis(90)));
        assert!(!hold.revealed());
    }

    #[test]
    fn exactly_the_threshold_is_not_yet_a_hold() {
        let start = Instant::now();
        let mut hold = hold(start);

        assert!(!hold.reveal(start + REVEAL_AFTER));
    }

    #[test]
    fn a_hold_past_the_threshold_reveals_once() {
        let start = Instant::now();
        let mut hold = hold(start);
        let later = start + CycleHold::reveal_delay();

        assert!(hold.reveal(later));
        assert!(hold.revealed());
        assert!(!hold.reveal(later + Duration::from_millis(500)));
    }

    #[test]
    fn the_delay_lands_past_the_threshold() {
        assert!(CycleHold::reveal_delay() > REVEAL_AFTER);
        assert!(CycleHold::reveal_delay() < REVEAL_AFTER + Duration::from_millis(5));
    }

    #[test]
    fn a_clock_running_backwards_reveals_nothing() {
        let earlier = Instant::now();
        let mut hold = hold(earlier + Duration::from_secs(1));

        assert!(!hold.reveal(earlier));
    }

    #[test]
    fn steps_move_the_selection_through_the_fixed_order() {
        let mut hold = hold(Instant::now());

        assert_eq!(hold.selected_index(), 1);
        assert_eq!(hold.cycle().selected(), TabId::new(1));
        hold.step(Direction::Forward);
        assert_eq!(hold.selected_index(), 2);
        hold.step(Direction::Backward);
        hold.step(Direction::Backward);
        assert_eq!(hold.selected_index(), 0);
        assert_eq!(hold.window(), WindowId::new(7));
    }
}
