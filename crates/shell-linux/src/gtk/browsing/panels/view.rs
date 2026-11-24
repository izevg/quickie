//! Native GTK layout, thumbnail presentation, and card accessibility.

use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::ptr;

use quickie_domain_browsing::{PinnedSlot, Tab, TabId};

use super::super::super::ffi::{self, Bool, Object};
use super::super::super::{backdrop, connect, palette};
use super::{Mode, close_clicked, outside_clicked};

const MAX_STRIP: c_int = 1140;
const MAX_OVERVIEW: c_int = 940;
const GAP: c_int = 12;
const PADDING: c_int = 42;
const CARD_WIDTH: c_int = 152;
const AUTOMATIC: c_int = 1;
const NEVER: c_int = 2;
const SELECT_SINGLE: c_int = 1;
const ELLIPSIZE_END: c_int = 3;
const CONTENT_COVER: c_int = 2;

#[derive(Clone, Copy)]
pub(super) struct View {
    pub(super) panel: Object,
    pub(super) entry: Object,
    pub(super) cards: Object,
    parent: Object,
    scrim: Object,
    picture: Object,
    heading: Object,
    count: Object,
    scroll: Object,
    empty: Object,
    footer: Object,
    data: Object,
}

pub(super) use palette::Allocation;

#[derive(Clone, Copy)]
pub(super) struct Card<'a> {
    pub(super) tab: &'a Tab,
    pub(super) slot: Option<PinnedSlot>,
    pub(super) picture: Object,
}

impl View {
    pub(super) fn new(parent: Object, data: Object) -> Self {
        let (panel, picture, content) = palette::panel_widgets(ffi::VERTICAL, GAP);
        // SAFETY: GTK is initialized. This box is adopted by the overlay below.
        let scrim = unsafe { ffi::gtk_box_new(ffi::VERTICAL, 0) };
        class(scrim, c"quickie-tab-scrim");
        // SAFETY: The new GtkBox can fill the overlay's height.
        unsafe { gtk_widget_set_vexpand(scrim, ffi::TRUE) };
        // SAFETY: The new GtkBox can fill the overlay's width.
        unsafe { ffi::gtk_widget_set_hexpand(scrim, ffi::TRUE) };
        // SAFETY: The GtkOverlay adopts the scrim before the panel.
        unsafe { ffi::gtk_overlay_add_overlay(parent, scrim) };
        // SAFETY: GTK initializes an independent gesture controller on this thread.
        let click = unsafe { gtk_gesture_click_new() };
        connect(click, c"pressed", outside_clicked as *const _, data);
        // SAFETY: The scrim owns the controller for its lifetime.
        unsafe { ffi::gtk_widget_add_controller(scrim, click) };
        // SAFETY: The new box is adopted by the panel below.
        let header = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, GAP) };
        let heading = label(c"", c"quickie-panel-section");
        let count = label(c"", c"quickie-panel-keycap");
        // SAFETY: Expanding the first live label leaves the count at the trailing edge.
        unsafe { ffi::gtk_widget_set_hexpand(heading, ffi::TRUE) };
        append(header, heading);
        append(header, count);
        // SAFETY: GTK initializes a new GtkEntry for the panel.
        let entry = unsafe { ffi::gtk_entry_new() };
        class(entry, c"quickie-panel-input");
        // SAFETY: The live GtkEntry copies its placeholder.
        unsafe { ffi::gtk_entry_set_placeholder_text(entry, c"Filter tabs".as_ptr()) };
        let (scroll, cards) = grid();
        let empty = label(c"No matching tabs", c"quickie-panel-empty");
        // SAFETY: Center the existing empty-state label instead of aligning it like a card.
        unsafe { ffi::gtk_label_set_xalign(empty, 0.5) };
        let footer = label(c"", c"quickie-panel-footer");
        for child in [header, entry, scroll, empty, footer] {
            append(content, child);
        }
        // SAFETY: The GtkOverlay adopts the new panel in front of the scrim.
        unsafe { ffi::gtk_overlay_add_overlay(parent, panel) };
        visible(scrim, false);
        visible(panel, false);
        Self {
            panel,
            entry,
            cards,
            parent,
            scrim,
            picture,
            heading,
            count,
            scroll,
            empty,
            footer,
            data,
        }
    }

    pub(super) fn show(self, mode: Mode) {
        visible(self.entry, mode == Mode::Overview);
        visible(self.scrim, true);
        visible(self.panel, true);
        // SAFETY: The GtkEditable copies the empty text on the GTK thread.
        unsafe { ffi::gtk_editable_set_text(self.entry, c"".as_ptr()) };
        self.focus(Some(mode));
    }

    pub(super) fn focus(self, mode: Option<Mode>) {
        if mode == Some(Mode::Overview) {
            // SAFETY: The shown GtkEntry is focusable and owned by the panel.
            unsafe { ffi::gtk_widget_grab_focus(self.entry) };
        }
    }

    pub(super) fn hide(self) {
        backdrop::clear(self.picture);
        visible(self.panel, false);
        visible(self.scrim, false);
        // SAFETY: The hidden GTK container can release its card pictures immediately.
        unsafe { gtk_flow_box_remove_all(self.cards) };
    }

    pub(super) fn refresh_backdrop(self, browser: Object) {
        if !browser.is_null() {
            backdrop::refresh(self.picture, browser);
        }
    }

    fn panel_width(self, mode: Mode) -> c_int {
        // SAFETY: The parent's GTK allocation width is valid while this attached view lives.
        let width = unsafe { gtk_widget_get_width(self.parent) };
        let limit = match mode {
            Mode::Switcher => MAX_STRIP,
            Mode::Overview => MAX_OVERVIEW,
        };
        (width - 32).clamp(1, limit)
    }

    fn fit(self, mode: Mode) -> c_int {
        let width = self.panel_width(mode) - PADDING;
        ((width + GAP) / (CARD_WIDTH + GAP)).clamp(1, if mode == Mode::Switcher { 6 } else { 4 })
    }

    pub(super) fn capacity(self) -> usize {
        usize::try_from(self.fit(Mode::Switcher)).unwrap_or(1)
    }

    pub(super) fn columns(self) -> usize {
        usize::try_from(self.fit(Mode::Overview)).unwrap_or(1)
    }

    pub(super) fn layout(self, parent: Object, mode: Mode, allocation: &mut Allocation) {
        // SAFETY: This live overlay is in its get-child-position signal.
        let width = unsafe { gtk_widget_get_width(parent) };
        // SAFETY: Layout reads the assigned height of the same live overlay.
        let height = unsafe { gtk_widget_get_height(parent) };
        allocation.width = self.panel_width(mode);
        allocation.x = (width - allocation.width) / 2;
        let maximum = (height - 166).clamp(96, 516);
        // SAFETY: The live scroller accepts the maximum natural height.
        unsafe { gtk_scrolled_window_set_max_content_height(self.scroll, maximum) };
        let mut natural = 0;
        // SAFETY: The panel is live and natural is a writable output for its height.
        unsafe {
            gtk_widget_measure(
                self.panel,
                ffi::VERTICAL,
                allocation.width,
                ptr::null_mut(),
                &raw mut natural,
                ptr::null_mut(),
                ptr::null_mut(),
            );
        }
        allocation.height = natural.clamp(1, (height - 24).max(1));
        allocation.y = (height - allocation.height) / 2;
    }

    pub(super) fn render<'a>(
        self,
        mode: Mode,
        query: &str,
        total: usize,
        kept: usize,
        cards: impl Iterator<Item = Card<'a>>,
        selected: Option<TabId>,
    ) {
        let heading = match mode {
            Mode::Overview => "TAB OVERVIEW".to_owned(),
            Mode::Switcher if query.is_empty() => "RECENT TABS".to_owned(),
            Mode::Switcher => format!("Filter  {query}"),
        };
        set_label(self.heading, &heading);
        let noun = if total == 1 { "tab" } else { "tabs" };
        let count = if kept < total {
            format!("{kept} of {total} {noun}")
        } else {
            format!("{total} {noun}")
        };
        set_label(self.count, &count);
        set_label(
            self.footer,
            match mode {
                Mode::Switcher => "Release Ctrl  Switch     ⇧  Reverse     W  Close",
                Mode::Overview => {
                    "↑ ↓ ← →  Select     ↵  Open     Ctrl+W  Close tab     Esc  Close"
                }
            },
        );
        visible(self.scroll, kept > 0);
        visible(self.empty, kept == 0);
        // SAFETY: This live flow box will replace its prior children with the filtered cards.
        unsafe { gtk_flow_box_remove_all(self.cards) };
        // GTK's natural size for a cached WebKit texture is the full viewport width.
        // Fix the number of columns to the fitting card widths so textures do not stack.
        let limit = self.fit(mode).unsigned_abs();
        let columns = limit.min(u32::try_from(kept.max(1)).unwrap_or(u32::MAX));
        // SAFETY: The flow box accepts this positive lower bound during reconfiguration.
        unsafe { gtk_flow_box_set_min_children_per_line(self.cards, 1) };
        // SAFETY: The live flow box accepts the fitting maximum column count.
        unsafe {
            gtk_flow_box_set_max_children_per_line(self.cards, columns);
        };
        // SAFETY: The same fitting count is valid as a lower bound.
        unsafe { gtk_flow_box_set_min_children_per_line(self.cards, columns) };
        let mut selection = None;
        for (index, card) in cards.enumerate() {
            if Some(card.tab.id) == selected {
                selection = Some(index);
            }
            let child = tab_card(card, mode, self.data);
            // SAFETY: The live flow box adopts the newly constructed card.
            unsafe { gtk_flow_box_append(self.cards, child) };
        }
        if let Some(index) = selection {
            self.select(index);
        }
    }

    pub(super) fn select(self, index: usize) {
        let Ok(index) = c_int::try_from(index) else {
            return;
        };
        // SAFETY: A nonnull result belongs to the live flow box.
        let child = unsafe { gtk_flow_box_get_child_at_index(self.cards, index) };
        if child.is_null() {
            return;
        }
        // SAFETY: GTK selects this child of the live container.
        unsafe { gtk_flow_box_select_child(self.cards, child) };
        let mut bounds = Allocation {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        // SAFETY: GTK writes the allocated bounds of this child in its flow box.
        unsafe { gtk_widget_get_allocation(child, &raw mut bounds) };
        // SAFETY: The scrolled window owns its live vertical adjustment.
        let adjustment = unsafe { gtk_scrolled_window_get_vadjustment(self.scroll) };
        // SAFETY: These finite bounds are in the adjustment's content coordinates.
        unsafe {
            gtk_adjustment_clamp_page(
                adjustment,
                f64::from(bounds.y),
                f64::from(bounds.y + bounds.height),
            );
        };
    }
}

fn grid() -> (Object, Object) {
    // SAFETY: GTK creates both containers on its main thread.
    let scroll = unsafe { ffi::gtk_scrolled_window_new() };
    // SAFETY: The new flow box will be owned by the scrolled window.
    let cards = unsafe { gtk_flow_box_new() };
    class(cards, c"quickie-tab-grid");
    // SAFETY: The flow box centers its natural-width cards in the viewport.
    unsafe { gtk_widget_set_halign(cards, ffi::ALIGN_CENTER) };
    // SAFETY: These GTK policy enum values hide horizontal scrolling.
    unsafe { gtk_scrolled_window_set_policy(scroll, NEVER, AUTOMATIC) };
    // SAFETY: The panel measures the live scrolled window's content up to its height cap.
    unsafe { gtk_scrolled_window_set_propagate_natural_height(scroll, ffi::TRUE) };
    // SAFETY: All cards in the live flow box share one column width and spacing.
    unsafe { gtk_flow_box_set_homogeneous(cards, ffi::TRUE) };
    // SAFETY: The live native container supports single selection.
    unsafe { gtk_flow_box_set_selection_mode(cards, SELECT_SINGLE) };
    // SAFETY: The live flow box sends child-activated for single pointer clicks.
    unsafe { gtk_flow_box_set_activate_on_single_click(cards, ffi::TRUE) };
    // SAFETY: The live flow box uses these fixed logical column and row gaps.
    unsafe { gtk_flow_box_set_column_spacing(cards, GAP.unsigned_abs()) };
    // SAFETY: The same flow box accepts the row gap.
    unsafe { gtk_flow_box_set_row_spacing(cards, GAP.unsigned_abs()) };
    // SAFETY: The scrolled window adopts the live flow box.
    unsafe { ffi::gtk_scrolled_window_set_child(scroll, cards) };
    (scroll, cards)
}

fn tab_card(card: Card<'_>, mode: Mode, data: Object) -> Object {
    // SAFETY: GTK creates a new card wrapper on this thread; its caller parents it.
    let child = unsafe { gtk_flow_box_child_new() };
    class(child, c"quickie-tab-card");
    // SAFETY: The new box will become the wrapper's child.
    let content = unsafe { ffi::gtk_box_new(ffi::VERTICAL, 6) };
    let host = card.tab.url.host_str().unwrap_or_default();
    let address = if host.is_empty() {
        card.tab.url.as_str()
    } else {
        host
    };
    let name = if card.tab.title.trim().is_empty() {
        if address == "about:blank" {
            "New Tab"
        } else {
            address
        }
    } else {
        &card.tab.title
    };
    let title = label(c"", c"quickie-panel-title");
    set_label(title, name);
    let detail = label(c"", c"quickie-panel-detail");
    set_label(detail, address);
    let preview = preview(card.picture);
    append(content, preview);
    append(content, title);
    append(content, detail);
    if mode == Mode::Overview || card.slot.is_some() {
        // SAFETY: The new row is adopted by the card's content.
        let actions = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 6) };
        if let Some(slot) = card.slot {
            let badge = label(c"", c"quickie-panel-keycap");
            set_label(badge, &slot.get().to_string());
            append(actions, badge);
        }
        if mode == Mode::Overview {
            // SAFETY: The new button copies the static themed icon name.
            let close = unsafe { gtk_button_new_from_icon_name(c"window-close-symbolic".as_ptr()) };
            class(close, c"quickie-tab-close");
            // SAFETY: GTK copies the close button's accessible tooltip.
            unsafe { gtk_widget_set_tooltip_text(close, c"Close tab".as_ptr()) };
            connect(close, c"clicked", close_clicked as *const _, data);
            append(actions, close);
        }
        append(content, actions);
    }
    if let Ok(tooltip) = CString::new(name) {
        // SAFETY: This is a live card and GTK copies the title.
        unsafe { gtk_widget_set_tooltip_text(child, tooltip.as_ptr()) };
    }
    // SAFETY: The card wrapper adopts its live content box.
    unsafe { gtk_flow_box_child_set_child(child, content) };
    child
}

fn preview(picture: Object) -> Object {
    let widget = if picture.is_null() {
        // SAFETY: GTK copies the static icon name into a new image widget.
        unsafe { gtk_image_new_from_icon_name(c"web-browser-symbolic".as_ptr()) }
    } else {
        // SAFETY: GTK retains the cached immutable paintable for this new picture.
        let widget = unsafe { gtk_picture_new_for_paintable(picture) };
        // SAFETY: The GtkPicture uses native cover sizing for tab previews.
        unsafe { gtk_picture_set_content_fit(widget, CONTENT_COVER) };
        widget
    };
    class(widget, c"quickie-tab-picture");
    widget
}

fn label(text: &CStr, style: &CStr) -> Object {
    // SAFETY: GTK is initialized and the new label copies this text.
    let label = unsafe { ffi::gtk_label_new(text.as_ptr()) };
    class(label, style);
    // SAFETY: The new label is a live GtkLabel with a valid text alignment.
    unsafe { ffi::gtk_label_set_xalign(label, 0.0) };
    // SAFETY: End ellipsizing keeps long, Unicode titles from growing the card.
    unsafe { gtk_label_set_ellipsize(label, ELLIPSIZE_END) };
    // SAFETY: The native label caps its natural width and retains all text for accessibility.
    unsafe { gtk_label_set_max_width_chars(label, 26) };
    label
}

fn set_label(label: Object, text: &str) {
    let safe_text = text.replace('\0', "�");
    if let Ok(text) = CString::new(safe_text) {
        // SAFETY: The live label copies the valid, NUL-terminated text.
        unsafe { ffi::gtk_label_set_text(label, text.as_ptr()) };
    }
}

fn append(container: Object, child: Object) {
    // SAFETY: The caller supplies a live box and newly constructed child widget.
    unsafe { ffi::gtk_box_append(container, child) };
}

fn class(widget: Object, style: &CStr) {
    // SAFETY: The live widget copies the NUL-terminated CSS class.
    unsafe { ffi::gtk_widget_add_css_class(widget, style.as_ptr()) };
}

fn visible(widget: Object, shown: bool) {
    // SAFETY: Callers retain the specified widgets in the live window.
    unsafe { ffi::gtk_widget_set_visible(widget, Bool::from(shown)) };
}

pub(super) fn input(entry: Object) -> String {
    // SAFETY: The live GtkEntry GtkEditable owns the returned NUL-terminated text.
    let text = unsafe { ffi::gtk_editable_get_text(entry) };
    // SAFETY: GtkEditable keeps the text valid until the next edit. Copy it here.
    unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned()
}

pub(super) fn parent_card(widget: Object) -> Object {
    // SAFETY: GTK supplies the registered type of a GtkFlowBoxChild.
    let kind = unsafe { gtk_flow_box_child_get_type() };
    // SAFETY: The live widget's parent chain remains valid during the click callback.
    unsafe { gtk_widget_get_ancestor(widget, kind) }
}

pub(super) fn index(card: Object) -> Option<usize> {
    if card.is_null() {
        return None;
    }
    // SAFETY: The flow box supplies a live GtkFlowBoxChild or null.
    usize::try_from(unsafe { gtk_flow_box_child_get_index(card) }).ok()
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_adjustment_clamp_page(adjustment: Object, start: f64, end: f64);
    fn gtk_button_new_from_icon_name(icon: *const c_char) -> Object;
    fn gtk_flow_box_append(box_: Object, child: Object);
    fn gtk_flow_box_child_get_index(child: Object) -> c_int;
    fn gtk_flow_box_child_get_type() -> usize;
    fn gtk_flow_box_child_new() -> Object;
    fn gtk_flow_box_child_set_child(child: Object, widget: Object);
    fn gtk_flow_box_get_child_at_index(box_: Object, index: c_int) -> Object;
    fn gtk_flow_box_new() -> Object;
    fn gtk_flow_box_remove_all(box_: Object);
    fn gtk_flow_box_select_child(box_: Object, child: Object);
    fn gtk_flow_box_set_activate_on_single_click(box_: Object, setting: Bool);
    fn gtk_flow_box_set_column_spacing(box_: Object, spacing: c_uint);
    fn gtk_flow_box_set_homogeneous(box_: Object, setting: Bool);
    fn gtk_flow_box_set_max_children_per_line(box_: Object, columns: c_uint);
    fn gtk_flow_box_set_min_children_per_line(box_: Object, columns: c_uint);
    fn gtk_flow_box_set_row_spacing(box_: Object, spacing: c_uint);
    fn gtk_flow_box_set_selection_mode(box_: Object, mode: c_int);
    fn gtk_gesture_click_new() -> Object;
    fn gtk_image_new_from_icon_name(icon: *const c_char) -> Object;
    fn gtk_label_set_ellipsize(label: Object, mode: c_int);
    fn gtk_label_set_max_width_chars(label: Object, characters: c_int);
    fn gtk_picture_new_for_paintable(paintable: Object) -> Object;
    fn gtk_picture_set_content_fit(picture: Object, fit: c_int);
    fn gtk_scrolled_window_get_vadjustment(scroll: Object) -> Object;
    fn gtk_scrolled_window_set_max_content_height(scroll: Object, height: c_int);
    fn gtk_scrolled_window_set_policy(scroll: Object, horizontal: c_int, vertical: c_int);
    fn gtk_scrolled_window_set_propagate_natural_height(scroll: Object, value: Bool);
    fn gtk_widget_get_allocation(widget: Object, bounds: *mut Allocation);
    fn gtk_widget_get_ancestor(widget: Object, kind: usize) -> Object;
    fn gtk_widget_get_height(widget: Object) -> c_int;
    fn gtk_widget_get_width(widget: Object) -> c_int;
    fn gtk_widget_set_halign(widget: Object, align: c_int);
    fn gtk_widget_measure(
        widget: Object,
        orientation: c_int,
        size: c_int,
        minimum: *mut c_int,
        natural: *mut c_int,
        min_baseline: *mut c_int,
        natural_baseline: *mut c_int,
    );
    fn gtk_widget_set_tooltip_text(widget: Object, text: *const c_char);
    fn gtk_widget_set_vexpand(widget: Object, setting: Bool);
}
