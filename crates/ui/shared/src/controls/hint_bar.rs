//! The hint bar: the footer of an E2 overlay that names its keys, `↑↓ select · ⏎ open · ⎋ close`.
//! Each key is set in SF Mono, its meaning in the caption style; hints are joined by ` · `.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSLineBreakMode, NSTextField};
use objc2_foundation::NSRange;

use super::StyledText;
use crate::tokens::{TextRole, color};

/// A one-line label of `(key, meaning)` hints.
#[must_use]
pub fn hint_bar(hints: &[(&str, &str)], mtm: MainThreadMarker) -> Retained<NSTextField> {
    let (text, keys) = layout(hints);
    let styled = StyledText::new(&text, &TextRole::Caption.font(), &color::secondary_label());
    let mono = TextRole::Caption.mono_font();
    let label = color::label();
    for range in keys {
        styled.style(range, &mono, &label);
    }
    let field = NSTextField::labelWithAttributedString(styled.string(), mtm);
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

/// The bar's text and the UTF-16 ranges of its keys.
fn layout(hints: &[(&str, &str)]) -> (String, Vec<NSRange>) {
    let mut text = String::new();
    let mut keys = Vec::new();
    for (index, (key, meaning)) in hints.iter().enumerate() {
        if index > 0 {
            text.push_str("  ·  ");
        }
        let start = text.encode_utf16().count();
        text.push_str(key);
        keys.push(NSRange::new(start, key.encode_utf16().count()));
        text.push(' ');
        text.push_str(meaning);
    }
    (text, keys)
}

#[cfg(test)]
mod tests {
    use objc2_foundation::NSRange;

    use super::layout;

    #[test]
    fn hints_are_joined_and_their_keys_located() {
        let (text, keys) = layout(&[(">", "commands"), ("⏎", "open")]);

        assert_eq!(text, "> commands  ·  ⏎ open");
        assert_eq!(keys, [NSRange::new(0, 1), NSRange::new(15, 1)]);
    }

    #[test]
    fn no_hints_is_an_empty_bar() {
        assert_eq!(layout(&[]), (String::new(), Vec::new()));
    }
}
