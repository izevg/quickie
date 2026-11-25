use std::ffi::{c_char, c_double, c_float, c_uint};

use super::super::ffi::{Bool, Object};

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub(super) struct Point {
    pub x: c_float,
    pub y: c_float,
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub(super) struct Size {
    pub width: c_float,
    pub height: c_float,
}

#[derive(Default)]
#[repr(C)]
pub(super) struct Rect {
    pub origin: Point,
    pub size: Size,
}

#[repr(C)]
pub(super) struct RoundedRect {
    pub bounds: Rect,
    pub corners: [Size; 4],
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    pub(super) fn gtk_widget_add_tick_callback(
        widget: Object,
        callback: extern "C" fn(Object, Object, Object) -> Bool,
        data: Object,
        destroy: extern "C" fn(Object),
    ) -> c_uint;
    pub(super) fn gtk_widget_remove_tick_callback(widget: Object, id: c_uint);
    pub(super) fn gtk_widget_compute_bounds(
        widget: Object,
        target: Object,
        bounds: *mut Rect,
    ) -> Bool;
    pub(super) fn gtk_widget_paintable_new(widget: Object) -> Object;
    pub(super) fn gdk_paintable_snapshot(
        paintable: Object,
        snapshot: Object,
        width: c_double,
        height: c_double,
    );
    pub(super) fn gtk_picture_set_paintable(picture: Object, paintable: Object);
    pub(super) fn gtk_snapshot_new() -> Object;
    pub(super) fn gtk_snapshot_translate(snapshot: Object, offset: *const Point);
    pub(super) fn gtk_snapshot_push_clip(snapshot: Object, bounds: *const Rect);
    pub(super) fn gtk_snapshot_push_rounded_clip(snapshot: Object, bounds: *const RoundedRect);
    pub(super) fn gtk_snapshot_push_blur(snapshot: Object, radius: c_double);
    pub(super) fn gtk_snapshot_pop(snapshot: Object);
    pub(super) fn gtk_snapshot_free_to_paintable(snapshot: Object, size: *const Size) -> Object;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    pub(super) fn g_object_ref(object: Object) -> Object;
    pub(super) fn g_object_get_data(object: Object, key: *const c_char) -> Object;
    pub(super) fn g_object_set_data(object: Object, key: *const c_char, data: Object);
}
