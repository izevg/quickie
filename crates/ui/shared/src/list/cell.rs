//! One row of the list as a view: the title with its fuzzy-match highlights, the detail in
//! SF Mono below it (a URL's host set apart), keycaps or a badge on the right. A selected row gets a light accent tint, never glass,
//! and keeps its primary text and accent highlights (`docs/design/system.md`, `selection-fill`).

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSFont, NSLineBreakMode, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::ListRow;
use crate::controls::{StyledText, char_ranges, keycap_group};
use crate::tokens::{TextRole, color, radius, space};

/// The heights a row is laid out with, measured once from the current text size.
#[derive(Clone, Copy, Debug)]
pub(super) struct Metrics {
    title: CGFloat,
    detail: CGFloat,
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
        }
    }

    /// The height of every row.
    pub(super) fn row_height(self) -> CGFloat {
        self.title + self.detail + space::SM * 2.0
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
    let height = metrics.row_height();
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
    );
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    if selected {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setBackgroundColor(Some(&color::selection_fill().CGColor()));
            layer.setCornerRadius(radius::ROW);
        }
    }
    let mut text_right = width - space::MD;
    if let Some(badge) = &row.badge {
        let label = NSTextField::labelWithString(&NSString::from_str(badge), mtm);
        label.setFont(Some(&TextRole::Caption.font()));
        label.setTextColor(Some(&color::secondary_label()));
        label.sizeToFit();
        let size = label.frame().size;
        label.setFrameOrigin(NSPoint::new(
            (text_right - size.width).floor(),
            ((height - size.height) / 2.0).floor(),
        ));
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        view.addSubview(&label);
        text_right -= size.width + space::SM;
    }
    if let Some(chord) = &row.keycaps {
        let caps = keycap_group(chord, selected, mtm);
        let size = caps.frame().size;
        caps.setFrameOrigin(NSPoint::new(
            (text_right - size.width).floor(),
            ((height - size.height) / 2.0).floor(),
        ));
        caps.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        view.addSubview(&caps);
        text_right -= size.width + space::SM;
    }
    let text_width = (text_right - space::MD).max(0.0);
    let title = label(
        &row.title,
        &row.title_matches,
        (&TextRole::RowTitle.font(), &bold(TextRole::RowTitle)),
        selected,
        mtm,
    );
    title.setFrame(NSRect::new(
        NSPoint::new(space::MD, height - space::SM - metrics.title),
        NSSize::new(text_width, metrics.title),
    ));
    let detail = detail(row, selected, mtm);
    detail.setFrame(NSRect::new(
        NSPoint::new(space::MD, space::SM),
        NSSize::new(text_width, metrics.detail),
    ));
    for text in [&title, &detail] {
        text.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        view.addSubview(text);
    }
    view
}

/// A one-line title with the characters at `matches` set in `fonts.1` and the accent, on a
/// selected row too: its fill is a light tint, so accent text stays readable on it.
fn label(
    text: &str,
    matches: &[u32],
    fonts: (&NSFont, &NSFont),
    selected: bool,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let base = primary(selected);
    let highlight = color::accent();
    let styled = StyledText::new(text, fonts.0, &base);
    for range in char_ranges(text, matches) {
        styled.style(range, fonts.1, &highlight);
    }
    one_line(&styled, mtm)
}

/// The detail line in SF Mono and the secondary colour: its strong characters (a host) in
/// semibold and the primary colour, its matches in the accent.
fn detail(row: &ListRow, selected: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let role = TextRole::RowDetail;
    let (regular, strong) = (role.mono_font(), role.strong_mono_font());
    let styled = StyledText::new(&row.detail, &regular, &color::secondary_label());
    let host: Vec<u32> = row.detail_strong.clone().collect();
    for range in char_ranges(&row.detail, &host) {
        styled.style(range, &strong, &primary(selected));
    }
    let highlight = color::accent();
    let (in_host, elsewhere): (Vec<u32>, Vec<u32>) = row
        .detail_matches
        .iter()
        .partition(|at| row.detail_strong.contains(at));
    for (positions, font) in [(in_host, &strong), (elsewhere, &regular)] {
        for range in char_ranges(&row.detail, &positions) {
            styled.style(range, font, &highlight);
        }
    }
    one_line(&styled, mtm)
}

/// The colour of primary text on a row.
fn primary(selected: bool) -> Retained<NSColor> {
    if selected {
        color::selection_text()
    } else {
        color::label()
    }
}

fn one_line(styled: &StyledText, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::labelWithAttributedString(styled.string(), mtm);
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field
}

/// The role's font in semibold, for matched characters of a title.
fn bold(role: TextRole) -> Retained<NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let semibold = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::systemFontOfSize_weight(role.point_size(), semibold)
}
