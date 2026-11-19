//! The command palette (`⇧⌘P`, `⌘K`, `⌘P`): an E2 glass surface 720 wide at 15 % from the top,
//! a query field, the answers in a list, and a hint bar of the prefixes. It is opened from the
//! keyboard, so it appears and disappears instantly.
//!
//! The palette only shows: the browser answers each edit with [`CommandPalette::set_items`] and
//! runs what the selected row targets.

mod field;
mod rows;

use std::cell::Cell;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSFocusRingType, NSTextField, NSTextInputClient, NSTextView, NSView,
    NSWindow,
};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};
use quickie_application::PaletteItem;
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{Elevation, GlassSurface, ListView, Scrim, Trigger, hint_bar, motion};

use field::FieldDelegate;
pub use rows::row;

/// At most this many rows show at once; the rest scroll.
pub const VISIBLE_ROWS: usize = 8;

/// The prefixes the hint bar names.
const HINTS: [(&str, &str); 5] = [
    (">", "commands"),
    ("@", "headings"),
    ("#", "tags"),
    ("?", "agent"),
    ("!", "search"),
];

pub struct CommandPalette {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    field: Retained<NSTextField>,
    delegate: Retained<FieldDelegate>,
    list: ListView,
    hints: Retained<NSTextField>,
    open: Cell<bool>,
}

impl CommandPalette {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let field = NSTextField::new(mtm);
        field.setFont(Some(&TextRole::PaletteInput.font()));
        field.setTextColor(Some(&color::label()));
        field.setPlaceholderString(Some(&NSString::from_str("Go to a tab, bookmark, or page")));
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
        let delegate = FieldDelegate::new(mtm);
        // SAFETY: the field holds its delegate weakly; the palette keeps it alive as long as
        // the field.
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        let list = ListView::new(mtm);
        let hints = hint_bar(&HINTS, mtm);
        for view in [&*field, list.view(), &*hints] {
            surface.content_view().addSubview(view);
        }
        Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface,
            field,
            delegate,
            list,
            hints,
            open: Cell::new(false),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Opens over `host` with `text` in the field and the caret after it, ready to type on.
    /// The caller answers with [`Self::set_items`].
    pub fn show(&self, host: &NSView, window: &NSWindow, text: &str) {
        self.field.setStringValue(&NSString::from_str(text));
        if !self.is_open() {
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
        self.layout(host);
        window.makeFirstResponder(Some(&self.field));
        self.caret_to_end();
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

    /// What the user typed.
    #[must_use]
    pub fn text(&self) -> String {
        self.field.stringValue().to_string()
    }

    /// Replaces the query, the caret after it, as when a tag row narrows it to `#tag `. The
    /// caller answers the new query.
    pub fn set_text(&self, text: &str) {
        self.field.setStringValue(&NSString::from_str(text));
        self.caret_to_end();
    }

    /// Shows the answer to the current query, best first, the first one selected.
    pub fn set_items(&self, items: &[PaletteItem]) {
        self.list.set_rows(items.iter().map(row).collect());
        // SAFETY: a getter; the superview is retained for the duration of the call.
        let host = unsafe { self.surface.view().superview() };
        if let Some(host) = host {
            self.layout(&host);
        }
    }

    /// Position of the selected row in the last [`Self::set_items`].
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.list.selected()
    }

    /// Moves the selection down by `delta` rows (up when negative), wrapping around.
    pub fn move_selection(&self, delta: isize) {
        self.list.move_selection(delta);
    }

    /// True while an input method is composing in the field: ⏎ then commits the composition.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.field
            .currentEditor()
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
            .is_some_and(|editor| editor.hasMarkedText())
    }

    /// Runs `changed` after each edit in the field.
    pub fn on_change(&self, changed: impl Fn() + 'static) {
        self.delegate.on_change(changed);
    }

    /// Runs `activate` with the position of a clicked row.
    pub fn on_click(&self, activate: impl Fn(usize) + 'static) {
        self.list.on_click(activate);
    }

    fn caret_to_end(&self) {
        if let Some(editor) = self.field.currentEditor() {
            let end = self.field.stringValue().length();
            editor.setSelectedRange(NSRange::new(end, 0));
        }
    }

    /// Sizes the surface to its rows and keeps its top edge at 15 % of the host's height:
    /// the field, the list, then the hint bar, `palette-padding` around them.
    fn layout(&self, host: &NSView) {
        let bounds = host.bounds();
        let padding = space::PALETTE_PADDING;
        let width = overlay::PALETTE_WIDTH.min(bounds.size.width - space::XL * 2.0);
        let inner = width - padding * 2.0;
        let field_height = self.field.frame().size.height;
        let hints_height = self.hints.frame().size.height;
        let shown = self.list.len().min(VISIBLE_ROWS);
        #[expect(
            clippy::cast_precision_loss,
            reason = "at most `VISIBLE_ROWS` rows, exact in a float"
        )]
        let list_height = shown as f64 * self.list.row_height();
        let list_gap = if shown == 0 { 0.0 } else { space::MD };
        let height =
            padding * 2.0 + field_height + list_gap + list_height + space::MD + hints_height;
        let top = bounds.size.height * (1.0 - overlay::PALETTE_TOP);
        self.surface.set_frame(NSRect::new(
            NSPoint::new(
                ((bounds.size.width - width) / 2.0).round(),
                (top - height).round(),
            ),
            NSSize::new(width, height),
        ));
        self.surface.view().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        self.hints.setFrame(NSRect::new(
            NSPoint::new(padding, padding),
            NSSize::new(inner, hints_height),
        ));
        self.list.view().setFrame(NSRect::new(
            NSPoint::new(padding - space::SM, padding + hints_height + space::MD),
            NSSize::new(inner + space::SM * 2.0, list_height),
        ));
        self.field.setFrame(NSRect::new(
            NSPoint::new(padding, height - padding - field_height),
            NSSize::new(inner, field_height),
        ));
    }
}
