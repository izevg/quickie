//! Ranking: which candidates answer a query, best first.

use std::cmp::Ordering;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use quickie_domain_kernel::Timestamp;

use crate::{Candidate, Kind, Query};

/// A candidate that answers the query, with what to highlight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ranked<K> {
    /// The caller's key of the candidate.
    pub key: K,
    /// Higher is better; only comparable within one ranking.
    pub score: u32,
    /// Character positions in the title that the query matched, ascending.
    pub title_matches: Vec<u32>,
    /// Character positions in the detail that the query matched, ascending.
    pub detail_matches: Vec<u32>,
}

/// Points per doubling of frecency. The logarithm keeps a page visited every day from burying a
/// clearly better match.
const FRECENCY_WEIGHT: u32 = 4;

/// Ranks the candidates that answer `query`, best first, and keeps at most `limit` of them.
///
/// A candidate takes part only if the query's mode admits its kind, and, unless the query text
/// is empty, only if the text fuzzily matches its title or its detail. Its score is
///
/// `best fuzzy score of title and detail + 4 × log2(frecency + 1) + kind boost`,
///
/// where an open tab gets 40 and a bookmark 20. Equal scores fall back to the title, then to the
/// input order, so the result never depends on the sort algorithm. Only the kept candidates
/// carry match positions, and only `limit` of them are fully sorted.
#[must_use]
pub fn rank<K: Clone>(
    query: &Query,
    candidates: &[Candidate<K>],
    now: Timestamp,
    limit: usize,
) -> Vec<Ranked<K>> {
    let mode = query.mode();
    let pattern = Pattern::new(
        query.needle(),
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut chars = Vec::new();

    let mut hits: Vec<Hit<'_, K>> = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| mode.admits(candidate.kind))
        .filter_map(|(position, candidate)| {
            let title = pattern.score(Utf32Str::new(&candidate.title, &mut chars), &mut matcher);
            let detail = pattern.score(Utf32Str::new(&candidate.detail, &mut chars), &mut matcher);
            let fuzzy = title.max(detail)?;
            Some(Hit {
                score: fuzzy + frecency_bonus(candidate.visits.score(now)) + boost(candidate.kind),
                position,
                candidate,
            })
        })
        .collect();

    if hits.len() > limit {
        hits.select_nth_unstable_by(limit, Hit::order);
        hits.truncate(limit);
    }
    hits.sort_unstable_by(Hit::order);

    hits.into_iter()
        .map(|hit| Ranked {
            key: hit.candidate.key.clone(),
            score: hit.score,
            title_matches: matches(&pattern, &hit.candidate.title, &mut matcher),
            detail_matches: matches(&pattern, &hit.candidate.detail, &mut matcher),
        })
        .collect()
}

/// A candidate that passed the filter, before it is kept or dropped.
struct Hit<'a, K> {
    score: u32,
    position: usize,
    candidate: &'a Candidate<K>,
}

impl<K> Hit<'_, K> {
    /// Best first: higher score, then title, then earlier input. Total, since positions differ.
    fn order(&self, other: &Self) -> Ordering {
        other
            .score
            .cmp(&self.score)
            .then_with(|| self.candidate.title.cmp(&other.candidate.title))
            .then_with(|| self.position.cmp(&other.position))
    }
}

/// How much a kind is preferred in the go-to list: switching to an open tab beats reopening a
/// page, and a page the user saved beats one they merely visited.
const fn boost(kind: Kind) -> u32 {
    match kind {
        Kind::OpenTab => 40,
        Kind::Bookmark => 20,
        Kind::History
        | Kind::Command
        | Kind::Heading
        | Kind::Tag
        | Kind::SearchShortcut
        | Kind::Skill
        | Kind::Question => 0,
    }
}

/// The share of the score that frecency contributes.
fn frecency_bonus(frecency: u64) -> u32 {
    FRECENCY_WEIGHT * frecency.saturating_add(1).ilog2()
}

/// Matched character positions of `text`, ascending and without repeats; empty when the
/// pattern does not match it as a whole.
fn matches(pattern: &Pattern, text: &str, matcher: &mut Matcher) -> Vec<u32> {
    let mut chars = Vec::new();
    let mut positions = Vec::new();
    if pattern
        .indices(Utf32Str::new(text, &mut chars), matcher, &mut positions)
        .is_none()
    {
        positions.clear();
    }
    positions.sort_unstable();
    positions.dedup();
    positions
}

#[cfg(test)]
mod tests;
