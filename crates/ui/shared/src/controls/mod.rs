//! Small reusable controls: keycaps, pills, the hint bar of an overlay, and the styled text
//! they and the list rows are set in.

pub mod hint_bar;
pub mod keycap;
pub mod pill;

use objc2::rc::Retained;
use objc2::{AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSColor, NSFont, NSFontAttributeName, NSFontWeightMedium, NSForegroundColorAttributeName,
    NSImage, NSImageSymbolConfiguration, NSImageView, NSLineBreakMode, NSMutableParagraphStyle,
    NSParagraphStyleAttributeName,
};
use objc2_foundation::{NSAttributedString, NSMutableAttributedString, NSRange, NSString};

pub use hint_bar::{HintStrip, hint_bar, hint_bar_active};
pub use keycap::{KeycapStyle, keycap, keycap_group, keycap_group_styled};
pub use pill::Pill;

use crate::tokens::TextRole;

/// An SF Symbol (`lock.fill`) at the size of `role`, tinted `tint`, sized to fit; `None` when
/// the system has no such symbol. `description` is what `VoiceOver` reads.
#[must_use]
pub fn symbol(
    name: &str,
    description: &str,
    role: TextRole,
    tint: &NSColor,
    mtm: MainThreadMarker,
) -> Option<Retained<NSImageView>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(description)),
    )?;
    // SAFETY: `NSFontWeightMedium` is an immutable `CGFloat` exported by AppKit.
    let medium = unsafe { NSFontWeightMedium };
    let configuration =
        NSImageSymbolConfiguration::configurationWithPointSize_weight(role.point_size(), medium);
    let image = image.imageWithSymbolConfiguration(&configuration)?;
    let view = NSImageView::imageViewWithImage(&image, mtm);
    view.setContentTintColor(Some(tint));
    view.setFrameSize(image.size());
    Some(view)
}

/// Text in one font and color, with ranges set in another: fuzzy-match highlights, the keys of
/// a hint bar.
pub struct StyledText {
    string: Retained<NSMutableAttributedString>,
    length: usize,
}

impl StyledText {
    /// `text` in `font` and `color`.
    #[must_use]
    pub fn new(text: &str, font: &NSFont, color: &NSColor) -> Self {
        let string = NSMutableAttributedString::initWithString(
            NSMutableAttributedString::alloc(),
            &NSString::from_str(text),
        );
        let styled = Self {
            string,
            length: text.encode_utf16().count(),
        };
        styled.style(NSRange::new(0, styled.length), font, color);
        styled
    }

    /// Sets `range`, in UTF-16 code units, in `font` and `color`; the part past the end is
    /// ignored.
    pub fn style(&self, range: NSRange, font: &NSFont, color: &NSColor) {
        let start = range.location.min(self.length);
        let range = NSRange::new(start, range.length.min(self.length - start));
        if range.length == 0 {
            return;
        }
        // SAFETY: an immutable `NSString` constant exported by AppKit.
        let font_key = unsafe { NSFontAttributeName };
        // SAFETY: as above.
        let color_key = unsafe { NSForegroundColorAttributeName };
        // SAFETY: the font attribute takes an `NSFont`, and the range lies inside the string.
        unsafe { self.string.addAttribute_value_range(font_key, font, range) };
        // SAFETY: the color attribute takes an `NSColor`, and the range lies inside the string.
        unsafe {
            self.string
                .addAttribute_value_range(color_key, color, range);
        }
    }

    /// Breaks the text by `mode` when it does not fit: an attributed label follows its string's
    /// paragraph style, not the label's own line break mode.
    pub fn set_line_break(&self, mode: NSLineBreakMode) {
        let style = NSMutableParagraphStyle::new();
        style.setLineBreakMode(mode);
        // SAFETY: an immutable `NSString` constant exported by AppKit.
        let key = unsafe { NSParagraphStyleAttributeName };
        // SAFETY: the paragraph style attribute takes an `NSParagraphStyle`, and the range is
        // the whole string.
        unsafe {
            self.string
                .addAttribute_value_range(key, &style, NSRange::new(0, self.length));
        }
    }

    /// The styled string, for a label.
    #[must_use]
    pub fn string(&self) -> &NSAttributedString {
        &self.string
    }
}

/// The UTF-16 ranges of the characters at `positions` (ascending character indexes) in
/// `text`, adjacent characters merged into one range. Positions past the end are dropped.
#[must_use]
pub fn char_ranges(text: &str, positions: &[u32]) -> Vec<NSRange> {
    let mut ranges: Vec<NSRange> = Vec::new();
    let mut wanted = positions.iter().copied().peekable();
    let mut offset = 0;
    for (index, c) in text.chars().enumerate() {
        let width = c.len_utf16();
        if wanted
            .peek()
            .is_some_and(|&p| usize::try_from(p) == Ok(index))
        {
            wanted.next();
            match ranges.last_mut() {
                Some(last) if last.location + last.length == offset => last.length += width,
                _ => ranges.push(NSRange::new(offset, width)),
            }
        }
        offset += width;
    }
    ranges
}

#[cfg(test)]
mod tests {
    use objc2_foundation::NSRange;

    use super::char_ranges;

    #[test]
    fn adjacent_matches_merge_into_one_range() {
        assert_eq!(
            char_ranges("Reload", &[0, 1, 2, 5]),
            [NSRange::new(0, 3), NSRange::new(5, 1)]
        );
    }

    #[test]
    fn ranges_count_utf16_code_units() {
        // `😀` takes two UTF-16 code units, `ö` one.
        assert_eq!(
            char_ranges("😀ab", &[1, 2]),
            [NSRange::new(2, 2)],
            "after a surrogate pair"
        );
        assert_eq!(char_ranges("öx", &[0]), [NSRange::new(0, 1)]);
        assert_eq!(char_ranges("😀", &[0]), [NSRange::new(0, 2)]);
    }

    #[test]
    fn positions_past_the_end_are_dropped() {
        assert_eq!(char_ranges("ab", &[1, 7]), [NSRange::new(1, 1)]);
        assert_eq!(char_ranges("", &[0]), []);
    }
}
