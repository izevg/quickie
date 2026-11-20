//! Palette context of the Quickie domain: what the user typed, what could answer it, and in which
//! order. Pure: no I/O, no clock, no threads; the caller passes `now` in.
//!
//! The first character of the input picks a [`Mode`] ([`Query::parse`]); the caller gathers
//! [`Candidate`]s under its own keys, scoring each one's [`Frecency`] once; a [`Ranker`] returns
//! the best [`Ranked`] few with what to highlight, keystroke after keystroke. Ranking runs within
//! the palette's first-frame budget for fifty thousand candidates.

mod candidate;
mod query;
mod rank;

pub use candidate::{Candidate, Frecency, FrecencyError, Kind, Text, TextSpan, Texts};
pub use query::{KeywordSearch, Mode, Query};
pub use rank::{Ranked, Ranker};
