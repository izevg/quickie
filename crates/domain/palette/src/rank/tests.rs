use quickie_domain_kernel::Timestamp;

use super::{Ranked, Ranker};
use crate::{Candidate, Frecency, Kind, Query, TextSpan, Texts};

const NOW: Timestamp = Timestamp::from_millis(1_000 * Timestamp::DAY_MS);

fn candidate<'a>(key: u32, kind: Kind, title: &'a str, detail: &'a str) -> Candidate<'a, u32> {
    Candidate {
        key,
        kind,
        title: title.into(),
        detail: detail.into(),
        frecency: 0,
    }
}

/// A candidate visited `times` times today, so its frecency is `100 × times`.
fn visited(times: u32, mut candidate: Candidate<'_, u32>) -> Candidate<'_, u32> {
    candidate.frecency = Frecency::from_visits((0..times).map(|_| NOW)).score(NOW);
    candidate
}

/// One ranking by a fresh ranker.
fn rank(query: &Query, candidates: &[Candidate<'_, u32>], limit: usize) -> Vec<Ranked<u32>> {
    Ranker::default().rank(query, candidates.iter().copied(), limit)
}

fn keys(ranked: &[Ranked<u32>]) -> Vec<u32> {
    ranked.iter().map(|hit| hit.key).collect()
}

fn scores(ranked: &[Ranked<u32>]) -> Vec<u32> {
    ranked.iter().map(|hit| hit.score).collect()
}

fn one_of_each_kind() -> Vec<Candidate<'static, u32>> {
    Kind::ALL
        .into_iter()
        .zip(1..)
        .map(|(kind, key)| candidate(key, kind, "Same", "same"))
        .collect()
}

#[test]
fn an_agent_query_ranks_skills_only() {
    let ranked = rank(&Query::parse("?same"), &one_of_each_kind(), 10);

    assert_eq!(keys(&ranked), [8]);
}

#[test]
fn go_to_ranks_tabs_bookmarks_and_history_only() {
    let ranked = rank(&Query::parse(""), &one_of_each_kind(), 10);

    assert_eq!(keys(&ranked), [1, 2, 3]);
}

#[test]
fn the_commands_mode_ranks_commands_only() {
    let ranked = rank(&Query::parse(">"), &one_of_each_kind(), 10);

    assert_eq!(keys(&ranked), [4]);
}

#[test]
fn an_empty_go_to_query_orders_by_kind_without_highlights() {
    let candidates = [
        candidate(1, Kind::History, "Same", "same"),
        candidate(2, Kind::Bookmark, "Same", "same"),
        candidate(3, Kind::OpenTab, "Same", "same"),
    ];

    let ranked = rank(&Query::parse(""), &candidates, 10);

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

    let ranked = rank(&Query::parse(""), &candidates, 10);

    // log2(1001) = 9 and log2(101) = 6, rounded down.
    assert_eq!((keys(&ranked), scores(&ranked)), (vec![2, 1], vec![36, 24]));
}

#[test]
fn frecency_and_kind_add_up() {
    let candidates = [visited(1, candidate(1, Kind::OpenTab, "Tab", ""))];

    let ranked = rank(&Query::parse(""), &candidates, 10);

    assert_eq!(scores(&ranked), [64]);
}

#[test]
fn frecency_adds_to_the_fuzzy_score() {
    let candidates = [
        candidate(1, Kind::History, "Docs", ""),
        visited(1, candidate(2, Kind::History, "Docs", "")),
    ];

    let ranked = rank(&Query::parse("doc"), &candidates, 10);

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

    let ranked = rank(&Query::parse("doc"), &candidates, 10);

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

    let ranked = rank(&Query::parse(">rel"), &candidates, 10);

    assert_eq!(keys(&ranked), [1]);
}

#[test]
fn a_title_match_is_highlighted_in_the_title() {
    let candidates = [candidate(1, Kind::Command, "Reload", "⌘R")];

    let ranked = rank(&Query::parse(">rel"), &candidates, 10);

    assert_eq!(
        (&ranked[0].title_matches, &ranked[0].detail_matches),
        (&vec![0, 1, 2], &vec![])
    );
}

#[test]
fn a_detail_match_alone_is_enough() {
    let candidates = [candidate(1, Kind::History, "Pull requests", "github.com")];

    let ranked = rank(&Query::parse("hub"), &candidates, 10);

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

    let ranked = rank(&Query::parse("github"), &candidates, 10);

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

    let ranked = rank(&Query::parse("rust book"), &candidates, 10);

    assert_eq!(
        (&ranked[0].title_matches, &ranked[0].detail_matches),
        (&vec![4, 5, 6, 7, 9, 10, 11, 12], &vec![])
    );
}

#[test]
fn highlights_are_ascending_and_unique() {
    let candidates = [candidate(1, Kind::History, "rust", "")];

    let ranked = rank(&Query::parse("st ru st"), &candidates, 10);

    assert_eq!(ranked[0].title_matches, [0, 1, 2, 3]);
}

#[test]
fn a_keyword_search_matches_the_keyword_not_the_terms() {
    let candidates = [
        candidate(1, Kind::SearchShortcut, "GitHub", "gh"),
        candidate(2, Kind::SearchShortcut, "DuckDuckGo", "ddg"),
    ];

    let ranked = rank(&Query::parse("!gh quickie"), &candidates, 10);

    assert_eq!(keys(&ranked), [1]);
}

#[test]
fn equal_scores_fall_back_to_the_title() {
    let candidates = [
        candidate(1, Kind::History, "Beta", ""),
        candidate(2, Kind::History, "Alpha", ""),
    ];

    let ranked = rank(&Query::parse(""), &candidates, 10);

    assert_eq!(keys(&ranked), [2, 1]);
}

#[test]
fn equal_scores_and_titles_keep_the_input_order() {
    let candidates = [
        candidate(7, Kind::History, "Same", ""),
        candidate(3, Kind::History, "Same", ""),
        candidate(5, Kind::History, "Same", ""),
    ];

    let ranked = rank(&Query::parse(""), &candidates, 10);

    assert_eq!(keys(&ranked), [7, 3, 5]);
}

/// Five history entries whose frecency rises with the key, so the best come last.
fn rising() -> Vec<Candidate<'static, u32>> {
    (1..=5)
        .map(|key| visited(key * key, candidate(key, Kind::History, "Page", "")))
        .collect()
}

#[test]
fn the_limit_keeps_the_best() {
    let ranked = rank(&Query::parse(""), &rising(), 2);

    assert_eq!(keys(&ranked), [5, 4]);
}

#[test]
fn a_limit_equal_to_the_matches_keeps_them_all() {
    let ranked = rank(&Query::parse(""), &rising(), 5);

    assert_eq!(keys(&ranked), [5, 4, 3, 2, 1]);
}

#[test]
fn a_limit_above_the_matches_keeps_them_all() {
    let ranked = rank(&Query::parse(""), &rising(), 6);

    assert_eq!(keys(&ranked), [5, 4, 3, 2, 1]);
}

#[test]
fn a_zero_limit_keeps_nothing() {
    let ranked = rank(&Query::parse(""), &rising(), 0);

    assert_eq!(ranked, []);
}

#[test]
fn ten_thousand_candidates_rank_like_a_full_sort() {
    let kinds = [Kind::OpenTab, Kind::Bookmark, Kind::History, Kind::Command];
    let texts: Vec<(String, String)> = (0..10_000_u32)
        .map(|key| {
            let title = format!("Page {key} about topic {}", key % 97);
            (title, format!("site{}.example/{key}", key % 13))
        })
        .collect();
    let candidates: Vec<Candidate<'_, u32>> = texts
        .iter()
        .zip(0..)
        .zip(kinds.into_iter().cycle())
        .map(|(((title, detail), key), kind)| {
            visited(key % 17, candidate(key, kind, title, detail))
        })
        .collect();
    let query = Query::parse("topic 4");

    let top = rank(&query, &candidates, 10);
    let all = rank(&query, &candidates, usize::MAX);

    assert_eq!(top.len(), 10);
    assert!(all.len() > 1_000);
    assert_eq!(top, all[..10]);
    assert!(all.windows(2).all(|pair| pair[0].score >= pair[1].score));
}

#[test]
fn text_beyond_ascii_matches_and_highlights_by_its_characters() {
    let candidates = [
        candidate(1, Kind::History, "Über uns", ""),
        candidate(2, Kind::History, "Café Rust", ""),
    ];

    let umlaut = rank(&Query::parse("ü"), &candidates, 10);
    let after_accent = rank(&Query::parse("rust"), &candidates, 10);

    assert_eq!(
        (keys(&umlaut), &umlaut[0].title_matches),
        (vec![1], &vec![0])
    );
    assert_eq!(after_accent[0].title_matches, [5, 6, 7, 8]);
}

#[test]
fn kept_text_ranks_as_the_same_text_given_plainly() {
    let titles = ["Über uns", "Café Rust", "Docs · Rust", "rust"];
    let mut texts = Texts::default();
    let spans: Vec<TextSpan> = titles.iter().map(|title| texts.keep(title)).collect();
    let kept: Vec<Candidate<'_, u32>> = spans
        .iter()
        .zip(1..)
        .map(|(span, key)| Candidate {
            title: texts.get(span),
            ..candidate(key, Kind::History, "", "")
        })
        .collect();
    let plain: Vec<Candidate<'_, u32>> = titles
        .iter()
        .zip(1..)
        .map(|(title, key)| candidate(key, Kind::History, title, ""))
        .collect();

    for typed in ["", "ü", "rust", "é r", "·"] {
        let query = Query::parse(typed);

        assert_eq!(rank(&query, &kept, 10), rank(&query, &plain, 10), "{typed}");
    }
}

#[test]
fn a_ranker_answers_each_query_as_a_fresh_one_would() {
    let candidates = [
        candidate(1, Kind::History, "Über uns", "café.example"),
        candidate(2, Kind::History, "Docs", "docs.example"),
        candidate(3, Kind::History, "Rust", "rust-lang.org"),
    ];
    let typed = ["ü", "doc", "", "rust", "ca"];
    let mut ranker = Ranker::default();

    let reused: Vec<Vec<Ranked<u32>>> = typed
        .iter()
        .map(|text| ranker.rank(&Query::parse(text), candidates, 10))
        .collect();

    let fresh: Vec<Vec<Ranked<u32>>> = typed
        .iter()
        .map(|text| rank(&Query::parse(text), &candidates, 10))
        .collect();
    assert_eq!(reused, fresh);
}
