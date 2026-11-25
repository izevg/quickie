//! Captures the browser paint behind an open panel and blurs only that area.

mod ffi;

use std::ffi::{CStr, c_uint};
use std::ptr;

use super::ffi::{Bool, FALSE, Object, TRUE, g_object_unref};
use ffi::{Point, Rect, RoundedRect, Size};

const CAPTURE: &CStr = c"quickie-panel-backdrop-capture";
const BLUR_RADIUS: f32 = 28.0;
const CORNER_RADIUS: f32 = 22.0;
const MAX_TICKS: u8 = 4;

struct Capture {
    view: Object,
    tick_id: c_uint,
    ticks_left: u8,
}

pub(super) fn refresh(picture: Object, view: Object) {
    cancel_capture(picture);
    // SAFETY: The browser view is live on the GTK thread. Its extra reference belongs to Capture.
    let view = unsafe { ffi::g_object_ref(view) };
    let capture = Box::into_raw(Box::new(Capture {
        view,
        tick_id: 0,
        ticks_left: MAX_TICKS,
    }));
    // SAFETY: This is the live backdrop picture. GTK owns capture until the tick is removed.
    let id = unsafe {
        ffi::gtk_widget_add_tick_callback(picture, capture_tick, capture.cast(), drop_capture)
    };
    // SAFETY: GTK's main thread cannot run the new tick before this function returns.
    unsafe { (*capture).tick_id = id };
    // SAFETY: Store a borrowed pointer so this picture can cancel its owned tick.
    unsafe { ffi::g_object_set_data(picture, CAPTURE.as_ptr(), capture.cast()) };
}

pub(super) fn clear(picture: Object) {
    cancel_capture(picture);
    // SAFETY: The live backdrop picture accepts NULL to release its previous page paintable.
    unsafe { ffi::gtk_picture_set_paintable(picture, ptr::null_mut()) };
}

fn cancel_capture(picture: Object) {
    // SAFETY: The live picture's data is either absent or the pending tick's owned Capture.
    let capture = unsafe { ffi::g_object_get_data(picture, CAPTURE.as_ptr()).cast::<Capture>() };
    if !capture.is_null() {
        // SAFETY: The pending Capture belongs to a tick on this picture.
        let id = unsafe { (*capture).tick_id };
        // SAFETY: Clear the borrowed pointer before GTK destroys the tick's Capture.
        unsafe { ffi::g_object_set_data(picture, CAPTURE.as_ptr(), ptr::null_mut()) };
        // SAFETY: This id denotes the registered tick on the live picture.
        unsafe { ffi::gtk_widget_remove_tick_callback(picture, id) };
    }
}

extern "C" fn capture_tick(picture: Object, _: Object, capture: Object) -> Bool {
    // SAFETY: GTK passes the live Capture owned by this registered tick.
    let capture = unsafe { &mut *capture.cast::<Capture>() };
    capture.ticks_left -= 1;
    if capture.ticks_left == MAX_TICKS - 1 {
        // Let GTK allocate the visible panel before finding the area behind it.
        return TRUE;
    }
    let paint = paintable(picture, capture.view);
    if paint.is_null() && capture.ticks_left > 0 {
        return TRUE;
    }
    // SAFETY: This tick's Capture is destroyed after returning FALSE.
    unsafe { ffi::g_object_set_data(picture, CAPTURE.as_ptr(), ptr::null_mut()) };
    // SAFETY: The picture remains live during its tick and retains paint on assignment.
    unsafe { ffi::gtk_picture_set_paintable(picture, paint) };
    if !paint.is_null() {
        // SAFETY: Drop the caller's reference to the paint now retained by the picture.
        unsafe { g_object_unref(paint) };
    }
    FALSE
}

extern "C" fn drop_capture(capture: Object) {
    // SAFETY: GTK calls this exactly once for a tick, reclaiming the box and view reference.
    let capture = unsafe { Box::from_raw(capture.cast::<Capture>()) };
    // SAFETY: The capture owns this reference to the browser view.
    unsafe { g_object_unref(capture.view) };
}

fn paintable(picture: Object, view: Object) -> Object {
    let Some(region) = bounds(picture, view) else {
        return ptr::null_mut();
    };
    let Some(page) = bounds(view, view) else {
        return ptr::null_mut();
    };
    let corner = CORNER_RADIUS.min(region.size.width.min(region.size.height) * 0.5);
    let clip = RoundedRect {
        bounds: Rect {
            origin: Point::default(),
            size: region.size,
        },
        corners: [Size {
            width: corner,
            height: corner,
        }; 4],
    };
    // The blur needs neighboring pixels. Keep its input within that distance of the panel.
    let margin = BLUR_RADIUS * 3.0;
    let sample = Rect {
        origin: Point {
            x: -margin,
            y: -margin,
        },
        size: Size {
            width: region.size.width + 2.0 * margin,
            height: region.size.height + 2.0 * margin,
        },
    };
    let offset = Point {
        x: -region.origin.x,
        y: -region.origin.y,
    };

    // SAFETY: The WebKit view is live and retained by Capture on the GTK thread.
    let source = unsafe { ffi::gtk_widget_paintable_new(view) };
    // SAFETY: Construct the new snapshot on the GTK thread.
    let snapshot = unsafe { ffi::gtk_snapshot_new() };
    // SAFETY: The rounded Graphene clip has positive dimensions and remains in scope.
    unsafe { ffi::gtk_snapshot_push_rounded_clip(snapshot, &raw const clip) };
    // SAFETY: Record the positive blur radius on the live snapshot.
    unsafe { ffi::gtk_snapshot_push_blur(snapshot, f64::from(BLUR_RADIUS)) };
    // SAFETY: The Graphene sample bounds remain in scope while GTK copies them.
    unsafe { ffi::gtk_snapshot_push_clip(snapshot, &raw const sample) };
    // SAFETY: The Graphene offset is valid and GTK copies it for this transform.
    unsafe { ffi::gtk_snapshot_translate(snapshot, &raw const offset) };
    // SAFETY: The paintable and snapshot are live. Page dimensions are positive.
    unsafe {
        ffi::gdk_paintable_snapshot(
            source,
            snapshot,
            f64::from(page.size.width),
            f64::from(page.size.height),
        );
    }
    for _ in 0..3 {
        // SAFETY: Pop the sample clip, blur, and rounded clip in reverse order.
        unsafe { ffi::gtk_snapshot_pop(snapshot) };
    }
    // SAFETY: The snapshot's render nodes retain the paint they need.
    unsafe { g_object_unref(source) };
    // SAFETY: Transfer the live snapshot to GTK for conversion into a pane-sized paintable.
    unsafe { ffi::gtk_snapshot_free_to_paintable(snapshot, &raw const region.size) }
}

fn bounds(widget: Object, target: Object) -> Option<Rect> {
    let mut rect = Rect::default();
    // SAFETY: Both pointers are live GTK widgets. GTK writes their relative bounds into rect.
    let valid = unsafe { ffi::gtk_widget_compute_bounds(widget, target, &raw mut rect) };
    (valid != FALSE && rect.size.width > 0.0 && rect.size.height > 0.0).then_some(rect)
}
