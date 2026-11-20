//! The address overlay's suggestions as list rows: the title, the URL with its host set apart,
//! and, for a page, what it is on the right.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use quickie_application::{Suggestion, SuggestionKind};
use quickie_ui_shared::ListRow;

/// The row of `suggestion`. Equal targets make equal keys, so a row that stays in the answer
/// stays in the table.
#[must_use]
pub fn row(suggestion: &Suggestion) -> ListRow {
    let mut hasher = DefaultHasher::new();
    suggestion.target.hash(&mut hasher);
    ListRow {
        key: hasher.finish(),
        title: suggestion.title.clone(),
        title_matches: suggestion.title_matches.clone(),
        detail: suggestion.url.clone(),
        detail_matches: suggestion.url_matches.clone(),
        detail_strong: suggestion.host.clone(),
        keycaps: None,
        badge: badge(suggestion.kind).map(str::to_owned),
    }
}

/// What a page row says it is. A `Go to` or search row says so in its title.
const fn badge(kind: SuggestionKind) -> Option<&'static str> {
    match kind {
        SuggestionKind::OpenTab => Some("Switch to tab"),
        SuggestionKind::Bookmark => Some("Bookmark"),
        SuggestionKind::History => Some("History"),
        SuggestionKind::GoTo | SuggestionKind::Search => None,
    }
}

#[cfg(test)]
mod tests {
    use quickie_application::{Suggestion, SuggestionKind, Target};
    use url::Url;

    use super::row;

    fn suggestion(kind: SuggestionKind, url: &str) -> Suggestion {
        Suggestion {
            target: Target::Url(Url::parse(url).unwrap()),
            kind,
            title: "GitHub".to_owned(),
            url: url.to_owned(),
            host: 8..18,
            title_matches: vec![0, 1],
            url_matches: vec![8],
            field: url.to_owned(),
        }
    }

    #[test]
    fn a_row_shows_the_title_and_the_url_with_its_host() {
        let row = row(&suggestion(SuggestionKind::History, "https://github.com/"));

        assert_eq!(row.title, "GitHub");
        assert_eq!(row.title_matches, [0, 1]);
        assert_eq!(row.detail, "https://github.com/");
        assert_eq!(row.detail_matches, [8]);
        assert_eq!(row.detail_strong, 8..18);
        assert_eq!(row.keycaps, None);
    }

    #[test]
    fn a_page_says_what_it_is() {
        let badge = |kind| row(&suggestion(kind, "https://github.com/")).badge;

        assert_eq!(
            badge(SuggestionKind::OpenTab).as_deref(),
            Some("Switch to tab")
        );
        assert_eq!(badge(SuggestionKind::Bookmark).as_deref(), Some("Bookmark"));
        assert_eq!(badge(SuggestionKind::History).as_deref(), Some("History"));
        assert_eq!(badge(SuggestionKind::GoTo), None);
        assert_eq!(badge(SuggestionKind::Search), None);
    }

    #[test]
    fn the_target_is_the_key() {
        let first = row(&suggestion(SuggestionKind::History, "https://a.example/"));
        let other_kind = row(&suggestion(SuggestionKind::Bookmark, "https://a.example/"));
        let other_page = row(&suggestion(SuggestionKind::History, "https://b.example/"));

        assert_eq!(first.key, other_kind.key);
        assert_ne!(first.key, other_page.key);
    }
}
