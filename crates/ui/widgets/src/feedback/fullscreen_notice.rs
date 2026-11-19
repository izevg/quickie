//! The full-screen notice: whenever a page takes the whole screen, an E1 pill at the top center
//! says whose page it is and how to leave, "`example.com` is full screen · ⎋ to exit", for
//! `fullscreen-notice-hold` (3 s). A page with no chrome around it cannot pose as the system or
//! another site (Dia's CVE-2025-13132, `docs/lessons.md`).

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSFont, NSTextField, NSView};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSTimer};
use quickie_domain_browsing::Origin;
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{Elevation, GlassSurface, StyledText, Trigger, motion};
use url::Url;

struct Shown {
    id: u64,
    surface: GlassSurface,
}

pub struct FullscreenNotice {
    shown: RefCell<Option<Shown>>,
    next_id: Cell<u64>,
    this: Weak<Self>,
    mtm: MainThreadMarker,
}

impl FullscreenNotice {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            shown: RefCell::new(None),
            next_id: Cell::new(0),
            this: this.clone(),
            mtm,
        })
    }

    /// Shows the notice for the page at `url` over `host`, the view that now fills the screen.
    pub fn show(&self, host: &NSView, url: &Url) {
        self.hide();
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let surface = notice(url, self.mtm);
        let size = surface.view().frame().size;
        let bounds = host.bounds();
        surface.set_frame(NSRect::new(
            NSPoint::new(
                ((bounds.size.width - size.width) / 2.0).round(),
                (bounds.size.height - space::XL - size.height).round(),
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
        self.after(motion::FULLSCREEN_NOTICE_HOLD, move |notice| {
            notice.fade(id);
        });
    }

    /// Takes the notice away at once: the page left full screen.
    pub fn hide(&self) {
        if let Some(shown) = self.shown.borrow_mut().take() {
            shown.surface.view().removeFromSuperview();
        }
    }

    /// The text on screen, while the notice shows.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        let shown = self.shown.borrow();
        let surface = &shown.as_ref()?.surface;
        surface.content_view().subviews().iter().find_map(|view| {
            view.downcast::<NSTextField>()
                .ok()
                .map(|field| field.stringValue().to_string())
        })
    }

    fn fade(&self, id: u64) {
        if let Some(shown) = self.shown.borrow().as_ref().filter(|shown| shown.id == id) {
            motion::hide(shown.surface.view(), motion::FADE_FAST, Trigger::App);
        }
        self.after(motion::FADE_FAST.duration, move |notice| {
            let current = notice.shown.borrow().as_ref().map(|shown| shown.id);
            if current == Some(id) {
                notice.hide();
            }
        });
    }

    /// Runs `then` on the main run loop `seconds` from now, if the notice still exists.
    fn after(&self, seconds: f64, then: impl Fn(&Self) + 'static) {
        let this = self.this.clone();
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            if let Some(notice) = this.upgrade() {
                then(&notice);
            }
        });
        // SAFETY: a one-shot timer on the main run loop, which retains it until it fires; the
        // block runs on the main thread that owns what it captures.
        let _timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(seconds, false, &block)
        };
    }
}

/// The words of the notice and the UTF-16 range of the site's name in them.
fn words(url: &Url) -> (String, NSRange) {
    let (before, owner, after) = match Origin::of(url) {
        Some(origin) => (
            origin.subdomain,
            origin.registrable,
            origin
                .port
                .map_or_else(String::new, |port| format!(":{port}")),
        ),
        None => (
            String::new(),
            url.as_str().trim_end_matches('/').to_owned(),
            String::new(),
        ),
    };
    let start = before.encode_utf16().count();
    let range = NSRange::new(start, owner.encode_utf16().count());
    (
        format!("{before}{owner}{after} is full screen  ·  ⎋ to exit"),
        range,
    )
}

fn notice(url: &Url, mtm: MainThreadMarker) -> GlassSurface {
    let surface = GlassSurface::new(Elevation::E1, mtm);
    let (text, owner) = words(url);
    let styled = StyledText::new(&text, &TextRole::RowTitle.font(), &color::label());
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let semibold = unsafe { objc2_app_kit::NSFontWeightSemibold };
    let bold = NSFont::monospacedSystemFontOfSize_weight(TextRole::RowTitle.point_size(), semibold);
    styled.style(owner, &bold, &color::label());
    let label = NSTextField::labelWithAttributedString(styled.string(), mtm);
    label.sizeToFit();
    let text_size = label.frame().size;
    let height = (text_size.height + space::SM * 2.0).ceil();
    let width = (text_size.width + space::LG * 2.0).ceil();
    label.setFrame(NSRect::new(
        NSPoint::new(space::LG, ((height - text_size.height) / 2.0).round()),
        text_size,
    ));
    surface.content_view().addSubview(&label);
    surface.set_frame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, height),
    ));
    surface
}

#[cfg(test)]
mod tests {
    use objc2_foundation::NSRange;
    use url::Url;

    use super::words;

    #[test]
    fn the_notice_names_the_site_and_the_way_out() {
        let (text, owner) = words(&Url::parse("https://www.example.com/video").unwrap());

        assert_eq!(text, "www.example.com is full screen  ·  ⎋ to exit");
        assert_eq!(owner, NSRange::new(4, 11));
    }

    #[test]
    fn a_page_off_the_web_is_named_whole() {
        let (text, owner) = words(&Url::parse("about:blank").unwrap());

        assert_eq!(text, "about:blank is full screen  ·  ⎋ to exit");
        assert_eq!(owner, NSRange::new(0, 11));
    }
}
