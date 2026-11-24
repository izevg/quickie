//! The GTK4, GIO and `WebKitGTK` 6.0 calls used by the Linux shell.

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

pub type Object = *mut c_void;
pub type Bool = c_int;

pub const TRUE: Bool = 1;
pub const FALSE: Bool = 0;
pub const NON_UNIQUE: c_uint = 1 << 5;
pub const ALIGN_CENTER: c_int = 3;
pub const PHASE_CAPTURE: c_int = 1;
pub const SHIFT: c_uint = 1;
pub const CONTROL: c_uint = 1 << 2;
pub const ALT: c_uint = 1 << 3;
pub const SUPER: c_uint = 1 << 26;
pub const HYPER: c_uint = 1 << 27;
pub const META: c_uint = 1 << 28;
pub const KEY_ESCAPE: c_uint = 0xff1b;
pub const KEY_F12: c_uint = 0xffc9;
pub const KEY_LEFT: c_uint = 0xff51;
pub const KEY_RIGHT: c_uint = 0xff53;
pub const KEY_COMMA: c_uint = 0x2c;
pub const HORIZONTAL: c_int = 0;
pub const VERTICAL: c_int = 1;

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
    pub fn gtk_window_new() -> Object;
    pub fn gtk_window_close(window: Object);
    pub fn gtk_window_set_transient_for(window: Object, parent: Object);
    pub fn gtk_window_set_modal(window: Object, modal: Bool);
    pub fn gtk_window_set_hide_on_close(window: Object, setting: Bool);
    pub fn gtk_window_set_destroy_with_parent(window: Object, setting: Bool);
    pub fn gtk_alert_dialog_new(format: *const c_char, ...) -> Object;
    pub fn gtk_alert_dialog_set_detail(dialog: Object, detail: *const c_char);
    pub fn gtk_alert_dialog_choose(
        dialog: Object,
        parent: Object,
        cancellable: Object,
        callback: extern "C" fn(Object, Object, Object),
        data: Object,
    );
    pub fn gtk_alert_dialog_choose_finish(
        dialog: Object,
        result: Object,
        error: *mut *mut GError,
    ) -> c_int;
    pub fn gtk_box_new(orientation: c_int, spacing: c_int) -> Object;
    pub fn gtk_box_append(container: Object, child: Object);
    pub fn gtk_label_new(text: *const c_char) -> Object;
    pub fn gtk_label_set_text(label: Object, text: *const c_char);
    pub fn gtk_label_set_markup(label: Object, text: *const c_char);
    pub fn gtk_label_set_wrap(label: Object, wrap: Bool);
    pub fn gtk_label_set_xalign(label: Object, alignment: f32);
    pub fn gtk_label_set_selectable(label: Object, selectable: Bool);
    pub fn gtk_button_new_with_label(label: *const c_char) -> Object;
    pub fn gtk_button_set_label(button: Object, label: *const c_char);
    pub fn gtk_switch_new() -> Object;
    pub fn gtk_switch_get_active(toggle: Object) -> Bool;
    pub fn gtk_switch_set_active(toggle: Object, active: Bool);
    pub fn gtk_expander_new(label: *const c_char) -> Object;
    pub fn gtk_expander_set_child(expander: Object, child: Object);
    pub fn gtk_scrolled_window_new() -> Object;
    pub fn gtk_scrolled_window_set_child(scroll: Object, child: Object);
    pub fn gtk_scrolled_window_set_min_content_height(scroll: Object, height: c_int);
    pub fn gtk_link_button_new_with_label(uri: *const c_char, label: *const c_char) -> Object;
    pub fn gtk_link_button_set_uri(button: Object, uri: *const c_char);
    pub fn gtk_overlay_new() -> Object;
    pub fn gtk_overlay_set_child(overlay: Object, child: Object);
    pub fn gtk_overlay_add_overlay(overlay: Object, child: Object);
    pub fn gtk_entry_new() -> Object;
    pub fn gtk_entry_set_placeholder_text(entry: Object, placeholder: *const c_char);
    pub fn gtk_editable_set_text(editable: Object, text: *const c_char);
    pub fn gtk_editable_get_text(editable: Object) -> *const c_char;
    pub fn gtk_editable_set_width_chars(editable: Object, chars: c_int);
    pub fn gtk_editable_select_region(editable: Object, start: c_int, end: c_int);
    pub fn gtk_widget_set_valign(widget: Object, align: c_int);
    pub fn gtk_widget_set_margin_top(widget: Object, margin: c_int);
    pub fn gtk_widget_set_margin_bottom(widget: Object, margin: c_int);
    pub fn gtk_widget_set_margin_start(widget: Object, margin: c_int);
    pub fn gtk_widget_set_margin_end(widget: Object, margin: c_int);
    pub fn gtk_widget_set_hexpand(widget: Object, expand: Bool);
    pub fn gtk_widget_set_sensitive(widget: Object, sensitive: Bool);
    pub fn gtk_widget_add_css_class(widget: Object, class: *const c_char);
    pub fn gtk_widget_set_visible(widget: Object, visible: Bool);
    pub fn gtk_widget_get_root(widget: Object) -> Object;
    pub fn gtk_widget_grab_focus(widget: Object) -> Bool;
    pub fn gtk_event_controller_key_new() -> Object;
    pub fn gtk_event_controller_set_propagation_phase(controller: Object, phase: c_int);
    pub fn gtk_widget_add_controller(widget: Object, controller: Object);
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    pub fn g_main_loop_new(context: Object, is_running: Bool) -> Object;
    pub fn g_main_loop_run(main_loop: Object);
    pub fn g_main_loop_quit(main_loop: Object);
    pub fn g_main_loop_unref(main_loop: Object);
    pub fn g_timeout_add(
        interval: c_uint,
        callback: extern "C" fn(Object) -> Bool,
        data: Object,
    ) -> c_uint;
    pub fn g_timeout_add_seconds(
        interval: c_uint,
        callback: extern "C" fn(Object) -> Bool,
        data: Object,
    ) -> c_uint;
    pub fn g_source_remove(source: c_uint) -> Bool;
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
    pub fn g_signal_connect_object(
        instance: Object,
        detailed_signal: *const c_char,
        handler: *const c_void,
        object: Object,
        flags: c_uint,
    ) -> c_ulong;
    pub fn g_object_get_data(object: Object, key: *const c_char) -> Object;
    pub fn g_object_set_data(object: Object, key: *const c_char, data: Object);
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
    pub fn webkit_web_inspector_get_web_view(inspector: Object) -> Object;
    pub fn webkit_web_inspector_is_attached(inspector: Object) -> Bool;
    pub fn webkit_web_inspector_show(inspector: Object);
    pub fn webkit_web_inspector_close(inspector: Object);
    pub fn webkit_web_view_load_uri(view: Object, uri: *const c_char);
    pub fn webkit_web_view_get_uri(view: Object) -> *const c_char;
    pub fn webkit_web_view_get_title(view: Object) -> *const c_char;
    pub fn webkit_web_view_go_back(view: Object);
    pub fn webkit_web_view_go_forward(view: Object);
    pub fn webkit_web_view_reload(view: Object);
}
