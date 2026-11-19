//! One row of the list as a view: the title with its fuzzy-match highlights, the detail in
//! SF Mono below it, keycaps on the right. A selected row gets a light accent tint, never glass,
//! and keeps its primary text and accent highlights (`docs/design/system.md`, `selection-fill`).

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSFont, NSLineBreakMode, NSTextField, NSView};
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
    if let Some(chord) = &row.keycaps {
        let caps = keycap_group(chord, selected, mtm);
        let size = caps.frame().size;
        caps.setFrameOrigin(NSPoint::new(
            (width - space::MD - size.width).floor(),
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
        true,
        mtm,
    );
    title.setFrame(NSRect::new(
        NSPoint::new(space::MD, height - space::SM - metrics.title),
        NSSize::new(text_width, metrics.title),
    ));
    let detail = label(
        &row.detail,
        &row.detail_matches,
        (
            &TextRole::RowDetail.mono_font(),
            &TextRole::RowDetail.mono_font(),
        ),
        selected,
        false,
        mtm,
    );
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

/// A one-line label with the characters at `matches` set in `fonts.1` and the accent, on a
/// selected row too: its fill is a light tint, so accent text stays readable on it.
fn label(
    text: &str,
    matches: &[u32],
    fonts: (&NSFont, &NSFont),
    selected: bool,
    primary: bool,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let base = match (selected, primary) {
        (true, true) => color::selection_text(),
        (false, true) => color::label(),
        (_, false) => color::secondary_label(),
    };
    let highlight = color::accent();
    let styled = StyledText::new(text, fonts.0, &base);
    for range in char_ranges(text, matches) {
        styled.style(range, fonts.1, &highlight);
    }
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
