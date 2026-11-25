//! GTK controls over the in-window player's live video surface.

use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::ptr;

use super::PlayerPin;
use crate::gtk::ffi::{self, Bool, Object};

const ALIGN_START: c_int = 1;
const ALIGN_END: c_int = 2;
const TEXT_DIR_LTR: c_int = 1;
const OVERFLOW_HIDDEN: c_int = 1;
const WIDTH: c_int = 340;
const HEIGHT: c_int = 192;
const MARGIN: c_int = 16;

#[derive(Clone, Copy)]
pub(super) struct View {
    root: Object,
    stage: Object,
    pub(super) play: Object,
    pub(super) back: Object,
    pub(super) position: Object,
    pub(super) minimize: Object,
    pub(super) close: Object,
}

impl View {
    pub(super) fn new(overlay: Object) -> Self {
        install_style(overlay);
        // SAFETY: GTK is initialized on this thread.
        let root = unsafe { ffi::gtk_box_new(ffi::VERTICAL, 0) };
        // SAFETY: The player adopts the floating reference and releases it at shutdown.
        unsafe { g_object_ref_sink(root) };
        // SAFETY: GTK copies the static class name onto the live widget.
        unsafe { ffi::gtk_widget_add_css_class(root, c"quickie-mini-player".as_ptr()) };
        // SAFETY: The live root clips its children at its outer bounds.
        unsafe { gtk_widget_set_overflow(root, OVERFLOW_HIDDEN) };
        // SAFETY: The live root uses physical left and right for its position and controls.
        unsafe { gtk_widget_set_direction(root, TEXT_DIR_LTR) };
        // SAFETY: The live root accepts this positive left margin.
        unsafe { ffi::gtk_widget_set_margin_start(root, MARGIN) };
        // SAFETY: The live root accepts this positive right margin.
        unsafe { ffi::gtk_widget_set_margin_end(root, MARGIN) };
        // SAFETY: The live root accepts this positive bottom margin.
        unsafe { ffi::gtk_widget_set_margin_bottom(root, MARGIN) };
        // SAFETY: The live root accepts this positive top margin.
        unsafe { ffi::gtk_widget_set_margin_top(root, MARGIN) };

        // SAFETY: GTK is initialized on this thread.
        let stage = unsafe { ffi::gtk_overlay_new() };
        // SAFETY: The live stage accepts this positive size request.
        unsafe { gtk_widget_set_size_request(stage, WIDTH, HEIGHT) };
        // SAFETY: GTK copies the static class name onto the live stage.
        unsafe { ffi::gtk_widget_add_css_class(stage, c"quickie-mini-video".as_ptr()) };
        // SAFETY: The live root takes ownership of the new stage.
        unsafe { ffi::gtk_box_append(root, stage) };
        // SAFETY: GTK is initialized on this thread.
        let controls = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 0) };
        // SAFETY: The live controls box gives each button the same space.
        unsafe { gtk_box_set_homogeneous(controls, ffi::TRUE) };
        // SAFETY: The live root takes ownership of the new controls box.
        unsafe { ffi::gtk_box_append(root, controls) };

        let play = button(controls, c"Pause");
        let back = button(controls, c"Back to tab");
        let position = button(controls, c"Move up");
        let minimize = button(controls, c"Minimize");
        let close = button(controls, c"Close");
        // SAFETY: The live root stays hidden until a video starts floating.
        unsafe { ffi::gtk_widget_set_visible(root, ffi::FALSE) };
        // SAFETY: The live overlay retains the new root.
        unsafe { ffi::gtk_overlay_add_overlay(overlay, root) };

        Self {
            root,
            stage,
            play,
            back,
            position,
            minimize,
            close,
        }
    }

    pub(super) fn show(self, page: Object) {
        // SAFETY: The live page leaves input with the native controls while floating.
        unsafe { gtk_widget_set_can_target(page, ffi::FALSE) };
        // SAFETY: The live stage retains the detached page.
        unsafe { ffi::gtk_overlay_set_child(self.stage, page) };
        // SAFETY: The live root now owns the stage and its page.
        unsafe { ffi::gtk_widget_set_visible(self.root, ffi::TRUE) };
    }

    pub(super) fn hide(self, page: Object) {
        // SAFETY: The engine or its pending script owns the live page, now interactive again.
        unsafe { gtk_widget_set_can_target(page, ffi::TRUE) };
        // SAFETY: The root is live when the player is dismissed.
        unsafe { ffi::gtk_widget_set_visible(self.root, ffi::FALSE) };
        // SAFETY: Passing null releases the live stage's optional page.
        unsafe { ffi::gtk_overlay_set_child(self.stage, ptr::null_mut()) };
    }

    pub(super) fn set_playing(self, playing: bool) {
        let label = if playing { c"Pause" } else { c"Play" };
        // SAFETY: The live native button copies its label.
        unsafe { ffi::gtk_button_set_label(self.play, label.as_ptr()) };
    }

    pub(super) fn set_pin(self, pin: PlayerPin) {
        let (horizontal, vertical, label) = match pin {
            PlayerPin::TopLeft => (ALIGN_START, ALIGN_START, c"Move down"),
            PlayerPin::TopRight => (ALIGN_END, ALIGN_START, c"Move left"),
            PlayerPin::BottomLeft => (ALIGN_START, ALIGN_END, c"Move right"),
            PlayerPin::BottomRight => (ALIGN_END, ALIGN_END, c"Move up"),
        };
        // SAFETY: The live overlay child can change its horizontal alignment.
        unsafe { gtk_widget_set_halign(self.root, horizontal) };
        // SAFETY: The live overlay child can change its vertical alignment.
        unsafe { ffi::gtk_widget_set_valign(self.root, vertical) };
        // SAFETY: The live button copies the label for the next move.
        unsafe { ffi::gtk_button_set_label(self.position, label.as_ptr()) };
    }

    pub(super) fn release(self) {
        // SAFETY: This releases our single retained root reference after the loop.
        unsafe { ffi::g_object_unref(self.root) };
    }
}

fn button(row: Object, label: &CStr) -> Object {
    // SAFETY: GTK is initialized and the new button copies the label.
    let button = unsafe { ffi::gtk_button_new_with_label(label.as_ptr()) };
    // SAFETY: The live button copies the static class name.
    unsafe { ffi::gtk_widget_add_css_class(button, c"flat".as_ptr()) };
    // SAFETY: The live row takes ownership of the new button.
    unsafe { ffi::gtk_box_append(row, button) };
    button
}

fn install_style(overlay: Object) {
    let Ok(css) = CString::new(include_str!("player.css")) else {
        return;
    };
    // SAFETY: GTK is initialized on this thread and returns an owned provider.
    let provider = unsafe { gtk_css_provider_new() };
    // SAFETY: The live CSS provider copies the NUL-terminated CSS.
    unsafe { gtk_css_provider_load_from_string(provider, css.as_ptr()) };
    // SAFETY: The live overlay supplies its display on the GTK thread.
    let display = unsafe { gtk_widget_get_display(overlay) };
    // SAFETY: The live display retains the configured provider.
    unsafe { gtk_style_context_add_provider_for_display(display, provider, 600) };
    // SAFETY: This releases the constructor's reference.
    unsafe { ffi::g_object_unref(provider) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_widget_set_halign(widget: Object, align: c_int);
    fn gtk_widget_set_direction(widget: Object, direction: c_int);
    fn gtk_widget_set_size_request(widget: Object, width: c_int, height: c_int);
    fn gtk_widget_set_overflow(widget: Object, overflow: c_int);
    fn gtk_widget_set_can_target(widget: Object, can_target: Bool);
    fn gtk_box_set_homogeneous(container: Object, homogeneous: Bool);
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
