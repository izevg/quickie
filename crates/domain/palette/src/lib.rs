//! Palette context of the Quickie domain: what the user typed, what could answer it, and in which
//! order. Pure: no I/O, no clock, no threads; the caller passes `now` in.
//!
//! The first character of the input picks a [`Mode`] ([`Query::parse`]); the caller gathers
//! [`Candidate`]s under its own keys, each with its [`Frecency`]; [`rank`] returns the best
//! [`Ranked`] few with what to highlight. Ranking runs within the palette's first-frame budget
//! for ten thousand candidates.

mod candidate;
mod query;
mod rank;

pub use candidate::{Candidate, Frecency, FrecencyError, Kind};
pub use query::{KeywordSearch, Mode, Query};
pub use rank::{Ranked, rank};
