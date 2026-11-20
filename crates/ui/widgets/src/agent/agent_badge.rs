//! The agent badge (`docs/design/mocks/agent/session/`): an E1 pill at the top of the page while
//! the agent works, `● Agent · answering · 24 tok/s   Stop ⌘.`, the dot in the agent color. It
//! shows only while the user's own request runs, and goes when the answer ends.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSLineBreakMode, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize};
use quickie_ui_shared::tokens::{TextRole, color, space};
use quickie_ui_shared::{Elevation, GlassSurface, StyledText, Trigger, motion};

/// The dot before the text.
const DOT: CGFloat = 8.0;
/// What the badge offers after what the agent does.
const STOP: &str = "Stop ⌘.";

pub struct AgentBadge {
    surface: GlassSurface,
    dot: Retained<NSView>,
    label: Retained<NSTextField>,
    shown: std::cell::Cell<bool>,
    /// What shows, and over how wide a page: the same again costs nothing.
    last: std::cell::RefCell<(String, CGFloat)>,
}

impl AgentBadge {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E1, mtm);
        let dot = NSView::new(mtm);
        dot.setWantsLayer(true);
        if let Some(layer) = dot.layer() {
            layer.setBackgroundColor(Some(&color::agent().CGColor()));
            layer.setCornerRadius(DOT / 2.0);
        }
        let label = NSTextField::labelWithString(&objc2_foundation::NSString::new(), mtm);
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        surface.content_view().addSubview(&dot);
        surface.content_view().addSubview(&label);
        Self {
            surface,
            dot,
            label,
            shown: std::cell::Cell::new(false),
            last: std::cell::RefCell::new((String::new(), 0.0)),
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
    /// the page not under a panel docked at the right.
    pub fn show(&self, host: &NSView, text: &str, width: CGFloat) {
        // SAFETY: a getter; the superview is retained for the comparison.
        let parent = unsafe { self.surface.view().superview() };
        let same_host = parent.is_some_and(|parent| &*parent == host);
        let shown = (text.to_owned(), width.round());
        if self.shown.get() && same_host && *self.last.borrow() == shown {
            return;
        }
        *self.last.borrow_mut() = shown;
        let styled = StyledText::new(
            &format!("{text}   {STOP}"),
            &TextRole::RowDetail.font(),
            &color::label(),
        );
        let start = text.encode_utf16().count() + 3;
        styled.style(
            NSRange::new(start, STOP.encode_utf16().count()),
            &TextRole::Caption.mono_font(),
            &color::secondary_label(),
        );
        self.label.setAttributedStringValue(styled.string());
        self.label.sizeToFit();
        let text_size = self.label.frame().size;
        let size = NSSize::new(
            (space::MD + DOT + space::SM + text_size.width + space::LG).ceil(),
            (text_size.height + space::SM * 2.0).ceil(),
        );
        self.dot.setFrame(NSRect::new(
            NSPoint::new(space::MD, ((size.height - DOT) / 2.0).round()),
            NSSize::new(DOT, DOT),
        ));
        self.label.setFrame(NSRect::new(
            NSPoint::new(space::MD + DOT + space::SM, space::SM),
            text_size,
        ));
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
}
