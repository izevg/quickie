//! A badge: a few words in SF Mono on a chip, hidden while empty. Filled with the accent for a
//! card's pinned slot and `ACTIVE`, tinted with it for the tab widgets' counters.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSFont, NSTextAlignment, NSTextField, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::{label, line};
use crate::form::Plate;
use crate::tokens::{TextRole, color, radius, space};

pub struct Badge {
    plate: Retained<Plate>,
    text: Retained<NSTextField>,
    /// Room around the text, across and down.
    padding: NSSize,
}

impl Badge {
    /// White bold text on the accent.
    #[must_use]
    pub fn filled(mtm: MainThreadMarker) -> Self {
        let plate = Plate::new(Some(color::accent), None, radius::KEYCAP - 2.0, mtm);
        let text = label(&mono_bold(), mtm);
        text.setTextColor(Some(&NSColor::whiteColor()));
        Self::with(plate, text, NSSize::new(space::SM, space::XS / 2.0))
    }

    /// Accent text on the selection tint, in a capsule.
    #[must_use]
    pub fn tinted(mtm: MainThreadMarker) -> Self {
        let height = line(TextRole::Caption) + space::XS;
        let plate = Plate::new(Some(color::selection_fill), None, height / 2.0, mtm);
        let text = label(&TextRole::Caption.mono_font(), mtm);
        text.setTextColor(Some(&color::accent()));
        Self::with(plate, text, NSSize::new(space::MD, space::XS))
    }

    fn with(plate: Retained<Plate>, text: Retained<NSTextField>, padding: NSSize) -> Self {
        text.setAlignment(NSTextAlignment::Center);
        plate.addSubview(&text);
        plate.setHidden(true);
        Self {
            plate,
            text,
            padding,
        }
    }

    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.plate
    }

    /// Shows `words`, sized to fit; nothing hides the chip.
    pub fn set_text(&self, words: &str) {
        self.plate.setHidden(words.is_empty());
        if words.is_empty() {
            return;
        }
        self.text.setStringValue(&NSString::from_str(words));
        self.text.sizeToFit();
        let size = self.text.frame().size;
        let chip = NSSize::new(
            (size.width + self.padding.width).ceil(),
            (size.height + self.padding.height).ceil(),
        );
        self.plate.setFrameSize(chip);
        self.text.setFrame(NSRect::new(
            NSPoint::new(0.0, ((chip.height - size.height) / 2.0).floor()),
            NSSize::new(chip.width, size.height),
        ));
    }
}

fn mono_bold() -> Retained<NSFont> {
    // SAFETY: `NSFontWeightBold` is an immutable `CGFloat` exported by AppKit.
    let bold = unsafe { objc2_app_kit::NSFontWeightBold };
    NSFont::monospacedSystemFontOfSize_weight(TextRole::Caption.point_size(), bold)
}
