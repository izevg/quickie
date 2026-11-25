use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::ptr;

use quickie_domain_browsing::{Origin, Transport};

use crate::gtk::ffi::{self, Bool, Object};
use crate::gtk::{backdrop, palette};

const ALIGN_START: c_int = 1;
const CROSSFADE: c_int = 1;
const FADE_MILLIS: c_uint = 120;
const PATH_CHARS: usize = 32;
const APPLICATION_CSS: c_uint = 601;
const ACCESSIBLE_LABEL: c_int = 4;

#[derive(Clone, Copy)]
pub(super) struct View {
    reveal: Object,
    panel: Object,
    picture: Object,
    before: Object,
    host: Object,
    after: Object,
    path: Object,
    icon: Object,
}

impl View {
    pub(super) fn new(overlay: Object) -> Self {
        let (panel, picture, content) = palette::panel_widgets(ffi::HORIZONTAL, 9);
        class(panel, c"quickie-origin-chip");
        install_css(panel);

        // SAFETY: GTK is initialized and the live box takes each new child below.
        let dot = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 0) };
        class(dot, c"quickie-origin-status");
        // SAFETY: Keep the dot at its natural height in the live foreground row.
        unsafe { ffi::gtk_widget_set_valign(dot, ffi::ALIGN_CENTER) };
        append(content, dot);
        // SAFETY: GTK is initialized and content adopts this box below.
        let address = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 0) };
        class(address, c"quickie-origin-address");
        let before = label(address, c"quickie-origin-muted");
        let host = label(address, c"quickie-origin-host");
        let after = label(address, c"quickie-origin-muted");
        append(content, address);
        let path = label(content, c"quickie-origin-path");
        // SAFETY: GTK is initialized and content adopts the new image below.
        let icon = unsafe { gtk_image_new() };
        class(icon, c"quickie-origin-transport");
        append(content, icon);

        // SAFETY: GTK is initialized and the live browser overlay adopts the revealer below.
        let reveal = unsafe { gtk_revealer_new() };
        // SAFETY: The live revealer accepts GTK's center alignment.
        unsafe { gtk_widget_set_halign(reveal, ffi::ALIGN_CENTER) };
        // SAFETY: The live revealer accepts GTK's start alignment.
        unsafe { ffi::gtk_widget_set_valign(reveal, ALIGN_START) };
        // SAFETY: The live revealer accepts a positive top margin.
        unsafe { ffi::gtk_widget_set_margin_top(reveal, 12) };
        // SAFETY: The live revealer accepts GTK's crossfade transition.
        unsafe { gtk_revealer_set_transition_type(reveal, CROSSFADE) };
        // SAFETY: The live revealer accepts the duration in milliseconds.
        unsafe { gtk_revealer_set_transition_duration(reveal, FADE_MILLIS) };
        // SAFETY: The live revealer adopts the detached panel.
        unsafe { gtk_revealer_set_child(reveal, panel) };
        // SAFETY: `panel_widgets` created the panel with a live base child.
        let base = unsafe { gtk_overlay_get_child(panel) };
        pass_through(base);
        pass_through(panel);
        pass_through(content);
        pass_through(reveal);
        // SAFETY: Keep the floating reference to the revealer until `release`.
        unsafe { g_object_ref_sink(reveal) };
        // SAFETY: The live browser overlay parents the retained revealer.
        unsafe { ffi::gtk_overlay_add_overlay(overlay, reveal) };

        Self {
            reveal,
            panel,
            picture,
            before,
            host,
            after,
            path,
            icon,
        }
    }

    pub(super) fn revealer(self) -> Object {
        self.reveal
    }

    pub(super) fn show(self, web_view: Object, origin: &Origin, path: &str) {
        text(self.before, &origin.subdomain);
        text(self.host, &origin.registrable);
        let port = origin
            .port
            .map_or_else(String::new, |port| format!(":{port}"));
        text(self.after, &port);
        let path = shown_path(path);
        text(self.path, path.as_deref().unwrap_or_default());
        let secure = origin.transport == Transport::Secure;
        let (icon, description) = if secure {
            (c"channel-secure-symbolic", c"Secure connection")
        } else {
            (c"dialog-warning-symbolic", c"Not secure")
        };
        if secure {
            // SAFETY: Removing a CSS class from the live panel is valid even when absent.
            unsafe { gtk_widget_remove_css_class(self.panel, c"insecure".as_ptr()) };
        } else {
            class(self.panel, c"insecure");
        }
        // SAFETY: The live GTK image copies the static icon name.
        unsafe { gtk_image_set_from_icon_name(self.icon, icon.as_ptr()) };
        // SAFETY: The live image implements GtkAccessible and the label takes a C string.
        unsafe {
            gtk_accessible_update_property(
                self.icon,
                ACCESSIBLE_LABEL,
                description.as_ptr(),
                -1_i32,
            );
        };
        // SAFETY: The optional path label is live and can toggle its visibility.
        unsafe { ffi::gtk_widget_set_visible(self.path, Bool::from(path.is_some())) };
        // SAFETY: The live revealer animates its current child on the GTK thread.
        unsafe { gtk_revealer_set_reveal_child(self.reveal, ffi::TRUE) };
        backdrop::refresh(self.picture, web_view);
    }

    pub(super) fn fade(self) {
        // SAFETY: The live revealer animates its current child on the GTK thread.
        unsafe { gtk_revealer_set_reveal_child(self.reveal, ffi::FALSE) };
    }

    pub(super) fn hide(self) {
        // SAFETY: A zero-duration transition is valid for the live revealer.
        unsafe { gtk_revealer_set_transition_duration(self.reveal, 0) };
        self.fade();
        // SAFETY: Restore the duration used by subsequent transitions on the live revealer.
        unsafe { gtk_revealer_set_transition_duration(self.reveal, FADE_MILLIS) };
        backdrop::clear(self.picture);
    }

    pub(super) fn clear_faded_backdrop(self) {
        // SAFETY: The getter reads the intended visibility on the live revealer.
        let hiding = unsafe { gtk_revealer_get_reveal_child(self.reveal) } == ffi::FALSE;
        // SAFETY: The getter reads the live revealer synchronously during its signal.
        let hidden =
            hiding && unsafe { gtk_revealer_get_child_revealed(self.reveal) } == ffi::FALSE;
        if hidden {
            backdrop::clear(self.picture);
        }
    }

    pub(super) fn release(self) {
        backdrop::clear(self.picture);
        // SAFETY: Release the revealer reference taken when the chip was attached.
        unsafe { ffi::g_object_unref(self.reveal) };
    }
}

fn class(widget: Object, style: &CStr) {
    // SAFETY: This is a live widget and GTK copies the static class name.
    unsafe { ffi::gtk_widget_add_css_class(widget, style.as_ptr()) };
}

fn append(parent: Object, child: Object) {
    pass_through(child);
    // SAFETY: The live box adopts the detached child.
    unsafe { ffi::gtk_box_append(parent, child) };
}

fn pass_through(widget: Object) {
    // SAFETY: This live widget can stop receiving pointer events.
    unsafe { gtk_widget_set_can_target(widget, ffi::FALSE) };
}

fn label(parent: Object, style: &CStr) -> Object {
    // SAFETY: GTK is initialized and the new label is adopted below.
    let label = unsafe { ffi::gtk_label_new(ptr::null()) };
    class(label, style);
    append(parent, label);
    label
}

fn text(label: Object, value: &str) {
    let value = CString::new(value).unwrap_or_default();
    // SAFETY: The live label copies the prepared plain text.
    unsafe { ffi::gtk_label_set_text(label, value.as_ptr()) };
}

fn shown_path(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    let encoded = CString::new(path).ok()?;
    // SAFETY: GLib reads the NUL-terminated encoded path and returns a new allocation or NULL.
    let decoded = unsafe { g_uri_unescape_string(encoded.as_ptr(), ptr::null()) };
    let value = if decoded.is_null() {
        path.to_owned()
    } else {
        // SAFETY: A successful GLib unescape returns a NUL-terminated allocation.
        let text = unsafe { CStr::from_ptr(decoded) };
        let value = text
            .to_str()
            .ok()
            .filter(|value| !value.chars().any(char::is_control))
            .unwrap_or(path)
            .to_owned();
        // SAFETY: GLib allocated this string, and the Rust copy now owns the display text.
        unsafe { g_free(decoded.cast()) };
        value
    };
    if value.chars().count() <= PATH_CHARS {
        Some(value)
    } else {
        let mut shortened: String = value.chars().take(PATH_CHARS - 1).collect();
        shortened.push('…');
        Some(shortened)
    }
}

fn install_css(panel: Object) {
    let css = concat!(include_str!("chip.css"), "\0");
    // SAFETY: GTK is initialized and the new provider reference belongs to this function.
    let provider = unsafe { gtk_css_provider_new() };
    // SAFETY: The provider copies the static NUL-terminated stylesheet.
    unsafe { gtk_css_provider_load_from_string(provider, css.as_ptr().cast()) };
    // SAFETY: The panel belongs to the live display on the GTK thread.
    let display = unsafe { gtk_widget_get_display(panel) };
    // SAFETY: The live display takes its own reference to the provider.
    unsafe { gtk_style_context_add_provider_for_display(display, provider, APPLICATION_CSS) };
    // SAFETY: Release the constructor reference now that the display retains the provider.
    unsafe { ffi::g_object_unref(provider) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_accessible_update_property(accessible: Object, property: c_int, ...);
    fn gtk_overlay_get_child(overlay: Object) -> Object;
    fn gtk_widget_set_halign(widget: Object, align: c_int);
    fn gtk_widget_set_can_target(widget: Object, enabled: Bool);
    fn gtk_widget_remove_css_class(widget: Object, style: *const c_char);
    fn gtk_widget_get_display(widget: Object) -> Object;
    fn gtk_revealer_new() -> Object;
    fn gtk_revealer_set_child(revealer: Object, child: Object);
    fn gtk_revealer_set_transition_type(revealer: Object, kind: c_int);
    fn gtk_revealer_set_transition_duration(revealer: Object, duration: c_uint);
    fn gtk_revealer_set_reveal_child(revealer: Object, reveal: Bool);
    fn gtk_revealer_get_reveal_child(revealer: Object) -> Bool;
    fn gtk_revealer_get_child_revealed(revealer: Object) -> Bool;
    fn gtk_image_new() -> Object;
    fn gtk_image_set_from_icon_name(widget: Object, name: *const c_char);
    fn gtk_css_provider_new() -> Object;
    fn gtk_css_provider_load_from_string(provider: Object, data: *const c_char);
    fn gtk_style_context_add_provider_for_display(
        display: Object,
        provider: Object,
        priority: c_uint,
    );
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_ref_sink(object: Object) -> Object;
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(allocation: Object);
    fn g_uri_unescape_string(escaped: *const c_char, illegal: *const c_char) -> *mut c_char;
}

#[cfg(test)]
mod tests {
    use super::shown_path;

    #[test]
    fn the_root_has_no_path_to_display() {
        assert_eq!(shown_path("/"), None);
        assert_eq!(shown_path("/docs/").as_deref(), Some("/docs"));
    }

    #[test]
    fn the_path_decodes_utf8_and_preserves_invalid_or_control_escapes() {
        let cyrillic = "/wiki/%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0";

        assert_eq!(shown_path(cyrillic).as_deref(), Some("/wiki/Москва"));
        assert_eq!(shown_path("/bad%FF").as_deref(), Some("/bad%FF"));
        assert_eq!(
            shown_path("/line%0Abreak").as_deref(),
            Some("/line%0Abreak")
        );
        assert_eq!(shown_path("/bad%00").as_deref(), Some("/bad%00"));
    }

    #[test]
    fn a_long_path_keeps_its_character_boundaries() {
        let path = format!("/{}", "п".repeat(40));
        let expected = format!("/{}…", "п".repeat(30));

        assert_eq!(shown_path(&path), Some(expected));
    }
}
