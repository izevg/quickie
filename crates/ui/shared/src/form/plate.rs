//! A plate: a rounded rectangle with a fill and a hairline border, behind a group of settings
//! rows, a keycap, or a status card. Its colors are semantic and resolved each time the layer
//! is drawn, so they follow light and dark, the accent color, and Increase Contrast (which also
//! doubles the border) without the owner doing anything.

use std::cell::Cell;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAppearanceCustomization, NSColor, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::NSRect;

use crate::tokens::increased_contrast;

/// A color that is looked up each time it is drawn: one of [`crate::tokens::color`].
pub type Paint = fn() -> Retained<NSColor>;

pub struct Ivars {
    fill: Cell<Option<Paint>>,
    border: Cell<Option<Paint>>,
    radius: Cell<CGFloat>,
}

define_class!(
    // SAFETY: `NSView` has no subclassing requirements, and `Plate` implements no `Drop`;
    // `isFlipped`, `wantsUpdateLayer`, and `updateLayer` are public API meant to be overridden.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickiePlate"]
    #[ivars = Ivars]
    pub struct Plate;

    impl Plate {
        /// Content lays out top down.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            self.paint();
        }
    }
);

impl Plate {
    /// A plate with `fill` and `border` (either may be `None`) and `radius` corners.
    #[must_use]
    pub fn new(
        fill: Option<Paint>,
        border: Option<Paint>,
        radius: CGFloat,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            fill: Cell::new(fill),
            border: Cell::new(border),
            radius: Cell::new(radius),
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer and the ivars are set.
        let plate: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        plate.setWantsLayer(true);
        plate
    }

    /// Changes the corner radius, as when a pill it rings changes height; drawn again.
    pub fn set_radius(&self, radius: CGFloat) {
        self.ivars().radius.set(radius);
        self.setNeedsDisplay(true);
    }

    /// Changes the colors; the plate is drawn again.
    pub fn set_paint(&self, fill: Option<Paint>, border: Option<Paint>) {
        self.ivars().fill.set(fill);
        self.ivars().border.set(border);
        self.setNeedsDisplay(true);
    }

    fn paint(&self) {
        let Some(layer) = self.layer() else {
            return;
        };
        let ivars = self.ivars();
        let (fill, border, radius) = (ivars.fill.get(), ivars.border.get(), ivars.radius.get());
        let strong = increased_contrast(self);
        // Resolved under the view's own appearance: a dynamic color has no single CGColor.
        let apply = RcBlock::new(move || {
            layer.setCornerRadius(radius);
            layer.setBackgroundColor(fill.map(|paint| paint().CGColor()).as_deref());
            layer.setBorderColor(border.map(|paint| paint().CGColor()).as_deref());
            let width = match (border, strong) {
                (None, _) => 0.0,
                (Some(_), false) => 1.0,
                (Some(_), true) => 2.0,
            };
            layer.setBorderWidth(width);
        });
        self.effectiveAppearance()
            .performAsCurrentDrawingAppearance(&apply);
    }
}
