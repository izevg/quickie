//! The `⌃⇥` switcher: an E2 glass strip of the cycle's tab titles, the selection filled with
//! the accent. It shows only after a hold, and the keyboard drives it, so it appears, moves and
//! disappears instantly. Titles only; thumbnails come with the `tab_thumbnail` entity.

use std::cell::{Cell, RefCell};
use std::ops::Range;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSLineBreakMode, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, overlay, radius, space};
use quickie_ui_shared::{Elevation, GlassSurface, Scrim, Trigger, motion};

pub struct TabSwitcher {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    highlight: Retained<NSView>,
    labels: RefCell<Vec<Retained<NSTextField>>>,
    titles: RefCell<Vec<String>>,
    /// How many cells fit across the host.
    capacity: Cell<usize>,
    open: Cell<bool>,
    mtm: MainThreadMarker,
}

impl TabSwitcher {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let highlight = NSView::new(mtm);
        highlight.setWantsLayer(true);
        surface.content_view().addSubview(&highlight);
        Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface,
            highlight,
            labels: RefCell::new(Vec::new()),
            titles: RefCell::new(Vec::new()),
            capacity: Cell::new(1),
            open: Cell::new(false),
            mtm,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Shows `titles` over `host`, centered, with `selected` highlighted.
    pub fn show(&self, host: &NSView, titles: Vec<String>, selected: usize) {
        *self.titles.borrow_mut() = titles;
        let room = host.bounds().size.width - space::XL * 2.0 - space::SM * 2.0;
        let fits = (room / overlay::SWITCHER_CELL_WIDTH).floor();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "floored, clamped to at least 1, and a window's width in cells"
        )]
        let capacity = fits.max(1.0) as usize;
        self.capacity.set(capacity);
        self.render(selected, host);
        if !self.is_open() {
            if let Some(scrim) = &self.scrim {
                scrim.view().setFrame(host.bounds());
                host.addSubview(scrim.view());
            }
            host.addSubview(self.surface.view());
            motion::show(
                self.surface.view(),
                motion::OVERLAY_ENTER,
                Trigger::Keyboard,
            );
            self.open.set(true);
        }
    }

    /// Moves the highlight to `selected`, scrolling the strip when it is out of view.
    pub fn select(&self, selected: usize) {
        // SAFETY: a getter; the superview is retained for the duration of the call.
        let host = unsafe { self.surface.view().superview() };
        if let Some(host) = host.filter(|_| self.is_open()) {
            self.render(selected, &host);
        }
    }

    pub fn hide(&self) {
        if !self.is_open() {
            return;
        }
        motion::hide(
            self.surface.view(),
            motion::OVERLAY_ENTER,
            Trigger::Keyboard,
        );
        self.surface.view().removeFromSuperview();
        if let Some(scrim) = &self.scrim {
            scrim.view().removeFromSuperview();
        }
        self.open.set(false);
    }

    fn render(&self, selected: usize, host: &NSView) {
        let titles = self.titles.borrow();
        let range = visible_range(titles.len(), selected, self.capacity.get());
        self.fit_labels(range.len());
        let labels = self.labels.borrow();
        let line = TextRole::RowTitle.font().boundingRectForFont().size.height;
        let cell = NSSize::new(
            overlay::SWITCHER_CELL_WIDTH,
            (line + space::MD * 2.0).ceil(),
        );
        for (slot, (label, index)) in labels.iter().zip(range.clone()).enumerate() {
            let origin = cell_origin(slot, cell);
            label.setStringValue(&NSString::from_str(&titles[index]));
            let text = if index == selected {
                color::selection_text()
            } else {
                color::label()
            };
            label.setTextColor(Some(&text));
            label.setFrame(NSRect::new(
                NSPoint::new(origin.x + space::SM, origin.y + (cell.height - line) / 2.0),
                NSSize::new(cell.width - space::SM * 2.0, line),
            ));
            if index == selected {
                self.highlight.setFrame(NSRect::new(origin, cell));
            }
        }
        self.paint_highlight();
        self.place(host, range.len(), cell);
    }

    /// Makes exactly `count` labels, reusing the ones there are.
    fn fit_labels(&self, count: usize) {
        let mut labels = self.labels.borrow_mut();
        while labels.len() > count {
            if let Some(label) = labels.pop() {
                label.removeFromSuperview();
            }
        }
        while labels.len() < count {
            let label = NSTextField::labelWithString(&NSString::new(), self.mtm);
            label.setFont(Some(&TextRole::RowTitle.font()));
            label.setAlignment(NSTextAlignment::Center);
            label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            self.surface.content_view().addSubview(&label);
            labels.push(label);
        }
    }

    fn paint_highlight(&self) {
        if let Some(layer) = self.highlight.layer() {
            layer.setBackgroundColor(Some(&color::selection_fill().CGColor()));
            layer.setCornerRadius(radius::ROW);
        }
    }

    /// Sizes the strip to `count` cells and centers it in `host`.
    fn place(&self, host: &NSView, count: usize, cell: NSSize) {
        let bounds = host.bounds();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a handful of cells, exact in a float"
        )]
        let cells = count as f64;
        let size = NSSize::new(
            cells * cell.width + space::SM * 2.0,
            cell.height + space::SM * 2.0,
        );
        self.surface.set_frame(NSRect::new(
            NSPoint::new(
                ((bounds.size.width - size.width) / 2.0).round(),
                ((bounds.size.height - size.height) / 2.0).round(),
            ),
            size,
        ));
        self.surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin
                | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
    }
}

fn cell_origin(slot: usize, cell: NSSize) -> NSPoint {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a handful of cells, exact in a float"
    )]
    let slot = slot as f64;
    NSPoint::new(space::SM + slot * cell.width, space::SM)
}

/// The tabs that fit: all of them, or `capacity` around the selection.
fn visible_range(len: usize, selected: usize, capacity: usize) -> Range<usize> {
    let capacity = capacity.max(1);
    if len <= capacity {
        return 0..len;
    }
    let start = selected.saturating_sub(capacity / 2).min(len - capacity);
    start..start + capacity
}

#[cfg(test)]
mod tests {
    use super::visible_range;

    #[test]
    fn everything_shows_when_it_fits() {
        assert_eq!(visible_range(3, 2, 5), 0..3);
        assert_eq!(visible_range(5, 4, 5), 0..5);
    }

    #[test]
    fn the_selection_stays_near_the_middle() {
        assert_eq!(visible_range(20, 10, 5), 8..13);
        assert_eq!(visible_range(20, 1, 5), 0..5);
        assert_eq!(visible_range(20, 19, 5), 15..20);
    }

    #[test]
    fn at_least_one_cell_shows() {
        assert_eq!(visible_range(4, 2, 0), 2..3);
        assert_eq!(visible_range(0, 0, 0), 0..0);
    }
}
