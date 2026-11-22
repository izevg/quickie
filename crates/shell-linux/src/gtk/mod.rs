//! The GTK4 application, on the main thread for the lifetime of its signal handlers.

#![expect(unsafe_code, reason = "GTK4 and WebKitGTK are native GObject APIs")]

mod ffi;
mod smoke;

use std::cell::{Cell, RefCell};
use std::env;
use std::ffi::{CStr, CString, c_int, c_uint};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

use crate::launch::{Options, address_bar_destination};
use ffi::{Bool, Object};

struct App {
    application: Object,
    options: Options,
    prefix: PathBuf,
    widgets: Cell<Option<Widgets>>,
    smoke: RefCell<Result<(), String>>,
}

#[derive(Clone, Copy)]
struct Widgets {
    view: Object,
    entry: Object,
}

pub fn run(options: Options) -> Result<(), String> {
    let executable = env::current_exe().map_err(|error| error.to_string())?;
    let prefix = executable
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or("cannot locate the installed WebKit runtime")?
        .to_path_buf();
    // SAFETY: The static application ID is valid and this is the main thread.
    let application =
        unsafe { ffi::gtk_application_new(c"dev.quickie.Quickie".as_ptr(), ffi::NON_UNIQUE) };
    if application.is_null() {
        return Err("cannot create the GTK application".into());
    }
    let app = App {
        application,
        options,
        prefix,
        widgets: Cell::new(None),
        smoke: RefCell::new(Err("the smoke page did not finish loading".into())),
    };
    let data = ptr::from_ref(&app).cast_mut().cast();
    connect(application, c"activate", activate as *const _, data);

    // GTK gets only argv[0]. The Rust entry point has already validated all arguments.
    let mut name = *b"quickie\0";
    let mut arguments = [name.as_mut_ptr().cast(), ptr::null_mut()];
    // SAFETY: The live application, writable argv and callback state outlive the event loop.
    let status = unsafe { ffi::g_application_run(application, 1, arguments.as_mut_ptr()) };
    // SAFETY: GTK returned from its event loop and this releases our reference.
    unsafe { ffi::g_object_unref(application) };

    if status != 0 {
        return Err(format!("GTK exited with status {status}"));
    }
    if matches!(app.options, Options::Smoke(_)) {
        app.smoke.into_inner()
    } else {
        Ok(())
    }
}

fn connect(instance: Object, signal: &CStr, handler: *const std::ffi::c_void, data: Object) {
    // SAFETY: Each caller uses the callback ABI for the named signal. GTK's signal connections
    // are on objects owned by the application, and data lives until the application is released.
    unsafe { ffi::g_signal_connect_data(instance, signal.as_ptr(), handler, data, None, 0) };
}

extern "C" fn activate(application: Object, data: Object) {
    // SAFETY: data is a live shared App reference from run and GTK calls on the main thread.
    let app = unsafe { &*data.cast::<App>() };
    if app.widgets.get().is_some() {
        return;
    }

    // SAFETY: The default context is available after GTK initializes, before any web views.
    let context = unsafe { ffi::webkit_web_context_get_default() };
    for directory in ["lib", "libexec/webkitgtk-6.0"] {
        if let Ok(path) = CString::new(app.prefix.join(directory).as_os_str().as_bytes()) {
            // SAFETY: The absolute path is valid for the duration of the API call, which copies
            // it. The default context has not been used to create a subprocess.
            unsafe {
                ffi::webkit_web_context_add_path_to_sandbox(context, path.as_ptr(), ffi::TRUE);
            };
        }
    }

    // SAFETY: The live GtkApplication is initialized and owns newly created application windows.
    let window = unsafe { ffi::gtk_application_window_new(application) };
    // SAFETY: This is the live GtkApplicationWindow, and GTK copies the title.
    unsafe { ffi::gtk_window_set_title(window, c"Quickie".as_ptr()) };
    // SAFETY: This is the live GtkApplicationWindow.
    unsafe { ffi::gtk_window_set_default_size(window, 1200, 780) };
    // SAFETY: GTK is initialized on this thread. The widget is adopted by the window.
    let overlay = unsafe { ffi::gtk_overlay_new() };
    // SAFETY: Both are live widgets. The window adopts its child.
    unsafe { ffi::gtk_window_set_child(window, overlay) };
    // SAFETY: GTK and the default WebKit context are initialized on the main thread.
    let view = unsafe { ffi::webkit_web_view_new() };
    // SAFETY: The overlay adopts the live WebKitWebView as its main child.
    unsafe { ffi::gtk_overlay_set_child(overlay, view) };
    // SAFETY: The WebKitWebView owns its settings for its entire lifetime.
    let settings = unsafe { ffi::webkit_web_view_get_settings(view) };
    // SAFETY: Settings belong to the live WebKitWebView on this thread.
    unsafe { ffi::webkit_settings_set_enable_developer_extras(settings, ffi::TRUE) };
    let entry = address_bar(overlay);
    app.widgets.set(Some(Widgets { view, entry }));
    connect(entry, c"activate", navigate as *const _, data);
    // SAFETY: GTK is initialized, and the returned event controller will be adopted by the window.
    let controller = unsafe { ffi::gtk_event_controller_key_new() };
    // SAFETY: The live key controller can be configured before adding it to a widget.
    unsafe { ffi::gtk_event_controller_set_propagation_phase(controller, ffi::PHASE_CAPTURE) };
    connect(controller, c"key-pressed", key_pressed as *const _, data);
    // SAFETY: The live window adopts the live controller.
    unsafe { ffi::gtk_widget_add_controller(window, controller) };
    // SAFETY: The live window's content and controller are in place.
    unsafe { ffi::gtk_window_present(window) };
    match &app.options {
        Options::Empty => {}
        Options::Navigate(uri) => load(view, uri),
        Options::Smoke(uri) => {
            connect(view, c"load-changed", smoke_finished as *const _, data);
            load(view, uri);
        }
    }
}

fn address_bar(overlay: Object) -> Object {
    // SAFETY: GTK is initialized on the main thread and the returned entry is adopted below.
    let entry = unsafe { ffi::gtk_entry_new() };
    // SAFETY: The live allocation is a GtkEntry, which implements GtkEditable.
    unsafe { ffi::gtk_editable_set_width_chars(entry, 45) };
    // SAFETY: The live GtkEntry copies the static placeholder text.
    unsafe { ffi::gtk_entry_set_placeholder_text(entry, c"Address or search".as_ptr()) };
    // SAFETY: The entry is a live widget.
    unsafe { ffi::gtk_widget_set_halign(entry, ffi::ALIGN_CENTER) };
    // SAFETY: The entry is a live widget.
    unsafe { ffi::gtk_widget_set_valign(entry, ffi::ALIGN_START) };
    // SAFETY: The entry is a live widget.
    unsafe { ffi::gtk_widget_set_margin_top(entry, 12) };
    // SAFETY: The live overlay adopts the entry as an overlay child.
    unsafe { ffi::gtk_overlay_add_overlay(overlay, entry) };
    // SAFETY: The entry is a live widget retained by the overlay.
    unsafe { ffi::gtk_widget_set_visible(entry, ffi::FALSE) };
    entry
}

extern "C" fn navigate(entry: Object, data: Object) {
    // SAFETY: GTK passed our live App reference, and the widget signal runs on its main thread.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(widgets) = app.widgets.get() {
        // SAFETY: This GtkEntry implements GtkEditable and its text is valid until edited.
        let text = unsafe { ffi::gtk_editable_get_text(entry) };
        // SAFETY: A live GtkEditable always returns a NUL-terminated string.
        let text = unsafe { CStr::from_ptr(text) };
        if let Some(uri) = text.to_str().ok().and_then(address_bar_destination) {
            dismiss_address_bar(widgets);
            load(widgets.view, &uri);
        }
    }
}

fn load(view: Object, uri: &CStr) {
    // SAFETY: The live WebKitWebView copies the validated URI from a NUL-terminated string.
    unsafe { ffi::webkit_web_view_load_uri(view, uri.as_ptr()) };
}

fn show_address_bar(widgets: Widgets) {
    // SAFETY: The live WebKitWebView returns a borrowed URI or null when nothing has loaded.
    let uri = unsafe { ffi::webkit_web_view_get_uri(widgets.view) };
    let uri = if uri.is_null() { c"".as_ptr() } else { uri };
    // SAFETY: The live GtkEntry copies the URI, which remains valid until subsequent navigation.
    unsafe { ffi::gtk_editable_set_text(widgets.entry, uri) };
    // SAFETY: The entry is a live widget retained by the overlay.
    unsafe { ffi::gtk_widget_set_visible(widgets.entry, ffi::TRUE) };
    // SAFETY: The visible entry is a focusable live widget.
    unsafe { ffi::gtk_widget_grab_focus(widgets.entry) };
    // SAFETY: The GtkEntry implements GtkEditable. End -1 means the end of the text.
    unsafe { ffi::gtk_editable_select_region(widgets.entry, 0, -1) };
}

fn dismiss_address_bar(widgets: Widgets) {
    // SAFETY: The entry is a live widget retained by the overlay.
    unsafe { ffi::gtk_widget_set_visible(widgets.entry, ffi::FALSE) };
    // SAFETY: The WebKitWebView is a focusable live widget retained by the overlay.
    unsafe { ffi::gtk_widget_grab_focus(widgets.view) };
}

extern "C" fn key_pressed(
    _controller: Object,
    key: c_uint,
    _hardware_code: c_uint,
    modifiers: c_uint,
    data: Object,
) -> Bool {
    // SAFETY: GTK passed our live App reference, and the widget signal runs on its main thread.
    let app = unsafe { &*data.cast::<App>() };
    let Some(widgets) = app.widgets.get() else {
        return ffi::FALSE;
    };
    match (key, modifiers & (ffi::SHIFT | ffi::CONTROL | ffi::ALT)) {
        (0x6c, ffi::CONTROL) => show_address_bar(widgets),
        (0x72, ffi::CONTROL) => {
            // SAFETY: This is a live WebKitWebView retained by the overlay.
            unsafe { ffi::webkit_web_view_reload(widgets.view) };
        }
        (0x49, modifiers) if modifiers == ffi::SHIFT | ffi::CONTROL => {
            // SAFETY: The live WebKitWebView owns its inspector for its lifetime.
            let inspector = unsafe { ffi::webkit_web_view_get_inspector(widgets.view) };
            // SAFETY: The inspector belongs to the live WebKitWebView and developer extras are on.
            unsafe { ffi::webkit_web_inspector_show(inspector) };
        }
        (ffi::KEY_LEFT, ffi::ALT) => {
            // SAFETY: This is a live WebKitWebView retained by the overlay.
            unsafe { ffi::webkit_web_view_go_back(widgets.view) };
        }
        (ffi::KEY_RIGHT, ffi::ALT) => {
            // SAFETY: This is a live WebKitWebView retained by the overlay.
            unsafe { ffi::webkit_web_view_go_forward(widgets.view) };
        }
        (ffi::KEY_ESCAPE, 0) => {
            // SAFETY: GTK retains the GtkEntry, so its visibility can be read here.
            if unsafe { ffi::gtk_widget_get_visible(widgets.entry) } == ffi::FALSE {
                return ffi::FALSE;
            }
            dismiss_address_bar(widgets);
        }
        _ => return ffi::FALSE,
    }
    ffi::TRUE
}

extern "C" fn smoke_finished(view: Object, event: c_int, data: Object) {
    if event != ffi::LOAD_FINISHED {
        return;
    }
    // SAFETY: GTK passed our live App reference from run, and this runs on the GTK main thread.
    let app = unsafe { &*data.cast::<App>() };
    // SAFETY: The view is live during the callback and owns the borrowed title, if non-null.
    let title = unsafe { ffi::webkit_web_view_get_title(view) };
    let result = if title.is_null() {
        Err("the smoke page returned no title".into())
    } else {
        // SAFETY: WebKit returns a NUL-terminated title owned by the live view.
        smoke::verify(unsafe { CStr::from_ptr(title) }, &app.prefix)
    };
    *app.smoke.borrow_mut() = result;
    // SAFETY: This is the live application that owns all the callback state.
    unsafe { ffi::g_application_quit(app.application) };
}
