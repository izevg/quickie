//! A tab-scoped media permission sheet and its pending `WebKitGTK` request.

use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_int, c_uint};

use quickie_application::Permission;
use url::Origin;

use super::super::{App, Widgets, connect, ffi, find, keys, palette};
use super::view::{button, class, column, label, row, set_label, visible};
use ffi::{Bool, Object};

#[derive(Default)]
pub(super) struct Permissions(RefCell<Option<Request>>);

impl Permissions {
    pub(super) fn clear(&self) {
        self.0.take();
    }
}

struct Request {
    web: Object,
    object: Object,
    origin: Origin,
    answered: bool,
}

impl Request {
    fn answer(&mut self, allow: bool) {
        self.answered = true;
        let reply = if allow {
            webkit_permission_request_allow
        } else {
            webkit_permission_request_deny
        };
        // SAFETY: We own a reference to the unanswered permission for this page.
        unsafe { reply(self.object) };
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if !self.answered {
            self.answer(false);
        }
        // SAFETY: Each retained WebKit permission is released once on the GTK thread.
        unsafe { ffi::g_object_unref(self.object) };
    }
}

#[derive(Clone, Copy)]
pub(super) struct View {
    scrim: Object,
    icon: Object,
    title: Object,
    detail: Object,
    block: Object,
}

impl View {
    pub(super) fn new(overlay: Object, data: Object) -> Self {
        // SAFETY: GTK is initialized and the browser overlay adopts the new scrim below.
        let scrim = unsafe { ffi::gtk_overlay_new() };
        class(scrim, c"quickie-permission-scrim");
        let (panel, _, content) = palette::panel_widgets(ffi::VERTICAL, 10);
        class(panel, c"quickie-privacy");
        class(panel, c"quickie-permission");
        let heading = row(content, 12);
        // SAFETY: GTK is initialized, and the row adopts the new symbolic device icon.
        let icon = unsafe { gtk_image_new_from_icon_name(c"camera-video-symbolic".as_ptr()) };
        class(icon, c"quickie-permission-icon");
        // SAFETY: This is a live box and a newly created GtkImage.
        unsafe { ffi::gtk_box_append(heading, icon) };
        let headline = column(heading, 3);
        let title = label(headline, c"", c"quickie-privacy-title");
        let detail = label(headline, c"", c"quickie-privacy-row-title");
        let note = label(
            content,
            c"The request comes from this tab. It may be from an embedded frame.",
            c"quickie-permission-context",
        );
        // SAFETY: The live context label can wrap to this natural character width.
        unsafe { gtk_label_set_max_width_chars(note, 50) };
        let actions = column(content, 7);
        let allow = button(actions, c"1  Allow once");
        let block = button(actions, c"Esc  Block");
        class(block, c"destructive-action");
        visible(scrim, false);
        // SAFETY: The live permission sheet aligns inside the scrim.
        unsafe { gtk_widget_set_halign(panel, ffi::ALIGN_CENTER) };
        // SAFETY: The live permission sheet aligns inside the scrim.
        unsafe { ffi::gtk_widget_set_valign(panel, ffi::ALIGN_CENTER) };
        // SAFETY: The scrim adopts the new permission sheet.
        unsafe { ffi::gtk_overlay_add_overlay(scrim, panel) };
        // SAFETY: The sheet now belongs to the overlay and determines its own size.
        unsafe { gtk_overlay_set_measure_overlay(scrim, panel, ffi::TRUE) };
        // SAFETY: The browser overlay adopts the whole-window scrim.
        unsafe { ffi::gtk_overlay_add_overlay(overlay, scrim) };
        connect(allow, c"clicked", allowed as *const _, data);
        connect(block, c"clicked", denied as *const _, data);
        Self {
            scrim,
            icon,
            title,
            detail,
            block,
        }
    }

    fn show(self, origin: &Origin, permission: Permission) {
        set_label(self.title, &origin.ascii_serialization());
        let symbol = match permission {
            Permission::Microphone => c"audio-input-microphone-symbolic",
            Permission::Camera | Permission::CameraAndMicrophone => c"camera-video-symbolic",
        };
        // SAFETY: The live native image copies the static symbolic icon name.
        unsafe { gtk_image_set_from_icon_name(self.icon, symbol.as_ptr()) };
        set_label(
            self.detail,
            match permission {
                Permission::Camera => "Wants to use your camera",
                Permission::Microphone => "Wants to use your microphone",
                Permission::CameraAndMicrophone => "Wants to use your camera and microphone",
            },
        );
        visible(self.scrim, true);
        // SAFETY: The block button on the live sheet is visible and accepts keyboard focus.
        unsafe { ffi::gtk_widget_grab_focus(self.block) };
    }

    fn hide(self) {
        visible(self.scrim, false);
    }
}

pub(super) fn observe(web: Object, data: Object) {
    // SAFETY: This live WebKit view owns its settings.
    let settings = unsafe { ffi::webkit_web_view_get_settings(web) };
    // SAFETY: Settings are live on the GTK thread. Media still requires a permission decision.
    unsafe { webkit_settings_set_enable_media_stream(settings, ffi::TRUE) };
    connect(web, c"permission-request", requested as *const _, data);
}

pub(super) fn load_requested(web: Object, uri: &CStr) {
    #[cfg(not(all(feature = "gtk-story-media", debug_assertions)))]
    let _ = (web, uri);
    // Only the debug story build substitutes synthetic media devices for native hardware.
    #[cfg(all(feature = "gtk-story-media", debug_assertions))]
    if std::env::var_os("QUICKIE_GTK_STORY_MEDIA").is_some_and(|value| value == "1") {
        let fixture = uri
            .to_str()
            .ok()
            .and_then(|uri| url::Url::parse(uri).ok())
            .is_some_and(|url| url.scheme() == "http" && url.host_str() == Some("127.0.0.1"));
        // SAFETY: This live WebKit view owns its settings.
        let settings = unsafe { ffi::webkit_web_view_get_settings(web) };
        // SAFETY: The settings are live on the GTK thread ahead of navigation.
        unsafe { webkit_settings_set_enable_mock_capture_devices(settings, Bool::from(fixture)) };
    }
}

pub(super) fn dismiss_for(app: &App, web: Object) {
    let same_view = app
        .privacy
        .permissions
        .0
        .borrow()
        .as_ref()
        .is_some_and(|request| request.web == web);
    if same_view && let Some(widgets) = app.widgets.get() {
        answer(app, &widgets, false);
    }
}

pub(super) fn active_changed(app: &App, widgets: &Widgets) {
    let inactive = app
        .privacy
        .permissions
        .0
        .borrow()
        .as_ref()
        .is_some_and(|request| request.web != widgets.view);
    if inactive {
        answer(app, widgets, false);
    }
}

pub(super) fn key(app: &App, widgets: &Widgets, key: c_uint, modifiers: c_uint) -> bool {
    if app.privacy.permissions.0.borrow().is_none() {
        return false;
    }
    match (key, modifiers & keys::SHORTCUT_MODIFIERS) {
        (ffi::KEY_ESCAPE, 0) => answer(app, widgets, false),
        (0x31 | 0xffb1, 0) => answer(app, widgets, true),
        (0xff09 | 0xfe20 | 0x20, 0 | ffi::SHIFT) => return false,
        _ => {}
    }
    true
}

extern "C" fn requested(web: Object, permission: Object, data: Object) -> Bool {
    // SAFETY: The emitting WebKit view and original App are live on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let Some(device) = media(permission) else {
        return ffi::FALSE;
    };
    let widgets = app.widgets.get().filter(|widgets| widgets.view == web);
    let origin = super::address(web)
        .map(|url| url.origin())
        .filter(Origin::is_tuple);
    let idle = app.privacy.permissions.0.borrow().is_none();
    if let (Some(widgets), Some(origin)) = (widgets, origin)
        && idle
    {
        super::close(app, &widgets, false);
        find::hide(app, &widgets, false);
        palette::hide(app, &widgets, false);
        // SAFETY: We take an extra reference to carry this permission past its signal.
        unsafe { g_object_ref(permission) };
        app.privacy.permissions.0.replace(Some(Request {
            web,
            object: permission,
            origin: origin.clone(),
            answered: false,
        }));
        widgets.privacy.permission.show(&origin, device);
    } else {
        // SAFETY: This request is borrowed from the current signal and can be denied immediately.
        unsafe { webkit_permission_request_deny(permission) };
    }
    ffi::TRUE
}

fn media(permission: Object) -> Option<Permission> {
    // SAFETY: The request is live and can be tested against WebKit's GType.
    let kind = unsafe { webkit_user_media_permission_request_get_type() };
    // SAFETY: GType checks the dynamic type before using media-specific functions.
    if unsafe { g_type_check_instance_is_a(permission, kind) } == ffi::FALSE {
        return None;
    }
    // SAFETY: The type check confirmed a live WebKitUserMediaPermissionRequest.
    let audio = unsafe { webkit_user_media_permission_is_for_audio_device(permission) };
    // SAFETY: The type check confirmed a live WebKitUserMediaPermissionRequest.
    let video = unsafe { webkit_user_media_permission_is_for_video_device(permission) };
    // SAFETY: The type check confirmed a live WebKitUserMediaPermissionRequest.
    let display = unsafe { webkit_user_media_permission_is_for_display_device(permission) };
    match (
        audio != ffi::FALSE,
        video != ffi::FALSE,
        display != ffi::FALSE,
    ) {
        (true, true, false) => Some(Permission::CameraAndMicrophone),
        (true, false, false) => Some(Permission::Microphone),
        (false, true, false) => Some(Permission::Camera),
        _ => None,
    }
}

fn answer(app: &App, widgets: &Widgets, allow: bool) {
    let Some(mut request) = app.privacy.permissions.0.take() else {
        return;
    };
    let same_document = request.web == widgets.view
        && super::address(widgets.view).is_some_and(|url| url.origin() == request.origin);
    widgets.privacy.permission.hide();
    request.answer(allow && same_document);
    if same_document {
        // SAFETY: The native WebKit view still belongs to the active page.
        unsafe { ffi::gtk_widget_grab_focus(widgets.view) };
    }
}

extern "C" fn allowed(_: Object, data: Object) {
    // SAFETY: The permission button emits with the original App on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(widgets) = app.widgets.get() {
        answer(app, &widgets, true);
    }
}

extern "C" fn denied(_: Object, data: Object) {
    // SAFETY: The permission button emits with the original App on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    if let Some(widgets) = app.widgets.get() {
        answer(app, &widgets, false);
    }
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn gtk_image_new_from_icon_name(name: *const c_char) -> Object;
    fn gtk_image_set_from_icon_name(image: Object, name: *const c_char);
    fn gtk_label_set_max_width_chars(label: Object, width: c_int);
    fn g_object_ref(object: Object) -> Object;
    fn g_type_check_instance_is_a(object: Object, g_type: usize) -> Bool;
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_overlay_set_measure_overlay(overlay: Object, widget: Object, measure: Bool);
    fn gtk_widget_set_halign(widget: Object, align: c_int);
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_settings_set_enable_media_stream(settings: Object, enabled: Bool);
    #[cfg(all(feature = "gtk-story-media", debug_assertions))]
    fn webkit_settings_set_enable_mock_capture_devices(settings: Object, enabled: Bool);
    fn webkit_permission_request_allow(request: Object);
    fn webkit_permission_request_deny(request: Object);
    fn webkit_user_media_permission_request_get_type() -> usize;
    fn webkit_user_media_permission_is_for_audio_device(request: Object) -> Bool;
    fn webkit_user_media_permission_is_for_video_device(request: Object) -> Bool;
    fn webkit_user_media_permission_is_for_display_device(request: Object) -> Bool;
}
