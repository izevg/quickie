//! Find in page: where the current match is among all matches of the query, as the find bar
//! says it ("3 of 12").

use std::fmt;

/// The matches of a find query on a page: how many, and which one is current (1-based).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindMatches {
    current: Option<usize>,
    total: usize,
}

impl FindMatches {
    /// No match at all.
    pub const NONE: Self = Self {
        current: None,
        total: 0,
    };

    /// `total` matches, the `current`th (counted from 1) selected. A current match outside
    /// `1..=total` is unknown: the page moved to a match that was not counted.
    #[must_use]
    pub fn new(current: Option<usize>, total: usize) -> Self {
        let current = current.filter(|&current| (1..=total).contains(&current));
        Self { current, total }
    }

    /// The selected match, counted from 1, when known.
    #[must_use]
    pub fn current(self) -> Option<usize> {
        self.current
    }

    /// How many matches the page has.
    #[must_use]
    pub fn total(self) -> usize {
        self.total
    }
}

impl fmt::Display for FindMatches {
    /// `3 of 12`; `12 matches` when the current one is unknown; `No matches`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.current, self.total) {
            (_, 0) => f.write_str("No matches"),
            (Some(current), total) => write!(f, "{current} of {total}"),
            (None, 1) => f.write_str("1 match"),
            (None, total) => write!(f, "{total} matches"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FindMatches;

    #[test]
    fn the_current_match_is_told_among_all() {
        let matches = FindMatches::new(Some(3), 12);

        assert_eq!(matches.current(), Some(3));
        assert_eq!(matches.total(), 12);
        assert_eq!(matches.to_string(), "3 of 12");
        assert_eq!(FindMatches::new(Some(1), 1).to_string(), "1 of 1");
        assert_eq!(FindMatches::new(Some(12), 12).to_string(), "12 of 12");
    }

    #[test]
    fn a_current_match_out_of_range_is_unknown() {
        assert_eq!(FindMatches::new(Some(0), 4).current(), None);
        assert_eq!(FindMatches::new(Some(5), 4).current(), None);
        assert_eq!(FindMatches::new(Some(5), 4).to_string(), "4 matches");
        assert_eq!(FindMatches::new(None, 1).to_string(), "1 match");
    }

    #[test]
    fn nothing_found_says_so() {
        assert_eq!(FindMatches::NONE, FindMatches::default());
        assert_eq!(FindMatches::NONE.to_string(), "No matches");
        assert_eq!(FindMatches::new(Some(1), 0).to_string(), "No matches");
        assert_eq!(FindMatches::new(Some(1), 0).current(), None);
    }
}
