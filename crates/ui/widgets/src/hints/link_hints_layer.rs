//! Link hints (`F`): a letter label over every link and control the page shows, and an E1
//! pill that names the keys. Typing a label follows it; `⇧` opens a link in a background tab.
//!
//! The labels are drawn natively, in a layer of their own above the page, and never in the
//! page: the site's DOM, styles, and layout stay untouched, its scripts cannot see or restyle
//! the hints, and a strict content security policy cannot block them. The page only answers
//! where its targets are. Opened from the keyboard, the layer appears and goes at once.
//!
//! - [`labels`]: the labels, and what the letters typed so far match.

pub use quickie_domain_commands::link_hint as labels;

use std::cell::{Cell, RefCell};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAttributedStringNSStringDrawing, NSAutoresizingMaskOptions, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSAttributedString, NSCopying, NSPoint, NSRange, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CATextLayer, CATransaction, kCACornerCurveContinuous};
use quickie_ui_shared::tokens::{TextRole, color, radius, space};
use quickie_ui_shared::{Elevation, GlassSurface, StyledText, hint_bar};

use labels::{HOME_ROW, Typed};

/// The keys the pill names.
const HINTS: [(&str, &str); 3] = [
    ("ASDF…", "type a hint"),
    ("⇧", "background tab"),
    ("⎋", "cancel"),
];

/// What a letter typed while the hints show did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKey {
    /// It narrowed the labels on screen.
    Typed,
    /// It completed the label of target `index`: the hints are gone, follow it.
    Follow(usize),
    /// It starts no label and changed nothing.
    Ignored,
}

struct Badge {
    fill: Retained<CALayer>,
    text: Retained<CATextLayer>,
}

pub struct LinkHintsLayer {
    view: Retained<NSView>,
    root: Retained<CALayer>,
    pill: GlassSurface,
    keys: Retained<NSView>,
    badges: RefCell<Vec<Badge>>,
    labels: RefCell<Vec<String>>,
    typed: RefCell<String>,
    background: Cell<bool>,
    open: Cell<bool>,
}

impl LinkHintsLayer {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let root = CALayer::new();
        let view = NSView::new(mtm);
        // A layer-hosting view: the badges are its layer's sublayers, laid out by hand.
        view.setLayer(Some(&root));
        view.setWantsLayer(true);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let pill = GlassSurface::new(Elevation::E1, mtm);
        let keys = hint_bar(&HINTS, mtm);
        pill.content_view().addSubview(&keys);
        Self {
            view,
            root,
            pill,
            keys,
            badges: RefCell::new(Vec::new()),
            labels: RefCell::new(Vec::new()),
            typed: RefCell::new(String::new()),
            background: Cell::new(false),
            open: Cell::new(false),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// True when the hints were opened to follow into a background tab (`⇧F`).
    #[must_use]
    pub fn background(&self) -> bool {
        self.background.get()
    }

    /// Labels `targets` (frames in `host`, which the page fills) over `host`. Nothing shows
    /// for no targets.
    pub fn show(&self, host: &NSView, targets: &[NSRect], background: bool) {
        self.hide();
        if targets.is_empty() {
            return;
        }
        *self.labels.borrow_mut() = labels::labels(targets.len(), &HOME_ROW);
        self.typed.borrow_mut().clear();
        self.background.set(background);
        self.view.setFrame(host.bounds());
        let scale = host
            .window()
            .map_or(2.0, |window| window.backingScaleFactor());
        quietly(|| {
            let bounds = host.bounds().size;
            let mut badges = self.badges.borrow_mut();
            for (target, label) in targets.iter().zip(self.labels.borrow().iter()) {
                let badge = badge(scale);
                place(&badge, label, "", *target, bounds);
                self.root.addSublayer(&badge.fill);
                badges.push(badge);
            }
        });
        host.addSubview(&self.view);
        self.place_pill(host);
        host.addSubview(self.pill.view());
        self.open.set(true);
    }

    pub fn hide(&self) {
        if !self.is_open() {
            return;
        }
        self.open.set(false);
        quietly(|| {
            for badge in self.badges.borrow_mut().drain(..) {
                badge.fill.removeFromSuperlayer();
            }
        });
        self.view.removeFromSuperview();
        self.pill.view().removeFromSuperview();
    }

    /// `letter` typed, on any layout: the key's place on a US keyboard. A completed label
    /// hides the hints.
    pub fn type_letter(&self, letter: char) -> HintKey {
        let typed = format!("{}{}", self.typed.borrow(), letter.to_ascii_lowercase());
        match labels::narrow(&self.labels.borrow(), &typed) {
            Typed::Miss => return HintKey::Ignored,
            Typed::Hit(index) => {
                self.hide();
                return HintKey::Follow(index);
            }
            Typed::Narrowed => {}
        }
        *self.typed.borrow_mut() = typed;
        self.restyle();
        HintKey::Typed
    }

    /// `⌫`: takes back the last letter typed.
    pub fn backspace(&self) {
        if self.typed.borrow_mut().pop().is_some() {
            self.restyle();
        }
    }

    /// Shows only the labels that start with what is typed, the typed part dimmed.
    fn restyle(&self) {
        let typed = self.typed.borrow();
        let labels = self.labels.borrow();
        quietly(|| {
            for (badge, label) in self.badges.borrow().iter().zip(labels.iter()) {
                let matches = label.starts_with(typed.as_str());
                badge.fill.setHidden(!matches);
                if matches {
                    set_string(&badge.text, &styled(label, &typed));
                }
            }
        });
    }

    fn place_pill(&self, host: &NSView) {
        let keys = self.keys.frame().size;
        let size = NSSize::new(
            (keys.width + space::LG * 2.0).ceil(),
            (keys.height + space::SM * 2.0).ceil(),
        );
        let bounds = host.bounds().size;
        self.pill.set_frame(NSRect::new(
            NSPoint::new(((bounds.width - size.width) / 2.0).round(), space::XL),
            size,
        ));
        self.keys.setFrameOrigin(NSPoint::new(
            space::LG,
            ((size.height - keys.height) / 2.0).floor(),
        ));
        self.pill.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
    }
}

/// Runs `change` with Core Animation's implicit animations off: the keyboard never waits.
fn quietly(change: impl FnOnce()) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    change();
    CATransaction::commit();
}

/// A keycap-shaped badge in the accent color, drawn at `scale`.
fn badge(scale: CGFloat) -> Badge {
    let fill = CALayer::new();
    fill.setBackgroundColor(Some(&color::accent().CGColor()));
    fill.setCornerRadius(radius::KEYCAP);
    // SAFETY: an immutable string exported by Core Animation.
    fill.setCornerCurve(unsafe { kCACornerCurveContinuous });
    fill.setShadowOpacity(0.25);
    fill.setShadowRadius(1.0);
    fill.setShadowOffset(NSSize::new(0.0, -0.5));
    let text = CATextLayer::new();
    text.setContentsScale(scale);
    fill.addSublayer(&text);
    Badge { fill, text }
}

/// Sets `label` (with `typed` dimmed) on `badge` and puts it at the top left of `target`,
/// kept inside `bounds`.
fn place(badge: &Badge, label: &str, typed: &str, target: NSRect, bounds: NSSize) {
    let string = styled(label, typed);
    let text = string.size();
    let size = NSSize::new(
        (text.width + space::XS * 2.0).ceil(),
        (text.height + space::XS / 2.0).ceil(),
    );
    let top = target.origin.y + target.size.height;
    let origin = NSPoint::new(
        (target.origin.x - space::XS)
            .min(bounds.width - size.width)
            .max(0.0),
        (top - size.height + space::XS)
            .min(bounds.height - size.height)
            .max(0.0),
    );
    badge.fill.setFrame(NSRect::new(origin, size));
    badge.text.setFrame(NSRect::new(
        NSPoint::new(space::XS, ((size.height - text.height) / 2.0).floor()),
        NSSize::new(text.width.ceil(), text.height.ceil()),
    ));
    set_string(&badge.text, &string);
}

fn set_string(layer: &CATextLayer, string: &NSAttributedString) {
    // SAFETY: a text layer draws an `NSAttributedString`, and copies what it is given.
    unsafe { layer.setString(Some(string)) };
}

/// The label in capitals, SF Mono at `mono-small`, the typed part dimmed.
fn styled(label: &str, typed: &str) -> Retained<NSAttributedString> {
    let caps = label.to_ascii_uppercase();
    let font = TextRole::Caption.mono_font();
    let styled = StyledText::new(&caps, &font, &color::selection_text());
    styled.style(
        NSRange::new(0, typed.len()),
        &font,
        &color::selection_text().colorWithAlphaComponent(0.55),
    );
    styled.string().copy()
}
