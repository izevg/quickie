//! Time as the domain sees it: a value passed in by the caller, never read from a clock.

/// A point in time: milliseconds since the Unix epoch. The application layer reads the clock
/// through a port and hands the value to the domain, so the domain stays deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(u64);

impl Timestamp {
    /// Milliseconds in one day.
    pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;

    /// Wraps milliseconds since the Unix epoch.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Milliseconds since the Unix epoch.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// Milliseconds from `earlier` to `self`; zero if `earlier` is later.
    #[must_use]
    pub const fn millis_since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Timestamp;

    #[test]
    fn round_trips_millis() {
        assert_eq!(Timestamp::from_millis(1_234).as_millis(), 1_234);
    }

    #[test]
    fn measures_elapsed_time() {
        let earlier = Timestamp::from_millis(1_000);
        let later = Timestamp::from_millis(1_250);

        assert_eq!(later.millis_since(earlier), 250);
    }

    #[test]
    fn elapsed_time_is_never_negative() {
        let earlier = Timestamp::from_millis(1_000);
        let later = Timestamp::from_millis(1_250);

        assert_eq!(earlier.millis_since(later), 0);
    }

    #[test]
    fn a_day_is_86_400_000_ms() {
        assert_eq!(Timestamp::DAY_MS, 86_400_000);
    }
}
