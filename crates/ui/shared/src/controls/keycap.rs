//! Keycaps: one glyph per chip, a chord as a group (`docs/design/mocks/components/keycap.html`).
//! Default on glass; on a selected row the glyph takes the primary text color and the border
//! the accent, to sit on the row's accent tint; in conflict both are red. The border is a
//! [`Plate`]'s, so it follows light and dark.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSTextAlignment, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::form::{Paint, Plate};
use crate::tokens::{TextRole, color, radius, space};

/// How a keycap looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeycapStyle {
    /// On glass or a plain row.
    Plain,
    /// On a selected row's accent tint.
    Selected,
    /// Keys that already run another command.
    Conflict,
}

impl KeycapStyle {
    fn text(self) -> Retained<NSColor> {
        match self {
            Self::Plain => color::secondary_label(),
            Self::Selected => color::selection_text(),
            Self::Conflict => color::destructive(),
        }
    }

    fn border(self) -> Paint {
        match self {
            Self::Plain => color::separator,
            Self::Selected => color::accent_line,
            Self::Conflict => color::destructive_line,
        }
    }
}

/// The modifier glyphs, each on its own cap.
const MODIFIERS: [char; 4] = ['⌃', '⌥', '⇧', '⌘'];

/// The caps of a chord written in glyphs, stroke by stroke: `⇧⌘P` is `[[⇧, ⌘, P]]` and
/// `⌘K ⌘S` is `[[⌘, K], [⌘, S]]`. A key glyph of several characters (`F12`, `Space`) stays on
/// one cap.
#[must_use]
pub fn caps(chord: &str) -> Vec<Vec<String>> {
    chord
        .split_whitespace()
        .map(|stroke| {
            let key = stroke.trim_start_matches(MODIFIERS);
            let mut caps: Vec<String> = stroke[..stroke.len() - key.len()]
                .chars()
                .map(String::from)
                .collect();
            if !key.is_empty() {
                caps.push(key.to_owned());
            }
            caps
        })
        .collect()
}

/// A chord as a row of caps, sized to fit: `keycap-gap` 4 inside a stroke, 8 between strokes.
/// `inverted` caps sit on a selected row.
#[must_use]
pub fn keycap_group(chord: &str, inverted: bool, mtm: MainThreadMarker) -> Retained<NSView> {
    let style = if inverted {
        KeycapStyle::Selected
    } else {
        KeycapStyle::Plain
    };
    keycap_group_styled(chord, style, mtm)
}

/// A chord as a row of caps in `style`, sized to fit.
#[must_use]
pub fn keycap_group_styled(
    chord: &str,
    style: KeycapStyle,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let group = NSView::new(mtm);
    let mut x: CGFloat = 0.0;
    let mut height: CGFloat = 0.0;
    for (index, stroke) in caps(chord).iter().enumerate() {
        if index > 0 {
            x += space::SM - space::XS;
        }
        for glyph in stroke {
            let cap = keycap(glyph, style, mtm);
            let size = cap.frame().size;
            cap.setFrameOrigin(NSPoint::new(x, 0.0));
            group.addSubview(&cap);
            x += size.width + space::XS;
            height = height.max(size.height);
        }
    }
    group.setFrameSize(NSSize::new((x - space::XS).max(0.0), height));
    group
}

/// One cap: the glyph in SF Mono, a hairline border, `radius-keycap` corners.
fn keycap(glyph: &str, style: KeycapStyle, mtm: MainThreadMarker) -> Retained<NSView> {
    let label = NSTextField::labelWithString(&NSString::from_str(glyph), mtm);
    label.setFont(Some(&TextRole::Caption.mono_font()));
    label.setAlignment(NSTextAlignment::Center);
    label.setTextColor(Some(&style.text()));
    label.sizeToFit();
    let text_size = label.frame().size;
    let height = (text_size.height + space::XS).ceil();
    let width = (text_size.width + space::SM).max(height).ceil();
    let cap = Plate::new(None, Some(style.border()), radius::KEYCAP, mtm);
    cap.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, height),
    ));
    // The plate is flipped: the label sits a hair above the middle, as it did unflipped.
    label.setFrame(NSRect::new(
        NSPoint::new(0.0, ((height - text_size.height) / 2.0).ceil()),
        NSSize::new(width, text_size.height),
    ));
    cap.addSubview(&label);
    cap.into_super()
}

#[cfg(test)]
mod tests {
    use super::caps;

    fn owned(strokes: &[&[&str]]) -> Vec<Vec<String>> {
        strokes
            .iter()
            .map(|stroke| stroke.iter().map(|&cap| cap.to_owned()).collect())
            .collect()
    }

    #[test]
    fn each_modifier_gets_its_own_cap() {
        assert_eq!(caps("⇧⌘P"), owned(&[&["⇧", "⌘", "P"]]));
        assert_eq!(caps("⌃⌥⇧⌘1"), owned(&[&["⌃", "⌥", "⇧", "⌘", "1"]]));
    }

    #[test]
    fn a_long_key_glyph_stays_on_one_cap() {
        assert_eq!(caps("⌘F12"), owned(&[&["⌘", "F12"]]));
        assert_eq!(caps("Space"), owned(&[&["Space"]]));
        assert_eq!(caps("⌃⇥"), owned(&[&["⌃", "⇥"]]));
    }

    #[test]
    fn a_two_stroke_chord_keeps_its_strokes_apart() {
        assert_eq!(caps("⌘K ⌘S"), owned(&[&["⌘", "K"], &["⌘", "S"]]));
        assert_eq!(caps(""), owned(&[]));
    }
}
