//! A group of form rows in a standard window, as System Settings draws them: a plate with
//! rounded corners and hairlines between the rows. The rows are the owner's views; the group
//! gives each its frame and draws what is around them.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBox, NSBoxType, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::Plate;
use crate::tokens::{color, radius, space};

/// Rows on a plate.
pub struct FormGroup {
    plate: Retained<Plate>,
    lines: RefCell<Vec<Retained<NSBox>>>,
}

impl FormGroup {
    /// How far a row's content sits in from the group's sides.
    pub const INSET: CGFloat = space::MD;

    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            plate: Plate::new(
                Some(color::group_fill),
                Some(color::separator),
                radius::ROW,
                mtm,
            ),
            lines: RefCell::new(Vec::new()),
        }
    }

    /// The group; add the rows to it.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.plate
    }

    /// Puts the group at `origin` in its (flipped) superview, `width` wide, for rows of
    /// `heights` from the top, with a hairline between each two. Returns each row's frame in
    /// the group, [`Self::INSET`] in from its sides, and the group's height.
    pub fn place(
        &self,
        origin: NSPoint,
        width: CGFloat,
        heights: &[CGFloat],
    ) -> (Vec<NSRect>, CGFloat) {
        let mtm = self.plate.mtm();
        let mut lines = self.lines.borrow_mut();
        let wanted = heights.len().saturating_sub(1);
        while lines.len() < wanted {
            let line = NSBox::new(mtm);
            line.setBoxType(NSBoxType::Separator);
            self.plate.addSubview(&line);
            lines.push(line);
        }
        for extra in lines.drain(wanted..) {
            extra.removeFromSuperview();
        }
        let inner = width - Self::INSET * 2.0;
        let mut top: CGFloat = 0.0;
        let mut frames = Vec::with_capacity(heights.len());
        for (index, &height) in heights.iter().enumerate() {
            frames.push(NSRect::new(
                NSPoint::new(Self::INSET, top),
                NSSize::new(inner, height),
            ));
            top += height;
            if let Some(line) = lines.get(index) {
                line.setFrame(NSRect::new(
                    NSPoint::new(Self::INSET, top),
                    NSSize::new(inner, 1.0),
                ));
            }
        }
        self.plate
            .setFrame(NSRect::new(origin, NSSize::new(width, top)));
        (frames, top)
    }
}
