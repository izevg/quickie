use std::num::NonZeroUsize;

use super::{
    CorruptHistory, Forgotten, HISTORY_CAP, History, HistoryEntry, HistorySnapshot, Recorded,
    VisitNumber,
};
use crate::Timestamp;
use url::Url;

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn page(path: &str) -> Url {
    url(&format!("https://example.test/{path}"))
}

fn at(millis: u64) -> Timestamp {
    Timestamp::from_millis(millis)
}

fn times(raw: impl IntoIterator<Item = u64>) -> Vec<Timestamp> {
    raw.into_iter().map(at).collect()
}

fn number(raw: u64) -> VisitNumber {
    VisitNumber::new(raw)
}

fn cap(pages: usize) -> NonZeroUsize {
    NonZeroUsize::new(pages).unwrap()
}

fn urls(history: &History) -> Vec<Url> {
    history.entries().map(|entry| entry.url.clone()).collect()
}

/// A history where each path was visited once, in order, at times 1, 2, 3...
fn visits(paths: &[&str]) -> History {
    let mut history = History::new();
    for (path, time) in paths.iter().zip(1..) {
        history.record_visit(&page(path), "", at(time));
    }
    history
}

/// A history where `path` was visited once at each of `visits`.
fn visited(path: &str, visits: impl IntoIterator<Item = u64>) -> History {
    let mut history = History::new();
    for visit in visits {
        history.record_visit(&page(path), "", at(visit));
    }
    history
}

fn entry(
    text: &str,
    visit_count: u64,
    recent: impl IntoIterator<Item = u64>,
    last_visit: u64,
) -> HistoryEntry {
    HistoryEntry {
        url: url(text),
        title: String::new(),
        visit_count,
        recent_visits: times(recent),
        last_visit: number(last_visit),
    }
}

fn restore(entries: Vec<HistoryEntry>) -> Result<History, CorruptHistory> {
    History::restore(HistorySnapshot { entries })
}

mod recording {
    use super::*;

    #[test]
    fn a_first_visit_creates_an_entry() {
        let mut history = History::new();

        let visited = history.record_visit(&page("a"), "A", at(10));

        assert_eq!(visited.recorded, Recorded::NewEntry);
        let expected = HistoryEntry {
            url: page("a"),
            title: "A".to_owned(),
            visit_count: 1,
            recent_visits: times([10]),
            last_visit: number(1),
        };
        assert_eq!(history.get(&page("a")), Some(&expected));
    }

    #[test]
    fn a_revisit_counts_and_keeps_the_time() {
        let mut history = visited("a", [10]);

        let visited = history.record_visit(&page("a"), "", at(20));

        assert_eq!(visited.recorded, Recorded::Revisit);
        let entry = history.get(&page("a")).unwrap();
        assert_eq!(entry.visit_count, 2);
        assert_eq!(entry.recent_visits, times([10, 20]));
    }

    #[test]
    fn each_visit_takes_the_next_number() {
        let mut history = visits(&["a", "b"]);

        history.record_visit(&page("a"), "", at(3));

        let numbers: Vec<VisitNumber> = history.entries().map(|e| e.last_visit).collect();
        assert_eq!(numbers, [number(3), number(2)]);
    }

    #[test]
    fn a_visit_numbers_after_a_clock_that_jumped_back() {
        let mut history = visits(&["a"]);

        history.record_visit(&page("b"), "", at(0));

        assert_eq!(urls(&history), [page("b"), page("a")]);
    }

    #[test]
    fn a_revisit_updates_the_title() {
        let mut history = History::new();
        history.record_visit(&page("a"), "Old", at(10));

        history.record_visit(&page("a"), "New", at(20));

        assert_eq!(history.get(&page("a")).unwrap().title, "New");
    }

    #[test]
    fn an_empty_title_keeps_the_known_one() {
        let mut history = History::new();
        history.record_visit(&page("a"), "Known", at(10));

        history.record_visit(&page("a"), "", at(20));

        assert_eq!(history.get(&page("a")).unwrap().title, "Known");
    }

    #[test]
    fn a_later_title_renames_the_entry_without_a_visit() {
        let mut history = History::new();
        history.record_visit(&page("a"), "", at(10));

        let changed = history.retitle(&url("https://example.test/a#top"), "Loaded");

        assert!(changed);
        let entry = history.get(&page("a")).unwrap();
        assert_eq!(entry.title, "Loaded");
        assert_eq!((entry.visit_count, entry.recent_visits.len()), (1, 1));
        assert_eq!(entry.last_visit, number(1));
    }

    #[test]
    fn retitling_with_nothing_new_changes_nothing() {
        let mut history = History::new();
        history.record_visit(&page("a"), "Same", at(10));
        let before = history.clone();

        assert!(!history.retitle(&page("a"), "Same"));
        assert!(!history.retitle(&page("a"), ""));
        assert_eq!(history, before);
    }

    #[test]
    fn a_title_alone_puts_no_page_in_history() {
        let mut history = History::new();

        let changed = history.retitle(&page("a"), "Never visited");

        assert!(!changed);
        assert_eq!(history, History::new());
    }

    #[test]
    fn fragments_of_one_page_share_its_entry() {
        let mut history = History::new();

        history.record_visit(&page("a#install"), "", at(10));
        let visited = history.record_visit(&page("a#usage"), "", at(20));

        assert_eq!(visited.recorded, Recorded::Revisit);
        assert_eq!(urls(&history), vec![page("a")]);
        assert_eq!(history.get(&page("a")).unwrap().visit_count, 2);
    }

    #[test]
    fn the_query_names_a_different_page() {
        let mut history = visited("a", [10]);

        let visited = history.record_visit(&page("a?page=2"), "", at(20));

        assert_eq!(visited.recorded, Recorded::NewEntry);
        assert_eq!(urls(&history), vec![page("a?page=2"), page("a")]);
    }

    #[test]
    fn ten_visit_times_are_all_kept() {
        let history = visited("a", 1..=10);

        let entry = history.get(&page("a")).unwrap();

        assert_eq!(entry.visit_count, 10);
        assert_eq!(entry.recent_visits, times(1..=10));
    }

    #[test]
    fn an_eleventh_visit_drops_the_oldest_time_but_counts() {
        let history = visited("a", 1..=11);

        let entry = history.get(&page("a")).unwrap();

        assert_eq!(entry.visit_count, 11);
        assert_eq!(entry.recent_visits, times(2..=11));
    }

    #[test]
    fn http_pages_are_recorded_too() {
        let mut history = History::new();

        let visited = history.record_visit(&url("http://example.test/"), "", at(10));

        assert_eq!(visited.recorded, Recorded::NewEntry);
    }

    #[test]
    fn pages_outside_the_web_are_ignored() {
        let mut history = History::new();

        let recorded = [
            "about:blank",
            "data:text/html,secret",
            "file:///Users/me/notes.txt",
        ]
        .map(|text| history.record_visit(&url(text), "", at(10)).recorded);

        assert_eq!(recorded, [Recorded::Ignored; 3]);
        assert_eq!(history, History::new());
    }
}

mod capping {
    use super::*;

    fn capped(pages: usize, paths: &[&str]) -> History {
        let mut history = History::with_cap(cap(pages));
        for (path, time) in paths.iter().zip(1..) {
            history.record_visit(&page(path), "", at(time));
        }
        history
    }

    #[test]
    fn a_history_keeps_a_hundred_thousand_pages() {
        assert_eq!(HISTORY_CAP.get(), 100_000);
    }

    #[test]
    fn a_history_at_its_cap_keeps_every_page() {
        let mut history = capped(2, &["a"]);

        let visited = history.record_visit(&page("b"), "", at(2));

        assert_eq!(visited.dropped, []);
        assert_eq!(urls(&history), [page("b"), page("a")]);
    }

    #[test]
    fn a_new_page_in_a_full_history_pushes_out_the_oldest() {
        let mut history = capped(2, &["a", "b"]);

        let visited = history.record_visit(&page("c"), "", at(3));

        assert_eq!(visited.recorded, Recorded::NewEntry);
        assert_eq!(
            visited.dropped,
            [entry("https://example.test/a", 1, [1], 1)]
        );
        assert_eq!(urls(&history), [page("c"), page("b")]);
    }

    #[test]
    fn a_revisited_page_is_no_longer_the_oldest() {
        let mut history = capped(2, &["a", "b", "a"]);

        let visited = history.record_visit(&page("c"), "", at(4));

        assert_eq!(visited.dropped.len(), 1);
        assert_eq!(visited.dropped[0].url, page("b"));
        assert_eq!(urls(&history), [page("c"), page("a")]);
    }

    #[test]
    fn a_revisit_in_a_full_history_drops_nothing() {
        let mut history = capped(1, &["a"]);

        let visited = history.record_visit(&page("a"), "", at(2));

        assert_eq!(visited.dropped, []);
        assert_eq!(urls(&history), [page("a")]);
    }

    #[test]
    fn a_lower_cap_waits_for_the_next_new_page() {
        let mut history = visits(&["a", "b", "c"]);
        history.set_cap(cap(1));

        let revisit = history.record_visit(&page("a"), "", at(4));
        let new = history.record_visit(&page("d"), "", at(5));

        assert_eq!(revisit.dropped, []);
        let dropped: Vec<Url> = new.dropped.into_iter().map(|e| e.url).collect();
        assert_eq!(dropped, [page("b"), page("c"), page("a")]);
        assert_eq!(urls(&history), [page("d")]);
    }

    #[test]
    fn new_and_restored_histories_keep_the_default_cap() {
        let capped = History::with_cap(HISTORY_CAP);

        let restored = restore(Vec::new()).unwrap();

        assert_eq!(History::new(), capped);
        assert_eq!(restored, capped);
    }

    #[test]
    fn a_restored_history_past_its_cap_shrinks_at_its_next_new_page() {
        let three = vec![
            entry("https://example.test/a", 1, [1], 1),
            entry("https://example.test/b", 1, [1], 2),
            entry("https://example.test/c", 1, [1], 3),
        ];
        let mut history = restore(three).unwrap();
        history.set_cap(cap(2));

        let visited = history.record_visit(&page("d"), "", at(2));

        let dropped: Vec<Url> = visited.dropped.into_iter().map(|e| e.url).collect();
        assert_eq!(dropped, [page("a"), page("b")]);
        assert_eq!(history.len(), 2);
    }
}

mod reading {
    use super::*;

    #[test]
    fn entries_are_most_recently_visited_first() {
        let history = visits(&["a", "b", "c", "a"]);

        let order = urls(&history);

        assert_eq!(order, vec![page("a"), page("c"), page("b")]);
    }

    #[test]
    fn get_finds_a_page_by_any_fragment() {
        let history = visited("a", [10]);

        let found = history.get(&page("a#top")).map(|entry| entry.url.clone());

        assert_eq!(found, Some(page("a")));
    }

    #[test]
    fn get_misses_a_page_never_visited() {
        let history = visited("a", [10]);

        assert_eq!(history.get(&page("b")), None);
    }

    #[test]
    fn among_many_pages_each_finds_its_own_and_no_other() {
        let paths: Vec<String> = (0..500).map(|n| format!("visited/{n}")).collect();
        let history = visits(&paths.iter().map(String::as_str).collect::<Vec<_>>());

        let found = (0..500).all(|n| {
            history.get(&page(&format!("visited/{n}"))).map(|e| &e.url)
                == Some(&page(&format!("visited/{n}")))
        });
        let missed = (0..500).all(|n| history.get(&page(&format!("other/{n}"))).is_none());

        assert!(found);
        assert!(missed);
    }

    #[test]
    fn a_history_counts_its_pages() {
        let empty = History::new();
        let two = visits(&["a", "b", "a"]);

        assert_eq!((empty.len(), empty.is_empty()), (0, true));
        assert_eq!((two.len(), two.is_empty()), (2, false));
    }

    #[test]
    fn a_visit_number_is_the_number_the_store_keeps() {
        assert_eq!(VisitNumber::new(7).get(), 7);
    }
}

mod comparing {
    use super::*;

    #[test]
    fn histories_with_the_same_visits_are_equal() {
        assert_eq!(visits(&["a", "b"]), visits(&["a", "b"]));
    }

    #[test]
    fn histories_with_other_entries_differ() {
        assert_ne!(visits(&["a", "b"]), visits(&["b", "a"]));
    }

    #[test]
    fn histories_with_other_caps_differ() {
        assert_ne!(History::new(), History::with_cap(cap(5)));
    }

    #[test]
    fn histories_that_number_the_next_visit_apart_differ() {
        let mut removed = visits(&["a", "b"]);
        removed.remove(&page("b"));

        assert_ne!(removed, visits(&["a"]));
    }

    #[test]
    fn where_an_entry_sits_in_memory_does_not_matter() {
        let mut moved = visits(&["a", "b", "c"]);
        moved.remove(&page("a"));
        let mut kept = visits(&["x", "b", "c"]);
        kept.remove(&page("x"));

        assert_eq!(moved, kept);
    }
}

mod forgetting {
    use super::*;

    #[test]
    fn remove_drops_the_entry_and_returns_it() {
        let mut history = visited("a", [10]);

        let removed = history.remove(&page("a#top"));

        assert_eq!(removed, Some(entry("https://example.test/a", 1, [10], 1)));
        assert_eq!(history.entries().count(), 0);
    }

    #[test]
    fn removing_an_unknown_page_changes_nothing() {
        let mut history = visited("a", [10]);

        let removed = history.remove(&page("b"));

        assert_eq!(removed, None);
        assert_eq!(urls(&history), vec![page("a")]);
    }

    #[test]
    fn removing_a_page_keeps_the_order_of_the_rest() {
        let mut history = visits(&["a", "b", "c", "d"]);

        history.remove(&page("b"));
        history.record_visit(&page("a"), "", at(5));

        assert_eq!(urls(&history), [page("a"), page("d"), page("c")]);
        assert_eq!(history.get(&page("d")).unwrap().last_visit, number(4));
    }

    #[test]
    fn removing_the_last_page_in_memory_keeps_the_rest() {
        let mut history = visits(&["a", "b"]);

        history.remove(&page("b"));

        assert_eq!(urls(&history), [page("a")]);
    }

    #[test]
    fn clear_since_forgets_visits_from_that_moment_on() {
        let mut history = visited("a", [10, 19, 20, 30]);

        let forgotten = history.clear_since(at(20));

        assert_eq!(
            forgotten,
            Forgotten {
                visits: 2,
                entries: 0
            }
        );
        let entry = history.get(&page("a")).unwrap();
        assert_eq!(entry.recent_visits, times([10, 19]));
        assert_eq!(entry.visit_count, 2);
    }

    #[test]
    fn clear_since_keeps_the_count_of_visits_whose_times_are_gone() {
        let mut history = visited("a", 1..=12);

        history.clear_since(at(12));

        let entry = history.get(&page("a")).unwrap();
        assert_eq!(entry.visit_count, 11);
        assert_eq!(entry.recent_visits, times(3..=11));
    }

    #[test]
    fn clear_since_drops_an_entry_with_no_visit_left() {
        let mut history = visited("a", 1..=12);

        let forgotten = history.clear_since(at(3));

        assert_eq!(
            forgotten,
            Forgotten {
                visits: 10,
                entries: 1
            }
        );
        assert_eq!(history.get(&page("a")), None);
    }

    #[test]
    fn clear_since_adds_up_every_entry() {
        let mut history = History::new();
        for (path, time) in [("a", 5), ("b", 6), ("a", 7), ("c", 1)] {
            history.record_visit(&page(path), "", at(time));
        }

        let forgotten = history.clear_since(at(5));

        assert_eq!(
            forgotten,
            Forgotten {
                visits: 3,
                entries: 2
            }
        );
        assert_eq!(urls(&history), vec![page("c")]);
    }

    #[test]
    fn clear_since_keeps_the_order_of_what_is_left() {
        let mut history = History::new();
        for (path, time) in [("a", 1), ("b", 9), ("a", 2), ("c", 3)] {
            history.record_visit(&page(path), "", at(time));
        }

        history.clear_since(at(5));

        assert_eq!(urls(&history), [page("c"), page("a")]);
    }

    #[test]
    fn clear_since_checks_times_from_a_clock_that_jumped_back() {
        let mut history = visited("a", [50, 10]);

        history.clear_since(at(20));

        assert_eq!(history.get(&page("a")).unwrap().recent_visits, times([10]));
    }
}

mod snapshot {
    use super::*;

    #[test]
    fn restore_gives_back_the_same_history() {
        let mut history = History::new();
        for (path, time) in [("a", 1), ("b", 2), ("a", 3)] {
            history.record_visit(&page(path), path, at(time));
        }

        let restored = History::restore(history.snapshot());

        assert_eq!(restored, Ok(history));
    }

    #[test]
    fn the_snapshot_lists_the_least_recent_first() {
        let mut history = visited("a", [1]);
        history.record_visit(&page("b"), "", at(2));

        let snapshot = history.snapshot();

        assert_eq!(
            snapshot.entries,
            vec![
                entry("https://example.test/a", 1, [1], 1),
                entry("https://example.test/b", 1, [2], 2)
            ]
        );
    }

    #[test]
    fn restore_orders_by_the_last_visit() {
        let restored = restore(vec![
            entry("https://example.test/b", 1, [1], 20),
            entry("https://example.test/a", 1, [1], 10),
            entry("https://example.test/c", 1, [1], 30),
        ])
        .unwrap();

        assert_eq!(urls(&restored), [page("c"), page("b"), page("a")]);
    }

    #[test]
    fn the_next_visit_after_a_restore_follows_the_latest() {
        let mut restored = restore(vec![
            entry("https://example.test/b", 1, [1], 20),
            entry("https://example.test/a", 1, [1], 10),
        ])
        .unwrap();

        restored.record_visit(&page("c"), "", at(2));

        assert_eq!(restored.get(&page("c")).unwrap().last_visit, number(21));
    }

    #[test]
    fn a_full_entry_with_an_exact_count_is_valid() {
        let full = entry("https://example.test/a", 10, 1..=10, 1);

        let restored = restore(vec![full.clone()]);

        assert_eq!(
            restored.map(|history| history.snapshot().entries),
            Ok(vec![full])
        );
    }

    #[test]
    fn a_page_twice_is_corrupt() {
        let text = "https://example.test/a";

        let result = restore(vec![entry(text, 1, [1], 1), entry(text, 1, [2], 2)]);

        assert_eq!(result, Err(CorruptHistory::DuplicateEntry(url(text))));
    }

    #[test]
    fn two_pages_with_one_last_visit_are_corrupt() {
        let result = restore(vec![
            entry("https://example.test/a", 1, [1], 5),
            entry("https://example.test/b", 1, [2], 5),
        ]);

        assert_eq!(result, Err(CorruptHistory::SharedLastVisit(page("b"))));
    }

    #[test]
    fn a_page_outside_the_web_is_corrupt() {
        let text = "file:///tmp/a";

        let result = restore(vec![entry(text, 1, [1], 1)]);

        assert_eq!(result, Err(CorruptHistory::NotWeb(url(text))));
    }

    #[test]
    fn a_fragment_is_corrupt() {
        let text = "https://example.test/a#top";

        let result = restore(vec![entry(text, 1, [1], 1)]);

        assert_eq!(result, Err(CorruptHistory::HasFragment(url(text))));
    }

    #[test]
    fn an_entry_without_visits_is_corrupt() {
        let text = "https://example.test/a";

        let result = restore(vec![entry(text, 1, [], 1)]);

        assert_eq!(result, Err(CorruptHistory::NoVisits(url(text))));
    }

    #[test]
    fn eleven_visit_times_are_corrupt() {
        let text = "https://example.test/a";

        let result = restore(vec![entry(text, 11, 1..=11, 1)]);

        assert_eq!(result, Err(CorruptHistory::TooManyVisits(url(text))));
    }

    #[test]
    fn a_count_below_the_kept_times_is_corrupt() {
        let text = "https://example.test/a";

        let result = restore(vec![entry(text, 1, [1, 2], 1)]);

        assert_eq!(result, Err(CorruptHistory::CountBelowVisits(url(text))));
    }
}
