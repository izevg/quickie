//! The full-screen notice (`docs/design/mocks/system/fullscreen/`): whenever a page takes the
//! whole screen, an E1 pill at the top center says whose page it is and how to leave,
//! "⛶ `example.com` is full screen — press ⎋ to exit", for `fullscreen-notice-hold` (3 s). A
//! page with no chrome around it cannot pose as the system or another site (Dia's
//! CVE-2025-13132, `docs/lessons.md`).

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSFont, NSView};
use objc2_foundation::{NSPoint, NSRange, NSTimer};
use quickie_domain_browsing::Origin;
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{Pill, PillPart, PillTone, StyledText, Trigger, motion};
use url::Url;

/// The key that leaves full screen, on its cap.
const EXIT_KEY: &str = "⎋";

struct Shown {
    id: u64,
    pill: Pill,
    text: String,
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
        let (pill, text) = notice(url, self.mtm);
        let size = pill.size();
        let bounds = host.bounds();
        pill.place(NSPoint::new(
            ((bounds.size.width - size.width) / 2.0).round(),
            (bounds.size.height - space::XL - size.height).round(),
        ));
        pill.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        host.addSubview(pill.view());
        motion::show(pill.view(), motion::FADE_FAST, Trigger::App);
        *self.shown.borrow_mut() = Some(Shown { id, pill, text });
        self.after(motion::FULLSCREEN_NOTICE_HOLD, move |notice| {
            notice.fade(id);
        });
    }

    /// Takes the notice away at once: the page left full screen.
    pub fn hide(&self) {
        if let Some(shown) = self.shown.borrow_mut().take() {
            shown.pill.view().removeFromSuperview();
        }
    }

    /// What the notice says, while it shows.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.shown.borrow().as_ref().map(|shown| shown.text.clone())
    }

    fn fade(&self, id: u64) {
        if let Some(shown) = self.shown.borrow().as_ref().filter(|shown| shown.id == id) {
            motion::hide(shown.pill.view(), motion::FADE_FAST, Trigger::App);
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

/// The words before the key and the UTF-16 range of the site's name in them.
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
        format!("{before}{owner}{after} is full screen — press"),
        range,
    )
}

/// The pill, and its words as one sentence.
fn notice(url: &Url, mtm: MainThreadMarker) -> (Pill, String) {
    let (text, owner) = words(url);
    let styled = StyledText::new(&text, &TextRole::RowTitle.font(), &color::label());
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let semibold = unsafe { objc2_app_kit::NSFontWeightSemibold };
    let bold = NSFont::monospacedSystemFontOfSize_weight(TextRole::RowTitle.point_size(), semibold);
    styled.style(owner, &bold, &color::label());
    let pill = Pill::with_parts(
        &[
            PillPart::Symbol {
                name: "arrow.up.left.and.arrow.down.right",
                tint: color::accent,
            },
            PillPart::Styled(styled),
            PillPart::Keys(EXIT_KEY.to_owned()),
            PillPart::Text("to exit".to_owned()),
        ],
        PillTone::Plain,
        mtm,
    );
    (pill, format!("{text} {EXIT_KEY} to exit"))
}

#[cfg(test)]
mod tests {
    use objc2_foundation::NSRange;
    use url::Url;

    use super::words;

    #[test]
    fn the_notice_names_the_site_and_the_way_out() {
        let (text, owner) = words(&Url::parse("https://www.example.com/video").unwrap());

        assert_eq!(text, "www.example.com is full screen — press");
        assert_eq!(owner, NSRange::new(4, 11));
    }

    #[test]
    fn a_page_off_the_web_is_named_whole() {
        let (text, owner) = words(&Url::parse("about:blank").unwrap());

        assert_eq!(text, "about:blank is full screen — press");
        assert_eq!(owner, NSRange::new(0, 11));
    }
}
