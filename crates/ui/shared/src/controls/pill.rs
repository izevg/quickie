//! A pill: a capsule of E1 glass holding a short line of text. Toasts, the origin chip, and
//! status chips are pills (`docs/design/mocks/components/toast-pill.html`).

use objc2::MainThreadMarker;
use objc2_app_kit::{NSLineBreakMode, NSTextField, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::glass::{Elevation, GlassSurface};
use crate::tokens::{TextRole, color, space};

/// How wide a pill may grow before its text is truncated.
const MAX_WIDTH: f64 = 480.0;

/// A pill with its text, sized to fit. Place it with [`Pill::view`] and [`Pill::size`].
pub struct Pill {
    surface: GlassSurface,
    size: NSSize,
}

impl Pill {
    /// A neutral pill reading `text`.
    #[must_use]
    pub fn new(text: &str, mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E1, mtm);
        let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        label.setFont(Some(&TextRole::RowTitle.font()));
        label.setTextColor(Some(&color::label()));
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        label.sizeToFit();
        let text_size = label.frame().size;
        let width = (text_size.width + space::LG * 2.0).min(MAX_WIDTH).ceil();
        let height = (text_size.height + space::SM * 2.0).ceil();
        label.setFrame(NSRect::new(
            NSPoint::new(space::LG, space::SM),
            NSSize::new(width - space::LG * 2.0, text_size.height),
        ));
        surface.content_view().addSubview(&label);
        let size = NSSize::new(width, height);
        surface.set_frame(NSRect::new(NSPoint::new(0.0, 0.0), size));
        Self { surface, size }
    }

    /// The view to add above the page.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.surface.view()
    }

    /// Its size, to lay it out.
    #[must_use]
    pub fn size(&self) -> NSSize {
        self.size
    }

    /// Moves it to `origin` in its superview.
    pub fn place(&self, origin: NSPoint) {
        self.surface.set_frame(NSRect::new(origin, self.size));
    }
}
