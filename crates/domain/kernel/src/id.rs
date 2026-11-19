//! Identifiers shared across bounded contexts.

use std::fmt;

/// Identifies a tab for its whole life, across restarts and the trash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabId(u64);

impl TabId {
    /// Wraps a raw value, as read from storage or allocated by a sequence.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value, for storage.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TabId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tab#{}", self.0)
    }
}

/// Identifies a browser window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(u64);

impl WindowId {
    /// Wraps a raw value, as read from storage or allocated by a sequence.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value, for storage.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "window#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{TabId, WindowId};

    #[test]
    fn tab_id_round_trips_and_displays() {
        let id = TabId::new(42);

        assert_eq!(id.get(), 42);
        assert_eq!(id.to_string(), "tab#42");
    }

    #[test]
    fn window_id_round_trips_and_displays() {
        let id = WindowId::new(7);

        assert_eq!(id.get(), 7);
        assert_eq!(id.to_string(), "window#7");
    }
}
