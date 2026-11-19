use super::{CorruptHistory, Forgotten, History, HistoryEntry, HistorySnapshot, Recorded};
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

fn urls(history: &History) -> Vec<Url> {
    history.entries().map(|entry| entry.url.clone()).collect()
}

/// A history where `path` was visited once at each of `visits`.
fn visited(path: &str, visits: impl IntoIterator<Item = u64>) -> History {
    let mut history = History::new();
    for visit in visits {
        history.record_visit(&page(path), "", at(visit));
    }
    history
}

fn entry(text: &str, visit_count: u64, recent: impl IntoIterator<Item = u64>) -> HistoryEntry {
    HistoryEntry {
        url: url(text),
        title: String::new(),
        visit_count,
        recent_visits: times(recent),
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

        let recorded = history.record_visit(&page("a"), "A", at(10));

        assert_eq!(recorded, Recorded::NewEntry);
        let expected = HistoryEntry {
            url: page("a"),
            title: "A".to_owned(),
            visit_count: 1,
            recent_visits: times([10]),
        };
        assert_eq!(history.get(&page("a")), Some(&expected));
    }

    #[test]
    fn a_revisit_counts_and_keeps_the_time() {
        let mut history = visited("a", [10]);

        let recorded = history.record_visit(&page("a"), "", at(20));

        assert_eq!(recorded, Recorded::Revisit);
        let entry = history.get(&page("a")).unwrap();
        assert_eq!(entry.visit_count, 2);
        assert_eq!(entry.recent_visits, times([10, 20]));
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
    fn fragments_of_one_page_share_its_entry() {
        let mut history = History::new();

        history.record_visit(&page("a#install"), "", at(10));
        let recorded = history.record_visit(&page("a#usage"), "", at(20));

        assert_eq!(recorded, Recorded::Revisit);
        assert_eq!(urls(&history), vec![page("a")]);
        assert_eq!(history.get(&page("a")).unwrap().visit_count, 2);
    }

    #[test]
    fn the_query_names_a_different_page() {
        let mut history = visited("a", [10]);

        let recorded = history.record_visit(&page("a?page=2"), "", at(20));

        assert_eq!(recorded, Recorded::NewEntry);
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

        let recorded = history.record_visit(&url("http://example.test/"), "", at(10));

        assert_eq!(recorded, Recorded::NewEntry);
    }

    #[test]
    fn pages_outside_the_web_are_ignored() {
        let mut history = History::new();

        let recorded = [
            "about:blank",
            "data:text/html,secret",
            "file:///Users/me/notes.txt",
        ]
        .map(|text| history.record_visit(&url(text), "", at(10)));

        assert_eq!(recorded, [Recorded::Ignored; 3]);
        assert_eq!(history, History::new());
    }
}

mod reading {
    use super::*;

    #[test]
    fn entries_are_most_recently_visited_first() {
        let mut history = History::new();
        for (path, time) in [("a", 1), ("b", 2), ("c", 3), ("a", 4)] {
            history.record_visit(&page(path), "", at(time));
        }

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
}

mod forgetting {
    use super::*;

    #[test]
    fn remove_drops_the_entry_and_returns_it() {
        let mut history = visited("a", [10]);

        let removed = history.remove(&page("a#top"));

        assert_eq!(removed, Some(entry("https://example.test/a", 1, [10])));
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
                entry("https://example.test/a", 1, [1]),
                entry("https://example.test/b", 1, [2])
            ]
        );
    }

    #[test]
    fn a_full_entry_with_an_exact_count_is_valid() {
        let full = entry("https://example.test/a", 10, 1..=10);

        let restored = restore(vec![full.clone()]);

        assert_eq!(
            restored.map(|history| history.snapshot().entries),
            Ok(vec![full])
        );
    }

    #[test]
    fn a_page_twice_is_corrupt() {
        let text = "https://example.test/a";

        let result = restore(vec![entry(text, 1, [1]), entry(text, 1, [2])]);

        assert_eq!(result, Err(CorruptHistory::DuplicateEntry(url(text))));
    }

    #[test]
    fn a_page_outside_the_web_is_corrupt() {
        let text = "file:///tmp/a";

        let result = restore(vec![entry(text, 1, [1])]);

        assert_eq!(result, Err(CorruptHistory::NotWeb(url(text))));
    }

    #[test]
    fn a_fragment_is_corrupt() {
        let text = "https://example.test/a#top";

        let result = restore(vec![entry(text, 1, [1])]);

        assert_eq!(result, Err(CorruptHistory::HasFragment(url(text))));
    }

    #[test]
    fn an_entry_without_visits_is_corrupt() {
        let text = "https://example.test/a";

        let result = restore(vec![entry(text, 1, [])]);

        assert_eq!(result, Err(CorruptHistory::NoVisits(url(text))));
    }

    #[test]
    fn eleven_visit_times_are_corrupt() {
        let text = "https://example.test/a";

        let result = restore(vec![entry(text, 11, 1..=11)]);

        assert_eq!(result, Err(CorruptHistory::TooManyVisits(url(text))));
    }

    #[test]
    fn a_count_below_the_kept_times_is_corrupt() {
        let text = "https://example.test/a";

        let result = restore(vec![entry(text, 1, [1, 2])]);

        assert_eq!(result, Err(CorruptHistory::CountBelowVisits(url(text))));
    }
}
