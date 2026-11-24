//! Endpoint API keys in the desktop's Secret Service, bound to the endpoint origin.

use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

use quickie_application::{SecretError, SecretStore};
use quickie_domain_agents::{ApiKey, KeyRef};

use crate::gtk::ffi::{Bool, GError, Object};

const STRING_ATTRIBUTE: c_int = 0;

pub(super) struct SecretService;

impl SecretStore for SecretService {
    fn read(&self, key: &KeyRef) -> Result<Option<ApiKey>, SecretError> {
        let origin = origin(key)?;
        let schema = Schema::new()?;
        let mut error = ptr::null_mut();
        // SAFETY: The live schema defines an origin string. The list terminates with NULL.
        let password = unsafe {
            secret_password_lookup_sync(
                schema.0,
                ptr::null_mut(),
                &raw mut error,
                c"origin".as_ptr(),
                origin.as_ptr(),
                ptr::null::<c_char>(),
            )
        };
        if password.is_null() {
            return check(error).map(|()| None);
        }
        // SAFETY: libsecret returns a freshly allocated NUL-terminated string.
        let result = unsafe { CStr::from_ptr(password) }
            .to_str()
            .ok()
            .and_then(|text| ApiKey::try_from(text).ok());
        // SAFETY: This releases the libsecret allocation and clears its value.
        unsafe { secret_password_free(password) };
        check(error)?;
        result
            .map(Some)
            .ok_or_else(|| SecretError("The keyring entry is not a valid API key".into()))
    }

    fn write(&self, key: &KeyRef, value: &ApiKey) -> Result<(), SecretError> {
        let origin = origin(key)?;
        let secret =
            CString::new(value.reveal()).map_err(|_| SecretError("Invalid API key".into()))?;
        let schema = Schema::new()?;
        let mut error = ptr::null_mut();
        // SAFETY: Schema, label, password and attributes remain live through the call.
        let saved = unsafe {
            secret_password_store_sync(
                schema.0,
                ptr::null(),
                c"Quickie LLM API key".as_ptr(),
                secret.as_ptr(),
                ptr::null_mut(),
                &raw mut error,
                c"origin".as_ptr(),
                origin.as_ptr(),
                ptr::null::<c_char>(),
            )
        };
        check(error)?;
        if saved == 0 {
            return Err(SecretError(
                "Could not save the API key in the desktop keyring".into(),
            ));
        }
        Ok(())
    }

    fn delete(&self, key: &KeyRef) -> Result<(), SecretError> {
        let origin = origin(key)?;
        let schema = Schema::new()?;
        let mut error = ptr::null_mut();
        // SAFETY: The live schema defines an origin string. The list terminates with NULL.
        unsafe {
            secret_password_clear_sync(
                schema.0,
                ptr::null_mut(),
                &raw mut error,
                c"origin".as_ptr(),
                origin.as_ptr(),
                ptr::null::<c_char>(),
            )
        };
        check(error)
    }
}

fn origin(key: &KeyRef) -> Result<CString, SecretError> {
    CString::new(key.as_str()).map_err(|_| SecretError("Invalid endpoint origin".into()))
}

fn check(error: *mut GError) -> Result<(), SecretError> {
    if error.is_null() {
        return Ok(());
    }
    // SAFETY: A non-null error slots holds an owned GError allocated by libsecret.
    unsafe { g_error_free(error) };
    Err(SecretError("The desktop keyring request failed".into()))
}

struct Schema(Object);

impl Schema {
    fn new() -> Result<Self, SecretError> {
        // SAFETY: The static name and string attribute are terminated by NULL.
        let schema = unsafe {
            secret_schema_new(
                c"dev.quickie.AgentEndpoint".as_ptr(),
                0,
                c"origin".as_ptr(),
                STRING_ATTRIBUTE,
                ptr::null::<c_char>(),
            )
        };
        if schema.is_null() {
            return Err(SecretError(
                "Cannot initialize the desktop keyring schema".into(),
            ));
        }
        Ok(Self(schema))
    }
}

impl Drop for Schema {
    fn drop(&mut self) {
        // SAFETY: This releases our single reference to the live libsecret schema.
        unsafe { secret_schema_unref(self.0) };
    }
}

#[link(name = "secret-1")]
unsafe extern "C" {
    fn secret_schema_new(name: *const c_char, flags: c_int, ...) -> Object;
    fn secret_schema_unref(schema: Object);
    fn secret_password_lookup_sync(
        schema: Object,
        cancellable: Object,
        error: *mut *mut GError,
        ...
    ) -> *mut c_char;
    fn secret_password_store_sync(
        schema: Object,
        collection: *const c_char,
        label: *const c_char,
        password: *const c_char,
        cancellable: Object,
        error: *mut *mut GError,
        ...
    ) -> Bool;
    fn secret_password_clear_sync(
        schema: Object,
        cancellable: Object,
        error: *mut *mut GError,
        ...
    ) -> Bool;
    fn secret_password_free(password: *mut c_char);
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_error_free(error: *mut GError);
}
