use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::ptr;

use super::{FADE_MS, Kind, Toast};
use crate::gtk::ffi::{self, Bool, Object};

const ALIGN_END: c_int = 2;
const CROSSFADE: c_int = 1;
const ELLIPSIZE_END: c_int = 3;
const APPLICATION_CSS: c_uint = 600;

pub(super) struct View {
    rows: Object,
}

impl View {
    pub(super) fn new(overlay: Object) -> Self {
        install_css(overlay);
        // SAFETY: GTK is initialized and creates the box on its main thread.
        let rows = unsafe { ffi::gtk_box_new(ffi::VERTICAL, 8) };
        center(rows);
        passive(rows);
        // SAFETY: The live container accepts a bottom alignment.
        unsafe { ffi::gtk_widget_set_valign(rows, ALIGN_END) };
        // SAFETY: The live container accepts a margin in logical pixels.
        unsafe { ffi::gtk_widget_set_margin_bottom(rows, 24) };
        // SAFETY: Hold the box even if the window has already destroyed its widget tree.
        unsafe { g_object_ref_sink(rows) };
        // SAFETY: The live overlay takes its own reference to the valid box.
        unsafe { ffi::gtk_overlay_add_overlay(overlay, rows) };
        Self { rows }
    }

    pub(super) fn insert(&self, toast: &Toast<'_>, previous: Option<Object>) -> Object {
        // SAFETY: GTK is initialized and creates the box on its main thread.
        let pill = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 8) };
        class(pill, c"quickie-toast");
        let (kind, icon) = match toast.kind {
            Kind::Info => (c"info", c"dialog-information-symbolic"),
            Kind::Done => (c"done", c"object-select-symbolic"),
            Kind::Success => (c"success", c"security-high-symbolic"),
            Kind::Problem => (c"problem", c"dialog-warning-symbolic"),
        };
        class(pill, kind);
        passive(pill);
        // SAFETY: GTK is initialized and copies the static icon name.
        let image = unsafe { gtk_image_new_from_icon_name(icon.as_ptr()) };
        // SAFETY: The live image uses logical pixels for its symbolic icon.
        unsafe { gtk_image_set_pixel_size(image, 16) };
        class(image, c"quickie-toast-icon");
        append(pill, image);
        append(pill, label(toast.text, c"quickie-toast-title", 35));
        if let Some(detail) = toast.detail {
            append(pill, label(detail, c"quickie-toast-detail", 22));
        }
        if let Some(keys) = toast.keys {
            append(pill, label(keys, c"quickie-panel-keycap", 12));
        }

        // SAFETY: GTK is initialized and creates the revealer on its main thread.
        let widget = unsafe { gtk_revealer_new() };
        center(widget);
        passive(widget);
        // SAFETY: The live revealer accepts this transition type.
        unsafe { gtk_revealer_set_transition_type(widget, CROSSFADE) };
        // SAFETY: The live revealer takes a duration in milliseconds.
        unsafe { gtk_revealer_set_transition_duration(widget, FADE_MS) };
        // SAFETY: The live revealer adopts the new pill.
        unsafe { gtk_revealer_set_child(widget, pill) };
        // SAFETY: The live box adopts the revealer after an optional existing child.
        unsafe {
            gtk_box_insert_child_after(self.rows, widget, previous.unwrap_or(ptr::null_mut()));
        }
        reveal(widget, true);
        widget
    }

    pub(super) fn remove(&self, widget: Object) {
        // SAFETY: The supplied revealer is still an owned child of the live box.
        unsafe { gtk_box_remove(self.rows, widget) };
    }
}

impl Drop for View {
    fn drop(&mut self) {
        // SAFETY: View owns a reference independent of the overlay.
        unsafe { ffi::g_object_unref(self.rows) };
    }
}

pub(super) fn reveal(widget: Object, show: bool) {
    // SAFETY: Callers supply a live revealer still owned by the stack's box.
    unsafe { gtk_revealer_set_reveal_child(widget, Bool::from(show)) };
}

fn label(text: &str, style: &CStr, columns: c_int) -> Object {
    let text = CString::new(text).ok();
    // SAFETY: GTK is initialized and copies either a valid C string or an empty label.
    let label =
        unsafe { ffi::gtk_label_new(text.as_ref().map_or(ptr::null(), |text| text.as_ptr())) };
    // SAFETY: The live label has one line even if its source contains a newline.
    unsafe { gtk_label_set_single_line_mode(label, ffi::TRUE) };
    // SAFETY: Pango's end ellipsis keeps the live label on one line.
    unsafe { gtk_label_set_ellipsize(label, ELLIPSIZE_END) };
    // SAFETY: The label's preferred width is limited in text columns.
    unsafe { gtk_label_set_max_width_chars(label, columns) };
    class(label, style);
    label
}

fn append(pill: Object, child: Object) {
    passive(child);
    // SAFETY: The live pill adopts the newly constructed child.
    unsafe { ffi::gtk_box_append(pill, child) };
}

fn passive(widget: Object) {
    // SAFETY: Each live toast widget leaves pointer targeting to the page below.
    unsafe { gtk_widget_set_can_target(widget, ffi::FALSE) };
}

fn center(widget: Object) {
    // SAFETY: The live widget uses its natural width and centers in its parent.
    unsafe { gtk_widget_set_halign(widget, ffi::ALIGN_CENTER) };
}

fn class(widget: Object, name: &CStr) {
    // SAFETY: The live widget copies the static class name.
    unsafe { ffi::gtk_widget_add_css_class(widget, name.as_ptr()) };
}

fn install_css(overlay: Object) {
    let Ok(css) = CString::new(include_str!("toasts.css")) else {
        return;
    };
    // SAFETY: GTK is initialized on the browser thread.
    let provider = unsafe { gtk_css_provider_new() };
    // SAFETY: The live provider copies the CSS string.
    unsafe { gtk_css_provider_load_from_string(provider, css.as_ptr()) };
    // SAFETY: The live overlay belongs to the browser's display.
    let display = unsafe { gtk_widget_get_display(overlay) };
    // SAFETY: The display retains the CSS provider for the window.
    unsafe { gtk_style_context_add_provider_for_display(display, provider, APPLICATION_CSS) };
    // SAFETY: Release the constructor's reference now that the display retains it.
    unsafe { ffi::g_object_unref(provider) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_revealer_new() -> Object;
    fn gtk_revealer_set_transition_type(widget: Object, kind: c_int);
    fn gtk_revealer_set_transition_duration(widget: Object, duration: c_uint);
    fn gtk_revealer_set_child(widget: Object, child: Object);
    fn gtk_revealer_set_reveal_child(widget: Object, reveal: Bool);
    fn gtk_box_insert_child_after(parent: Object, child: Object, sibling: Object);
    fn gtk_box_remove(parent: Object, child: Object);
    fn gtk_widget_set_can_target(widget: Object, target: Bool);
    fn gtk_widget_set_halign(widget: Object, alignment: c_int);
    fn gtk_image_new_from_icon_name(icon: *const c_char) -> Object;
    fn gtk_image_set_pixel_size(image: Object, pixels: c_int);
    fn gtk_label_set_single_line_mode(label: Object, enabled: Bool);
    fn gtk_label_set_ellipsize(label: Object, mode: c_int);
    fn gtk_label_set_max_width_chars(label: Object, columns: c_int);
    fn gtk_widget_get_display(widget: Object) -> Object;
    fn gtk_css_provider_new() -> Object;
    fn gtk_css_provider_load_from_string(provider: Object, css: *const c_char);
    fn gtk_style_context_add_provider_for_display(
        display: Object,
        provider: Object,
        priority: c_uint,
    );
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_ref_sink(object: Object) -> Object;
}
