use super::{CorruptShortcuts, Keyword, SearchShortcuts, SearchShortcutsSnapshot, UrlTemplate};

fn keyword(text: &str) -> Keyword {
    Keyword::try_from(text).unwrap()
}

fn template(text: &str) -> UrlTemplate {
    UrlTemplate::try_from(text).unwrap()
}

fn expanded(shortcuts: &SearchShortcuts, keyword: &str, terms: &str) -> Option<String> {
    shortcuts
        .expand(keyword, terms)
        .map(|url| url.as_str().to_owned())
}

/// `x` searching example.test.
fn one_shortcut() -> SearchShortcuts {
    let mut shortcuts = SearchShortcuts::new();
    shortcuts.set(keyword("x"), template("https://example.test/?q=%s"));
    shortcuts
}

mod defaults {
    use super::*;

    #[test]
    fn a_new_profile_gets_six_web_searches() {
        let shortcuts = SearchShortcuts::defaults();

        let keywords: Vec<&str> = shortcuts.iter().map(|(k, _)| k.as_str()).collect();

        assert_eq!(keywords, ["d", "w", "gh", "mdn", "crates", "docs"]);
    }

    #[test]
    fn every_default_is_https() {
        let shortcuts = SearchShortcuts::defaults();

        for (_, template) in shortcuts.iter() {
            assert!(template.as_str().starts_with("https://"), "{template:?}");
        }
    }

    #[test]
    fn each_default_searches_its_site() {
        let shortcuts = SearchShortcuts::defaults();

        let urls = ["d", "w", "gh", "mdn", "crates", "docs"]
            .map(|keyword| expanded(&shortcuts, keyword, "rust lang").unwrap());

        assert_eq!(
            urls,
            [
                "https://duckduckgo.com/?q=rust+lang",
                "https://en.wikipedia.org/w/index.php?search=rust+lang",
                "https://github.com/search?q=rust+lang",
                "https://developer.mozilla.org/en-US/search?q=rust+lang",
                "https://crates.io/search?q=rust+lang",
                "https://docs.rs/releases/search?query=rust+lang",
            ]
        );
    }

    #[test]
    fn a_new_shortcut_set_starts_empty() {
        assert_eq!(SearchShortcuts::new().iter().count(), 0);
    }
}

mod editing {
    use super::*;

    #[test]
    fn set_adds_a_shortcut() {
        let shortcuts = one_shortcut();

        let found = shortcuts.get("x");

        assert_eq!(found, Some(&template("https://example.test/?q=%s")));
    }

    #[test]
    fn set_replaces_and_returns_the_old_template() {
        let mut shortcuts = one_shortcut();

        let old = shortcuts.set(keyword("x"), template("https://other.test/?q=%s"));

        assert_eq!(old, Some(template("https://example.test/?q=%s")));
        assert_eq!(
            expanded(&shortcuts, "x", "a"),
            Some("https://other.test/?q=a".to_owned())
        );
    }

    #[test]
    fn remove_unbinds_the_keyword() {
        let mut shortcuts = one_shortcut();

        let removed = shortcuts.remove("x");

        assert_eq!(removed, Some(template("https://example.test/?q=%s")));
        assert_eq!(shortcuts.get("x"), None);
    }

    #[test]
    fn removing_an_unknown_keyword_changes_nothing() {
        let mut shortcuts = one_shortcut();

        let removed = shortcuts.remove("y");

        assert_eq!(removed, None);
        assert_eq!(shortcuts, one_shortcut());
    }
}

mod expanding {
    use super::*;

    #[test]
    fn expand_fills_the_encoded_terms_in() {
        let shortcuts = one_shortcut();

        let url = expanded(&shortcuts, "x", "c++ & rust");

        assert_eq!(
            url,
            Some("https://example.test/?q=c%2B%2B+%26+rust".to_owned())
        );
    }

    #[test]
    fn an_unknown_keyword_expands_to_nothing() {
        assert_eq!(one_shortcut().expand("y", "rust"), None);
    }
}

mod snapshot {
    use super::*;

    #[test]
    fn restore_gives_back_the_same_shortcuts() {
        let shortcuts = SearchShortcuts::defaults();

        let restored = SearchShortcuts::restore(shortcuts.snapshot());

        assert_eq!(restored, Ok(shortcuts));
    }

    #[test]
    fn the_snapshot_lists_shortcuts_in_order() {
        let snapshot = one_shortcut().snapshot();

        assert_eq!(
            snapshot.shortcuts,
            vec![(keyword("x"), template("https://example.test/?q=%s"))]
        );
    }

    #[test]
    fn a_keyword_twice_is_corrupt() {
        let snapshot = SearchShortcutsSnapshot {
            shortcuts: vec![
                (keyword("x"), template("https://a.test/?q=%s")),
                (keyword("y"), template("https://b.test/?q=%s")),
                (keyword("x"), template("https://c.test/?q=%s")),
            ],
        };

        let result = SearchShortcuts::restore(snapshot);

        assert_eq!(
            result,
            Err(CorruptShortcuts::DuplicateKeyword(keyword("x")))
        );
    }
}
