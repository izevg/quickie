//! The first stroke of a two-stroke chord while it waits for the second.
//!
//! A chord that waits forever turns the next unrelated stroke, minutes later, into "not bound".
//! So the first stroke waits at most [`WAIT`], and anything that moves the user's attention
//! drops it: a click, the window losing the keyboard, the app going to the background, a
//! full-screen transition, or a key the keymap cannot read.

use std::time::{Duration, Instant};

use quickie_domain_commands::KeyStroke;

/// How long the first stroke of a chord waits for the second.
pub const WAIT: Duration = Duration::from_secs(2);

/// The first stroke of a chord, if one waits, and since when.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PendingChord(Option<(KeyStroke, Instant)>);

impl PendingChord {
    /// `first` started a chord at `now`; it replaces any stroke that waited before.
    #[must_use]
    pub fn begin(first: KeyStroke, now: Instant) -> Self {
        Self(Some((first, now)))
    }

    /// The waiting stroke at `now`, `None` when none waits or it waited longer than [`WAIT`].
    #[must_use]
    pub fn first(self, now: Instant) -> Option<KeyStroke> {
        self.0
            .filter(|&(_, since)| now.saturating_duration_since(since) <= WAIT)
            .map(|(first, _)| first)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use quickie_domain_commands::KeyStroke;

    use super::{PendingChord, WAIT};

    fn stroke(text: &str) -> KeyStroke {
        KeyStroke::try_from(text).unwrap()
    }

    #[test]
    fn nothing_waits_at_first() {
        assert_eq!(PendingChord::default().first(Instant::now()), None);
    }

    #[test]
    fn the_first_stroke_waits_up_to_the_limit() {
        let start = Instant::now();
        let pending = PendingChord::begin(stroke("cmd+k"), start);

        assert_eq!(pending.first(start), Some(stroke("cmd+k")));
        assert_eq!(pending.first(start + WAIT), Some(stroke("cmd+k")));
        assert_eq!(pending.first(start + WAIT + Duration::from_millis(1)), None);
    }

    #[test]
    fn a_clock_that_seems_to_run_backwards_keeps_it() {
        let later = Instant::now() + Duration::from_secs(1);
        let pending = PendingChord::begin(stroke("cmd+k"), later);

        assert_eq!(pending.first(Instant::now()), Some(stroke("cmd+k")));
    }

    #[test]
    fn a_new_first_stroke_replaces_the_old_one() {
        let start = Instant::now();
        let _old = PendingChord::begin(stroke("cmd+k"), start);
        let new = PendingChord::begin(stroke("cmd+j"), start + Duration::from_secs(5));

        assert_eq!(
            new.first(start + Duration::from_secs(6)),
            Some(stroke("cmd+j"))
        );
    }
}
