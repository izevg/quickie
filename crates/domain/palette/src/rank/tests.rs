use quickie_domain_kernel::Timestamp;

use super::{Ranked, rank};
use crate::{Candidate, Frecency, Kind, Query};

const NOW: Timestamp = Timestamp::from_millis(1_000 * Timestamp::DAY_MS);

fn candidate(key: u32, kind: Kind, title: &str, detail: &str) -> Candidate<u32> {
    Candidate {
        key,
        kind,
        title: title.to_owned(),
        detail: detail.to_owned(),
        visits: Frecency::default(),
    }
}

/// A candidate visited `times` times today, so its frecency is `100 × times`.
fn visited(times: u32, mut candidate: Candidate<u32>) -> Candidate<u32> {
    candidate.visits = Frecency::from_visits((0..times).map(|_| NOW));
    candidate
}

fn keys(ranked: &[Ranked<u32>]) -> Vec<u32> {
    ranked.iter().map(|hit| hit.key).collect()
}

fn scores(ranked: &[Ranked<u32>]) -> Vec<u32> {
    ranked.iter().map(|hit| hit.score).collect()
}

fn one_of_each_kind() -> Vec<Candidate<u32>> {
    Kind::ALL
        .into_iter()
        .zip(1..)
        .map(|(kind, key)| candidate(key, kind, "Same", "same"))
        .collect()
}

#[test]
fn an_agent_query_ranks_skills_only() {
    let ranked = rank(&Query::parse("?same"), &one_of_each_kind(), NOW, 10);

    assert_eq!(keys(&ranked), [8]);
}

#[test]
fn go_to_ranks_tabs_bookmarks_and_history_only() {
    let ranked = rank(&Query::parse(""), &one_of_each_kind(), NOW, 10);

    assert_eq!(keys(&ranked), [1, 2, 3]);
}

#[test]
fn the_commands_mode_ranks_commands_only() {
    let ranked = rank(&Query::parse(">"), &one_of_each_kind(), NOW, 10);

    assert_eq!(keys(&ranked), [4]);
}

#[test]
fn an_empty_go_to_query_orders_by_kind_without_highlights() {
    let candidates = [
        candidate(1, Kind::History, "Same", "same"),
        candidate(2, Kind::Bookmark, "Same", "same"),
        candidate(3, Kind::OpenTab, "Same", "same"),
    ];

    let ranked = rank(&Query::parse(""), &candidates, NOW, 10);

    assert_eq!(
        ranked,
        [
            Ranked {
                key: 3,
                score: 40,
                title_matches: vec![],
                detail_matches: vec![]
            },
            Ranked {
                key: 2,
                score: 20,
                title_matches: vec![],
                detail_matches: vec![]
            },
            Ranked {
                key: 1,
                score: 0,
                title_matches: vec![],
                detail_matches: vec![]
            },
        ]
    );
}

#[test]
fn frecency_adds_four_points_per_doubling() {
    let candidates = [
        visited(1, candidate(1, Kind::History, "Once", "")),
        visited(10, candidate(2, Kind::History, "Often", "")),
    ];

    let ranked = rank(&Query::parse(""), &candidates, NOW, 10);

    // log2(1001) = 9 and log2(101) = 6, rounded down.
    assert_eq!((keys(&ranked), scores(&ranked)), (vec![2, 1], vec![36, 24]));
}

#[test]
fn frecency_and_kind_add_up() {
    let candidates = [visited(1, candidate(1, Kind::OpenTab, "Tab", ""))];

    let ranked = rank(&Query::parse(""), &candidates, NOW, 10);

    assert_eq!(scores(&ranked), [64]);
}

#[test]
fn frecency_adds_to_the_fuzzy_score() {
    let candidates = [
        candidate(1, Kind::History, "Docs", ""),
        visited(1, candidate(2, Kind::History, "Docs", "")),
    ];

    let ranked = rank(&Query::parse("doc"), &candidates, NOW, 10);

    assert_eq!(keys(&ranked), [2, 1]);
    assert_eq!(ranked[0].score - ranked[1].score, 24);
}

#[test]
fn the_kind_boost_adds_to_the_fuzzy_score() {
    let candidates = [
        candidate(1, Kind::History, "Docs", ""),
        candidate(2, Kind::Bookmark, "Docs", ""),
        candidate(3, Kind::OpenTab, "Docs", ""),
    ];

    let ranked = rank(&Query::parse("doc"), &candidates, NOW, 10);

    assert_eq!(keys(&ranked), [3, 2, 1]);
    assert_eq!(ranked[0].score - ranked[2].score, 40);
    assert_eq!(ranked[1].score - ranked[2].score, 20);
}

#[test]
fn a_candidate_that_does_not_match_is_dropped() {
    let candidates = [
        candidate(1, Kind::Command, "Reload", "⌘R"),
        candidate(2, Kind::Command, "Close tab", "⌘W"),
    ];

    let ranked = rank(&Query::parse(">rel"), &candidates, NOW, 10);

    assert_eq!(keys(&ranked), [1]);
}

#[test]
fn a_title_match_is_highlighted_in_the_title() {
    let candidates = [candidate(1, Kind::Command, "Reload", "⌘R")];

    let ranked = rank(&Query::parse(">rel"), &candidates, NOW, 10);

    assert_eq!(
        (&ranked[0].title_matches, &ranked[0].detail_matches),
        (&vec![0, 1, 2], &vec![])
    );
}

#[test]
fn a_detail_match_alone_is_enough() {
    let candidates = [candidate(1, Kind::History, "Pull requests", "github.com")];

    let ranked = rank(&Query::parse("hub"), &candidates, NOW, 10);

    assert_eq!(
        (&ranked[0].title_matches, &ranked[0].detail_matches),
        (&vec![], &vec![3, 4, 5])
    );
}

#[test]
fn the_better_of_title_and_detail_sets_the_score() {
    let candidates = [
        candidate(1, Kind::History, "github", "zzz"),
        candidate(2, Kind::History, "zzz", "github"),
        candidate(3, Kind::History, "github", "g-i-t-h-u-b"),
    ];

    let ranked = rank(&Query::parse("github"), &candidates, NOW, 10);

    assert_eq!(ranked[0].score, ranked[1].score);
    assert_eq!(ranked[1].score, ranked[2].score);
}

#[test]
fn a_field_matching_only_some_words_is_not_highlighted() {
    let candidates = [candidate(
        1,
        Kind::Bookmark,
        "The Rust Book",
        "rust-lang.org",
    )];

    let ranked = rank(&Query::parse("rust book"), &candidates, NOW, 10);

    assert_eq!(
        (&ranked[0].title_matches, &ranked[0].detail_matches),
        (&vec![4, 5, 6, 7, 9, 10, 11, 12], &vec![])
    );
}

#[test]
fn highlights_are_ascending_and_unique() {
    let candidates = [candidate(1, Kind::History, "rust", "")];

    let ranked = rank(&Query::parse("st ru st"), &candidates, NOW, 10);

    assert_eq!(ranked[0].title_matches, [0, 1, 2, 3]);
}

#[test]
fn a_keyword_search_matches_the_keyword_not_the_terms() {
    let candidates = [
        candidate(1, Kind::SearchShortcut, "GitHub", "gh"),
        candidate(2, Kind::SearchShortcut, "DuckDuckGo", "ddg"),
    ];

    let ranked = rank(&Query::parse("!gh quickie"), &candidates, NOW, 10);

    assert_eq!(keys(&ranked), [1]);
}

#[test]
fn equal_scores_fall_back_to_the_title() {
    let candidates = [
        candidate(1, Kind::History, "Beta", ""),
        candidate(2, Kind::History, "Alpha", ""),
    ];

    let ranked = rank(&Query::parse(""), &candidates, NOW, 10);

    assert_eq!(keys(&ranked), [2, 1]);
}

#[test]
fn equal_scores_and_titles_keep_the_input_order() {
    let candidates = [
        candidate(7, Kind::History, "Same", ""),
        candidate(3, Kind::History, "Same", ""),
        candidate(5, Kind::History, "Same", ""),
    ];

    let ranked = rank(&Query::parse(""), &candidates, NOW, 10);

    assert_eq!(keys(&ranked), [7, 3, 5]);
}

/// Five history entries whose frecency rises with the key, so the best come last.
fn rising() -> Vec<Candidate<u32>> {
    (1..=5)
        .map(|key| visited(key * key, candidate(key, Kind::History, "Page", "")))
        .collect()
}

#[test]
fn the_limit_keeps_the_best() {
    let ranked = rank(&Query::parse(""), &rising(), NOW, 2);

    assert_eq!(keys(&ranked), [5, 4]);
}

#[test]
fn a_limit_equal_to_the_matches_keeps_them_all() {
    let ranked = rank(&Query::parse(""), &rising(), NOW, 5);

    assert_eq!(keys(&ranked), [5, 4, 3, 2, 1]);
}

#[test]
fn a_limit_above_the_matches_keeps_them_all() {
    let ranked = rank(&Query::parse(""), &rising(), NOW, 6);

    assert_eq!(keys(&ranked), [5, 4, 3, 2, 1]);
}

#[test]
fn a_zero_limit_keeps_nothing() {
    let ranked = rank(&Query::parse(""), &rising(), NOW, 0);

    assert_eq!(ranked, []);
}

#[test]
fn ten_thousand_candidates_rank_like_a_full_sort() {
    let kinds = [Kind::OpenTab, Kind::Bookmark, Kind::History, Kind::Command];
    let candidates: Vec<Candidate<u32>> = (0..10_000_u32)
        .zip(kinds.into_iter().cycle())
        .map(|(key, kind)| {
            let title = format!("Page {key} about topic {}", key % 97);
            let detail = format!("site{}.example/{key}", key % 13);
            visited(key % 17, candidate(key, kind, &title, &detail))
        })
        .collect();
    let query = Query::parse("topic 4");

    let top = rank(&query, &candidates, NOW, 10);
    let all = rank(&query, &candidates, NOW, usize::MAX);

    assert_eq!(top.len(), 10);
    assert!(all.len() > 1_000);
    assert_eq!(top, all[..10]);
    assert!(all.windows(2).all(|pair| pair[0].score >= pair[1].score));
}
