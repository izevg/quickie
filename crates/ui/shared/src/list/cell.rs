//! One row of the list as a view (`docs/design/mocks/components/command-row.html`).
//!
//! A row without detail is one line, 40 pt: a 16 pt symbol, the title with its fuzzy-match
//! highlights, an uppercase SF Mono category tag that is never highlighted, keycaps on the
//! right. A row with detail (a page) is two lines: a 24 pt tile with the favicon or a glyph, the
//! title and a note (`5 min ago`), the address below in SF Mono, and on the right the slot badge,
//! a trailing note (`Visited 6 times`), and keycaps. A selected row gets the accent tint with a
//! hairline accent border, never glass, and keeps its primary text and accent highlights
//! (`docs/design/system.md`, `selection-fill`).

use objc2::rc::Retained;
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSColor, NSFont, NSImage, NSImageScaling,
    NSImageView, NSLineBreakMode, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize, NSString};

use super::{IconTint, ListRow, RowIcon};
use crate::controls::{KeycapStyle, StyledText, char_ranges, keycap, keycap_group, symbol};
use crate::form::Plate;
use crate::tokens::{TextRole, color, control, radius, space};

/// The heights a row is laid out with, measured once from the current text size.
#[derive(Clone, Copy, Debug)]
pub(super) struct Metrics {
    title: CGFloat,
    detail: CGFloat,
    pub(super) caption: CGFloat,
}

impl Metrics {
    pub(super) fn measure(mtm: MainThreadMarker) -> Self {
        let line = |font: &NSFont| {
            let label = NSTextField::labelWithString(&NSString::from_str("Ag"), mtm);
            label.setFont(Some(font));
            label.sizeToFit();
            label.frame().size.height.ceil()
        };
        Self {
            title: line(&TextRole::RowTitle.font()),
            detail: line(&TextRole::RowDetail.mono_font()),
            caption: line(&TextRole::Caption.mono_font()),
        }
    }

    /// The height of a two-line row.
    pub(super) fn two_lines(self) -> CGFloat {
        self.title + self.detail + space::SM * 2.0
    }

    /// The height of `row`: one line without detail, two with.
    pub(super) fn height_of(self, row: &ListRow) -> CGFloat {
        if row.detail.is_empty() {
            control::ROW_HEIGHT
        } else {
            self.two_lines()
        }
    }
}

/// A row being laid out: its view and size, and whether it is selected.
struct Line<'a> {
    row: &'a ListRow,
    view: Retained<NSView>,
    height: CGFloat,
    selected: bool,
    mtm: MainThreadMarker,
}

impl Line<'_> {
    /// `view`'s origin that centres it vertically, at `x`.
    fn centre(&self, view: &NSView, x: CGFloat) {
        let size = view.frame().size;
        view.setFrameOrigin(NSPoint::new(x, ((self.height - size.height) / 2.0).floor()));
        self.view.addSubview(view);
    }

    /// The right side, from the edge inwards: keycaps, the trailing note, the slot badge. The
    /// x where the text must end.
    fn trailing(&self, width: CGFloat) -> CGFloat {
        let row = self.row;
        let mut items: Vec<Retained<NSView>> = Vec::new();
        items.extend(
            row.keycaps
                .as_deref()
                .map(|chord| keycap_group(chord, self.selected, self.mtm)),
        );
        items.extend(
            row.trailing
                .as_deref()
                .map(|note| note_view(note, self.mtm)),
        );
        items.extend(
            row.badge
                .as_deref()
                .map(|badge| keycap(badge, KeycapStyle::Accent, self.mtm)),
        );
        let mut right = width - space::MD;
        for item in items {
            right -= item.frame().size.width;
            self.centre(&item, right.floor());
            item.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
            right -= space::SM;
        }
        right + space::SM - space::MD
    }

    /// The icon: a bare symbol on a one-line row, a tile on a page row. The x where the text
    /// starts.
    fn leading(&self, two_lines: bool) -> CGFloat {
        let left = space::MD;
        if two_lines {
            self.centre(&tile(self.row.icon.as_ref(), self.mtm), left);
            return left + control::TILE + space::MD;
        }
        let Some(icon) = &self.row.icon else {
            return left;
        };
        if let Some(view) = icon_view(icon, control::ICON, self.mtm) {
            self.centre(&view, left);
        }
        left + control::ICON + space::MD
    }

    /// The title with its tag or note after it, and the address below it on a two-line row,
    /// between `left` and `right`.
    fn text(&self, metrics: Metrics, left: CGFloat, right: CGFloat, two_lines: bool) {
        let row = self.row;
        let room = (right - left).max(0.0);
        let top = if two_lines {
            self.height - space::SM - metrics.title
        } else {
            ((self.height - metrics.title) / 2.0).floor()
        };
        let title = highlighted(&row.title, &row.title_matches, self.selected, self.mtm);
        // What follows the title on its line: the category tag, or the page's note.
        let after = match (&row.tag, &row.note) {
            (Some(tag), _) => Some(tag_view(tag, self.mtm)),
            (None, Some(note)) => Some(note_view(note, self.mtm)),
            (None, None) => None,
        };
        let after_width = after
            .as_ref()
            .map_or(0.0, |view| view.frame().size.width + space::SM);
        title.sizeToFit();
        let title_width = title
            .frame()
            .size
            .width
            .ceil()
            .min((room - after_width).max(0.0));
        title.setFrame(NSRect::new(
            NSPoint::new(left, top),
            NSSize::new(title_width, metrics.title),
        ));
        self.view.addSubview(&title);
        if let Some(after) = after {
            let size = after.frame().size;
            after.setFrameOrigin(NSPoint::new(
                left + title_width + space::SM,
                (top + (metrics.title - size.height) / 2.0).floor(),
            ));
            self.view.addSubview(&after);
        }
        if two_lines {
            let tint = if self.selected {
                color::accent()
            } else {
                color::secondary_label()
            };
            let detail = label(
                &row.detail,
                &TextRole::RowDetail.mono_font(),
                &tint,
                self.mtm,
            );
            detail.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
            detail.setFrame(NSRect::new(
                NSPoint::new(left, space::SM),
                NSSize::new(room, metrics.detail),
            ));
            detail.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            self.view.addSubview(&detail);
        }
    }
}

/// The view of `row`, `width` wide.
pub(super) fn cell(
    row: &ListRow,
    selected: bool,
    width: CGFloat,
    metrics: Metrics,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let height = metrics.height_of(row);
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
    );
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    view.setAccessibilityElement(true);
    view.setAccessibilityLabel(Some(&NSString::from_str(&spoken(row))));
    if selected {
        let fill = Plate::new(
            Some(color::selection_fill),
            Some(color::accent_line),
            radius::ROW,
            mtm,
        );
        fill.setFrame(view.bounds());
        fill.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        view.addSubview(&fill);
    }
    let line = Line {
        row,
        view,
        height,
        selected,
        mtm,
    };
    let two_lines = !row.detail.is_empty();
    let right = line.trailing(width);
    let left = line.leading(two_lines);
    line.text(metrics, left, right, two_lines);
    line.view
}

/// What `VoiceOver` reads for a row: its title, tag, note, address, slot, and keys, in the
/// order they are seen.
pub(super) fn spoken(row: &ListRow) -> String {
    [
        Some(row.title.as_str()),
        row.tag.as_deref(),
        row.note.as_deref(),
        (!row.detail.is_empty()).then_some(row.detail.as_str()),
        row.badge.as_deref(),
        row.trailing.as_deref(),
        row.keycaps.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ")
}

/// The title, with the characters at `matches` in semibold and the accent, on a selected row
/// too: its fill is a light tint, so accent text stays readable on it.
fn highlighted(
    title: &str,
    matches: &[u32],
    selected: bool,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let base = if selected {
        color::selection_text()
    } else {
        color::label()
    };
    let styled = StyledText::new(title, &TextRole::RowTitle.font(), &base);
    let bold = semibold(TextRole::RowTitle);
    let accent = color::accent();
    for range in char_ranges(title, matches) {
        styled.style(range, &bold, &accent);
    }
    styled.set_line_break(NSLineBreakMode::ByTruncatingTail);
    let field = NSTextField::labelWithAttributedString(styled.string(), mtm);
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field
}

/// A one-line label, sized to fit.
fn label(
    text: &str,
    font: &NSFont,
    tint: &NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(font));
    field.setTextColor(Some(tint));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

/// A note in tertiary SF Mono caption: `5 min ago`, `Visited 6 times`.
fn note_view(note: &str, mtm: MainThreadMarker) -> Retained<NSView> {
    label(
        note,
        &TextRole::Caption.mono_font(),
        &color::tertiary_label(),
        mtm,
    )
    .into_super()
    .into_super()
}

/// The category tag after a command's title: uppercase SF Mono caption on a subtle fill.
fn tag_view(tag: &str, mtm: MainThreadMarker) -> Retained<NSView> {
    let text = label(
        &tag.to_uppercase(),
        &TextRole::Caption.mono_font(),
        &color::secondary_label(),
        mtm,
    );
    let size = text.frame().size;
    let plate = Plate::new(Some(color::subtle_fill), None, radius::KEYCAP - 2.0, mtm);
    let height = (size.height + space::XS / 2.0).ceil();
    plate.setFrameSize(NSSize::new((size.width + space::SM * 1.5).ceil(), height));
    text.setFrame(NSRect::new(
        NSPoint::new(space::SM * 0.75, ((height - size.height) / 2.0).floor()),
        size,
    ));
    plate.addSubview(&text);
    plate.into_super()
}

/// The 24 pt tile of a page row: the favicon, or the row's glyph, on a subtle fill.
fn tile(icon: Option<&RowIcon>, mtm: MainThreadMarker) -> Retained<NSView> {
    let plate = Plate::new(Some(color::subtle_fill), None, radius::KEYCAP, mtm);
    plate.setFrameSize(NSSize::new(control::TILE, control::TILE));
    if let Some(inner) = icon.and_then(|icon| icon_view(icon, control::ICON, mtm)) {
        let size = inner.frame().size;
        inner.setFrameOrigin(NSPoint::new(
            ((control::TILE - size.width) / 2.0).floor(),
            ((control::TILE - size.height) / 2.0).floor(),
        ));
        plate.addSubview(&inner);
    }
    plate.into_super()
}

/// `icon` at most `edge` points on each side; `None` for a symbol the system lacks or a
/// picture that does not decode.
fn icon_view(icon: &RowIcon, edge: CGFloat, mtm: MainThreadMarker) -> Option<Retained<NSView>> {
    match icon {
        RowIcon::Symbol { name, tint } => {
            let tint = match tint {
                IconTint::Secondary => color::secondary_label(),
                IconTint::Agent => color::agent(),
                IconTint::Accent => color::accent(),
            };
            let view = symbol(name, "", TextRole::RowTitle, &tint, mtm)?;
            let natural = view.frame().size;
            let scale = (edge / natural.width.max(natural.height)).min(1.0);
            view.setFrameSize(NSSize::new(
                (natural.width * scale).ceil(),
                (natural.height * scale).ceil(),
            ));
            Some(view.into_super().into_super())
        }
        RowIcon::Picture(bytes) => {
            let data = NSData::with_bytes(bytes);
            let image = NSImage::initWithData(NSImage::alloc(), &data)?;
            image.setSize(NSSize::new(edge, edge));
            let view = NSImageView::imageViewWithImage(&image, mtm);
            view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            view.setFrameSize(NSSize::new(edge, edge));
            Some(view.into_super().into_super())
        }
    }
}

/// The role's font in semibold, for matched characters of a title.
fn semibold(role: TextRole) -> Retained<NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let semibold = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::systemFontOfSize_weight(role.point_size(), semibold)
}

#[cfg(test)]
mod tests {
    use super::spoken;
    use crate::list::ListRow;

    #[test]
    fn a_command_reads_title_tag_and_keys() {
        let row = ListRow {
            title: "Reload".to_owned(),
            tag: Some("Navigation".to_owned()),
            keycaps: Some("⌘R".to_owned()),
            ..ListRow::default()
        };

        assert_eq!(spoken(&row), "Reload, Navigation, ⌘R");
    }

    #[test]
    fn a_page_reads_everything_it_shows() {
        let row = ListRow {
            title: "Hacker News".to_owned(),
            note: Some("5 min ago".to_owned()),
            detail: "news.ycombinator.com".to_owned(),
            badge: Some("⌘3".to_owned()),
            trailing: Some("Visited 6 times".to_owned()),
            ..ListRow::default()
        };

        assert_eq!(
            spoken(&row),
            "Hacker News, 5 min ago, news.ycombinator.com, ⌘3, Visited 6 times"
        );
    }
}
