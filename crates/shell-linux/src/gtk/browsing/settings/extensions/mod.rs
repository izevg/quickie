//! The extension controller attached to the ordinary browsing profile.

mod host;

use std::ffi::{CStr, c_char, c_void};
use std::ptr::{self, NonNull};
use std::rc::Rc;

use quickie_application::{ExtensionEvent, ExtensionHost, Extensions, ReadOutcome};
use quickie_domain_commands::Platform;
use quickie_store::PendingWrites;

use crate::gtk::ffi::{self, Object};

use super::super::{SystemClock, report};

const CONTROLLER_PROPERTY: &CStr = c"web-extension-controller";
type NewController = unsafe extern "C" fn(Object) -> Object;

pub(super) struct State {
    manager: Extensions<PendingWrites, host::Host, SystemClock>,
}

impl State {
    pub(super) fn restore(store: PendingWrites) -> Option<(Rc<Profile>, Self)> {
        let profile = Rc::new(Profile::open()?);
        let host = host::Host::open(Rc::clone(&profile))
            .inspect_err(report)
            .ok()?;
        let mut manager = Extensions::start(store, host, SystemClock, Platform::Linux)
            .inspect_err(report)
            .ok()?;
        if let Some(error) = manager.corrupt_on_start() {
            report(error);
        }
        let mut events = Vec::new();
        manager.host_mut().drain(&mut events);
        for event in events {
            if let ExtensionEvent::Read { id, info } = event {
                match manager.read(&id, info) {
                    Ok(ReadOutcome::Unreadable { error, .. }) => report(&error),
                    Err(error) => report(&error),
                    _ => {}
                }
            }
        }
        Some((profile, Self { manager }))
    }

    pub(super) fn stop(mut self) {
        let enabled: Vec<_> = self
            .manager
            .list()
            .iter()
            .filter(|extension| extension.enabled())
            .map(|extension| extension.id().clone())
            .collect();
        for id in enabled {
            self.manager.host_mut().stop(&id);
        }
    }
}

#[derive(Debug)]
pub(in crate::gtk::browsing) struct Profile {
    view_type: usize,
    session: Object,
    controller: NonNull<c_void>,
}

impl Profile {
    fn open() -> Option<Self> {
        // SAFETY: RTLD_DEFAULT searches the loaded WebKitGTK library for the fork's constructor.
        let constructor = unsafe {
            dlsym(
                ptr::null_mut(),
                c"webkit_web_extension_controller_new".as_ptr(),
            )
        };
        if constructor.is_null() {
            return None;
        }
        // SAFETY: GTK is initialized on the main thread and WebKit registers this GObject type.
        let view_type = unsafe { webkit_web_view_get_type() };
        // SAFETY: The type belongs to WebKit and its class reference is owned by this call.
        let class = unsafe { g_type_class_ref(view_type) };
        if class.is_null() {
            return None;
        }
        // SAFETY: The class reference is live; the property name is a static C string.
        let property = unsafe { g_object_class_find_property(class, CONTROLLER_PROPERTY.as_ptr()) };
        // SAFETY: This releases exactly the reference acquired above.
        unsafe { g_type_class_unref(class) };
        if property.is_null() {
            return None;
        }
        // SAFETY: WebKitGTK owns this singleton for the lifetime of the browser process.
        let session = unsafe { webkit_network_session_get_default() };
        if session.is_null() {
            return None;
        }
        // SAFETY: This function has the C signature of the named constructor in the fork's header.
        let open = unsafe { std::mem::transmute::<Object, NewController>(constructor) };
        // SAFETY: The constructor borrows the matching default session and returns an owned ref.
        let controller = NonNull::new(unsafe { open(session) })?;
        Some(Self {
            view_type,
            session,
            controller,
        })
    }

    pub(in crate::gtk::browsing) fn view(&self) -> Object {
        // SAFETY: The default session and controller are live GObjects. WebKit retains them
        // during construction and returns a floating reference to the view.
        unsafe {
            g_object_new(
                self.view_type,
                c"network-session".as_ptr(),
                self.session,
                CONTROLLER_PROPERTY.as_ptr(),
                self.controller.as_ptr(),
                ptr::null::<c_char>(),
            )
        }
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        // SAFETY: The profile owns the reference returned by the GLib constructor.
        unsafe { ffi::g_object_unref(self.controller.as_ptr()) };
    }
}

unsafe extern "C" {
    fn dlsym(handle: Object, name: *const c_char) -> Object;
    fn g_type_class_ref(kind: usize) -> Object;
    fn g_type_class_unref(class: Object);
    fn g_object_class_find_property(class: Object, name: *const c_char) -> Object;
    fn g_object_new(kind: usize, name: *const c_char, ...) -> Object;
    fn webkit_web_view_get_type() -> usize;
    fn webkit_network_session_get_default() -> Object;
}
