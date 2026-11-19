//! The address overlay (`⌘L`): an E2 glass surface with one SF Mono field, at 15 % from the top.
//! It is opened from the keyboard, so it appears and disappears instantly.
//!
//! It belongs in `ui/widgets/palette/address_overlay` once that crate exists.

use std::cell::Cell;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSFocusRingType, NSTextField, NSTextInputClient, NSTextView, NSView,
    NSWindow,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{Elevation, GlassSurface, Scrim, Trigger, motion};

pub struct AddressOverlay {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    field: Retained<NSTextField>,
    open: Cell<bool>,
}

impl AddressOverlay {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let field = NSTextField::new(mtm);
        field.setFont(Some(&TextRole::PaletteInput.mono_font()));
        field.setTextColor(Some(&color::label()));
        field.setPlaceholderString(Some(&NSString::from_str("Search or enter address")));
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setUsesSingleLineMode(true);
        if let Some(cell) = field.cell() {
            cell.setScrollable(true);
            cell.setWraps(false);
        }
        field.sizeToFit();
        field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        surface.content_view().addSubview(&field);
        Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface,
            field,
            open: Cell::new(false),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Opens over `host` with `text` selected, ready to be typed over.
    pub fn show(&self, host: &NSView, window: &NSWindow, text: &str) {
        self.field.setStringValue(&NSString::from_str(text));
        if !self.is_open() {
            self.place(host);
            if let Some(scrim) = &self.scrim {
                scrim.view().setFrame(host.bounds());
                host.addSubview(scrim.view());
            }
            host.addSubview(self.surface.view());
            motion::show(
                self.surface.view(),
                motion::OVERLAY_ENTER,
                Trigger::Keyboard,
            );
            self.open.set(true);
        }
        window.makeFirstResponder(Some(&self.field));
        self.select_all();
    }

    pub fn hide(&self) {
        if !self.is_open() {
            return;
        }
        motion::hide(
            self.surface.view(),
            motion::OVERLAY_ENTER,
            Trigger::Keyboard,
        );
        self.surface.view().removeFromSuperview();
        if let Some(scrim) = &self.scrim {
            scrim.view().removeFromSuperview();
        }
        self.open.set(false);
    }

    pub fn select_all(&self) {
        // SAFETY: `selectText:` accepts any sender, including none.
        unsafe { self.field.selectText(None) };
    }

    pub fn text(&self) -> String {
        self.field.stringValue().to_string()
    }

    /// True while an input method is composing in the field: ⏎ then commits the composition.
    pub fn is_composing(&self) -> bool {
        self.field
            .currentEditor()
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
            .is_some_and(|editor| editor.hasMarkedText())
    }

    /// Centers the surface horizontally with its top edge at 15 % of the host's height.
    fn place(&self, host: &NSView) {
        let bounds = host.bounds();
        let padding = space::PALETTE_PADDING;
        let field_height = self.field.frame().size.height;
        let width = overlay::ADDRESS_WIDTH.min(bounds.size.width - space::XL * 2.0);
        let height = field_height + padding * 2.0;
        let top = bounds.size.height * (1.0 - overlay::PALETTE_TOP);
        let frame = NSRect::new(
            NSPoint::new((bounds.size.width - width) / 2.0, top - height),
            NSSize::new(width, height),
        );
        self.surface.set_frame(frame);
        self.surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin
                | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        self.field.setFrame(NSRect::new(
            NSPoint::new(padding, padding),
            NSSize::new(width - padding * 2.0, field_height),
        ));
    }
}
