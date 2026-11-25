//! Candidate links from a single evaluation of the current main document.

mod parse;
#[cfg(test)]
mod tests;

pub(crate) use parse::parse;

pub(crate) const COLLECT_SCRIPT: &str = include_str!("links.js");
