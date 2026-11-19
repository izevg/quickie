//! The origin chip: an E1 pill near the top center that says whose page this is after a
//! cross-origin navigation or a tab switch, so a page without chrome cannot pose as another
//! site. The registrable domain is bold, the rest of the host and the port dimmed, a lock
//! for `https` and a warning for `http` before them. It fades in with `fade-fast`, holds
//! `origin-chip-hold` (2.4 s), and fades out; a newer origin replaces it at once.
//!
//! The blocked counter is a slot for now: the shell passes `None` until Shields counts.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSFont, NSLineBreakMode, NSTextField, NSView};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSTimer};
use quickie_domain_browsing::{Origin, Transport};
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{Elevation, GlassSurface, StyledText, Trigger, motion, symbol};

/// The chip on screen, by the number of its showing.
struct Shown {
    id: u64,
    surface: GlassSurface,
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

    /// Shows `origin` at the top center of `host` for `origin-chip-hold`, with `blocked`
    /// requests counted when Shields knows them.
    pub fn show(&self, host: &NSView, origin: &Origin, blocked: Option<u32>) {
        self.remove();
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let surface = chip(origin, blocked, self.mtm);
        let size = surface.view().frame().size;
        let bounds = host.bounds();
        surface.set_frame(NSRect::new(
            NSPoint::new(
                ((bounds.size.width - size.width) / 2.0).round(),
                (bounds.size.height - space::MD - size.height).round(),
            ),
            size,
        ));
        surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        host.addSubview(surface.view());
        motion::show(surface.view(), motion::FADE_FAST, Trigger::App);
        *self.shown.borrow_mut() = Some(Shown { id, surface });
        self.after(motion::ORIGIN_CHIP_HOLD, move |chip| chip.fade(id));
    }

    /// Takes the chip away at once, as when an overlay opens over it.
    pub fn hide(&self) {
        self.remove();
    }

    /// The text on screen, while the chip shows.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        let shown = self.shown.borrow();
        let surface = &shown.as_ref()?.surface;
        let content = surface.content_view().subviews();
        content.iter().find_map(|view| {
            view.downcast::<NSTextField>()
                .ok()
                .map(|field| field.stringValue().to_string())
        })
    }

    fn fade(&self, id: u64) {
        if let Some(shown) = self.shown.borrow().as_ref().filter(|shown| shown.id == id) {
            motion::hide(shown.surface.view(), motion::FADE_FAST, Trigger::App);
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
            shown.surface.view().removeFromSuperview();
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

/// The chip's surface, sized to its content: the transport glyph, then the host.
fn chip(origin: &Origin, blocked: Option<u32>, mtm: MainThreadMarker) -> GlassSurface {
    let surface = GlassSurface::new(Elevation::E1, mtm);
    let label = NSTextField::labelWithAttributedString(text(origin, blocked).string(), mtm);
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    label.sizeToFit();
    let text_size = label.frame().size;
    let glyph = match origin.transport {
        Transport::Secure => symbol(
            "lock.fill",
            "Secure connection",
            TextRole::Caption,
            &color::secondary_label(),
            mtm,
        ),
        Transport::Insecure => symbol(
            "exclamationmark.triangle.fill",
            "Not secure",
            TextRole::Caption,
            &color::destructive(),
            mtm,
        ),
    };
    let glyph_width = glyph
        .as_ref()
        .map_or(0.0, |glyph| glyph.frame().size.width + space::SM);
    let height = (text_size.height + space::SM * 2.0).ceil();
    let width = (space::MD + glyph_width + text_size.width + space::MD).ceil();
    if let Some(glyph) = glyph {
        let size = glyph.frame().size;
        glyph.setFrameOrigin(NSPoint::new(
            space::MD,
            ((height - size.height) / 2.0).round(),
        ));
        surface.content_view().addSubview(&glyph);
    }
    label.setFrame(NSRect::new(
        NSPoint::new(
            space::MD + glyph_width,
            ((height - text_size.height) / 2.0).round(),
        ),
        text_size,
    ));
    surface.content_view().addSubview(&label);
    surface.set_frame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, height),
    ));
    surface
}

/// `developer.` `apple.com` `:8443` in SF Mono, the registrable domain bold and the rest
/// dimmed, then the blocked count when there is one.
fn text(origin: &Origin, blocked: Option<u32>) -> StyledText {
    let port = origin
        .port
        .map_or_else(String::new, |port| format!(":{port}"));
    let count = blocked.map_or_else(String::new, |n| format!("  ·  {n} blocked"));
    let whole = format!("{}{}{port}{count}", origin.subdomain, origin.registrable);
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

fn utf16(text: &str) -> usize {
    text.encode_utf16().count()
}

/// SF Mono semibold at the detail size.
fn bold() -> Retained<NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let semibold = unsafe { objc2_app_kit::NSFontWeightSemibold };
    NSFont::monospacedSystemFontOfSize_weight(TextRole::RowDetail.point_size(), semibold)
}
