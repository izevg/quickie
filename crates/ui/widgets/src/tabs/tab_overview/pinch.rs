//! The pinch that opens and closes the overview, as numbers: magnification deltas in, the
//! overview's progress (0 hidden, 1 open) out, 1:1 with the fingers; on release, whether it
//! settles open or closed and how fast it was moving. Pure, so the thresholds are tested
//! without a trackpad.

/// How far a pinch in on the page goes before the overview takes it: a smaller one stays the
/// page's.
pub const DEAD_ZONE: f64 = 0.04;
/// Magnification of a whole open or close: fingers 40 % closer.
pub const TRAVEL: f64 = 0.4;
/// A release at least this far opens; short of it, closes.
pub const COMMIT: f64 = 0.5;
/// A release moving at least this fast (progress per second) goes its way wherever it is.
pub const FLING: f64 = 1.5;

/// Weight of the newest sample in the smoothed velocity.
const SMOOTHING: f64 = 0.6;

/// What one magnify event did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Not the overview's (yet): the page keeps the event.
    Page,
    /// The overview follows the fingers to this progress.
    Follow(f64),
}

/// How a released pinch settles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settle {
    pub open: bool,
    /// Where the fingers left it.
    pub progress: f64,
    /// Progress per second at release, positive toward open.
    pub velocity: f64,
}

/// One pinch, from the first touch to release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pinch {
    /// Progress when the fingers came down: 0 on the page, 1 on the open overview, or
    /// wherever a settle was when the fingers caught it.
    from: f64,
    /// Magnification so far; negative is a pinch in.
    total: f64,
    progress: f64,
    velocity: f64,
    last: Option<f64>,
    engaged: bool,
}

impl Pinch {
    /// Fingers down with the overview at `from`.
    #[must_use]
    pub fn begin(from: f64) -> Self {
        let from = from.clamp(0.0, 1.0);
        Self {
            from,
            total: 0.0,
            progress: from,
            velocity: 0.0,
            last: None,
            engaged: from > 0.0,
        }
    }

    /// The fingers moved by `magnification` at `time` (seconds, the event's timestamp).
    pub fn change(&mut self, magnification: f64, time: f64) -> Step {
        self.total += magnification;
        if !self.engaged {
            if -self.total < DEAD_ZONE {
                return Step::Page;
            }
            self.engaged = true;
            self.total += DEAD_ZONE;
        }
        let progress = (self.from - self.total / TRAVEL).clamp(0.0, 1.0);
        if let Some(last) = self.last {
            let elapsed = time - last;
            if elapsed > 0.0 {
                let sample = (progress - self.progress) / elapsed;
                self.velocity = SMOOTHING * sample + (1.0 - SMOOTHING) * self.velocity;
            }
        }
        self.last = Some(time);
        self.progress = progress;
        Step::Follow(progress)
    }

    /// The fingers lifted; `None` when the pinch never reached the overview.
    #[must_use]
    pub fn end(self) -> Option<Settle> {
        if !self.engaged {
            return None;
        }
        let open = if self.velocity.abs() >= FLING {
            self.velocity > 0.0
        } else {
            self.progress >= COMMIT
        };
        Some(Settle {
            open,
            progress: self.progress,
            velocity: self.velocity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{COMMIT, DEAD_ZONE, FLING, Pinch, Settle, Step, TRAVEL};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn follows(step: Step, expected: f64) -> bool {
        matches!(step, Step::Follow(p) if close(p, expected))
    }

    #[test]
    fn a_small_pinch_in_or_any_pinch_out_stays_with_the_page() {
        let mut pinch = Pinch::begin(0.0);

        assert_eq!(pinch.change(-DEAD_ZONE / 2.0, 0.0), Step::Page);
        assert_eq!(pinch.change(0.3, 0.01), Step::Page);
        assert_eq!(pinch.end(), None);
    }

    #[test]
    fn past_the_dead_zone_the_overview_follows_one_to_one() {
        let mut pinch = Pinch::begin(0.0);

        assert!(follows(pinch.change(-DEAD_ZONE, 0.0), 0.0));
        assert!(follows(pinch.change(-TRAVEL / 4.0, 0.01), 0.25));
        assert!(follows(pinch.change(TRAVEL / 8.0, 0.02), 0.125));
        assert!(follows(pinch.change(-TRAVEL * 5.0, 0.03), 1.0), "clamped");
    }

    #[test]
    fn a_slow_release_settles_by_position() {
        let mut far = Pinch::begin(0.0);
        let mut near = Pinch::begin(0.0);
        let at = |p: f64| -DEAD_ZONE - p * TRAVEL;
        // Steps of 0.1 progress over a second each: 0.1 per second, no fling.
        for (index, step) in [0.1, 0.1, 0.1, 0.1, 0.1, 0.1].into_iter().enumerate() {
            let first = if index == 0 { at(step) } else { -step * TRAVEL };
            let time = f64::from(u32::try_from(index).unwrap());
            let _ = far.change(first, time);
            if index < 4 {
                let _ = near.change(first, time);
            }
        }

        let far = far.end().unwrap();
        assert!(far.open);
        assert!(close(far.progress, 0.6));
        assert!(far.velocity.abs() < FLING);
        let near = near.end().unwrap();
        assert!(!near.open);
        assert!(near.progress < COMMIT);
    }

    #[test]
    fn exactly_the_commit_point_opens() {
        let mut pinch = Pinch::begin(0.0);
        let _ = pinch.change(-DEAD_ZONE, 0.0);
        let _ = pinch.change(-COMMIT * TRAVEL, 0.0);

        assert_eq!(
            pinch.end(),
            Some(Settle {
                open: true,
                progress: COMMIT,
                velocity: 0.0,
            })
        );
    }

    #[test]
    fn a_fling_wins_over_position() {
        let mut flung_open = Pinch::begin(0.0);
        let _ = flung_open.change(-DEAD_ZONE, 0.0);
        let _ = flung_open.change(-0.1 * TRAVEL, 0.02);
        let open = flung_open.end().unwrap();
        assert!(open.open, "0.1 in 20 ms is 5 per second");
        assert!(open.velocity >= FLING);

        let mut flung_shut = Pinch::begin(1.0);
        let _ = flung_shut.change(0.0, 0.0);
        let _ = flung_shut.change(0.1 * TRAVEL, 0.02);
        let shut = flung_shut.end().unwrap();
        assert!(
            !shut.open,
            "0.9 is past the commit point, but moving out fast"
        );
        assert!(close(shut.progress, 0.9));
    }

    #[test]
    fn the_open_overview_follows_a_pinch_out_without_a_dead_zone() {
        let mut pinch = Pinch::begin(1.0);

        assert!(follows(pinch.change(0.01, 0.0), 1.0 - 0.01 / TRAVEL));
        assert!(follows(pinch.change(-0.5, 0.01), 1.0), "clamped");
    }

    #[test]
    fn a_settle_caught_midway_starts_where_it_was() {
        let mut pinch = Pinch::begin(0.4);

        assert!(follows(pinch.change(0.0, 0.0), 0.4));
        assert!(close(Pinch::begin(7.0).end().unwrap().progress, 1.0));
        assert_eq!(Pinch::begin(-1.0).end(), None);
    }

    #[test]
    fn velocity_ignores_events_with_no_time_between_them() {
        let mut pinch = Pinch::begin(1.0);
        let _ = pinch.change(0.0, 1.0);
        let _ = pinch.change(0.04, 1.0);

        let settle = pinch.end().unwrap();
        assert!(close(settle.velocity, 0.0));
        assert!(settle.open);
    }

    #[test]
    fn velocity_is_smoothed_over_samples() {
        let mut pinch = Pinch::begin(1.0);
        let _ = pinch.change(0.0, 0.0);
        let _ = pinch.change(0.04, 0.1);
        // One sample of -1 per second, weighted 0.6 against rest.
        assert!(close(pinch.end().unwrap().velocity, -0.6));
        let _ = pinch.change(0.04, 0.2);
        // A second: 0.6 × -1 + 0.4 × -0.6.
        assert!(close(pinch.end().unwrap().velocity, -0.84));
    }
}
