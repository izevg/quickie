//! An extension's action popup (`docs/design/inventory.md`, popovers): its page in an E2
//! surface at the top right, where the toolbar button would be if Quickie had a toolbar. It
//! opens from the palette or the extension's key, never on hover, and closes with `⎋`, a click
//! on the page, or another overlay.
//!
//! The popup page is the extension's own `WKWebView`; the popover only frames it. It takes the
//! size the page asks for (its intrinsic size), within what Chrome allows a popup: 25 × 25 to
//! 800 × 600 points.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindow};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_ui_shared::tokens::space;
use quickie_ui_shared::{Elevation, GlassSurface, Scrim, Trigger, motion};

/// The size a popup gets when its page does not say.
const DEFAULT: NSSize = NSSize::new(380.0, 560.0);

/// The smallest and largest popup, as Chrome bounds them.
const MIN: CGFloat = 25.0;
const MAX: NSSize = NSSize::new(800.0, 600.0);

pub struct ExtensionPopover {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    page: RefCell<Option<Retained<NSView>>>,
    open: Cell<bool>,
}

impl ExtensionPopover {
    #[must_use]
    pub fn new(mtm: objc2::MainThreadMarker) -> Self {
        Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface: GlassSurface::new(Elevation::E2, mtm),
            page: RefCell::new(None),
            open: Cell::new(false),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Opens over `host` with the popup `page`, which takes the keyboard.
    pub fn show(&self, host: &NSView, window: &NSWindow, page: &NSView, trigger: Trigger) {
        self.take_page();
        let size = popup_size(page.intrinsicContentSize(), page.frame().size);
        let inset = space::XS;
        let outer = NSSize::new(size.width + inset * 2.0, size.height + inset * 2.0);
        self.surface.place_top_right(host, outer);
        let content = self.surface.content_view();
        page.setFrame(NSRect::new(NSPoint::new(inset, inset), size));
        page.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        content.addSubview(page);
        *self.page.borrow_mut() = Some(Retained::from(page));
        if !self.is_open() {
            if let Some(scrim) = &self.scrim {
                scrim.view().setFrame(host.bounds());
                host.addSubview(scrim.view());
            }
            host.addSubview(self.surface.view());
            motion::show(self.surface.view(), motion::OVERLAY_ENTER, trigger);
            self.open.set(true);
        }
        window.makeFirstResponder(Some(page));
    }

    /// Closes it; the popup page leaves the popover.
    pub fn hide(&self, trigger: Trigger) {
        if !self.is_open() {
            return;
        }
        motion::hide(self.surface.view(), motion::OVERLAY_ENTER, trigger);
        self.surface.view().removeFromSuperview();
        if let Some(scrim) = &self.scrim {
            scrim.view().removeFromSuperview();
        }
        self.take_page();
        self.open.set(false);
    }

    /// The popover's frame in its host, to tell a click inside it from one on the page.
    #[must_use]
    pub fn frame(&self) -> NSRect {
        self.surface.view().frame()
    }

    fn take_page(&self) {
        if let Some(page) = self.page.borrow_mut().take() {
            page.removeFromSuperview();
        }
    }
}

/// The popup's size: what the page asks for, else its frame, else the default, within
/// Chrome's bounds.
fn popup_size(intrinsic: NSSize, frame: NSSize) -> NSSize {
    let asked = |size: NSSize| size.width > 0.0 && size.height > 0.0;
    let size = if asked(intrinsic) {
        intrinsic
    } else if asked(frame) {
        frame
    } else {
        DEFAULT
    };
    NSSize::new(
        size.width.clamp(MIN, MAX.width),
        size.height.clamp(MIN, MAX.height),
    )
}

#[cfg(test)]
mod tests {
    use objc2_foundation::NSSize;

    use super::{DEFAULT, popup_size};

    #[test]
    fn a_popup_takes_the_size_its_page_asks_for_within_bounds() {
        let none = NSSize::new(-1.0, -1.0);

        assert_eq!(
            popup_size(NSSize::new(300.0, 400.0), none),
            NSSize::new(300.0, 400.0)
        );
        assert_eq!(
            popup_size(none, NSSize::new(320.0, 200.0)),
            NSSize::new(320.0, 200.0)
        );
        assert_eq!(popup_size(none, NSSize::ZERO), DEFAULT);
        assert_eq!(
            popup_size(NSSize::new(10.0, 2_000.0), none),
            NSSize::new(25.0, 600.0)
        );
    }
}
