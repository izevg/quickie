//! The GTK4, GIO and `WebKitGTK` 6.0 calls used by the Linux shell.

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

pub type Object = *mut c_void;
pub type Bool = c_int;

pub const TRUE: Bool = 1;
pub const FALSE: Bool = 0;
pub const NON_UNIQUE: c_uint = 1 << 5;
pub const ALIGN_START: c_int = 1;
pub const ALIGN_CENTER: c_int = 3;
pub const PHASE_CAPTURE: c_int = 1;
pub const SHIFT: c_uint = 1;
pub const CONTROL: c_uint = 1 << 2;
pub const ALT: c_uint = 1 << 3;
pub const KEY_ESCAPE: c_uint = 0xff1b;
pub const KEY_LEFT: c_uint = 0xff51;
pub const KEY_RIGHT: c_uint = 0xff53;

#[repr(C)]
pub struct GError {
    pub domain: c_uint,
    pub code: c_int,
    pub message: *mut c_char,
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    pub fn gtk_application_new(id: *const c_char, flags: c_uint) -> Object;
    pub fn gtk_application_window_new(application: Object) -> Object;
    pub fn gtk_window_set_title(window: Object, title: *const c_char);
    pub fn gtk_window_set_default_size(window: Object, width: c_int, height: c_int);
    pub fn gtk_window_set_child(window: Object, child: Object);
    pub fn gtk_window_present(window: Object);
    pub fn gtk_overlay_new() -> Object;
    pub fn gtk_overlay_set_child(overlay: Object, child: Object);
    pub fn gtk_overlay_add_overlay(overlay: Object, child: Object);
    pub fn gtk_entry_new() -> Object;
    pub fn gtk_entry_set_placeholder_text(entry: Object, placeholder: *const c_char);
    pub fn gtk_editable_set_text(editable: Object, text: *const c_char);
    pub fn gtk_editable_get_text(editable: Object) -> *const c_char;
    pub fn gtk_editable_set_width_chars(editable: Object, chars: c_int);
    pub fn gtk_editable_select_region(editable: Object, start: c_int, end: c_int);
    pub fn gtk_widget_set_halign(widget: Object, align: c_int);
    pub fn gtk_widget_set_valign(widget: Object, align: c_int);
    pub fn gtk_widget_set_margin_top(widget: Object, margin: c_int);
    pub fn gtk_widget_set_visible(widget: Object, visible: Bool);
    pub fn gtk_widget_get_visible(widget: Object) -> Bool;
    pub fn gtk_widget_grab_focus(widget: Object) -> Bool;
    pub fn gtk_event_controller_key_new() -> Object;
    pub fn gtk_event_controller_set_propagation_phase(controller: Object, phase: c_int);
    pub fn gtk_widget_add_controller(widget: Object, controller: Object);
}

#[link(name = "gio-2.0")]
unsafe extern "C" {
    pub fn g_application_run(app: Object, argc: c_int, argv: *mut *mut c_char) -> c_int;
    pub fn g_application_quit(app: Object);
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    pub fn g_signal_connect_data(
        instance: Object,
        detailed_signal: *const c_char,
        handler: *const c_void,
        data: Object,
        destroy_data: Option<unsafe extern "C" fn(Object, Object)>,
        flags: c_uint,
    ) -> c_ulong;
    pub fn g_object_unref(object: Object);
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    pub fn webkit_web_context_get_default() -> Object;
    pub fn webkit_web_context_add_path_to_sandbox(
        context: Object,
        path: *const c_char,
        read_only: Bool,
    );
    pub fn webkit_web_view_new() -> Object;
    pub fn webkit_web_view_get_settings(view: Object) -> Object;
    pub fn webkit_settings_set_enable_developer_extras(settings: Object, enabled: Bool);
    pub fn webkit_web_view_get_inspector(view: Object) -> Object;
    pub fn webkit_web_inspector_show(inspector: Object);
    pub fn webkit_web_view_load_uri(view: Object, uri: *const c_char);
    pub fn webkit_web_view_get_uri(view: Object) -> *const c_char;
    pub fn webkit_web_view_get_title(view: Object) -> *const c_char;
    pub fn webkit_web_view_go_back(view: Object);
    pub fn webkit_web_view_go_forward(view: Object);
    pub fn webkit_web_view_reload(view: Object);
}
