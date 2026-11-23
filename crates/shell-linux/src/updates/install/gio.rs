#![expect(
    unsafe_code,
    reason = "GIO supplies native authorization for protected AppImages"
)]

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::path::Path;
use std::ptr::{self, NonNull};

use super::image::image_uri;
use super::{InstallError, failure};

type Object = *mut c_void;
type Progress = Option<unsafe extern "C" fn(i64, i64, Object)>;

const COPY_OVERWRITE: c_int = 1;
const COPY_NOFOLLOW_SYMLINKS: c_int = 1 << 2;
const COPY_ALL_METADATA: c_int = 1 << 3;
const COPY_NO_FALLBACK_FOR_MOVE: c_int = 1 << 4;
const QUERY_NOFOLLOW_SYMLINKS: c_int = 1;
const IO_PERMISSION_DENIED: c_int = 14;
const IO_CANCELLED: c_int = 19;

#[repr(C)]
struct GError {
    domain: c_uint,
    code: c_int,
    message: *mut c_char,
}

#[link(name = "gio-2.0")]
unsafe extern "C" {
    fn g_file_new_for_uri(uri: *const c_char) -> Object;
    fn g_file_new_for_path(path: *const c_char) -> Object;
    fn g_file_make_directory(file: Object, cancel: Object, error: *mut *mut GError) -> c_int;
    fn g_file_delete(file: Object, cancel: Object, error: *mut *mut GError) -> c_int;
    fn g_file_copy(
        source: Object,
        target: Object,
        flags: c_int,
        cancel: Object,
        progress: Progress,
        data: Object,
        error: *mut *mut GError,
    ) -> c_int;
    fn g_file_move(
        source: Object,
        target: Object,
        flags: c_int,
        cancel: Object,
        progress: Progress,
        data: Object,
        error: *mut *mut GError,
    ) -> c_int;
    fn g_file_set_attribute_uint32(
        file: Object,
        attribute: *const c_char,
        value: c_uint,
        flags: c_int,
        cancel: Object,
        error: *mut *mut GError,
    ) -> c_int;
    fn g_io_error_quark() -> c_uint;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_unref(object: Object);
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_error_free(error: *mut GError);
}

pub(super) struct GioFile(NonNull<c_void>);

impl GioFile {
    pub(super) fn admin(path: &Path) -> Result<Self, InstallError> {
        let uri = CString::new(image_uri(path)).map_err(|error| failure(&error.to_string()))?;
        // SAFETY: The URI is a live C string for the duration of this call.
        let pointer = unsafe { g_file_new_for_uri(uri.as_ptr()) };
        Self::from_pointer(pointer)
    }

    pub(super) fn local(path: &Path) -> Result<Self, InstallError> {
        let path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|error| failure(&error.to_string()))?;
        // SAFETY: The path is a live C string for the duration of this call.
        let pointer = unsafe { g_file_new_for_path(path.as_ptr()) };
        Self::from_pointer(pointer)
    }

    fn from_pointer(pointer: Object) -> Result<Self, InstallError> {
        NonNull::new(pointer)
            .map(Self)
            .ok_or_else(|| failure("GIO could not open the AppImage location"))
    }

    pub(super) fn create_directory(&self) -> Result<(), InstallError> {
        // The first protected operation is where GVfs translates declined polkit authorization
        // into a permission-denied error.
        perform(true, |error| {
            // SAFETY: The GFile and error slot are live. Cancellation is optional.
            unsafe { g_file_make_directory(self.0.as_ptr(), ptr::null_mut(), error) }
        })
    }

    pub(super) fn delete(&self) -> Result<(), InstallError> {
        perform(false, |error| {
            // SAFETY: The GFile and error slot are live. Cancellation is optional.
            unsafe { g_file_delete(self.0.as_ptr(), ptr::null_mut(), error) }
        })
    }

    pub(super) fn copy_to(&self, target: &Self, preserve: bool) -> Result<(), InstallError> {
        let flags = COPY_NOFOLLOW_SYMLINKS | if preserve { COPY_ALL_METADATA } else { 0 };
        perform(false, |error| {
            // SAFETY: Both GFiles and the error slot are live. No callback is registered.
            unsafe {
                g_file_copy(
                    self.0.as_ptr(),
                    target.0.as_ptr(),
                    flags,
                    ptr::null_mut(),
                    None,
                    ptr::null_mut(),
                    error,
                )
            }
        })
    }

    pub(super) fn replace(&self, target: &Self) -> Result<(), InstallError> {
        perform(false, |error| {
            // SAFETY: Both GFiles and the error slot are live. No callback is registered.
            unsafe {
                g_file_move(
                    self.0.as_ptr(),
                    target.0.as_ptr(),
                    COPY_OVERWRITE | COPY_NOFOLLOW_SYMLINKS | COPY_NO_FALLBACK_FOR_MOVE,
                    ptr::null_mut(),
                    None,
                    ptr::null_mut(),
                    error,
                )
            }
        })
    }

    pub(super) fn set_uint32(&self, name: &CStr, value: u32) -> Result<(), InstallError> {
        perform(false, |error| {
            // SAFETY: The GFile, attribute C string, and error slot are live.
            unsafe {
                g_file_set_attribute_uint32(
                    self.0.as_ptr(),
                    name.as_ptr(),
                    value,
                    QUERY_NOFOLLOW_SYMLINKS,
                    ptr::null_mut(),
                    error,
                )
            }
        })
    }
}

impl Drop for GioFile {
    fn drop(&mut self) {
        // SAFETY: The constructor transferred one reference to this GFile.
        unsafe { g_object_unref(self.0.as_ptr()) };
    }
}

fn perform(
    authorizing: bool,
    operation: impl FnOnce(*mut *mut GError) -> c_int,
) -> Result<(), InstallError> {
    let mut error = ptr::null_mut();
    if operation(&raw mut error) != 0 {
        return Ok(());
    }
    let Some(pointer) = NonNull::new(error) else {
        return Err(failure("the system AppImage file operation failed"));
    };
    // SAFETY: GIO returned a live GError owned by this operation.
    let error = unsafe { pointer.as_ref() };
    // SAFETY: The argument-free call returns a process-wide GIO error domain.
    let io_domain = unsafe { g_io_error_quark() };
    let cancelled = error.domain == io_domain
        && (error.code == IO_CANCELLED || (authorizing && error.code == IO_PERMISSION_DENIED));
    // SAFETY: A live GError owns a non-null, NUL-terminated message.
    let message = unsafe { CStr::from_ptr(error.message) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: The GError and its message are no longer borrowed.
    unsafe { g_error_free(pointer.as_ptr()) };
    if cancelled {
        Err(InstallError::Cancelled)
    } else {
        Err(InstallError::Failed(message))
    }
}
