//! The hint bar: what an overlay's keys do, `[↑↓] select  [⏎] open  [⎋] close`, each key on
//! keycaps and its meaning in the caption style beside it. [`hint_bar`] is the bare row;
//! [`HintStrip`] is the footer of an E2 overlay (`docs/design/mocks/navigation/commands`): a
//! subtle fill under a hairline, the hints on the left with the active one in the accent, and
//! the hints that act on the selection (`⏎ select`) on the right.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSColor, NSFont, NSLineBreakMode, NSTextField,
    NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::keycap::{KeycapStyle, keycap_group_styled};
use crate::form::Plate;
use crate::tokens::{TextRole, color, control, space};

/// A row of `(key, meaning)` hints, sized to fit, [`control::KEYCAP_HEIGHT`] tall.
#[must_use]
pub fn hint_bar(hints: &[(&str, &str)], mtm: MainThreadMarker) -> Retained<NSView> {
    hint_bar_active(hints, None, mtm)
}

/// As [`hint_bar`], with hint `active` (the palette's current mode) in the accent.
#[must_use]
pub fn hint_bar_active(
    hints: &[(&str, &str)],
    active: Option<usize>,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let bar = HintRow::new(mtm);
    let height = control::KEYCAP_HEIGHT;
    let mut x: CGFloat = 0.0;
    for (index, &(key, meaning)) in hints.iter().enumerate() {
        if index > 0 {
            x += space::LG;
        }
        // Each hint is a group of its own, so a bar too narrow for it hides it whole.
        let group = NSView::new(mtm);
        let start = x;
        x = 0.0;
        let on = active == Some(index);
        let caps = keycap_group_styled(
            key,
            if on {
                KeycapStyle::Selected
            } else {
                KeycapStyle::Plain
            },
            mtm,
        );
        caps.setFrameOrigin(NSPoint::new(x, 0.0));
        group.addSubview(&caps);
        x += caps.frame().size.width + space::XS + space::XS / 2.0;
        let (font, tint) = if on {
            (semibold(TextRole::Caption), color::accent())
        } else {
            (TextRole::Caption.font(), color::secondary_label())
        };
        let text = caption(meaning, &font, &tint, mtm);
        let size = text.frame().size;
        text.setFrameOrigin(NSPoint::new(x, ((height - size.height) / 2.0).floor()));
        group.addSubview(&text);
        group.setFrame(NSRect::new(
            NSPoint::new(start, 0.0),
            NSSize::new((x + size.width).ceil(), height),
        ));
        bar.addSubview(&group);
        x = start + x + size.width;
    }
    bar.setFrameSize(NSSize::new(x.ceil(), height));
    bar.setAccessibilityElement(true);
    bar.setAccessibilityLabel(Some(&NSString::from_str(&spoken(hints))));
    bar.into_super()
}

define_class!(
    // SAFETY: `NSView` has no subclassing requirements, and `HintRow` implements no `Drop`;
    // `setFrameSize:` is public API meant to be overridden, and the override calls super.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieHintRow"]
    struct HintRow;

    impl HintRow {
        /// Hints that would not fit whole in the new width are hidden, from the end.
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: `setFrameSize:` takes an `NSSize`, as `NSView` declares it.
            let () = unsafe { msg_send![super(self), setFrameSize: size] };
            for group in self.subviews() {
                let frame = group.frame();
                group.setHidden(frame.origin.x + frame.size.width > size.width + 0.5);
            }
        }
    }
);

impl HintRow {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: `init` is `NSView`'s convenience initializer, and `HintRow` has no ivars.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

/// What `VoiceOver` reads for a bar: `↑↓ select, ⏎ open`.
fn spoken(hints: &[(&str, &str)]) -> String {
    hints
        .iter()
        .map(|(key, meaning)| format!("{key} {meaning}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn caption(
    text: &str,
    font: &NSFont,
    tint: &NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(font));
    field.setTextColor(Some(tint));
    field.setLineBreakMode(NSLineBreakMode::ByClipping);
    field.sizeToFit();
    field
}

fn semibold(role: TextRole) -> Retained<NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let weight = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::systemFontOfSize_weight(role.point_size(), weight)
}

/// The footer strip of an E2 overlay: a subtle fill under a top hairline, as wide as the
/// surface and [`HintStrip::HEIGHT`] tall, the leading hints at `palette-padding` from the left
/// and the trailing ones as far from the right. Place it at the bottom edge of the surface's
/// content with [`HintStrip::set_frame`]; the surface's corners clip it.
pub struct HintStrip {
    view: Retained<Plate>,
    leading_hints: Vec<(String, String)>,
    leading: RefCell<Retained<NSView>>,
    trailing: Retained<NSView>,
    active: Cell<Option<usize>>,
    mtm: MainThreadMarker,
}

impl HintStrip {
    /// The strip's height: a keycap with `space-md` above and below.
    pub const HEIGHT: CGFloat = control::KEYCAP_HEIGHT + space::MD * 2.0;

    #[must_use]
    pub fn new(leading: &[(&str, &str)], trailing: &[(&str, &str)], mtm: MainThreadMarker) -> Self {
        let view = Plate::new(Some(color::subtle_fill), None, 0.0, mtm);
        let line = Plate::new(Some(color::separator), None, 0.0, mtm);
        line.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 1.0)));
        line.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        view.addSubview(&line);
        let first = hint_bar(leading, mtm);
        let trailing = hint_bar(trailing, mtm);
        trailing.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        view.addSubview(&first);
        view.addSubview(&trailing);
        Self {
            view,
            leading_hints: leading
                .iter()
                .map(|&(key, meaning)| (key.to_owned(), meaning.to_owned()))
                .collect(),
            leading: RefCell::new(first),
            trailing,
            active: Cell::new(None),
            mtm,
        }
    }

    /// The strip, to add to a surface's content view.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// Shows leading hint `active` in the accent, or none.
    pub fn set_active(&self, active: Option<usize>) {
        if self.active.replace(active) == active {
            return;
        }
        let hints: Vec<(&str, &str)> = self
            .leading_hints
            .iter()
            .map(|(key, meaning)| (key.as_str(), meaning.as_str()))
            .collect();
        let fresh = hint_bar_active(&hints, active, self.mtm);
        let old = self.leading.replace(fresh);
        fresh_in_place(&old, &self.leading.borrow());
        self.view.replaceSubview_with(&old, &self.leading.borrow());
    }

    /// Places the strip at `frame` (its height should be [`Self::HEIGHT`]) and lays out its
    /// hints.
    pub fn set_frame(&self, frame: NSRect) {
        self.view.setFrame(frame);
        let width = frame.size.width;
        let top = space::MD;
        let trailing = self.trailing.frame().size.width;
        self.trailing.setFrameOrigin(NSPoint::new(
            (width - space::PALETTE_PADDING - trailing).floor(),
            top,
        ));
        let leading = self.leading.borrow();
        let room = (width - space::PALETTE_PADDING * 2.0 - trailing - space::LG).max(0.0);
        leading.setFrame(NSRect::new(
            NSPoint::new(space::PALETTE_PADDING, top),
            NSSize::new(room, control::KEYCAP_HEIGHT),
        ));
    }
}

/// Gives `fresh` the frame of the view it replaces.
fn fresh_in_place(old: &NSView, fresh: &NSView) {
    let frame = old.frame();
    fresh.setFrame(frame);
}

#[cfg(test)]
mod tests {
    use super::spoken;

    #[test]
    fn a_bar_is_read_hint_by_hint() {
        assert_eq!(
            spoken(&[(">", "commands"), ("⏎", "open")]),
            "> commands, ⏎ open"
        );
    }

    #[test]
    fn no_hints_read_as_nothing() {
        assert_eq!(spoken(&[]), "");
    }
}
