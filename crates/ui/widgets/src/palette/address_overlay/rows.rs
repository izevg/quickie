//! The address overlay's suggestions as list rows: the title, the URL with its host set apart,
//! and, for a page, what it is on the right.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use quickie_application::{Suggestion, SuggestionKind};
use quickie_ui_shared::{IconTint, ListRow, RowIcon};

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
        icon: Some(icon(suggestion.kind)),
        trailing: what(suggestion.kind).map(str::to_owned),
        ..ListRow::default()
    }
}

/// The glyph in a row's tile, the same the palette shows for each kind of page.
fn icon(kind: SuggestionKind) -> RowIcon {
    let (name, tint) = match kind {
        SuggestionKind::GoTo => ("arrow.right.circle", IconTint::Accent),
        SuggestionKind::Search => ("magnifyingglass", IconTint::Accent),
        SuggestionKind::OpenTab => ("globe", IconTint::Secondary),
        SuggestionKind::Bookmark => ("bookmark", IconTint::Secondary),
        SuggestionKind::History => ("clock.arrow.circlepath", IconTint::Secondary),
    };
    RowIcon::Symbol {
        name: name.to_owned(),
        tint,
    }
}

/// What a page row says it is, as a note on the right. A `Go to` or search row says so in its
/// title.
const fn what(kind: SuggestionKind) -> Option<&'static str> {
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
    use quickie_ui_shared::{IconTint, RowIcon};
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
        let what = |kind| row(&suggestion(kind, "https://github.com/")).trailing;

        assert_eq!(
            what(SuggestionKind::OpenTab).as_deref(),
            Some("Switch to tab")
        );
        assert_eq!(what(SuggestionKind::Bookmark).as_deref(), Some("Bookmark"));
        assert_eq!(what(SuggestionKind::History).as_deref(), Some("History"));
        assert_eq!(what(SuggestionKind::GoTo), None);
        assert_eq!(what(SuggestionKind::Search), None);
    }

    #[test]
    fn each_kind_has_its_glyph() {
        let glyph = |kind| match row(&suggestion(kind, "https://github.com/")).icon {
            Some(RowIcon::Symbol { name, tint }) => (name, tint),
            other => panic!("no symbol: {other:?}"),
        };

        assert_eq!(
            glyph(SuggestionKind::GoTo),
            ("arrow.right.circle".to_owned(), IconTint::Accent)
        );
        assert_eq!(
            glyph(SuggestionKind::Search),
            ("magnifyingglass".to_owned(), IconTint::Accent)
        );
        assert_eq!(
            glyph(SuggestionKind::OpenTab),
            ("globe".to_owned(), IconTint::Secondary)
        );
        assert_eq!(
            glyph(SuggestionKind::Bookmark),
            ("bookmark".to_owned(), IconTint::Secondary)
        );
        assert_eq!(
            glyph(SuggestionKind::History),
            ("clock.arrow.circlepath".to_owned(), IconTint::Secondary)
        );
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
