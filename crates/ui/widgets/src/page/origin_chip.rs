//! The origin chip (`docs/design/mocks/components/origin-chip.html`): an E1 pill near the top
//! center that says whose page this is after a cross-origin navigation or a tab switch, so a
//! page without chrome cannot pose as another site. A status dot (green for `https`, red for
//! `http`), the host in SF Mono with the registrable domain bold and the rest dimmed, the path
//! in the secondary color, a lock or a warning, and the requests Shields blocked on a green
//! capsule. It fades in with `fade-fast`, holds `origin-chip-hold` (2.4 s), and fades out; a
//! newer origin replaces it at once.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSFont, NSView};
use objc2_foundation::{NSPoint, NSRange, NSTimer};
use quickie_domain_browsing::{Origin, Transport};
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{Pill, PillPart, PillTone, StyledText, Trigger, motion};

/// The most characters of a path the chip shows; the rest is cut with an ellipsis.
const PATH_CHARS: usize = 32;

/// The chip on screen, by the number of its showing.
struct Shown {
    id: u64,
    pill: Pill,
    text: String,
}

pub struct OriginChip {
    shown: RefCell<Option<Shown>>,
    next_id: Cell<u64>,
    this: Weak<Self>,
    mtm: MainThreadMarker,
}

impl OriginChip {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            shown: RefCell::new(None),
            next_id: Cell::new(0),
            this: this.clone(),
            mtm,
        })
    }

    /// Shows `origin` and the page's `path` at the top center of `host` for
    /// `origin-chip-hold`, with `blocked` requests counted when Shields knows them.
    pub fn show(&self, host: &NSView, origin: &Origin, path: &str, blocked: Option<u32>) {
        self.remove();
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let (pill, text) = chip(origin, path, blocked, self.mtm);
        let size = pill.size();
        let bounds = host.bounds();
        pill.place(NSPoint::new(
            ((bounds.size.width - size.width) / 2.0).round(),
            (bounds.size.height - space::MD - size.height).round(),
        ));
        pill.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        host.addSubview(pill.view());
        motion::show(pill.view(), motion::FADE_FAST, Trigger::App);
        *self.shown.borrow_mut() = Some(Shown { id, pill, text });
        self.after(motion::ORIGIN_CHIP_HOLD, move |chip| chip.fade(id));
    }

    /// Takes the chip away at once, as when an overlay opens over it.
    pub fn hide(&self) {
        self.remove();
    }

    /// What the chip says, while it shows.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.shown.borrow().as_ref().map(|shown| shown.text.clone())
    }

    fn fade(&self, id: u64) {
        if let Some(shown) = self.shown.borrow().as_ref().filter(|shown| shown.id == id) {
            motion::hide(shown.pill.view(), motion::FADE_FAST, Trigger::App);
        }
        self.after(motion::FADE_FAST.duration, move |chip| {
            let current = chip.shown.borrow().as_ref().map(|shown| shown.id);
            if current == Some(id) {
                chip.remove();
            }
        });
    }

    fn remove(&self) {
        if let Some(shown) = self.shown.borrow_mut().take() {
            shown.pill.view().removeFromSuperview();
        }
    }

    /// Runs `then` on the main run loop `seconds` from now, if the chip still exists.
    fn after(&self, seconds: f64, then: impl Fn(&Self) + 'static) {
        let this = self.this.clone();
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            if let Some(chip) = this.upgrade() {
                then(&chip);
            }
        });
        // SAFETY: a one-shot timer on the main run loop, which retains it until it fires; the
        // block runs on the main thread that owns what it captures.
        let _timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(seconds, false, &block)
        };
    }
}

/// The chip's pill and what it says in one line.
fn chip(
    origin: &Origin,
    path: &str,
    blocked: Option<u32>,
    mtm: MainThreadMarker,
) -> (Pill, String) {
    let secure = origin.transport == Transport::Secure;
    let host = host(origin);
    let path = shown_path(path);
    let mut parts = vec![
        PillPart::Dot(if secure {
            color::success
        } else {
            color::destructive
        }),
        PillPart::Styled(host),
    ];
    if let Some(path) = &path {
        parts.push(PillPart::Detail(path.clone()));
    }
    parts.push(if secure {
        PillPart::Glyph {
            name: "lock",
            description: "Secure connection",
            tint: color::secondary_label,
        }
    } else {
        PillPart::Glyph {
            name: "exclamationmark.triangle.fill",
            description: "Not secure",
            tint: color::destructive,
        }
    });
    if let Some(blocked) = blocked {
        parts.push(PillPart::Count {
            text: format!("{blocked} blocked"),
            tint: color::success,
            fill: color::success_fill,
        });
    }
    let pill = Pill::with_parts(&parts, PillTone::Plain, mtm);
    let port = origin
        .port
        .map_or_else(String::new, |port| format!(":{port}"));
    let mut text = format!("{}{}{port}", origin.subdomain, origin.registrable);
    if let Some(path) = path {
        text.push_str(&path);
    }
    if let Some(blocked) = blocked {
        text.push_str(" · ");
        text.push_str(&blocked.to_string());
        text.push_str(" blocked");
    }
    (pill, text)
}

/// `developer.` `apple.com` `:8443` in SF Mono, the registrable domain bold and the rest
/// dimmed.
fn host(origin: &Origin) -> StyledText {
    let port = origin
        .port
        .map_or_else(String::new, |port| format!(":{port}"));
    let whole = format!("{}{}{port}", origin.subdomain, origin.registrable);
    let dimmed = TextRole::RowDetail.mono_font();
    let styled = StyledText::new(&whole, &dimmed, &color::secondary_label());
    let start = utf16(&origin.subdomain);
    styled.style(
        NSRange::new(start, utf16(&origin.registrable)),
        &bold(),
        &color::label(),
    );
    styled
}

/// The path as the chip shows it: none for the root, percent-escapes decoded when they spell
/// UTF-8, cut after [`PATH_CHARS`] characters.
fn shown_path(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    let decoded = decode(path).unwrap_or_else(|| path.to_owned());
    if decoded.chars().count() <= PATH_CHARS {
        return Some(decoded);
    }
    let kept: String = decoded.chars().take(PATH_CHARS - 1).collect();
    Some(format!("{kept}…"))
}

/// `text` with its `%XX` escapes decoded; `None` when they do not spell UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let escaped = (byte == b'%')
            .then(|| bytes.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(value) = escaped {
            out.push(value);
            at += 3;
        } else {
            out.push(byte);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn utf16(text: &str) -> usize {
    text.encode_utf16().count()
}

/// SF Mono semibold at the detail size.
fn bold() -> Retained<NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let semibold = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::monospacedSystemFontOfSize_weight(TextRole::RowDetail.point_size(), semibold)
}

#[cfg(test)]
mod tests {
    use super::shown_path;

    #[test]
    fn the_root_shows_no_path() {
        assert_eq!(shown_path("/"), None);
        assert_eq!(shown_path(""), None);
        assert_eq!(shown_path("/news/"), Some("/news".to_owned()));
    }

    #[test]
    fn escapes_are_decoded_when_they_spell_utf8() {
        assert_eq!(
            shown_path("/wiki/%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0"),
            Some("/wiki/Москва".to_owned())
        );
        assert_eq!(shown_path("/a%20b"), Some("/a b".to_owned()));
        assert_eq!(shown_path("/bad%FF"), Some("/bad%FF".to_owned()));
        assert_eq!(shown_path("/odd%2"), Some("/odd%2".to_owned()));
    }

    #[test]
    fn a_long_path_is_cut() {
        let long = format!("/{}", "a".repeat(40));
        let shown = shown_path(&long).unwrap_or_default();

        assert_eq!(shown.chars().count(), 32);
        assert!(shown.ends_with('…'));
    }
}
