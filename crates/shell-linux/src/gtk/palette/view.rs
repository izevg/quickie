//! Native widgets shared by the address overlay and command palette.

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::ptr;

use super::super::ffi::{self, Bool, Object};
use super::{Item, Mode};

const POLICY_AUTOMATIC: c_int = 1;
const POLICY_NEVER: c_int = 2;

#[derive(Clone, Copy)]
pub(in crate::gtk) struct View {
    pub(super) panel: Object,
    pub(super) entry: Object,
    pub(super) list: Object,
    picture: Object,
    scroll: Object,
    empty: Object,
    section: Object,
    footer: Object,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::gtk) struct Allocation {
    pub(in crate::gtk) x: c_int,
    pub(in crate::gtk) y: c_int,
    pub(in crate::gtk) width: c_int,
    pub(in crate::gtk) height: c_int,
}

impl View {
    pub(in crate::gtk) fn new(parent: Object) -> Self {
        let (panel, picture, content) = panel_widgets(ffi::VERTICAL, 10);
        let section = label(c"", c"quickie-panel-section");
        // SAFETY: GTK is initialized on this thread and content adopts the entry below.
        let entry = unsafe { ffi::gtk_entry_new() };
        class(entry, c"quickie-panel-input");
        // SAFETY: This is a live GtkEntry implementing GtkEditable.
        unsafe { ffi::gtk_editable_set_width_chars(entry, 1) };
        let (scroll, list) = results_widgets();
        let empty = label(c"", c"quickie-panel-empty");
        let footer = label(
            c"↑ ↓  Select     ↵  Open     Esc  Close",
            c"quickie-panel-footer",
        );
        for child in [section, entry, scroll, empty, footer] {
            // SAFETY: The live content box adopts each new widget.
            unsafe { ffi::gtk_box_append(content, child) };
        }
        // SAFETY: The browser's live overlay adopts this new panel.
        unsafe { ffi::gtk_overlay_add_overlay(parent, panel) };
        visible(panel, false);
        Self {
            panel,
            entry,
            list,
            picture,
            scroll,
            empty,
            section,
            footer,
        }
    }

    pub(super) fn observe_focus(self, on_leave: *const c_void, data: Object) {
        // SAFETY: GTK is initialized and the panel adopts the new controller below.
        let controller = unsafe { gtk_event_controller_focus_new() };
        crate::gtk::connect(controller, c"leave", on_leave, data);
        // SAFETY: The live panel tracks focus anywhere inside its subtree.
        unsafe { ffi::gtk_widget_add_controller(self.panel, controller) };
    }

    pub(super) fn show(self, mode: Mode, text: &CStr) {
        let (section, placeholder) = match mode {
            Mode::Address => (c"ADDRESS", c"Enter an address or search"),
            Mode::Commands => (c"COMMAND PALETTE", c"Search commands"),
            Mode::GoTo => (c"GO TO", c"Search open tabs and recent history"),
        };
        // SAFETY: The panel owns its label and GTK copies the static text.
        unsafe { ffi::gtk_label_set_text(self.section, section.as_ptr()) };
        // SAFETY: The panel's live entry copies the static placeholder.
        unsafe { ffi::gtk_entry_set_placeholder_text(self.entry, placeholder.as_ptr()) };
        visible(self.panel, true);
        // SAFETY: The live entry copies text borrowed from the WebKit view on this thread.
        unsafe { ffi::gtk_editable_set_text(self.entry, text.as_ptr()) };
        // SAFETY: The shown entry is a live focusable widget on the GTK thread.
        unsafe { ffi::gtk_widget_grab_focus(self.entry) };
        if mode == Mode::Address {
            // SAFETY: The live entry selects all text with the caret at the start.
            unsafe { ffi::gtk_editable_select_region(self.entry, -1, 0) };
        }
    }

    pub(super) fn refresh_backdrop(self, browser: Object) {
        crate::gtk::backdrop::refresh(self.picture, browser);
    }

    pub(super) fn fill(self, address: &CStr) {
        // SAFETY: The live GtkEditable copies the accepted completion.
        unsafe { ffi::gtk_editable_set_text(self.entry, address.as_ptr()) };
        // SAFETY: The live entry accepts -1 to move the cursor to the end.
        unsafe { gtk_editable_set_position(self.entry, -1) };
    }

    pub(super) fn hide(self) {
        crate::gtk::backdrop::clear(self.picture);
        visible(self.panel, false);
    }

    pub(super) fn layout(
        self,
        parent: Object,
        mode: Mode,
        allocation: &mut Allocation,
    ) -> (c_int, c_int) {
        // SAFETY: GTK emits layout with this live GtkOverlay.
        let width = unsafe { gtk_widget_get_width(parent) };
        // SAFETY: The same live parent has an assigned height.
        let height = unsafe { gtk_widget_get_height(parent) };
        let (limit, top_percent) = if mode == Mode::Address {
            (640, 20)
        } else {
            (720, 15)
        };
        allocation.width = (width - 24).clamp(1, limit);
        allocation.x = (width - allocation.width) / 2;
        let mut natural = 0;
        // SAFETY: Output pointer is writable; unused output parameters are nullable.
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
        allocation.y = (height.saturating_mul(top_percent) / 100)
            .clamp(12, (height - allocation.height - 12).max(12));
        (width, height)
    }

    pub(super) fn render(self, items: &[Item], mode: Mode, blank_query: bool, can_complete: bool) {
        // SAFETY: The list belongs to the live palette panel.
        unsafe { gtk_list_box_remove_all(self.list) };
        for item in items {
            let row = item_row(item);
            // SAFETY: The live list adopts the new row.
            unsafe { gtk_list_box_append(self.list, row) };
        }
        let is_empty = items.is_empty();
        visible(self.scroll, !is_empty);
        let empty = match mode {
            Mode::Address => c"Type an address or search term",
            Mode::Commands => c"No matching commands",
            Mode::GoTo if blank_query => c"No open tabs or visited pages",
            Mode::GoTo => c"No matching tabs or history",
        };
        // SAFETY: The live label copies the static empty-state text.
        unsafe { ffi::gtk_label_set_text(self.empty, empty.as_ptr()) };
        visible(self.empty, is_empty);
        // SAFETY: This returns the first row of the live list or null if empty.
        let first = unsafe { gtk_list_box_get_row_at_index(self.list, 0) };
        // SAFETY: A null selection is accepted, otherwise the row belongs to this list.
        unsafe { gtk_list_box_select_row(self.list, first) };
        // SAFETY: The live scrolled window owns its adjustment.
        let adjustment = unsafe { gtk_scrolled_window_get_vadjustment(self.scroll) };
        // SAFETY: The live adjustment accepts zero and clamps it to the scrollable range.
        unsafe { gtk_adjustment_set_value(adjustment, 0.0) };
        let footer = if can_complete {
            c"↑ ↓  Select     Tab  Complete     ↵  Open     Esc  Close"
        } else {
            c"↑ ↓  Select     ↵  Open     Esc  Close"
        };
        // SAFETY: The live label copies the static shortcut help.
        unsafe { ffi::gtk_label_set_text(self.footer, footer.as_ptr()) };
    }

    pub(super) fn selected(self) -> Option<usize> {
        // SAFETY: The current row is borrowed from this live GtkListBox.
        index(unsafe { gtk_list_box_get_selected_row(self.list) })
    }

    pub(super) fn step(self, down: bool, count: usize) {
        let Some(last) = count.checked_sub(1) else {
            return;
        };
        let current = self.selected().unwrap_or_default();
        let next = if down {
            current.saturating_add(1).min(last)
        } else {
            current.saturating_sub(1)
        };
        let Ok(next) = c_int::try_from(next) else {
            return;
        };
        // SAFETY: The row, if present, is borrowed from the live list.
        let row = unsafe { gtk_list_box_get_row_at_index(self.list, next) };
        if row.is_null() {
            return;
        }
        // SAFETY: The row belongs to the same live list.
        unsafe { gtk_list_box_select_row(self.list, row) };
        let mut allocation = Allocation {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        // SAFETY: The row is live and reports its allocated rectangle in list coordinates.
        unsafe { gtk_widget_get_allocation(row, &raw mut allocation) };
        // SAFETY: The scroll container owns the adjustment for its live list.
        let adjustment = unsafe { gtk_scrolled_window_get_vadjustment(self.scroll) };
        // SAFETY: The borrowed adjustment can scroll the selected rectangle into view.
        unsafe {
            gtk_adjustment_clamp_page(
                adjustment,
                f64::from(allocation.y),
                f64::from(allocation.y + allocation.height),
            );
        };
    }
}

pub(super) fn index(row: Object) -> Option<usize> {
    if row.is_null() {
        return None;
    }
    // SAFETY: A nonnull row was supplied by a live GtkListBox or its row-activated signal.
    usize::try_from(unsafe { gtk_list_box_row_get_index(row) }).ok()
}

pub(super) fn install_theme(widget: Object) {
    let Ok(css) = CString::new(concat!(
        include_str!("panel.css"),
        "\n",
        include_str!("../find/find.css"),
        "\n",
        include_str!("../browsing/panels/panel.css")
    )) else {
        return;
    };
    // SAFETY: GTK is initialized on this thread. The caller owns the returned provider.
    let provider = unsafe { gtk_css_provider_new() };
    // SAFETY: The live CSS provider copies the NUL-terminated string.
    unsafe { gtk_css_provider_load_from_string(provider, css.as_ptr()) };
    // SAFETY: The display belongs to the live browser overlay.
    let display = unsafe { gtk_widget_get_display(widget) };
    // SAFETY: The live display retains this CSS provider.
    unsafe { gtk_style_context_add_provider_for_display(display, provider, 600) };
    // SAFETY: The display retained the provider, so this releases our constructor reference.
    unsafe { ffi::g_object_unref(provider) };
}

pub(crate) fn panel_widgets(orientation: c_int, spacing: c_int) -> (Object, Object, Object) {
    // SAFETY: GTK is initialized on the main thread.
    let panel = unsafe { ffi::gtk_overlay_new() };
    class(panel, c"quickie-panel");
    // SAFETY: GTK is initialized and the new overlay takes ownership below.
    let base = unsafe { ffi::gtk_box_new(ffi::VERTICAL, 0) };
    // SAFETY: The panel adopts the empty, nonmeasuring base.
    unsafe { ffi::gtk_overlay_set_child(panel, base) };
    // SAFETY: GTK is initialized and the panel takes ownership below.
    let picture = unsafe { gtk_picture_new() };
    class(picture, c"quickie-panel-backdrop");
    // SAFETY: The newly created picture supports shrinking its image allocation.
    unsafe { gtk_picture_set_can_shrink(picture, ffi::TRUE) };
    // SAFETY: The live backdrop never receives input.
    unsafe { gtk_widget_set_can_target(picture, ffi::FALSE) };
    // SAFETY: The panel owns the backdrop as its first overlay child. It is not measured.
    unsafe { ffi::gtk_overlay_add_overlay(panel, picture) };
    // SAFETY: GTK is initialized and the panel takes ownership below.
    let content = unsafe { ffi::gtk_box_new(orientation, spacing) };
    class(content, c"quickie-panel-content");
    // SAFETY: The panel owns the foreground overlay, which paints above the backdrop.
    unsafe { ffi::gtk_overlay_add_overlay(panel, content) };
    // SAFETY: Only the live content child contributes to the panel's natural height.
    unsafe { gtk_overlay_set_measure_overlay(panel, content, ffi::TRUE) };
    (panel, picture, content)
}

fn results_widgets() -> (Object, Object) {
    // SAFETY: GTK is initialized on the main thread.
    let scroll = unsafe { ffi::gtk_scrolled_window_new() };
    // SAFETY: Both policies are valid for this live scrolled window.
    unsafe { gtk_scrolled_window_set_policy(scroll, POLICY_NEVER, POLICY_AUTOMATIC) };
    // SAFETY: The live scrolled window can request the list's full height. The panel
    // allocation clamps that height to the viewport, preserving scrolling on short windows.
    unsafe { gtk_scrolled_window_set_propagate_natural_height(scroll, ffi::TRUE) };
    // SAFETY: GTK is initialized and the scrolled window takes ownership below.
    let list = unsafe { gtk_list_box_new() };
    class(list, c"quickie-panel-list");
    // SAFETY: The list is live and row activation also has a keyboard path.
    unsafe { gtk_list_box_set_activate_on_single_click(list, ffi::TRUE) };
    // SAFETY: The scroll container adopts the new list.
    unsafe { ffi::gtk_scrolled_window_set_child(scroll, list) };
    (scroll, list)
}

fn item_row(item: &Item) -> Object {
    // SAFETY: GTK is initialized on the main thread and the list adopts the row.
    let row = unsafe { gtk_list_box_row_new() };
    class(row, c"quickie-panel-row");
    // SAFETY: GTK is initialized and the row adopts the box below.
    let layout = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 10) };
    // SAFETY: The live row adopts the new box.
    unsafe { gtk_list_box_row_set_child(row, layout) };
    if let Ok(name) = CString::new(item.icon) {
        // SAFETY: GTK copies the NUL-terminated icon name.
        let icon = unsafe { gtk_image_new_from_icon_name(name.as_ptr()) };
        class(icon, c"quickie-panel-icon");
        // SAFETY: The live row layout adopts this icon.
        unsafe { ffi::gtk_box_append(layout, icon) };
    }
    let text = item_text(item);
    // SAFETY: The live row layout adopts its text column.
    unsafe { ffi::gtk_box_append(layout, text) };
    if !item.shortcut.is_empty()
        && let Ok(shortcut) = CString::new(item.shortcut.as_str())
    {
        let key = label(&shortcut, c"quickie-panel-keycap");
        // SAFETY: The live keycap label is centered vertically within the row.
        unsafe { ffi::gtk_widget_set_valign(key, ffi::ALIGN_CENTER) };
        // SAFETY: The live row layout adopts its shortcut label.
        unsafe { ffi::gtk_box_append(layout, key) };
    }
    row
}

fn item_text(item: &Item) -> Object {
    // SAFETY: GTK is initialized and the row will adopt the text column.
    let text = unsafe { ffi::gtk_box_new(ffi::VERTICAL, 0) };
    // SAFETY: The live text box expands between the row icon and keycap.
    unsafe { ffi::gtk_widget_set_hexpand(text, ffi::TRUE) };
    let title = label(c"", c"quickie-panel-title");
    // SAFETY: PANGO_ELLIPSIZE_END is a valid mode for this live title label.
    unsafe { gtk_label_set_ellipsize(title, 3) };
    if let Ok(markup) = CString::new(highlighted(&item.title, &item.emphasis)) {
        // SAFETY: The live label copies the escaped, NUL-terminated markup.
        unsafe { ffi::gtk_label_set_markup(title, markup.as_ptr()) };
    }
    // SAFETY: The text column adopts this live title label.
    unsafe { ffi::gtk_box_append(text, title) };
    if let Ok(detail) = CString::new(item.detail.as_str()) {
        let detail = label(&detail, c"quickie-panel-detail");
        // SAFETY: PANGO_ELLIPSIZE_END is a valid mode for this live detail label.
        unsafe { gtk_label_set_ellipsize(detail, 3) };
        // SAFETY: The text column adopts the live detail label.
        unsafe { ffi::gtk_box_append(text, detail) };
    }
    text
}

fn highlighted(value: &str, emphasis: &[u32]) -> String {
    let mut markup = String::with_capacity(value.len());
    for (position, character) in (0..).zip(value.chars()) {
        let highlighted = emphasis.binary_search(&position).is_ok();
        if highlighted {
            markup.push_str("<u>");
        }
        match character {
            '&' => markup.push_str("&amp;"),
            '<' => markup.push_str("&lt;"),
            '>' => markup.push_str("&gt;"),
            other => markup.push(other),
        }
        if highlighted {
            markup.push_str("</u>");
        }
    }
    markup
}

fn label(text: &CStr, style: &CStr) -> Object {
    // SAFETY: GTK is initialized on this thread and copies the NUL-terminated text.
    let label = unsafe { ffi::gtk_label_new(text.as_ptr()) };
    // SAFETY: This is a live GTK label and zero represents left alignment.
    unsafe { ffi::gtk_label_set_xalign(label, 0.0) };
    class(label, style);
    label
}

fn class(widget: Object, style: &CStr) {
    // SAFETY: Every caller passes its newly constructed widget and a valid CSS class name.
    unsafe { ffi::gtk_widget_add_css_class(widget, style.as_ptr()) };
}

fn visible(widget: Object, visible: bool) {
    // SAFETY: Every caller has a live widget retained by the palette's container hierarchy.
    unsafe { ffi::gtk_widget_set_visible(widget, Bool::from(visible)) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_picture_new() -> Object;
    fn gtk_picture_set_can_shrink(picture: Object, can_shrink: Bool);
    fn gtk_overlay_set_measure_overlay(overlay: Object, widget: Object, measure: Bool);
    fn gtk_event_controller_focus_new() -> Object;
    fn gtk_widget_set_can_target(widget: Object, target: Bool);
    fn gtk_widget_get_width(widget: Object) -> c_int;
    fn gtk_widget_get_height(widget: Object) -> c_int;
    fn gtk_widget_measure(
        widget: Object,
        orientation: c_int,
        for_size: c_int,
        minimum: *mut c_int,
        natural: *mut c_int,
        minimum_baseline: *mut c_int,
        natural_baseline: *mut c_int,
    );
    fn gtk_widget_get_allocation(widget: Object, allocation: *mut Allocation);
    fn gtk_widget_get_display(widget: Object) -> Object;
    fn gtk_css_provider_new() -> Object;
    fn gtk_css_provider_load_from_string(provider: Object, css: *const c_char);
    fn gtk_style_context_add_provider_for_display(
        display: Object,
        provider: Object,
        priority: c_uint,
    );
    fn gtk_scrolled_window_set_policy(scroll: Object, horizontal: c_int, vertical: c_int);
    fn gtk_scrolled_window_set_propagate_natural_height(scroll: Object, propagate: Bool);
    fn gtk_scrolled_window_get_vadjustment(scroll: Object) -> Object;
    fn gtk_adjustment_set_value(adjustment: Object, value: f64);
    fn gtk_adjustment_clamp_page(adjustment: Object, lower: f64, upper: f64);
    fn gtk_list_box_new() -> Object;
    fn gtk_list_box_set_activate_on_single_click(list: Object, activate: Bool);
    fn gtk_list_box_append(list: Object, widget: Object);
    fn gtk_list_box_remove_all(list: Object);
    fn gtk_list_box_get_selected_row(list: Object) -> Object;
    fn gtk_list_box_get_row_at_index(list: Object, index: c_int) -> Object;
    fn gtk_list_box_select_row(list: Object, row: Object);
    fn gtk_list_box_row_new() -> Object;
    fn gtk_list_box_row_set_child(row: Object, child: Object);
    fn gtk_list_box_row_get_index(row: Object) -> c_int;
    fn gtk_label_set_ellipsize(label: Object, mode: c_int);
    fn gtk_image_new_from_icon_name(name: *const c_char) -> Object;
    fn gtk_editable_set_position(entry: Object, position: c_int);
}
