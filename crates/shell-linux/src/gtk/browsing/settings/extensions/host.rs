//! The optional `WebKitGTK` fork API and its extension context ownership.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString, c_char, c_void};
use std::io::{self, Write as _};
use std::ptr::{self, NonNull};
use std::rc::Rc;

use quickie_application::{
    Access, EngineError, ExtensionEvent, ExtensionHost, ExtensionInfo, ReplyToken, WindowTabs,
};
use quickie_domain_browsing::TabId;
use quickie_domain_settings::{ExtensionId, ExtensionVersion, Grants};

use crate::gtk::ffi::{self, Bool, GError, Object};

use super::{Profile, dlsym};

const UNAVAILABLE: &str = "This WebKitGTK build does not support Quickie extensions.";
type ErrorSlot = *mut *mut GError;
type NewPath = unsafe extern "C" fn(*const c_char, ErrorSlot) -> Object;
type NewContext = unsafe extern "C" fn(Object, ErrorSlot) -> Object;
type Control = unsafe extern "C" fn(Object, Object, ErrorSlot) -> Bool;
type Grant = unsafe extern "C" fn(Object, *const *const c_char, ErrorSlot) -> Bool;
type StringProperty = unsafe extern "C" fn(Object) -> *const c_char;
type BoolProperty = unsafe extern "C" fn(Object) -> Bool;
type Strings = unsafe extern "C" fn(Object) -> *const *const c_char;
type Patterns = unsafe extern "C" fn(Object) -> *mut Object;
type UnrefPattern = unsafe extern "C" fn(Object);

#[derive(Clone, Copy, Debug)]
struct Symbols {
    extension: NewPath,
    context: NewContext,
    load: Control,
    unload: Control,
    grant: Grant,
    name: StringProperty,
    version: StringProperty,
    description: StringProperty,
    content: BoolProperty,
    background: BoolProperty,
    rules: BoolProperty,
    permissions: Strings,
    hosts: Patterns,
    pattern_name: StringProperty,
    pattern_unref: UnrefPattern,
}

// A partial API must never be used, and the normal browser can still use stock WebKitGTK.
macro_rules! symbol {
    ($name:literal, $signature:ty) => {{
        // SAFETY: RTLD_DEFAULT searches libraries already loaded by Quickie. The caller must
        // give the exact signature from the fork's public headers.
        let symbol = unsafe { dlsym(ptr::null_mut(), $name.as_ptr()) };
        if symbol.is_null() {
            return Err(UNAVAILABLE.to_owned());
        }
        // SAFETY: A non-null lookup of the named WebKitGTK function has the declared C ABI.
        unsafe { std::mem::transmute::<Object, $signature>(symbol) }
    }};
}

impl Symbols {
    fn resolve() -> Result<Self, String> {
        Ok(Self {
            extension: symbol!(c"webkit_web_extension_new", NewPath),
            context: symbol!(
                c"webkit_web_extension_context_new_for_extension",
                NewContext
            ),
            load: symbol!(c"webkit_web_extension_controller_load", Control),
            unload: symbol!(c"webkit_web_extension_controller_unload", Control),
            grant: symbol!(c"webkit_web_extension_context_set_granted_hosts", Grant),
            name: symbol!(c"webkit_web_extension_get_display_name", StringProperty),
            version: symbol!(c"webkit_web_extension_get_version", StringProperty),
            description: symbol!(
                c"webkit_web_extension_get_display_description",
                StringProperty
            ),
            content: symbol!(
                c"webkit_web_extension_get_has_injected_content",
                BoolProperty
            ),
            background: symbol!(
                c"webkit_web_extension_get_has_background_content",
                BoolProperty
            ),
            rules: symbol!(
                c"webkit_web_extension_get_has_content_modification_rules",
                BoolProperty
            ),
            permissions: symbol!(c"webkit_web_extension_get_requested_permissions", Strings),
            hosts: symbol!(
                c"webkit_web_extension_get_requested_host_match_patterns",
                Patterns
            ),
            pattern_name: symbol!(
                c"webkit_web_extension_match_pattern_get_string",
                StringProperty
            ),
            pattern_unref: symbol!(c"webkit_web_extension_match_pattern_unref", UnrefPattern),
        })
    }

    fn read(self, location: &str) -> Result<Prepared, String> {
        let path = CString::new(location).map_err(|error| error.to_string())?;
        let extension = object_call(
            |error| {
                // SAFETY: The package path is a live C string and the writable error is local.
                unsafe { (self.extension)(path.as_ptr(), error) }
            },
            "WebKit could not read the extension package.",
        )?;
        let pointer = extension.get();
        // SAFETY: This is a live WebKitWebExtension, borrowed synchronously by the getter.
        let injected_content = unsafe { (self.content)(pointer) == ffi::TRUE };
        let unsupported_content = [self.background, self.rules].iter().any(|getter| {
            // SAFETY: Each getter borrows the live WebKitWebExtension.
            unsafe { getter(pointer) == ffi::TRUE }
        });
        // SAFETY: The getter returns a null pointer or an extension-owned, zero-terminated array.
        let permissions = unsafe { (self.permissions)(pointer) };
        // SAFETY: The live extension owns the array, and it contains a valid first slot if non-null.
        let has_permissions = !permissions.is_null() && !unsafe { *permissions }.is_null();
        if !injected_content || unsupported_content || has_permissions {
            return Err("This Linux runtime supports content scripts and host permissions. The package requests other extension capabilities.".into());
        }
        let patterns = self.requested_hosts(&extension);
        validate_hosts(&patterns)?;
        let info = ExtensionInfo {
            name: extension_text(self.name, &extension),
            version: ExtensionVersion::try_from(extension_text(self.version, &extension).as_str())
                .map_err(|error| error.to_string())?,
            description: extension_text(self.description, &extension),
            requested: Grants {
                patterns,
                ..Grants::default()
            },
            action: None,
            commands: Vec::new(),
        };
        if info.name.trim().is_empty() {
            return Err("The extension has no name.".into());
        }
        let context = object_call(
            |error| {
                // SAFETY: The live extension and local writable error outlive construction.
                unsafe { (self.context)(pointer, error) }
            },
            "WebKit could not create the extension context.",
        )?;
        Ok(Prepared { context, info })
    }

    fn requested_hosts(self, extension: &Owned) -> BTreeSet<String> {
        // SAFETY: The getter returns a new zero-terminated array of owned match patterns or null.
        let start = unsafe { (self.hosts)(extension.get()) };
        let mut answer = BTreeSet::new();
        if start.is_null() {
            return answer;
        }
        let mut cursor = start;
        loop {
            // SAFETY: Every slot through the terminating null belongs to this owned C array.
            let pattern = unsafe { *cursor };
            if pattern.is_null() {
                break;
            }
            // SAFETY: This slot holds one live, owned match pattern.
            let pattern_text = unsafe { (self.pattern_name)(pattern) };
            // SAFETY: The getter returned a borrowed string or null from the live pattern.
            answer.insert(unsafe { borrowed_text(pattern_text) });
            // SAFETY: The match pattern is fully owned by this array and has been read.
            unsafe { (self.pattern_unref)(pattern) };
            // SAFETY: The owned array has at least its terminating null after this occupied slot.
            cursor = unsafe { cursor.add(1) };
        }
        // SAFETY: The array was allocated by GLib and its element references are released.
        unsafe { g_free(start.cast()) };
        answer
    }
}

fn validate_hosts(patterns: &BTreeSet<String>) -> Result<(), String> {
    if patterns.is_empty() {
        return Err("The extension has no supported content-script sites.".into());
    }
    // The GTK preview grants web sites only. In particular it must not grant access to files
    // or privileged browser pages through a wildcard intended to match all URL schemes.
    if patterns.iter().any(|pattern| {
        !["http://", "https://", "*://"]
            .iter()
            .any(|scheme| pattern.starts_with(scheme))
    }) {
        return Err("Linux extensions must declare HTTP or HTTPS sites.".into());
    }
    Ok(())
}

#[derive(Debug)]
struct Prepared {
    context: Owned,
    info: ExtensionInfo,
}

#[derive(Debug)]
struct Slot {
    prepared: Option<Prepared>,
    running: Option<Prepared>,
}

#[derive(Debug)]
pub(super) struct Host {
    symbols: Symbols,
    profile: Rc<Profile>,
    entries: BTreeMap<ExtensionId, Slot>,
    events: Vec<ExtensionEvent>,
}

impl Host {
    pub(super) fn open(profile: Rc<Profile>) -> Result<Self, String> {
        Ok(Self {
            symbols: Symbols::resolve()?,
            profile,
            entries: BTreeMap::new(),
            events: Vec::new(),
        })
    }

    fn control(&self, command: Control, context: &Owned) -> Result<(), EngineError> {
        bool_call(|error| {
            // SAFETY: The controller and context are live and the caller owns the error slot.
            unsafe { command(self.profile.controller.as_ptr(), context.get(), error) }
        })
    }

    fn revoke(&self, context: &Owned) {
        // Revocation is applied even if the engine refuses to unload.
        let result = self
            .set_grants(context, &[])
            .and(self.control(self.symbols.unload, context));
        if let Err(error) = result {
            let _ = writeln!(io::stderr().lock(), "Quickie: {error}");
        }
    }

    fn set_grants(&self, context: &Owned, patterns: &[CString]) -> Result<(), EngineError> {
        let mut pointers: Vec<_> = patterns.iter().map(|pattern| pattern.as_ptr()).collect();
        pointers.push(ptr::null());
        bool_call(|error| {
            // SAFETY: The context is live, and this is a null-terminated array of live C strings.
            unsafe { (self.symbols.grant)(context.get(), pointers.as_ptr(), error) }
        })
    }
}

impl ExtensionHost for Host {
    type Popup = ();

    fn read(&mut self, id: &ExtensionId, location: &str) -> Result<(), EngineError> {
        let entry = self.entries.entry(id.clone()).or_insert(Slot {
            prepared: None,
            running: None,
        });
        let info = self.symbols.read(location).map(|prepared| {
            let info = prepared.info.clone();
            entry.prepared = Some(prepared);
            info
        });
        if info.is_err() {
            entry.prepared = None;
        }
        self.events.push(ExtensionEvent::Read {
            id: id.clone(),
            info,
        });
        Ok(())
    }

    fn start(&mut self, id: &ExtensionId, access: &Access) -> Result<(), EngineError> {
        let mut entry = self
            .entries
            .remove(id)
            .ok_or_else(|| EngineError("The extension package has not been read.".into()))?;
        let result = self.start_entry(&mut entry, access);
        self.entries.insert(id.clone(), entry);
        result
    }

    fn stop(&mut self, id: &ExtensionId) {
        if let Some(mut slot) = self.entries.remove(id) {
            if let Some(running) = slot.running.take() {
                self.revoke(&running.context);
                if slot.prepared.is_none() {
                    slot.prepared = Some(running);
                }
            }
            self.entries.insert(id.clone(), slot);
        }
    }

    fn discard(&mut self, id: &ExtensionId) {
        if let Some(slot) = self.entries.get_mut(id) {
            slot.prepared = None;
        }
    }

    fn remove(&mut self, id: &ExtensionId) {
        if let Some(running) = self.entries.remove(id).and_then(|entry| entry.running) {
            self.revoke(&running.context);
        }
    }

    fn perform_action(&mut self, _: &ExtensionId, _: Option<TabId>) -> Result<(), EngineError> {
        Err(EngineError(
            "Extension toolbar actions are unavailable on Linux.".into(),
        ))
    }

    fn perform_command(&mut self, _: &ExtensionId, _: &str) -> Result<(), EngineError> {
        Err(EngineError(
            "Extension commands are unavailable on Linux.".into(),
        ))
    }

    fn answer(&mut self, _: ReplyToken, _: &Grants) {}
    fn opened(&mut self, _: ReplyToken, _: Option<TabId>) {}
    fn arrange(&mut self, _: &[WindowTabs]) {}

    fn drain(&mut self, into: &mut Vec<ExtensionEvent>) {
        into.append(&mut self.events);
    }

    fn set_waker(&mut self, _: Box<dyn Fn()>) {}
    fn popup(&self) -> Option<()> {
        None
    }
    fn close_popup(&self) {}
}

impl Host {
    fn start_entry(&self, slot: &mut Slot, access: &Access) -> Result<(), EngineError> {
        if !access.sites.is_empty() {
            return Err(EngineError(
                "Per-site extension overrides are unavailable on Linux.".into(),
            ));
        }
        let prepared = slot
            .prepared
            .as_ref()
            .or(slot.running.as_ref())
            .ok_or_else(|| EngineError("The extension package has not been read.".into()))?;
        // A restored package can have changed. Only grants still declared by the package
        // and previously approved by the user can reach the engine.
        let patterns = prepared
            .info
            .requested
            .patterns
            .intersection(&access.granted.patterns)
            .map(|pattern| CString::new(pattern.as_str()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| EngineError(error.to_string()))?;
        self.set_grants(&prepared.context, &patterns)?;
        if slot.prepared.is_some() {
            // Each prepared context has its own native base URL. Keep the running
            // context until WebKit accepts the replacement.
            self.control(self.symbols.load, &prepared.context)?;
            if let Some(running) = slot.running.take() {
                self.revoke(&running.context);
            }
            slot.running = slot.prepared.take();
        }
        Ok(())
    }
}

fn extension_text(getter: StringProperty, extension: &Owned) -> String {
    // SAFETY: The getter takes the live extension and returns a borrowed C string or null.
    let pointer = unsafe { getter(extension.get()) };
    // SAFETY: The pointer is null or an extension-owned C string and the extension is live.
    unsafe { borrowed_text(pointer) }
}

unsafe fn borrowed_text(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: The caller guarantees that the non-null string is live and terminated.
    unsafe { CStr::from_ptr(pointer).to_string_lossy().into_owned() }
}

fn object_call(call: impl FnOnce(ErrorSlot) -> Object, failure: &str) -> Result<Owned, String> {
    let mut error = ptr::null_mut();
    let object = Owned::new(call(&raw mut error));
    let detail = take_error(error);
    object.ok_or_else(|| detail.unwrap_or_else(|| failure.to_owned()))
}

fn bool_call(call: impl FnOnce(ErrorSlot) -> Bool) -> Result<(), EngineError> {
    let mut error = ptr::null_mut();
    let ok = call(&raw mut error);
    let detail = take_error(error);
    if ok == ffi::TRUE {
        Ok(())
    } else {
        Err(EngineError(detail.unwrap_or_else(|| {
            "WebKit refused the extension operation.".into()
        })))
    }
}

fn take_error(error: *mut GError) -> Option<String> {
    // SAFETY: A non-null error is caller-owned and valid until freed.
    let detail = unsafe { error.as_ref() }?;
    // SAFETY: This GError holds a live GLib string.
    let message = unsafe { borrowed_text(detail.message) };
    // SAFETY: This releases the GError after copying its message.
    unsafe { g_error_free(error) };
    Some(message)
}

#[derive(Debug)]
struct Owned(NonNull<c_void>);

impl Owned {
    fn new(pointer: Object) -> Option<Self> {
        NonNull::new(pointer).map(Self)
    }
    fn get(&self) -> Object {
        self.0.as_ptr()
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: This value owns one GObject reference.
        unsafe { ffi::g_object_unref(self.get()) };
    }
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_error_free(error: *mut GError);
    fn g_free(pointer: Object);
}
