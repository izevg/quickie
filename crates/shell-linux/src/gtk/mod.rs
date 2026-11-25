//! The GTK4 application, on the main thread for the lifetime of its signal handlers.

#![expect(unsafe_code, reason = "GTK4 and WebKitGTK are native GObject APIs")]

mod agent;
pub(crate) use agent::automation;
mod backdrop;
mod browsing;
pub(crate) mod ffi;
mod find;
pub(crate) mod inspector;
pub(crate) mod keys;
pub(crate) mod palette;
mod privacy;
mod smoke;
mod update_controls;

use std::cell::{Cell, OnceCell};
use std::env;
use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

use crate::launch::Options;
use ffi::{Bool, Object};
use quickie_domain_commands::{ContextFlag, Resolution};
use update_controls::UpdateControls;

pub(crate) struct App {
    application: Object,
    options: Options,
    prefix: PathBuf,
    widgets: Cell<Option<Widgets>>,
    agent: agent::Agent,
    pub(crate) browsing: browsing::Browsing,
    palette: palette::Palette,
    find: find::Find,
    keys: keys::Keybindings,
    privacy: privacy::Privacy,
    mcp: crate::mcp::gtk::GtkMcpHost,
    smoke: OnceCell<Result<(), String>>,
    updates: UpdateControls,
}

#[derive(Clone, Copy)]
struct Widgets {
    view: Object,
    agent: agent::View,
    palette: palette::View,
    find: find::View,
    privacy: privacy::View,
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
    let keybindings =
        keys::Keybindings::new(matches!(options, Options::Smoke(_) | Options::Updates));
    let app = App {
        application,
        options,
        prefix,
        widgets: Cell::new(None),
        agent: agent::Agent::default(),
        browsing: browsing::Browsing::default(),
        palette: palette::Palette::default(),
        find: find::Find::default(),
        keys: keybindings,
        privacy: privacy::Privacy::default(),
        mcp: crate::mcp::gtk::GtkMcpHost::default(),
        smoke: OnceCell::new(),
        updates: UpdateControls::new(),
    };
    let data = ptr::from_ref(&app).cast_mut().cast();
    connect(application, c"activate", activate as *const _, data);

    // GTK gets only argv[0]. The Rust entry point has already validated all arguments.
    let mut name = *b"quickie\0";
    let mut arguments = [name.as_mut_ptr().cast(), ptr::null_mut()];
    // SAFETY: The live application, writable argv and callback state outlive the event loop.
    let status = unsafe { ffi::g_application_run(application, 1, arguments.as_mut_ptr()) };
    app.mcp.shutdown();
    app.updates.clear_sources();
    app.keys.detach();
    app.agent.shutdown();
    app.privacy.stop();
    browsing::detach(&app);
    // SAFETY: GTK returned from its event loop and this releases our reference.
    unsafe { ffi::g_object_unref(application) };

    if status != 0 {
        return Err(format!("GTK exited with status {status}"));
    }
    let smoke = app.smoke.into_inner();
    if matches!(app.options, Options::Smoke(_)) {
        return smoke.unwrap_or_else(|| {
            Err("the smoke fixture did not report its completion title".into())
        });
    }
    smoke.unwrap_or(Ok(()))?;
    app.updates
        .finish()
        .inspect_err(|error| crate::updates::dialog::install_error(error))
}

pub(crate) fn connect(
    instance: Object,
    signal: &CStr,
    handler: *const std::ffi::c_void,
    data: Object,
) {
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
    // SAFETY: GTK is initialized on this thread. The live window adopts the new host.
    let host = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 0) };
    // SAFETY: The window and host are live widgets on the GTK thread.
    unsafe { ffi::gtk_window_set_child(window, host) };
    // SAFETY: The live host adopts the newly created overlay.
    let overlay = unsafe { ffi::gtk_overlay_new() };
    // SAFETY: The live overlay fills the available width next to the agent panel.
    unsafe { ffi::gtk_widget_set_hexpand(overlay, ffi::TRUE) };
    // SAFETY: The live host assumes ownership of the overlay.
    unsafe { ffi::gtk_box_append(host, overlay) };
    let privacy = privacy::attach(overlay, data);
    let view = match browsing::attach(app, window, overlay, data) {
        Ok(view) => view,
        Err(error) => {
            complete_smoke(app, Err(error));
            return;
        }
    };
    let agent = agent::attach(host, view, data);
    let palette = palette::attach(overlay, data);
    let find = find::attach(overlay, view, data);
    let isolated = matches!(app.options, Options::Smoke(_) | Options::Updates);
    app.mcp.attach(app, overlay, isolated);
    app.widgets.set(Some(Widgets {
        view,
        agent,
        palette,
        find,
        privacy,
    }));
    let controller = keys::capture_controller();
    connect(controller, c"key-pressed", key_pressed as *const _, data);
    connect(controller, c"key-released", key_released as *const _, data);
    connect(
        window,
        c"notify::is-active",
        key_focus_changed as *const _,
        data,
    );
    connect(
        window,
        c"notify::focus-widget",
        key_focus_changed as *const _,
        data,
    );
    // SAFETY: The live window adopts the live controller.
    unsafe { ffi::gtk_widget_add_controller(window, controller) };
    // SAFETY: The live window's content and controller are in place.
    unsafe { ffi::gtk_window_present(window) };
    if !matches!(app.options, Options::Smoke(_)) {
        app.updates
            .attach(window, data, matches!(app.options, Options::Updates));
    }
    match &app.options {
        Options::Empty | Options::Mcp { .. } => {}
        Options::Updates => {
            app.updates.present(data, &app.prefix);
            app.updates.check(data);
        }
        Options::Navigate(uri) => privacy::navigate(app, view, uri),
        Options::Smoke(uri) => {
            connect(
                view,
                c"notify::title",
                smoke_title_changed as *const _,
                data,
            );
            connect(view, c"load-failed", smoke_load_failed as *const _, data);
            load(view, uri);
        }
    }
}

fn active_view_changed(app: &App, view: Object) {
    crate::mcp::gtk::on_active_tab_changed(app.browsing.active_tab());
    if let Some(mut widgets) = app.widgets.get() {
        find::rebind(app, &mut widgets, view);
        widgets.view = view;
        app.widgets.set(Some(widgets));
        privacy::refresh(app, &widgets);
    }
}

fn load(view: Object, uri: &CStr) {
    // SAFETY: The live WebKitWebView copies the validated URI from a NUL-terminated string.
    unsafe { ffi::webkit_web_view_load_uri(view, uri.as_ptr()) };
}

extern "C" fn key_pressed(
    controller: Object,
    key: c_uint,
    hardware_code: c_uint,
    modifiers: c_uint,
    data: Object,
) -> Bool {
    // SAFETY: GTK passed our live App reference, and the widget signal runs on its main thread.
    let app = unsafe { &*data.cast::<App>() };
    let Some(widgets) = app.widgets.get() else {
        return ffi::FALSE;
    };
    if crate::mcp::gtk::key_pressed(key, modifiers) {
        app.keys.cancel_chord();
        return ffi::TRUE;
    }
    if let Some(handled) =
        inspector::docked_key_pressed(app, widgets.view, controller, key, hardware_code, modifiers)
    {
        return handled;
    }
    if browsing::downloads::key_pressed(app, key, modifiers)
        || browsing::key_pressed(app, key, modifiers)
    {
        return ffi::TRUE;
    }
    if privacy::key_pressed(app, &widgets, key, modifiers)
        || palette::key_pressed(app, &widgets, key, modifiers)
        || agent::key_pressed(app, &widgets, key, modifiers)
    {
        app.keys.cancel_chord();
        return ffi::TRUE;
    }
    if keys::is_modifier(key) {
        return ffi::FALSE;
    }
    let Some(stroke) = keys::read(controller, key, hardware_code, modifiers) else {
        app.keys.cancel_chord();
        return ffi::FALSE;
    };
    if browsing::hints::key(app, stroke) {
        return ffi::TRUE;
    }
    let mut context = palette::key_context(app).unwrap_or_else(|| browsing::key_context(app));
    if browsing::hints::active(app) {
        context = context.with(ContextFlag::HintsOpen);
    }
    if keys::input_focused(widgets.view) {
        context = context.with(ContextFlag::InputFocused);
    }
    if privacy::key_popover_open(app) {
        context = context.with(ContextFlag::PopoverOpen);
    }
    if agent::key_panel_open(&widgets) {
        context = context.with(ContextFlag::AgentOpen);
    }
    let resolution = app.keys.resolve(stroke, &context);
    if stroke.modifiers.makes_shortcut()
        && !matches!(&resolution, Some(Resolution::Command(id)) if matches!(
            id.as_str(), "quickie.hints.toggle" | "quickie.hints.openInBackground"
        ))
    {
        browsing::hints::hide(app);
    }
    match resolution {
        Some(Resolution::Command(id)) => {
            Bool::from(palette::dispatch(app, &widgets, &id, stroke, data))
        }
        Some(Resolution::Pending(_) | Resolution::Unbound) => ffi::TRUE,
        None => ffi::FALSE,
    }
}

extern "C" fn key_focus_changed(_: Object, _: Object, data: Object) {
    // SAFETY: GTK emits these property changes with the live App as its callback data.
    let app = unsafe { &*data.cast::<App>() };
    app.keys.cancel_chord();
}

extern "C" fn key_released(_: Object, key: c_uint, _: c_uint, modifiers: c_uint, data: Object) {
    // SAFETY: The live controller supplies the App reference on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    browsing::key_released(app, key, modifiers);
}

extern "C" fn smoke_title_changed(view: Object, _property: Object, data: Object) {
    // SAFETY: WebKit owns the live view and its NUL-terminated title, if non-null.
    let title = unsafe { ffi::webkit_web_view_get_title(view) };
    // SAFETY: The non-null pointer remains valid for this synchronous notification.
    let title = (!title.is_null()).then(|| unsafe { CStr::from_ptr(title) });
    if smoke::fixture_title(title) {
        // SAFETY: GTK passed our live App reference and invokes this on the GTK main thread.
        let app = unsafe { &*data.cast::<App>() };
        complete_smoke(app, smoke::verify_runtime(&app.prefix));
    }
}

extern "C" fn smoke_load_failed(
    _view: Object,
    _event: c_int,
    failing_uri: *const c_char,
    error: *const ffi::GError,
    data: Object,
) -> Bool {
    // SAFETY: GTK passed our live App reference and invokes this on the GTK main thread.
    let app = unsafe { &*data.cast::<App>() };
    let uri = if failing_uri.is_null() {
        "unknown URI".into()
    } else {
        // SAFETY: The signal supplies a NUL-terminated URI for the duration of this call.
        unsafe { CStr::from_ptr(failing_uri) }.to_string_lossy()
    };
    // SAFETY: A non-null GError is owned by the signal emitter and lives through this call.
    let error = unsafe { error.as_ref() };
    let detail = error.map_or_else(
        || "unknown WebKit error".to_owned(),
        |error| {
            let message = if error.message.is_null() {
                "no error message".into()
            } else {
                // SAFETY: GError's message is a NUL-terminated string owned by the emitter.
                unsafe { CStr::from_ptr(error.message) }.to_string_lossy()
            };
            format!("{message} (domain {}, code {})", error.domain, error.code)
        },
    );
    complete_smoke(
        app,
        Err(format!("could not load smoke fixture at {uri}: {detail}")),
    );
    ffi::TRUE
}

fn complete_smoke(app: &App, result: Result<(), String>) {
    if app.smoke.set(result).is_ok() {
        // SAFETY: This is the live application that owns the callback state.
        unsafe { ffi::g_application_quit(app.application) };
    }
}
