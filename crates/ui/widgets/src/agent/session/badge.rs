//! The agent badge (`docs/design/mocks/agent/session/`): an E1 pill at the top of the page while
//! the agent works, `● Agent · answering · 24 tok/s │ Stop ⌘.`, ringed in the agent color, the
//! dot and `Agent` orange, `Stop` red beside its keycaps. It shows only while the user's own
//! request runs, and goes when the answer ends.

use std::cell::{Cell, RefCell};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSLineBreakMode, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{
    Elevation, GlassSurface, Plate, StyledText, Trigger, keycap_group, motion,
};

/// The dot before the text.
const DOT: CGFloat = 8.0;
/// Who acts, the first word of the badge.
const WHO: &str = "Agent";
/// What the badge offers after what the agent does, and its keys.
const STOP: &str = "Stop";
const STOP_KEYS: &str = "⌘.";

pub struct AgentBadge {
    surface: GlassSurface,
    ring: Retained<Plate>,
    dot: Retained<Plate>,
    label: Retained<NSTextField>,
    divider: Retained<Plate>,
    stop: Retained<NSTextField>,
    keys: Retained<NSView>,
    shown: Cell<bool>,
    /// What shows, and over how wide a page: the same again costs nothing.
    last: RefCell<(String, CGFloat)>,
}

impl AgentBadge {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E1, mtm);
        let ring = Plate::new(None, Some(agent_line), 0.0, mtm);
        let dot = Plate::new(Some(color::agent), None, DOT / 2.0, mtm);
        let label = NSTextField::labelWithString(&NSString::new(), mtm);
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let divider = Plate::new(Some(color::separator), None, 0.0, mtm);
        let stop = NSTextField::labelWithString(&NSString::from_str(STOP), mtm);
        stop.setFont(Some(&semibold(TextRole::RowDetail)));
        stop.setTextColor(Some(&color::destructive()));
        stop.sizeToFit();
        let keys = keycap_group(STOP_KEYS, false, mtm);
        let content = surface.content_view();
        for view in [&*dot as &NSView, &label, &divider, &stop, &keys, &ring] {
            content.addSubview(view);
        }
        Self {
            surface,
            ring,
            dot,
            label,
            divider,
            stop,
            keys,
            shown: Cell::new(false),
            last: RefCell::new((String::new(), 0.0)),
        }
    }

    #[must_use]
    pub fn is_shown(&self) -> bool {
        self.shown.get()
    }

    /// The badge's surface, for a render.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.surface.view()
    }

    /// Shows `text` at the top of `host`, centered over its left `width` points, the part of
    /// the page not under a panel docked at the right. `text` starts with `Agent`, which is
    /// set in the agent color.
    pub fn show(&self, host: &NSView, text: &str, width: CGFloat) {
        // SAFETY: a getter; the superview is retained for the comparison.
        let parent = unsafe { self.surface.view().superview() };
        let same_host = parent.is_some_and(|parent| &*parent == host);
        let shown = (text.to_owned(), width.round());
        if self.shown.get() && same_host && *self.last.borrow() == shown {
            return;
        }
        *self.last.borrow_mut() = shown;
        self.label.setAttributedStringValue(styled(text).string());
        self.label.sizeToFit();
        let size = self.lay_out();
        let bounds = host.bounds().size;
        let room = width.min(bounds.width);
        self.surface.set_frame(NSRect::new(
            NSPoint::new(
                ((room - size.width) / 2.0).max(space::LG).round(),
                (bounds.height - space::LG - size.height).round(),
            ),
            size,
        ));
        self.surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        if !self.shown.get() || !same_host {
            self.surface.view().removeFromSuperview();
            host.addSubview(self.surface.view());
            motion::show(self.surface.view(), motion::FADE_FAST, Trigger::App);
            self.shown.set(true);
        }
    }

    pub fn hide(&self) {
        if !self.shown.get() {
            return;
        }
        motion::hide(self.surface.view(), motion::FADE_FAST, Trigger::App);
        self.surface.view().removeFromSuperview();
        self.shown.set(false);
    }

    /// Places the parts left to right and returns the badge's size.
    fn lay_out(&self) -> NSSize {
        let text = self.label.frame().size;
        let stop = self.stop.frame().size;
        let keys = self.keys.frame().size;
        let height = (text.height.max(keys.height) + space::SM * 2.0).ceil();
        let middle = |of: CGFloat| ((height - of) / 2.0).round();
        let mut x = space::MD;
        self.dot.setFrame(NSRect::new(
            NSPoint::new(x, middle(DOT)),
            NSSize::new(DOT, DOT),
        ));
        x += DOT + space::SM;
        self.label
            .setFrame(NSRect::new(NSPoint::new(x, middle(text.height)), text));
        x += text.width + space::MD;
        let divider = (height - space::MD * 2.0).max(0.0);
        self.divider.setFrame(NSRect::new(
            NSPoint::new(x, middle(divider)),
            NSSize::new(1.0, divider),
        ));
        x += 1.0 + space::MD;
        self.stop
            .setFrame(NSRect::new(NSPoint::new(x, middle(stop.height)), stop));
        x += stop.width + space::SM;
        self.keys
            .setFrameOrigin(NSPoint::new(x, middle(keys.height)));
        x += keys.width + space::MD;
        let size = NSSize::new(x.ceil(), height);
        self.ring.set_radius(height / 2.0);
        self.ring
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
        size
    }
}

/// `text` in the label color, its leading `Agent` semibold in the agent color.
fn styled(text: &str) -> StyledText {
    let font = TextRole::RowDetail.font();
    let styled = StyledText::new(text, &font, &color::label());
    if text.starts_with(WHO) {
        styled.style(
            NSRange::new(0, WHO.encode_utf16().count()),
            &semibold(TextRole::RowDetail),
            &color::agent(),
        );
    }
    styled
}

/// The ring around the badge: the agent color at the line alpha.
fn agent_line() -> Retained<NSColor> {
    color::agent().colorWithAlphaComponent(color::LINE_ALPHA)
}

fn semibold(role: TextRole) -> Retained<objc2_app_kit::NSFont> {
    // SAFETY: `NSFontWeightSemibold` is an immutable `CGFloat` exported by AppKit.
    let weight = unsafe { objc2_app_kit::NSFontWeightSemibold };
    objc2_app_kit::NSFont::systemFontOfSize_weight(role.point_size(), weight)
}
