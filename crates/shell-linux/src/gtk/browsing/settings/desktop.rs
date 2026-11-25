//! Desktop handlers for web links and the Downloads folder.

use std::ffi::{CStr, CString, c_char};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{self, NonNull};

use crate::gtk::ffi::{self, Bool, GError, Object};

const DESKTOP_ID: &CStr = c"quickie.desktop";
const WEB_LINKS: [&CStr; 2] = [c"x-scheme-handler/http", c"x-scheme-handler/https"];

pub(super) struct BrowserStatus {
    pub(super) description: String,
    pub(super) can_make_default: bool,
    pub(super) installed: bool,
}

pub(super) fn browser_status() -> BrowserStatus {
    let [http, https] = WEB_LINKS.map(AppInfo::default_for);
    let already_default = http.as_ref().is_some_and(AppInfo::is_quickie)
        && https.as_ref().is_some_and(AppInfo::is_quickie);
    let installed = AppInfo::quickie().is_some();
    let description = match (http, https) {
        (Some(http), Some(https)) if http.same_as(&https) => {
            format!("{} is your default browser.", http.name())
        }
        (None, None) => "No default browser is set.".into(),
        _ => "HTTP and HTTPS use different default browser settings.".into(),
    };
    BrowserStatus {
        description,
        can_make_default: installed && !already_default,
        installed,
    }
}

pub(super) fn make_default() -> Result<(), String> {
    let quickie = AppInfo::quickie().ok_or("Quickie is not registered with this desktop.")?;
    let mut errors = Vec::new();
    for (kind, scheme) in WEB_LINKS.into_iter().zip(["HTTP", "HTTPS"]) {
        if !AppInfo::default_for(kind).is_some_and(|handler| handler.is_quickie())
            && let Err(error) = quickie.set_default_for(kind)
        {
            errors.push(format!("{scheme}: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" "))
    }
}

struct AppInfo(NonNull<std::ffi::c_void>);

impl AppInfo {
    fn quickie() -> Option<Self> {
        // SAFETY: GIO reads the registered app for this static desktop ID and returns a new
        // reference, or NULL if Quickie is unavailable in the desktop's search paths.
        NonNull::new(unsafe { g_desktop_app_info_new(DESKTOP_ID.as_ptr()) }).map(Self)
    }

    fn default_for(kind: &CStr) -> Option<Self> {
        // SAFETY: GIO returns a new reference to the default handler, or NULL.
        NonNull::new(unsafe { g_app_info_get_default_for_type(kind.as_ptr(), ffi::TRUE) }).map(Self)
    }

    fn same_as(&self, other: &Self) -> bool {
        // SAFETY: Both owned references implement GAppInfo.
        unsafe { g_app_info_equal(self.0.as_ptr(), other.0.as_ptr()) != ffi::FALSE }
    }

    fn is_quickie(&self) -> bool {
        // SAFETY: GIO returns a borrowed NUL-terminated identifier or NULL.
        let id = unsafe { g_app_info_get_id(self.0.as_ptr()) };
        if id.is_null() {
            return false;
        }
        // SAFETY: GIO owns this non-null, NUL-terminated identifier while the app is live.
        unsafe { CStr::from_ptr(id) == DESKTOP_ID }
    }

    fn name(&self) -> String {
        // SAFETY: GIO returns the borrowed display name owned by this live AppInfo.
        let name = unsafe { g_app_info_get_display_name(self.0.as_ptr()) };
        // SAFETY: The display name is a non-null, NUL-terminated string.
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    }

    fn set_default_for(&self, kind: &CStr) -> Result<(), String> {
        let mut error = ptr::null_mut();
        // SAFETY: The app and type are live, and GIO can store an error in the writable output.
        let changed = unsafe {
            g_app_info_set_as_default_for_type(self.0.as_ptr(), kind.as_ptr(), &raw mut error)
        };
        if changed == ffi::FALSE {
            Err(error_message(error, "The desktop rejected the request."))
        } else {
            Ok(())
        }
    }
}

impl Drop for AppInfo {
    fn drop(&mut self) {
        // SAFETY: Each AppInfo owns one reference.
        unsafe { ffi::g_object_unref(self.0.as_ptr()) };
    }
}

pub(super) fn open_downloads(path: &Path, window: Object, status: Object) -> Result<(), String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err("The Downloads location is not a folder.".into()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err("The Downloads folder does not exist yet.".into());
        }
        Err(error) => return Err(format!("Could not open Downloads. {error}")),
    }
    launch(path, window, status, opened_downloads)
}

pub(super) fn open_keybindings(path: &Path, window: Object, status: Object) -> Result<(), String> {
    launch(path, window, status, opened_keybindings)
}

fn launch(
    path: &Path,
    window: Object,
    status: Object,
    finished: extern "C" fn(Object, Object, Object),
) -> Result<(), String> {
    let path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| "The target has an invalid path.")?;
    // SAFETY: GIO copies the path into a new file reference.
    let file = unsafe { g_file_new_for_path(path.as_ptr()) };
    // SAFETY: GTK's new launcher retains the live file.
    let launcher = unsafe { gtk_file_launcher_new(file) };
    // SAFETY: The callback receives its own reference to the status label, even after a close.
    let status = unsafe { g_object_ref(status) };
    // SAFETY: The live settings window parents this asynchronous request; GTK retains the
    // launcher as needed. The callback releases the status reference.
    unsafe {
        gtk_file_launcher_launch(launcher, window, ptr::null_mut(), finished, status);
    }
    // SAFETY: The launch request retains the launcher as needed.
    unsafe { ffi::g_object_unref(launcher) };
    // SAFETY: The launcher took its own reference to the file.
    unsafe { ffi::g_object_unref(file) };
    Ok(())
}

extern "C" fn opened_downloads(launcher: Object, result: Object, status: Object) {
    opened(launcher, result, status, "Downloads");
}

extern "C" fn opened_keybindings(launcher: Object, result: Object, status: Object) {
    opened(launcher, result, status, "keybindings.json");
}

fn opened(launcher: Object, result: Object, status: Object, name: &str) {
    let mut error = ptr::null_mut();
    // SAFETY: GTK supplies the launcher and its matching result, with storage for a GError.
    let opened = unsafe { gtk_file_launcher_launch_finish(launcher, result, &raw mut error) };
    if opened == ffi::FALSE {
        let error = error_message(error, "The desktop rejected the request.");
        super::view::message(status, &format!("Could not open {name}. {error}"));
    }
    // SAFETY: The callback owns a reference to the label.
    unsafe { ffi::g_object_unref(status) };
}

fn error_message(error: *mut GError, fallback: &str) -> String {
    let Some(error) = NonNull::new(error) else {
        return fallback.to_owned();
    };
    // SAFETY: On failure GIO hands the caller an owned GError with a non-null message.
    let message = unsafe { (*error.as_ptr()).message };
    // SAFETY: The message is NUL-terminated while its owning GError remains live.
    let message = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: This releases the error GIO transferred to this caller.
    unsafe { g_error_free(error.as_ptr()) };
    message
}

#[link(name = "gio-2.0")]
unsafe extern "C" {
    fn g_desktop_app_info_new(desktop_id: *const c_char) -> Object;
    fn g_app_info_get_default_for_type(
        content_type: *const c_char,
        must_support_uris: Bool,
    ) -> Object;
    fn g_app_info_equal(info1: Object, info2: Object) -> Bool;
    fn g_app_info_get_id(info: Object) -> *const c_char;
    fn g_app_info_get_display_name(info: Object) -> *const c_char;
    fn g_app_info_set_as_default_for_type(
        info: Object,
        content_type: *const c_char,
        error: *mut *mut GError,
    ) -> Bool;
    fn g_file_new_for_path(path: *const c_char) -> Object;
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_file_launcher_new(file: Object) -> Object;
    fn gtk_file_launcher_launch(
        launcher: Object,
        parent: Object,
        cancellable: Object,
        callback: extern "C" fn(Object, Object, Object),
        data: Object,
    );
    fn gtk_file_launcher_launch_finish(
        launcher: Object,
        result: Object,
        error: *mut *mut GError,
    ) -> Bool;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_ref(object: Object) -> Object;
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_error_free(error: *mut GError);
}
