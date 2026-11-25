//! Serializes find requests because GTK's find signals do not identify their query.
//!
//! Keep the pending request across navigation and close. `WebKit`'s default find path replies
//! even when IPC is cancelled, so a late result cannot answer the next query. The optional
//! Site Isolation engine path can lose that signal when the main and focused frames disappear.

use std::ffi::{CString, c_uint};

use quickie_domain_browsing::FindMatches;

pub(super) const MATCH_LIMIT: c_uint = 999;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Previous,
    Next,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Request {
    pub(super) query: CString,
    pub(super) direction: Option<Direction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Count {
    Known(FindMatches),
    Overflow,
}

impl Count {
    pub(super) fn can_step(self) -> bool {
        self != Self::Known(FindMatches::NONE)
    }

    fn from_native(count: c_uint) -> Self {
        if count == c_uint::MAX {
            return Self::Overflow;
        }
        let total = count as usize;
        Self::Known(FindMatches::new(None, total))
    }
}

#[derive(Default)]
pub(super) struct State {
    query: Option<CString>,
    generation: u64,
    flight: Option<u64>,
    count: Option<Count>,
    queued_steps: i32,
}

impl State {
    pub(super) fn set_query(&mut self, query: Option<CString>) {
        if self.query != query {
            self.generation = self.generation.wrapping_add(1);
            self.query = query;
            self.count = None;
            self.queued_steps = 0;
        }
    }

    pub(super) fn count(&self) -> Option<Count> {
        self.count
    }

    pub(super) fn queue_step(&mut self, direction: Direction) {
        if self.query.is_some() {
            let offset = if direction == Direction::Next { 1 } else { -1 };
            self.queued_steps = self.queued_steps.saturating_add(offset);
        }
    }

    pub(super) fn take_request(&mut self) -> Option<Request> {
        if self.flight.is_some() {
            return None;
        }
        let query = self.query.as_ref()?;
        let direction = if self.count.is_none() {
            None
        } else {
            let direction = match self.queued_steps.cmp(&0) {
                std::cmp::Ordering::Less => Direction::Previous,
                std::cmp::Ordering::Greater => Direction::Next,
                std::cmp::Ordering::Equal => return None,
            };
            self.queued_steps -= self.queued_steps.signum();
            Some(direction)
        };
        self.flight = Some(self.generation);
        Some(Request {
            query: query.clone(),
            direction,
        })
    }

    pub(super) fn finish(&mut self, matches: c_uint) {
        let Some(generation) = self.flight.take() else {
            return;
        };
        if generation != self.generation {
            return;
        }
        if matches == 0 {
            self.queued_steps = 0;
        }
        self.count = Some(Count::from_native(matches));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(query: &std::ffi::CStr, direction: Option<Direction>) -> Request {
        Request {
            query: query.to_owned(),
            direction,
        }
    }

    #[test]
    fn only_the_latest_query_updates_the_count() {
        let mut state = State::default();
        state.set_query(Some(c"sovereign".to_owned()));
        assert_eq!(state.take_request(), Some(request(c"sovereign", None)));

        state.set_query(Some(c"sovereignty".to_owned()));
        assert_eq!(state.take_request(), None);
        state.finish(4);

        assert_eq!(state.count(), None);
        assert_eq!(state.take_request(), Some(request(c"sovereignty", None)));
        state.finish(0);
        assert_eq!(state.count(), Some(Count::Known(FindMatches::NONE)));
    }

    #[test]
    fn each_step_reports_the_new_native_total_without_an_ordinal() {
        let mut state = State::default();
        state.set_query(Some(c"sovereign".to_owned()));
        state.take_request();
        state.finish(4);

        state.queue_step(Direction::Previous);
        assert_eq!(
            state.take_request(),
            Some(request(c"sovereign", Some(Direction::Previous)))
        );
        state.finish(5);

        assert_eq!(state.count(), Some(Count::Known(FindMatches::new(None, 5))));
        state.queue_step(Direction::Next);
        assert_eq!(
            state.take_request(),
            Some(request(c"sovereign", Some(Direction::Next)))
        );
        state.finish(4);
        assert_eq!(state.count(), Some(Count::Known(FindMatches::new(None, 4))));
    }

    #[test]
    fn repeated_steps_wait_for_the_previous_result() {
        let mut state = State::default();
        state.set_query(Some(c"sovereign".to_owned()));
        state.take_request();
        state.finish(4);

        state.queue_step(Direction::Next);
        state.queue_step(Direction::Next);
        assert_eq!(
            state.take_request(),
            Some(request(c"sovereign", Some(Direction::Next)))
        );
        assert_eq!(state.take_request(), None);
        state.finish(4);

        assert_eq!(
            state.take_request(),
            Some(request(c"sovereign", Some(Direction::Next)))
        );
        state.finish(4);
        assert_eq!(state.count(), Some(Count::Known(FindMatches::new(None, 4))));
    }

    #[test]
    fn closing_drops_the_count_and_ignores_outstanding_results() {
        let mut state = State::default();
        state.set_query(Some(c"sovereign".to_owned()));
        state.take_request();
        state.queue_step(Direction::Next);

        state.set_query(None);
        state.finish(4);

        assert_eq!(state.count(), None);
        assert_eq!(state.take_request(), None);
    }

    #[test]
    fn an_engine_overflow_keeps_navigation_until_a_known_total_arrives() {
        let mut state = State::default();
        state.set_query(Some(c"or".to_owned()));
        state.take_request();
        state.finish(c_uint::MAX);
        assert_eq!(state.count(), Some(Count::Overflow));

        state.queue_step(Direction::Next);
        state.take_request();
        state.finish(1);

        assert_eq!(state.count(), Some(Count::Known(FindMatches::new(None, 1))));
    }
}
