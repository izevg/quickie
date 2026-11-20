//! Sections of the list: a header (`COMMANDS` on the left, `3 MATCHES` on the right, in
//! uppercase SF Mono caption) above its rows, a hairline above every header but the first. The
//! list keeps headers and rows as one run of entries; the selection only ever lands on rows.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSLineBreakMode, NSTextAlignment, NSTextField,
    NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::ListRow;
use crate::form::Plate;
use crate::tokens::{TextRole, color, space};

/// Rows under a header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListSection {
    /// What the rows are: `Commands`, `Open tabs`; shown in uppercase.
    pub title: String,
    /// How many there are, measured: `3 matches`; shown in uppercase on the right.
    pub counter: String,
    pub rows: Vec<ListRow>,
}

/// One line of the table: a section's header or one of its rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Header {
        title: String,
        counter: String,
        /// The first header has no hairline above it.
        first: bool,
    },
    Row(ListRow),
}

impl Entry {
    /// The entry's identity across updates: a row's own key, a header's title.
    pub(super) fn key(&self) -> u64 {
        match self {
            Self::Header { title, .. } => {
                let mut hasher = DefaultHasher::new();
                ("list-section-header", title).hash(&mut hasher);
                hasher.finish()
            }
            Self::Row(row) => row.key,
        }
    }

    pub(super) fn row(&self) -> Option<&ListRow> {
        match self {
            Self::Header { .. } => None,
            Self::Row(row) => Some(row),
        }
    }
}

/// The entries of `sections` in order: each non-empty section's header, then its rows. A
/// section without a title shows its rows without a header.
pub(super) fn entries(sections: Vec<ListSection>) -> Vec<Entry> {
    let mut entries = Vec::new();
    for section in sections
        .into_iter()
        .filter(|section| !section.rows.is_empty())
    {
        if !section.title.is_empty() {
            let first = entries.is_empty();
            entries.push(Entry::Header {
                title: section.title,
                counter: section.counter,
                first,
            });
        }
        entries.extend(section.rows.into_iter().map(Entry::Row));
    }
    entries
}

/// A header's height: a caption line with `space-sm` above and `space-xs` below, and room for
/// the hairline that parts it from the section before.
pub(super) fn header_height(caption: CGFloat) -> CGFloat {
    caption + space::SM + space::XS + space::SM
}

/// The view of a header `width` wide and `height` tall.
pub(super) fn header(
    title: &str,
    counter: &str,
    first: bool,
    width: CGFloat,
    height: CGFloat,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
    );
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    if !first {
        let line = Plate::new(Some(color::separator), None, 0.0, mtm);
        line.setFrame(NSRect::new(
            NSPoint::new(space::MD, height - space::XS),
            NSSize::new(width - space::MD * 2.0, 1.0),
        ));
        line.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        view.addSubview(&line);
    }
    let left = caption(title, mtm);
    let right = caption(counter, mtm);
    right.setAlignment(NSTextAlignment::Right);
    let line_height = left.frame().size.height;
    let half = ((width - space::MD * 2.0) / 2.0).floor();
    left.setFrame(NSRect::new(
        NSPoint::new(space::MD, space::XS),
        NSSize::new(half, line_height),
    ));
    right.setFrame(NSRect::new(
        NSPoint::new(width - space::MD - half, space::XS),
        NSSize::new(half, line_height),
    ));
    right.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
    view.addSubview(&left);
    view.addSubview(&right);
    view.setAccessibilityElement(true);
    view.setAccessibilityLabel(Some(&NSString::from_str(&spoken(title, counter))));
    view
}

/// What `VoiceOver` reads for a header: `Commands, 3 matches`.
pub(super) fn spoken(title: &str, counter: &str) -> String {
    if counter.is_empty() {
        title.to_owned()
    } else {
        format!("{title}, {counter}")
    }
}

/// `text` in uppercase SF Mono caption, tertiary, one line.
fn caption(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(&text.to_uppercase()), mtm);
    field.setFont(Some(&TextRole::Caption.mono_font()));
    field.setTextColor(Some(&color::secondary_label()));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

#[cfg(test)]
mod tests {
    use super::{Entry, ListSection, entries, spoken};
    use crate::list::ListRow;

    fn row(key: u64) -> ListRow {
        ListRow {
            key,
            title: format!("row {key}"),
            ..ListRow::default()
        }
    }

    fn section(title: &str, keys: &[u64]) -> ListSection {
        ListSection {
            title: title.to_owned(),
            counter: format!("{} rows", keys.len()),
            rows: keys.iter().copied().map(row).collect(),
        }
    }

    #[test]
    fn each_section_is_its_header_then_its_rows() {
        let entries = entries(vec![section("Tabs", &[1, 2]), section("History", &[3])]);

        let shape: Vec<String> = entries
            .iter()
            .map(|entry| match entry {
                Entry::Header { title, first, .. } => {
                    format!("{title}{}", if *first { "!" } else { "" })
                }
                Entry::Row(row) => row.key.to_string(),
            })
            .collect();
        assert_eq!(shape, ["Tabs!", "1", "2", "History", "3"]);
    }

    #[test]
    fn an_empty_section_shows_no_header() {
        let entries = entries(vec![section("Tabs", &[]), section("History", &[3])]);

        assert_eq!(entries.len(), 2);
        assert!(
            matches!(&entries[0], Entry::Header { title, first: true, .. } if title == "History")
        );
    }

    #[test]
    fn rows_without_a_title_have_no_header() {
        let entries = entries(vec![section("", &[1, 2])]);

        assert!(entries.iter().all(|entry| entry.row().is_some()));
    }

    #[test]
    fn headers_and_rows_keep_their_keys_apart() {
        let header = &entries(vec![section("Tabs", &[7])])[0];

        assert_ne!(header.key(), 7);
        assert_eq!(header.key(), entries(vec![section("Tabs", &[8])])[0].key());
    }

    #[test]
    fn a_header_reads_its_title_and_counter() {
        assert_eq!(spoken("Commands", "3 matches"), "Commands, 3 matches");
        assert_eq!(spoken("History", ""), "History");
    }
}
