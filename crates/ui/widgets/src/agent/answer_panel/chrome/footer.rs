//! The panel's footer (`docs/design/mocks/agent/ask/`): a hairline, the keys that work now on
//! the left, and what to do next on the right: follow up or copy a finished answer, retry a
//! failed one. Without an endpoint it says where the page goes, which is nowhere but there.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{Emphasis, Plate, SheetAnswer, hint_bar, keyed_button};

/// What the footer offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FooterKind {
    /// No endpoint yet: where the page goes.
    Setup,
    /// The agent works: stop or close.
    Working,
    /// An answer is there: follow up, copy it.
    Answered,
    /// It failed: try again.
    Failed,
}

/// The footer: a flipped bar the panel places at its bottom.
pub struct Footer {
    bar: Retained<Plate>,
    line: Retained<Plate>,
    parts: RefCell<Vec<Retained<NSView>>>,
    /// Buttons on the right, then the left side's views.
    split: RefCell<usize>,
    mtm: MainThreadMarker,
}

impl Footer {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let bar = Plate::new(None, None, 0.0, mtm);
        let line = Plate::new(Some(color::separator), None, 0.0, mtm);
        bar.addSubview(&line);
        Self {
            bar,
            line,
            parts: RefCell::new(Vec::new()),
            split: RefCell::new(0),
            mtm,
        }
    }

    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.bar
    }

    /// Shows what `kind` offers; its buttons send `follow:`, `copy:`, and `retry:` to `target`.
    ///
    /// # Safety
    ///
    /// `target` must implement those actions, each taking the sender, and outlive the footer.
    pub unsafe fn set(&self, kind: FooterKind, target: &AnyObject) {
        let mtm = self.mtm;
        for part in self.parts.borrow_mut().drain(..) {
            part.removeFromSuperview();
        }
        let button = |label: &str, keys: &str, emphasis, glyph, action: Sel| {
            let answer = SheetAnswer {
                label: label.to_owned(),
                keys: keys.to_owned(),
                emphasis,
                symbol: glyph,
            };
            // SAFETY: the caller promises `target` implements the footer's actions and
            // outlives its buttons.
            unsafe { keyed_button(&answer, None, target, action, 0, mtm) }
        };
        let buttons: Vec<Retained<NSView>> = match kind {
            FooterKind::Setup | FooterKind::Working => Vec::new(),
            FooterKind::Answered => vec![
                button("Follow-up", "⏎", Emphasis::Plain, None, sel!(follow:)),
                button("Copy Answer", "", Emphasis::Plain, None, sel!(copy:)),
            ],
            FooterKind::Failed => vec![button(
                "Retry",
                "⏎",
                Emphasis::Agent,
                Some("arrow.clockwise"),
                sel!(retry:),
            )],
        };
        let left: Retained<NSView> = match kind {
            FooterKind::Setup => {
                let note = NSTextField::labelWithString(
                    &NSString::from_str("No data leaves your Mac when the model runs locally"),
                    mtm,
                );
                note.setFont(Some(&TextRole::Caption.mono_font()));
                note.setTextColor(Some(&color::secondary_label()));
                note.sizeToFit();
                note.into_super().into_super()
            }
            FooterKind::Working => hint_bar(&[("⌘.", "stop"), ("⎋", "close")], mtm),
            FooterKind::Answered | FooterKind::Failed => hint_bar(&[("⎋", "close")], mtm),
        };
        *self.split.borrow_mut() = buttons.len();
        let mut parts = buttons;
        parts.push(left);
        for part in &parts {
            self.bar.addSubview(part);
        }
        *self.parts.borrow_mut() = parts;
    }

    /// Lays the footer out `width` wide and returns its height.
    pub fn lay_out(&self, width: CGFloat) -> CGFloat {
        let parts = self.parts.borrow();
        let split = *self.split.borrow();
        let row = parts
            .iter()
            .map(|part| part.frame().size.height)
            .fold(0.0, CGFloat::max);
        let top = 1.0 + space::MD;
        let height = (top + row).ceil();
        self.line
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 1.0)));
        let middle = |of: CGFloat| top + ((row - of) / 2.0).round();
        let mut right = width;
        for button in parts.iter().take(split).rev() {
            let size = button.frame().size;
            right -= size.width;
            button.setFrameOrigin(NSPoint::new(right, middle(size.height)));
            right -= space::SM;
        }
        if let Some(left) = parts.get(split) {
            let size = left.frame().size;
            left.setFrame(NSRect::new(
                NSPoint::new(0.0, middle(size.height)),
                NSSize::new(size.width.min((right - space::SM).max(0.0)), size.height),
            ));
        }
        self.bar.setFrameSize(NSSize::new(width, height));
        height
    }
}
