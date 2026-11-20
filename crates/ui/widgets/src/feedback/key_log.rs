//! The key debug overlay: an E1 panel at the bottom left listing the last key presses and what
//! the browser made of each, in SF Mono, so a flaky shortcut can be reported precisely. It
//! opens from the palette only ("Show Key Debug Overlay"), takes no keys, and stays until the
//! same command closes it. The browser hands it the lines; it only shows them.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSLineBreakMode, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, radius, space};
use quickie_ui_shared::{Elevation, GlassSurface, Trigger, motion};

/// The panel's width; the lines are cut, not wrapped, past it.
const WIDTH: CGFloat = 560.0;

pub struct KeyLogPanel {
    surface: GlassSurface,
    text: Retained<NSTextField>,
    host: RefCell<Option<Retained<NSView>>>,
}

impl KeyLogPanel {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let text = NSTextField::labelWithString(&NSString::new(), mtm);
        text.setFont(Some(&TextRole::Caption.mono_font()));
        text.setTextColor(Some(&color::label()));
        text.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        text.setMaximumNumberOfLines(0);
        let surface = GlassSurface::new(Elevation::E1, mtm);
        surface.content_view().addSubview(&text);
        Self {
            surface,
            text,
            host: RefCell::new(None),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.host.borrow().is_some()
    }

    /// Shows `lines` at the bottom left of `host`, at once: a command opened it.
    pub fn show(&self, host: &NSView, lines: &[String]) {
        self.hide();
        host.addSubview(self.surface.view());
        *self.host.borrow_mut() = Some(host.retain());
        self.update(lines);
        motion::show(self.surface.view(), motion::FADE_FAST, Trigger::Keyboard);
    }

    /// Replaces the lines, newest last.
    pub fn update(&self, lines: &[String]) {
        let Some(host) = self.host.borrow().clone() else {
            return;
        };
        let shown = if lines.is_empty() {
            "Press keys: each press shows here with what Quickie made of it.".to_owned()
        } else {
            lines.join("\n")
        };
        self.text.setStringValue(&NSString::from_str(&shown));
        let padding = space::MD;
        let width = WIDTH.min(host.bounds().size.width - space::LG * 2.0);
        let inner = width - padding * 2.0;
        let line_height = (TextRole::Caption.point_size() * 1.3).ceil();
        let count = shown.lines().count().max(1);
        let text_height = line_height * CGFloat::from(u16::try_from(count).unwrap_or(u16::MAX));
        let height = text_height + padding * 2.0;
        self.surface.set_frame_rounded(
            NSRect::new(
                NSPoint::new(space::LG, space::LG),
                NSSize::new(width, height),
            ),
            radius::CARD,
        );
        self.surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMaxXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        self.text.setFrame(NSRect::new(
            NSPoint::new(padding, padding),
            NSSize::new(inner.max(0.0), text_height),
        ));
    }

    pub fn hide(&self) {
        if self.host.borrow_mut().take().is_some() {
            self.surface.view().removeFromSuperview();
        }
    }
}
