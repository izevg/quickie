//! A table row as the window's tables draw it: the selection is the design system's
//! `selection-fill` (a light accent tint with rounded corners, the text keeps its colors), and
//! a row may carry a tint of its own: red for a conflict, accent while it records keys.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBezierPath, NSTableRowView, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_ui_shared::tokens::{color, increased_contrast, radius, space};

/// How a row's background reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    /// Something on it is in conflict.
    Conflict,
    /// It waits for keys.
    Recording,
}

pub struct RowIvars {
    tone: Cell<Tone>,
}

define_class!(
    // SAFETY: `NSTableRowView` has no subclassing requirements, and `RowView` implements no
    // `Drop`; `drawBackgroundInRect:` and `drawSelectionInRect:` are public API meant to be
    // overridden.
    #[unsafe(super(NSTableRowView, NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieSettingsRow"]
    #[ivars = RowIvars]
    pub struct RowView;

    impl RowView {
        #[unsafe(method(drawBackgroundInRect:))]
        fn draw_background(&self, dirty: NSRect) {
            // SAFETY: the superclass draws the table's own background, alternating rows too.
            let () = unsafe { msg_send![super(self), drawBackgroundInRect: dirty] };
            let tone = self.ivars().tone.get();
            let fill = match tone {
                Tone::Plain => return,
                Tone::Conflict => color::destructive_fill(),
                Tone::Recording => color::selection_fill(),
            };
            fill.setFill();
            NSBezierPath::fillRect(self.bounds());
            if tone == Tone::Conflict && increased_contrast(self) {
                color::destructive().setStroke();
                NSBezierPath::strokeRect(inset(self.bounds(), 0.5, 0.5));
            }
        }

        /// A selected row is a light accent tint with rounded corners, as in the palette; a
        /// tinted row keeps its own tint.
        #[unsafe(method(drawSelectionInRect:))]
        fn draw_selection(&self, _dirty: NSRect) {
            if self.ivars().tone.get() != Tone::Plain {
                return;
            }
            color::selection_fill().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                inset(self.bounds(), space::XS, 1.0),
                radius::ROW,
                radius::ROW,
            )
            .fill();
        }
    }
);

impl RowView {
    pub fn new(tone: Tone, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RowIvars {
            tone: Cell::new(tone),
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    /// Tints the row for `tone` from now on.
    pub fn set_tone(&self, tone: Tone) {
        self.ivars().tone.set(tone);
        self.setNeedsDisplay(true);
    }
}

fn inset(rect: NSRect, dx: CGFloat, dy: CGFloat) -> NSRect {
    NSRect::new(
        NSPoint::new(rect.origin.x + dx, rect.origin.y + dy),
        NSSize::new(
            (rect.size.width - dx * 2.0).max(0.0),
            (rect.size.height - dy * 2.0).max(0.0),
        ),
    )
}
