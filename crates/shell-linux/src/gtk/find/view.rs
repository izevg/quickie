//! The compact, top-right E2 find surface.

use std::ffi::{CStr, CString, c_char, c_int};

use super::super::{backdrop, ffi, palette};
use super::state::{Count, MATCH_LIMIT};
use ffi::{Bool, Object};

const WIDTH: c_int = 380;
const HEIGHT: c_int = 44;
const TOP: c_int = 16;
const SIDE: c_int = 24;
const ACCESSIBLE_LABEL: c_int = 4;

#[derive(Clone, Copy)]
pub(in crate::gtk) struct View {
    pub(super) controller: Object,
    pub(super) panel: Object,
    pub(super) entry: Object,
    pub(super) previous: Object,
    pub(super) next: Object,
    pub(super) close: Object,
    picture: Object,
    count: Object,
}

impl View {
    pub(super) fn new(parent: Object, controller: Object) -> Self {
        let (panel, picture, content) = palette::panel_widgets(ffi::HORIZONTAL, 5);
        class(panel, c"quickie-find");
        // SAFETY: GTK is initialized and the box adopts the image below.
        let search = unsafe { gtk_image_new_from_icon_name(c"edit-find-symbolic".as_ptr()) };
        class(search, c"quickie-find-icon");
        // SAFETY: The live foreground box takes the newly created image.
        unsafe { ffi::gtk_box_append(content, search) };
        // SAFETY: GTK is initialized and the entry is adopted below by content.
        let entry = unsafe { ffi::gtk_entry_new() };
        class(entry, c"quickie-panel-input");
        // SAFETY: The live entry copies its placeholder.
        unsafe { ffi::gtk_entry_set_placeholder_text(entry, c"Find in page".as_ptr()) };
        // SAFETY: The live entry implements GtkEditable and can shrink with the window.
        unsafe { ffi::gtk_editable_set_width_chars(entry, 1) };
        // SAFETY: The entry accepts extra width in the live horizontal box.
        unsafe { ffi::gtk_widget_set_hexpand(entry, ffi::TRUE) };
        accessible_label(entry, c"Find in page");
        // SAFETY: The live foreground box adopts the entry.
        unsafe { ffi::gtk_box_append(content, entry) };

        // SAFETY: GTK is initialized and content adopts the label below.
        let count = unsafe { ffi::gtk_label_new(c"".as_ptr()) };
        class(count, c"quickie-find-count");
        // SAFETY: The live label aligns its count to the right.
        unsafe { ffi::gtk_label_set_xalign(count, 1.0) };
        // SAFETY: The live foreground box adopts the label.
        unsafe { ffi::gtk_box_append(content, count) };
        let previous = button(content, c"go-up-symbolic", c"Previous match");
        let next = button(content, c"go-down-symbolic", c"Next match");
        let close = button(content, c"window-close-symbolic", c"Close find");

        // SAFETY: Hide the live panel before adding it to the browser overlay.
        unsafe { ffi::gtk_widget_set_visible(panel, ffi::FALSE) };
        // SAFETY: The live browser overlay owns the panel.
        unsafe { ffi::gtk_overlay_add_overlay(parent, panel) };
        Self {
            controller,
            panel,
            entry,
            previous,
            next,
            close,
            picture,
            count,
        }
    }

    pub(super) fn layout(parent: Object, allocation: &mut palette::Allocation) -> (c_int, c_int) {
        // SAFETY: GTK is laying out the browser's live overlay.
        let width = unsafe { gtk_widget_get_width(parent) };
        // SAFETY: GTK is laying out the browser's live overlay.
        let height = unsafe { gtk_widget_get_height(parent) };
        let margin = SIDE.min(width / 8);
        allocation.width = (width - margin * 2).clamp(1, WIDTH);
        allocation.height = height.clamp(1, HEIGHT);
        allocation.x = (width - margin - allocation.width).max(0);
        allocation.y = TOP.min((height - allocation.height).max(0));
        (width, height)
    }

    pub(super) fn show(self) {
        // SAFETY: The live panel stays attached to the browser overlay.
        unsafe { ffi::gtk_widget_set_visible(self.panel, ffi::TRUE) };
        // SAFETY: The entry is a focusable child of the now-visible panel.
        unsafe { ffi::gtk_widget_grab_focus(self.entry) };
        // SAFETY: The live entry implements GtkEditable and accepts -1 for the end.
        unsafe { ffi::gtk_editable_select_region(self.entry, 0, -1) };
    }

    pub(super) fn hide(self) {
        // SAFETY: The live panel stays attached to the browser overlay.
        unsafe { ffi::gtk_widget_set_visible(self.panel, ffi::FALSE) };
        backdrop::clear(self.picture);
    }

    pub(super) fn refresh_backdrop(self, web_view: Object) {
        backdrop::refresh(self.picture, web_view);
    }

    pub(super) fn set_count(self, count: Option<Count>) {
        let label = match count {
            Some(Count::Known(matches)) => matches.to_string(),
            Some(Count::Overflow) => format!("{MATCH_LIMIT}+ matches"),
            None => String::new(),
        };
        let label = CString::new(label).unwrap_or_default();
        // SAFETY: The live label copies the prepared text.
        unsafe { ffi::gtk_label_set_text(self.count, label.as_ptr()) };
        accessible_label(self.count, &label);
        let has_matches = count.is_some_and(Count::can_step);
        let no_matches = count.is_some() && !has_matches;
        if no_matches {
            class(self.count, c"quickie-find-missing");
        } else {
            // SAFETY: The count label remains a live child of the foreground box.
            unsafe { gtk_widget_remove_css_class(self.count, c"quickie-find-missing".as_ptr()) };
        }
        for button in [self.previous, self.next] {
            // SAFETY: Both buttons stay live as children of the find panel.
            unsafe { ffi::gtk_widget_set_sensitive(button, Bool::from(has_matches)) };
        }
    }
}

fn class(widget: Object, style: &CStr) {
    // SAFETY: All callers pass a live GTK widget and a static CSS class.
    unsafe { ffi::gtk_widget_add_css_class(widget, style.as_ptr()) };
}

fn button(content: Object, icon: &CStr, description: &CStr) -> Object {
    // SAFETY: GTK is initialized, and the function copies the static icon name.
    let button = unsafe { gtk_button_new_from_icon_name(icon.as_ptr()) };
    class(button, c"quickie-find-button");
    // SAFETY: The live button copies the static description.
    unsafe { gtk_widget_set_tooltip_text(button, description.as_ptr()) };
    accessible_label(button, description);
    // SAFETY: The live content box adopts the new button.
    unsafe { ffi::gtk_box_append(content, button) };
    button
}

fn accessible_label(widget: Object, label: &CStr) {
    // SAFETY: GTK widgets implement GtkAccessible. LABEL takes a string, and -1
    // terminates the property list.
    unsafe { gtk_accessible_update_property(widget, ACCESSIBLE_LABEL, label.as_ptr(), -1_i32) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_accessible_update_property(accessible: Object, property: c_int, ...);
    fn gtk_button_new_from_icon_name(name: *const c_char) -> Object;
    fn gtk_image_new_from_icon_name(name: *const c_char) -> Object;
    fn gtk_widget_get_width(widget: Object) -> c_int;
    fn gtk_widget_get_height(widget: Object) -> c_int;
    fn gtk_widget_remove_css_class(widget: Object, class: *const c_char);
    fn gtk_widget_set_tooltip_text(widget: Object, text: *const c_char);
}
