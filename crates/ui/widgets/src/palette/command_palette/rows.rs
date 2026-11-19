//! The palette's answers as list rows.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use quickie_application::PaletteItem;
use quickie_ui_shared::ListRow;

/// The row of `item`: its title and detail with their highlights, and, for a command, the
/// first chord bound to it as keycaps. Equal targets make equal keys, so a row that stays in
/// the answer stays in the table.
#[must_use]
pub fn row(item: &PaletteItem) -> ListRow {
    let mut hasher = DefaultHasher::new();
    item.target.hash(&mut hasher);
    ListRow {
        key: hasher.finish(),
        title: item.title.clone(),
        title_matches: item.title_matches.clone(),
        detail: item.detail.clone(),
        detail_matches: item.detail_matches.clone(),
        keycaps: item.glyphs.first().cloned(),
    }
}

#[cfg(test)]
mod tests {
    use quickie_application::{PaletteItem, Target};
    use quickie_domain_palette::Kind;
    use url::Url;

    use super::row;

    fn item(url: &str, title: &str, glyphs: &[&str]) -> PaletteItem {
        PaletteItem {
            target: Target::Url(Url::parse(url).unwrap()),
            kind: Kind::History,
            title: title.to_owned(),
            detail: url.to_owned(),
            glyphs: glyphs.iter().map(|&glyph| glyph.to_owned()).collect(),
            title_matches: vec![0, 1],
            detail_matches: vec![8],
        }
    }

    #[test]
    fn a_row_shows_the_item_with_its_highlights() {
        let row = row(&item("https://a.example/", "Alpha", &[]));

        assert_eq!(row.title, "Alpha");
        assert_eq!(row.title_matches, [0, 1]);
        assert_eq!(row.detail, "https://a.example/");
        assert_eq!(row.detail_matches, [8]);
        assert_eq!(row.keycaps, None);
    }

    #[test]
    fn a_command_shows_its_first_chord() {
        let row = row(&item("https://a.example/", "Palette", &["⇧⌘P", "⌘K"]));

        assert_eq!(row.keycaps.as_deref(), Some("⇧⌘P"));
    }

    #[test]
    fn the_target_is_the_key() {
        let first = row(&item("https://a.example/", "Alpha", &[]));
        let renamed = row(&item("https://a.example/", "Alpha, renamed", &[]));
        let other = row(&item("https://b.example/", "Alpha", &[]));

        assert_eq!(first.key, renamed.key);
        assert_ne!(first.key, other.key);
    }
}
