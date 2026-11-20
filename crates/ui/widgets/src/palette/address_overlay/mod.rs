//! The address overlay (`⌘L`): an E2 glass surface 640 wide at 20 % from the top, an SF Mono
//! field, rows under it in the palette's list, and a hint bar of its keys. It is opened from
//! the keyboard, so it appears and disappears instantly.
//!
//! It opens showing the page: its URL read like the origin chip ([`origin`]), with the page's
//! actions under it (copy the URL, copy a Markdown link, Shields), each with its keys. The
//! first key that is not one of the overlay's own (`↑` `↓` `⏎` `⎋`), or a click, turns the
//! line into the field, the URL selected, ready to be typed over or edited ([`AddressOverlay::
//! wake`]); the browser routes keys, so it calls that.
//!
//! Typing asks the browser for suggestions (local only: open tabs, bookmarks, history) after
//! every edit, and they replace the actions. A completion shows as selected text after what
//! was typed: typing goes on over it, `⌫` takes it away, `→` or `⏎` takes it. `↑` `↓` move
//! through the rows and the field shows the selected one. `⎋` first brings the page back,
//! then closes. No completion is offered after a deletion, while an input method composes, or
//! with the caret anywhere but at the end.

mod origin;
mod rows;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAccessibility, NSAutoresizingMaskOptions, NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_application::{AddressSuggestions, PaletteItem, Target};
use quickie_ui_shared::tokens::{TextRole, overlay, space};
use quickie_ui_shared::{
    Elevation, GlassSurface, ListRow, ListView, QueryField, Scrim, Trigger, hint_bar, motion,
};

use origin::OriginLine;
pub use rows::row;

/// At most this many rows show at once.
pub const VISIBLE_ROWS: usize = 8;

/// The keys the hint bar names.
const HINTS: [(&str, &str); 5] = [
    ("↑↓", "select"),
    ("⏎", "open"),
    ("⌘⏎", "new tab"),
    ("⌥⏎", "throwaway tab"),
    ("⎋", "close"),
];

/// What an edit runs.
type Changed = Rc<dyn Fn()>;

pub struct AddressOverlay {
    inner: Rc<Inner>,
}

struct Inner {
    scrim: Option<Scrim>,
    surface: GlassSurface,
    field: QueryField,
    origin: OriginLine,
    list: ListView,
    hints: Retained<NSView>,
    open: Cell<bool>,
    /// The page's URL as the overlay opened on it, which `⎋` brings back.
    original: RefCell<String>,
    /// What Shields blocked on the page, when it counted.
    blocked: Cell<Option<u32>>,
    /// True while the overlay shows the page: its URL as the origin line, its actions.
    showing_page: Cell<bool>,
    /// What the user typed, without a completion.
    typed: RefCell<String>,
    /// Whether the last edit asks for a completion.
    completes: Cell<bool>,
    /// The page's actions, shown until the user types, and their rows.
    actions: RefCell<Vec<PaletteItem>>,
    action_rows: RefCell<Vec<ListRow>>,
    /// True while the list holds the actions rather than suggestions.
    listing_actions: Cell<bool>,
    /// The suggestions on screen.
    answer: RefCell<AddressSuggestions>,
    changed: RefCell<Option<Changed>>,
}

impl AddressOverlay {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let surface = GlassSurface::new(Elevation::E2, mtm);
        let field = QueryField::new(
            &TextRole::PaletteInput.mono_font(),
            "Search or enter address",
            mtm,
        );
        field
            .view()
            .setAccessibilityLabel(Some(&NSString::from_str("Address")));
        let origin = OriginLine::new(mtm);
        let list = ListView::new(mtm);
        let hints = hint_bar(&HINTS, mtm);
        // No autoresizing: the glass resizes its content in a later layout pass, which would
        // widen frames `layout` has already set.
        for view in [field.view(), list.view(), &*hints]
            .into_iter()
            .chain(origin.views())
        {
            surface.content_view().addSubview(view);
        }
        let inner = Rc::new(Inner {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface,
            field,
            origin,
            list,
            hints,
            open: Cell::new(false),
            original: RefCell::new(String::new()),
            blocked: Cell::new(None),
            showing_page: Cell::new(false),
            typed: RefCell::new(String::new()),
            completes: Cell::new(false),
            actions: RefCell::new(Vec::new()),
            action_rows: RefCell::new(Vec::new()),
            listing_actions: Cell::new(false),
            answer: RefCell::new(AddressSuggestions::default()),
            changed: RefCell::new(None),
        });
        let weak: Weak<Inner> = Rc::downgrade(&inner);
        inner.field.on_change(move || {
            if let Some(inner) = weak.upgrade() {
                inner.edited();
            }
        });
        Self { inner }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.inner.open.get()
    }

    /// Opens over `host` on the page at `url` (empty for the start page, which opens straight
    /// into the field), with what Shields `blocked` on it when it counted.
    pub fn show(&self, host: &NSView, window: &NSWindow, url: &str, blocked: Option<u32>) {
        let inner = &self.inner;
        inner.original.replace(url.to_owned());
        inner.blocked.set(blocked);
        inner.reset();
        if !self.is_open() {
            if let Some(scrim) = &inner.scrim {
                scrim.view().setFrame(host.bounds());
                host.addSubview(scrim.view());
            }
            host.addSubview(inner.surface.view());
            motion::show(
                inner.surface.view(),
                motion::OVERLAY_ENTER,
                Trigger::Keyboard,
            );
            inner.open.set(true);
        }
        inner.layout(host);
        window.makeFirstResponder(Some(inner.field.view()));
        inner.field.select_all();
    }

    pub fn hide(&self) {
        let inner = &self.inner;
        if !self.is_open() {
            return;
        }
        motion::hide(
            inner.surface.view(),
            motion::OVERLAY_ENTER,
            Trigger::Keyboard,
        );
        inner.surface.view().removeFromSuperview();
        if let Some(scrim) = &inner.scrim {
            scrim.view().removeFromSuperview();
        }
        inner.open.set(false);
    }

    /// The page's actions, shown while the overlay shows the page, each as `row` draws it: the
    /// actions are commands, so the caller hands in the palette's command row, and they look
    /// the same in both.
    pub fn set_actions(&self, actions: Vec<PaletteItem>, row: impl Fn(&PaletteItem) -> ListRow) {
        let inner = &self.inner;
        inner.action_rows.replace(actions.iter().map(row).collect());
        inner.actions.replace(actions);
        if inner.listing_actions.get() {
            inner.list_actions();
            inner.relayout();
        }
    }

    /// The user is about to edit: the origin line gives way to the field, the URL in it
    /// selected. A key the overlay does not take, or a click, calls this before it lands.
    pub fn wake(&self) {
        let inner = &self.inner;
        if inner.showing_page.replace(false) {
            inner.show_page(false);
        }
    }

    /// True while the overlay shows the page rather than the field.
    #[must_use]
    pub fn shows_page(&self) -> bool {
        self.inner.showing_page.get()
    }

    /// What the field holds: the typed text with any completion, or a selected row's URL.
    #[must_use]
    pub fn text(&self) -> String {
        self.inner.field.text()
    }

    /// What the user typed, without the completion.
    #[must_use]
    pub fn typed(&self) -> String {
        self.inner.typed.borrow().clone()
    }

    /// Whether the suggestions for [`Self::typed`] may complete it: the last edit typed on at
    /// the end, and no input method composes.
    #[must_use]
    pub fn completes(&self) -> bool {
        self.inner.completes.get()
    }

    /// Shows the suggestions for the typed text in place of whatever the list held, the first
    /// row selected, and the completion after the typed text, selected.
    pub fn set_suggestions(&self, answer: AddressSuggestions) {
        let inner = &self.inner;
        inner.listing_actions.set(false);
        inner.list.set_rows(answer.rows.iter().map(row).collect());
        inner.answer.replace(answer);
        inner.show_completion();
        inner.relayout();
    }

    /// Position of the selected row.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.inner.list.selected()
    }

    /// Moves the selection down by `delta` rows (up when negative), wrapping around; the field
    /// shows a selected suggestion.
    pub fn move_selection(&self, delta: isize) {
        let inner = &self.inner;
        if inner.list.is_empty() {
            return;
        }
        inner.list.move_selection(delta);
        if !inner.listing_actions.get() {
            inner.show_selected();
        }
    }

    /// What the selected row does; `None` while no row shows.
    #[must_use]
    pub fn chosen(&self) -> Option<Target> {
        self.selected().and_then(|at| self.target_at(at))
    }

    /// What row `at` does.
    #[must_use]
    pub fn target_at(&self, at: usize) -> Option<Target> {
        let inner = &self.inner;
        if inner.listing_actions.get() {
            let actions = inner.actions.borrow();
            return actions.get(at).map(|item| item.target.clone());
        }
        let answer = inner.answer.borrow();
        answer.rows.get(at).map(|row| row.target.clone())
    }

    /// `⎋`: brings the page back when the overlay shows anything else, and returns true;
    /// returns false when it shows the page already, and the overlay should close.
    #[must_use = "false means the overlay should close"]
    pub fn revert(&self) -> bool {
        let inner = &self.inner;
        let blank = inner.original.borrow().is_empty() && inner.field.text().is_empty();
        if inner.showing_page.get() || blank && inner.list.is_empty() {
            return false;
        }
        inner.reset();
        inner.field.select_all();
        inner.relayout();
        true
    }

    /// True while an input method is composing in the field: `⏎` then commits the composition.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.inner.field.is_composing()
    }

    /// Runs `changed` after each edit in the field; it answers with [`Self::set_suggestions`].
    pub fn on_change(&self, changed: impl Fn() + 'static) {
        self.inner.changed.replace(Some(Rc::new(changed)));
    }

    /// Runs `activate` with the position of a clicked row.
    pub fn on_click(&self, activate: impl Fn(usize) + 'static) {
        self.inner.list.on_click(activate);
    }
}

impl Inner {
    /// Shows the page as the overlay opened on it: the origin line and the actions, or, on
    /// the start page, an empty field and nothing under it.
    fn reset(&self) {
        let original = self.original.borrow().clone();
        self.field.set_text(&original);
        self.typed.replace(original.clone());
        self.completes.set(false);
        self.answer.replace(AddressSuggestions::default());
        self.origin.set(&original, self.blocked.get());
        let page = !original.is_empty();
        self.showing_page.set(page);
        self.show_page(page);
        self.listing_actions.set(page);
        if page {
            self.list_actions();
        } else {
            self.list.set_rows(Vec::new());
        }
    }

    /// The origin line in place of the field, or the field again. The field stays the first
    /// responder under the line, only unseen, so the first key reaches it.
    fn show_page(&self, page: bool) {
        self.field
            .view()
            .setAlphaValue(if page { 0.0 } else { 1.0 });
        self.origin.set_visible(page);
    }

    fn list_actions(&self) {
        self.list.set_rows(self.action_rows.borrow().clone());
    }

    /// The user edited the field: what they typed now, whether it may complete, and then the
    /// browser's answer.
    fn edited(&self) {
        if self.showing_page.replace(false) {
            self.show_page(false);
        }
        let now = self.field.text();
        let before = self.typed.replace(now.clone());
        self.completes.set(
            !self.field.is_composing() && self.field.caret_at_end() && types_on(&before, &now),
        );
        let changed = self.changed.borrow().clone();
        if let Some(changed) = changed {
            changed();
        }
    }

    /// The field shows the selected row: the first as typed with its completion, any other as
    /// its URL.
    fn show_selected(&self) {
        let at = self.list.selected().filter(|&at| at > 0);
        let row = at.and_then(|at| {
            self.answer
                .borrow()
                .rows
                .get(at)
                .map(|row| row.field.clone())
        });
        if let Some(text) = row {
            self.field.set_text(&text);
        } else if !self.show_completion() {
            self.field.set_text(&self.typed.borrow());
        }
    }

    /// Shows the completion after the typed text, selected, if there is one to show.
    fn show_completion(&self) -> bool {
        let answer = self.answer.borrow();
        let Some(completion) = answer.completion.as_ref().filter(|_| self.completes.get()) else {
            return false;
        };
        self.field
            .set_text_selecting_after(&completion.text(), completion.typed.encode_utf16().count());
        true
    }

    fn relayout(&self) {
        // SAFETY: a getter; the superview is retained for the duration of the call.
        let host = unsafe { self.surface.view().superview() };
        if let Some(host) = host {
            self.layout(&host);
        }
    }

    /// Sizes the surface to its rows and keeps its top edge at 20 % of the host's height: the
    /// field (or the origin line over it), the list, then the hint bar, `palette-padding`
    /// around them.
    fn layout(&self, host: &NSView) {
        let bounds = host.bounds();
        let padding = space::PALETTE_PADDING;
        let width = overlay::ADDRESS_WIDTH.min(bounds.size.width - space::XL * 2.0);
        let inner = width - padding * 2.0;
        let field_height = self.field.height();
        let hints_height = self.hints.frame().size.height;
        let shown = self.list.len().min(VISIBLE_ROWS);
        // Rows differ in height: a page is two lines, an action one.
        let list_height = self.list.height_for(shown);
        let list_gap = if shown == 0 { 0.0 } else { space::MD };
        let height =
            padding * 2.0 + field_height + list_gap + list_height + space::MD + hints_height;
        let top = bounds.size.height * (1.0 - overlay::ADDRESS_TOP);
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
        let line = NSRect::new(
            NSPoint::new(padding, height - padding - field_height),
            NSSize::new(inner, field_height),
        );
        self.field.view().setFrame(line);
        self.origin.layout(line);
    }
}

/// Whether an edit from `before` to `now` typed on at the end, so a completion may follow: not
/// a deletion, not a change in the middle.
fn types_on(before: &str, now: &str) -> bool {
    now.len() > before.len() && now.starts_with(before)
}

#[cfg(test)]
mod tests {
    use super::types_on;

    #[test]
    fn typing_on_at_the_end_may_complete() {
        assert!(types_on("", "g"));
        assert!(types_on("git", "gith"));
        assert!(types_on("git", "git/"));
    }

    #[test]
    fn deleting_or_changing_earlier_text_does_not() {
        assert!(!types_on("git", "gi"));
        assert!(!types_on("git", "git"));
        assert!(!types_on("git", "gat"));
        assert!(!types_on("git", "xgit"));
    }
}
