//! The command palette (`⇧⌘P`, `⌘K`, `⌘P`): an E2 glass surface 720 wide at 15 % from the top,
//! a query field, the answers in a list, and a hint bar of the prefixes. It is opened from the
//! keyboard, so it appears and disappears instantly.
//!
//! The palette only shows: the browser answers each edit with [`CommandPalette::set_items`] and
//! runs what the selected row targets.

mod rows;

use std::cell::Cell;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSTextField, NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_application::PaletteItem;
use quickie_ui_shared::tokens::{TextRole, overlay, space};
use quickie_ui_shared::{
    Elevation, GlassSurface, ListView, QueryField, Scrim, Trigger, hint_bar, motion,
};

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
    field: QueryField,
    list: ListView,
    hints: Retained<NSTextField>,
    open: Cell<bool>,
}

impl CommandPalette {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let field = QueryField::new(
            &TextRole::PaletteInput.font(),
            "Go to a tab, bookmark, or page",
            mtm,
        );
        let list = ListView::new(mtm);
        let hints = hint_bar(&HINTS, mtm);
        for view in [field.view(), list.view(), &*hints] {
            surface.content_view().addSubview(view);
        }
        Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface,
            field,
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
        self.field.set_text(text);
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
        window.makeFirstResponder(Some(self.field.view()));
        self.field.caret_to_end();
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
        self.field.text()
    }

    /// Replaces the query, the caret after it, as when a tag row narrows it to `#tag `. The
    /// caller answers the new query.
    pub fn set_text(&self, text: &str) {
        self.field.set_text(text);
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
        self.field.is_composing()
    }

    /// Runs `changed` after each edit in the field.
    pub fn on_change(&self, changed: impl Fn() + 'static) {
        self.field.on_change(changed);
    }

    /// Runs `activate` with the position of a clicked row.
    pub fn on_click(&self, activate: impl Fn(usize) + 'static) {
        self.list.on_click(activate);
    }

    /// Sizes the surface to its rows and keeps its top edge at 15 % of the host's height:
    /// the field, the list, then the hint bar, `palette-padding` around them.
    fn layout(&self, host: &NSView) {
        let bounds = host.bounds();
        let padding = space::PALETTE_PADDING;
        let width = overlay::PALETTE_WIDTH.min(bounds.size.width - space::XL * 2.0);
        let inner = width - padding * 2.0;
        let field_height = self.field.height();
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
        self.field.view().setFrame(NSRect::new(
            NSPoint::new(padding, height - padding - field_height),
            NSSize::new(inner, field_height),
        ));
    }
}
