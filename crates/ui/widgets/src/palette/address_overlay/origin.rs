//! The page's URL as the overlay opens on it, read like the origin chip: a lock for `https`, a
//! warning for `http`, the scheme and path in the secondary colour and the host bold. The
//! scheme and host are one label and the rest another, cut in its middle, so a long path never
//! hides the host. On the right, what Shields blocked on the page, when it counted that. It
//! stands over the field until the user starts editing.

use std::cell::Cell;
use std::ops::Range;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSImageView, NSLineBreakMode, NSMutableParagraphStyle, NSParagraphStyleAttributeName,
    NSTextField, NSView,
};
use objc2_foundation::{
    NSMutableAttributedString, NSMutableCopying, NSPoint, NSRange, NSRect, NSSize, NSString,
};
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{StyledText, symbol};
use url::{Position, Url};

pub(super) struct OriginLine {
    secure: Option<Retained<NSImageView>>,
    insecure: Option<Retained<NSImageView>>,
    /// The scheme and the host, with its port.
    head: Retained<NSTextField>,
    /// The path, the query, and the fragment.
    rest: Retained<NSTextField>,
    shield: Option<Retained<NSImageView>>,
    blocked: Retained<NSTextField>,
    /// The glyph that shows for the current URL.
    transport: Cell<Option<Transport>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    Secure,
    Insecure,
}

impl OriginLine {
    pub(super) fn new(mtm: MainThreadMarker) -> Self {
        let role = TextRole::RowDetail;
        let blocked = NSTextField::labelWithString(&NSString::new(), mtm);
        blocked.setFont(Some(&TextRole::Caption.font()));
        blocked.setTextColor(Some(&color::success()));
        Self {
            secure: symbol(
                "lock.fill",
                "Secure connection",
                role,
                &color::success(),
                mtm,
            ),
            insecure: symbol(
                "exclamationmark.triangle.fill",
                "Not secure",
                role,
                &color::destructive(),
                mtm,
            ),
            head: NSTextField::labelWithString(&NSString::new(), mtm),
            rest: NSTextField::labelWithString(&NSString::new(), mtm),
            shield: symbol(
                "checkmark.shield.fill",
                "Shields",
                TextRole::Caption,
                &color::success(),
                mtm,
            ),
            blocked,
            transport: Cell::new(None),
        }
    }

    /// Every view of the line, to add to the overlay.
    pub(super) fn views(&self) -> Vec<&NSView> {
        let glyphs = [&self.secure, &self.insecure, &self.shield];
        let labels: [&NSView; 3] = [&self.head, &self.rest, &self.blocked];
        glyphs
            .into_iter()
            .flatten()
            .map(|glyph| -> &NSView { glyph })
            .chain(labels)
            .collect()
    }

    /// Shows `text`, the page's URL, and `blocked`, what Shields counted on the page.
    pub(super) fn set(&self, text: &str, blocked: Option<u32>) {
        let parts = parts(text);
        self.transport
            .set(parts.as_ref().and_then(|parts| parts.transport));
        let split = parts.as_ref().map_or(text.len(), |parts| parts.head_end);
        let (head, rest) = text.split_at(split);
        let font = TextRole::PaletteInput.mono_font();
        let head_text = StyledText::new(head, &font, &color::secondary_label());
        if let Some(parts) = parts {
            head_text.style(
                utf16_range(head, parts.host),
                &TextRole::PaletteInput.strong_mono_font(),
                &color::label(),
            );
        }
        let rest_text = StyledText::new(rest, &font, &color::secondary_label());
        for (label, styled, breaks) in [
            (&self.head, head_text, NSLineBreakMode::ByTruncatingTail),
            (&self.rest, rest_text, NSLineBreakMode::ByTruncatingMiddle),
        ] {
            label.setAttributedStringValue(&breaking(&styled, breaks));
            label.sizeToFit();
        }
        let count = blocked.map_or_else(String::new, |n| format!("{n} blocked · Protection is on"));
        self.blocked.setStringValue(&NSString::from_str(&count));
        self.blocked.sizeToFit();
    }

    /// Shows or hides the whole line.
    pub(super) fn set_visible(&self, visible: bool) {
        let counted = !self.blocked.stringValue().is_empty();
        let shown = |view: &NSView, own: bool| view.setHidden(!(visible && own));
        let transport = self.transport.get();
        if let Some(glyph) = &self.secure {
            shown(glyph, transport == Some(Transport::Secure));
        }
        if let Some(glyph) = &self.insecure {
            shown(glyph, transport == Some(Transport::Insecure));
        }
        if let Some(shield) = &self.shield {
            shown(shield, counted);
        }
        shown(&self.blocked, counted);
        shown(&self.head, true);
        shown(&self.rest, true);
    }

    /// Lays the line out across `line`, the field's frame.
    pub(super) fn layout(&self, line: NSRect) {
        let middle = line.origin.y + line.size.height / 2.0;
        let centered = |view: &NSView, x: f64| {
            let size = view.frame().size;
            view.setFrameOrigin(NSPoint::new(
                x.round(),
                (middle - size.height / 2.0).round(),
            ));
            size.width
        };
        let mut left = line.origin.x;
        let glyph = match self.transport.get() {
            Some(Transport::Secure) => self.secure.as_ref(),
            Some(Transport::Insecure) => self.insecure.as_ref(),
            None => None,
        };
        if let Some(glyph) = glyph {
            left += centered(glyph, left) + space::SM;
        }
        let mut right = line.origin.x + line.size.width;
        if !self.blocked.stringValue().is_empty() {
            right -= self.blocked.frame().size.width;
            centered(&self.blocked, right);
            if let Some(shield) = &self.shield {
                right -= shield.frame().size.width + space::XS;
                centered(shield, right);
            }
            right -= space::MD;
        }
        let head = self.head.fittingSize();
        let head_width = head.width.ceil().min((right - left).max(0.0));
        self.head.setFrame(NSRect::new(
            NSPoint::new(left, (middle - head.height / 2.0).round()),
            NSSize::new(head_width, head.height),
        ));
        // Each label insets its text by half of `space::XS`: the two read as one line.
        left += head_width - space::XS;
        let rest = self.rest.fittingSize();
        self.rest.setFrame(NSRect::new(
            NSPoint::new(left, (middle - rest.height / 2.0).round()),
            NSSize::new((right - left).max(0.0), rest.height),
        ));
    }
}

/// `styled` breaking its line as `breaks` says: an attributed label breaks as its string
/// says, whatever its cell says.
fn breaking(styled: &StyledText, breaks: NSLineBreakMode) -> Retained<NSMutableAttributedString> {
    let string = styled.string().mutableCopy();
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setLineBreakMode(breaks);
    // SAFETY: an immutable `NSString` constant exported by AppKit.
    let key = unsafe { NSParagraphStyleAttributeName };
    let whole = NSRange::new(0, string.length());
    // SAFETY: the paragraph style attribute takes an `NSParagraphStyle`, and the range is the
    // whole string.
    unsafe { string.addAttribute_value_range(key, &paragraph, whole) };
    string
}

/// What the line sets apart in a URL.
#[derive(Debug, PartialEq, Eq)]
struct Parts {
    transport: Option<Transport>,
    /// Bytes of the URL that are its host.
    host: Range<usize>,
    /// Where the scheme, the host, and its port end, and the rest begins.
    head_end: usize,
}

/// The transport and host of `text` when it is a URL with a host; the text is the URL as the
/// page reports it, written the way [`Url`] writes it.
fn parts(text: &str) -> Option<Parts> {
    let url = Url::parse(text).ok()?;
    url.host_str()?;
    if url.as_str() != text {
        return None;
    }
    let transport = match url.scheme() {
        "https" => Some(Transport::Secure),
        "http" => Some(Transport::Insecure),
        _ => None,
    };
    Some(Parts {
        transport,
        host: url[..Position::BeforeHost].len()..url[..Position::AfterHost].len(),
        head_end: url[..Position::AfterPort].len(),
    })
}

/// A byte range of `text` in UTF-16 units, as `AppKit` counts.
fn utf16_range(text: &str, bytes: Range<usize>) -> NSRange {
    let units = |part: &str| part.encode_utf16().count();
    let start = text.get(..bytes.start).map_or(0, units);
    let length = text.get(bytes).map_or(0, units);
    NSRange::new(start, length)
}

#[cfg(test)]
mod tests {
    use objc2_foundation::NSRange;

    use super::{Parts, Transport, parts, utf16_range};

    #[test]
    fn a_web_url_shows_its_transport_and_host() {
        assert_eq!(
            parts("https://developer.apple.com/documentation"),
            Some(Parts {
                transport: Some(Transport::Secure),
                host: 8..27,
                head_end: 27,
            })
        );
        assert_eq!(
            parts("http://localhost:3000/"),
            Some(Parts {
                transport: Some(Transport::Insecure),
                host: 7..16,
                head_end: 21,
            })
        );
        assert_eq!(
            parts("file://server/a"),
            Some(Parts {
                transport: None,
                host: 7..13,
                head_end: 13,
            })
        );
    }

    #[test]
    fn text_that_is_no_url_with_a_host_is_plain() {
        assert_eq!(parts("about:blank"), None);
        assert_eq!(parts("example.com"), None);
        assert_eq!(parts("https://EXAMPLE.com/"), None);
    }

    #[test]
    fn ranges_count_utf16_units() {
        assert_eq!(utf16_range("ab", 0..1), NSRange::new(0, 1));
        assert_eq!(utf16_range("ёab", 2..4), NSRange::new(1, 2));
    }
}
