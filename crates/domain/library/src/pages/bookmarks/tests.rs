use std::collections::{BTreeMap, BTreeSet};

use super::{Bookmark, Bookmarks, BookmarksSnapshot, CorruptBookmarks, NotSaved, Saved, Tag};
use crate::Timestamp;
use url::Url;

fn page(path: &str) -> Url {
    Url::parse(&format!("https://example.test/{path}")).unwrap()
}

fn at(millis: u64) -> Timestamp {
    Timestamp::from_millis(millis)
}

fn tag(text: &str) -> Tag {
    Tag::try_from(text).unwrap()
}

fn save(bookmarks: &mut Bookmarks, path: &str) {
    bookmarks.save(&page(path), path.to_owned(), at(10));
}

/// Pages a, b, c saved in that order.
fn three_saved() -> Bookmarks {
    let mut bookmarks = Bookmarks::new();
    for path in ["a", "b", "c"] {
        save(&mut bookmarks, path);
    }
    bookmarks
}

fn urls<'a>(bookmarks: impl Iterator<Item = &'a Bookmark>) -> Vec<Url> {
    bookmarks.map(|bookmark| bookmark.url.clone()).collect()
}

fn bookmark(path: &str) -> Bookmark {
    Bookmark {
        url: page(path),
        title: String::new(),
        tags: BTreeSet::new(),
        saved_at: at(1),
    }
}

mod saving {
    use super::*;

    #[test]
    fn saving_a_page_creates_an_untagged_bookmark() {
        let mut bookmarks = Bookmarks::new();

        let saved = bookmarks.save(&page("a"), "A".to_owned(), at(10));

        assert_eq!(saved, Saved { created: true });
        let expected = Bookmark {
            url: page("a"),
            title: "A".to_owned(),
            tags: BTreeSet::new(),
            saved_at: at(10),
        };
        assert_eq!(bookmarks.get(&page("a")), Some(&expected));
    }

    #[test]
    fn saving_again_keeps_the_bookmark_as_it_was() {
        let mut bookmarks = Bookmarks::new();
        bookmarks.save(&page("a"), "A".to_owned(), at(10));
        bookmarks.tag(&page("a"), Tag::later()).unwrap();
        let before = bookmarks.clone();

        let saved = bookmarks.save(&page("a#top"), "Other".to_owned(), at(20));

        assert_eq!(saved, Saved { created: false });
        assert_eq!(bookmarks, before);
    }

    #[test]
    fn a_bookmark_drops_the_fragment() {
        let mut bookmarks = Bookmarks::new();

        bookmarks.save(&page("a#usage"), String::new(), at(10));

        assert_eq!(urls(bookmarks.all()), vec![page("a")]);
    }

    #[test]
    fn all_lists_bookmarks_in_saving_order() {
        let mut bookmarks = three_saved();

        bookmarks.remove(&page("a"));
        save(&mut bookmarks, "a");

        assert_eq!(urls(bookmarks.all()), vec![page("b"), page("c"), page("a")]);
    }

    #[test]
    fn remove_returns_the_bookmark() {
        let mut bookmarks = three_saved();

        let removed = bookmarks
            .remove(&page("b#top"))
            .map(|bookmark| bookmark.url);

        assert_eq!(removed, Some(page("b")));
        assert_eq!(urls(bookmarks.all()), vec![page("a"), page("c")]);
    }

    #[test]
    fn removing_an_unsaved_page_changes_nothing() {
        let mut bookmarks = three_saved();

        let removed = bookmarks.remove(&page("d"));

        assert_eq!(removed, None);
        assert_eq!(bookmarks, three_saved());
    }

    #[test]
    fn get_misses_an_unsaved_page() {
        assert_eq!(three_saved().get(&page("d")), None);
    }

    #[test]
    fn retitle_renames_a_bookmark() {
        let mut bookmarks = three_saved();

        let result = bookmarks.retitle(&page("a"), "Renamed".to_owned());

        assert_eq!(result, Ok(()));
        assert_eq!(bookmarks.get(&page("a")).unwrap().title, "Renamed");
    }

    #[test]
    fn retitling_an_unsaved_page_fails() {
        let mut bookmarks = three_saved();

        let result = bookmarks.retitle(&page("d#top"), "D".to_owned());

        assert_eq!(result, Err(NotSaved(page("d"))));
    }
}

mod tagging {
    use super::*;

    fn tags_of(bookmarks: &Bookmarks, path: &str) -> Vec<Tag> {
        bookmarks
            .get(&page(path))
            .unwrap()
            .tags
            .iter()
            .cloned()
            .collect()
    }

    #[test]
    fn tags_are_kept_once_and_sorted() {
        let mut bookmarks = three_saved();

        for text in ["rust", "later", "rust"] {
            bookmarks.tag(&page("a"), tag(text)).unwrap();
        }

        assert_eq!(tags_of(&bookmarks, "a"), vec![tag("later"), tag("rust")]);
    }

    #[test]
    fn tagging_an_unsaved_page_fails() {
        let mut bookmarks = three_saved();

        let result = bookmarks.tag(&page("d"), Tag::later());

        assert_eq!(result, Err(NotSaved(page("d"))));
    }

    #[test]
    fn untag_takes_only_that_tag_off() {
        let mut bookmarks = three_saved();
        bookmarks.tag(&page("a"), tag("rust")).unwrap();
        bookmarks.tag(&page("a"), Tag::later()).unwrap();

        let result = bookmarks.untag(&page("a"), &Tag::later());

        assert_eq!(result, Ok(()));
        assert_eq!(tags_of(&bookmarks, "a"), vec![tag("rust")]);
    }

    #[test]
    fn untagging_an_unsaved_page_fails() {
        let mut bookmarks = three_saved();

        let result = bookmarks.untag(&page("d"), &Tag::later());

        assert_eq!(result, Err(NotSaved(page("d"))));
    }

    #[test]
    fn by_tag_lists_the_tagged_pages_in_saving_order() {
        let mut bookmarks = three_saved();
        bookmarks.tag(&page("c"), Tag::later()).unwrap();
        bookmarks.tag(&page("a"), Tag::later()).unwrap();
        bookmarks.tag(&page("b"), tag("rust")).unwrap();

        let later = urls(bookmarks.by_tag(&Tag::later()));

        assert_eq!(later, vec![page("a"), page("c")]);
    }

    #[test]
    fn tags_counts_the_pages_of_each_tag() {
        let mut bookmarks = three_saved();
        for (path, text) in [("a", "rust"), ("b", "rust"), ("b", "later"), ("c", "rust")] {
            bookmarks.tag(&page(path), tag(text)).unwrap();
        }

        let counts = bookmarks.tags();

        let (later, rust) = (tag("later"), tag("rust"));
        assert_eq!(counts, BTreeMap::from([(&later, 1), (&rust, 3)]));
    }
}

mod snapshot {
    use super::*;

    #[test]
    fn restore_gives_back_the_same_bookmarks() {
        let mut bookmarks = three_saved();
        bookmarks.tag(&page("b"), Tag::later()).unwrap();

        let restored = Bookmarks::restore(bookmarks.snapshot());

        assert_eq!(restored, Ok(bookmarks));
    }

    #[test]
    fn the_snapshot_lists_bookmarks_in_saving_order() {
        let snapshot = three_saved().snapshot();

        assert_eq!(
            urls(snapshot.bookmarks.iter()),
            vec![page("a"), page("b"), page("c")]
        );
    }

    #[test]
    fn a_page_saved_twice_is_corrupt() {
        let snapshot = BookmarksSnapshot {
            bookmarks: vec![bookmark("a"), bookmark("b"), bookmark("a")],
        };

        let result = Bookmarks::restore(snapshot);

        assert_eq!(result, Err(CorruptBookmarks::DuplicateBookmark(page("a"))));
    }

    #[test]
    fn a_fragment_is_corrupt() {
        let snapshot = BookmarksSnapshot {
            bookmarks: vec![bookmark("a#top")],
        };

        let result = Bookmarks::restore(snapshot);

        assert_eq!(result, Err(CorruptBookmarks::HasFragment(page("a#top"))));
    }
}
