//! Native GTK4 modal consent sheet and active-diagnostics indicator for MCP.
//!
//! Reuses the existing `quickie-permission-scrim`, `quickie-privacy`, and `quickie-permission`
//! stylesheets installed by the GTK privacy surface.

#![expect(unsafe_code, reason = "GTK4 widget and signal FFI on the main thread")]

use std::ffi::{CStr, CString, c_int, c_uint};
use std::ptr;

use quickie_application::mcp::GrantScope;

use crate::gtk::ffi::{self, Bool, Object, TRUE};
use crate::gtk::palette;
use crate::mcp::agent::AgentApprovalPrompt;
use crate::mcp::runtime::controller::ConsentPrompt;

const ALIGN_END: c_int = 2;
const ALIGN_START: c_int = 1;
const SHORTCUT_MODIFIERS: c_uint =
    ffi::SHIFT | ffi::CONTROL | ffi::ALT | ffi::SUPER | ffi::HYPER | ffi::META;

#[derive(Clone, Copy)]
pub(super) struct ConsentView {
    scrim: Object,
    origin: Object,
    detail: Object,
    deny: Object,
    banner: Object,
    banner_label: Object,
}

impl ConsentView {
    pub(super) fn new(overlay: Object) -> Self {
        // SAFETY: GTK is initialized on the main thread and the overlay adopts the scrim below.
        let scrim = unsafe { ffi::gtk_overlay_new() };
        add_class(scrim, c"quickie-permission-scrim");

        let (panel, _, content) = palette::panel_widgets(ffi::VERTICAL, 10);
        add_class(panel, c"quickie-privacy");
        add_class(panel, c"quickie-permission");

        let heading = make_box(ffi::VERTICAL, 3);
        // SAFETY: Both boxes are live GTK widgets on the main thread.
        unsafe { ffi::gtk_box_append(content, heading) };
        let title = make_label(c"Allow read-only diagnostics?", c"quickie-privacy-title");
        // SAFETY: The heading box adopts the title label.
        unsafe { ffi::gtk_box_append(heading, title) };
        let origin = make_label(c"", c"quickie-privacy-row-title");
        // SAFETY: The heading box adopts the origin label.
        unsafe { ffi::gtk_box_append(heading, origin) };

        let detail = make_label(c"", c"quickie-permission-context");
        // SAFETY: The live context label wraps cleanly at 50 characters.
        unsafe { gtk_label_set_max_width_chars(detail, 50) };
        // SAFETY: The panel content adopts the detail label.
        unsafe { ffi::gtk_box_append(content, detail) };

        let note = make_label(
            c"Access is read-only and bound to this tab. Switching tabs never transfers it, and opening Web Inspector disconnects it.",
            c"quickie-privacy-muted",
        );
        // SAFETY: The live note label wraps cleanly at 50 characters.
        unsafe { gtk_label_set_max_width_chars(note, 50) };
        // SAFETY: The panel content adopts the note label.
        unsafe { ffi::gtk_box_append(content, note) };

        let actions = make_box(ffi::VERTICAL, 7);
        // SAFETY: The panel content adopts the action button column.
        unsafe { ffi::gtk_box_append(content, actions) };
        let allow = make_button(c"1  Allow");
        add_class(allow, c"suggested-action");
        // SAFETY: The action column adopts the allow button.
        unsafe { ffi::gtk_box_append(actions, allow) };
        let deny = make_button(c"Esc  Deny");
        add_class(deny, c"destructive-action");
        // SAFETY: The action column adopts the deny button.
        unsafe { ffi::gtk_box_append(actions, deny) };

        set_visible(scrim, false);
        // SAFETY: Centers the consent dialog horizontally within the modal scrim.
        unsafe { gtk_widget_set_halign(panel, ffi::ALIGN_CENTER) };
        // SAFETY: Centers the consent dialog vertically within the modal scrim.
        unsafe { ffi::gtk_widget_set_valign(panel, ffi::ALIGN_CENTER) };
        // SAFETY: The scrim adopts the consent dialog.
        unsafe { ffi::gtk_overlay_add_overlay(scrim, panel) };
        // SAFETY: The scrim measures the consent dialog naturally.
        unsafe { gtk_overlay_set_measure_overlay(scrim, panel, TRUE) };
        // SAFETY: The browser overlay adopts the modal scrim.
        unsafe { ffi::gtk_overlay_add_overlay(overlay, scrim) };

        connect_signal(allow, c"clicked", allow_clicked as *const _);
        connect_signal(deny, c"clicked", deny_clicked as *const _);

        // Connected diagnostics banner with explicit Stop control.
        let (banner, _, banner_content) = palette::panel_widgets(ffi::HORIZONTAL, 8);
        add_class(banner, c"quickie-privacy");
        // SAFETY: Positions the active-diagnostics banner at the top-end corner.
        unsafe { gtk_widget_set_halign(banner, ALIGN_END) };
        // SAFETY: Positions the active-diagnostics banner at the top-end corner.
        unsafe { ffi::gtk_widget_set_valign(banner, ALIGN_START) };
        // SAFETY: Keeps spacing from the window edges.
        unsafe { ffi::gtk_widget_set_margin_top(banner, 12) };
        // SAFETY: Keeps spacing from the window edges.
        unsafe { ffi::gtk_widget_set_margin_end(banner, 12) };

        let banner_label = make_label(c"Diagnostics active", c"quickie-privacy-row-title");
        // SAFETY: Centers the status label vertically next to the Stop button.
        unsafe { ffi::gtk_widget_set_valign(banner_label, ffi::ALIGN_CENTER) };
        // SAFETY: The banner content adopts the status label.
        unsafe { ffi::gtk_box_append(banner_content, banner_label) };
        let stop = make_button(c"Stop");
        add_class(stop, c"destructive-action");
        // SAFETY: The banner content adopts the explicit Stop button.
        unsafe { ffi::gtk_box_append(banner_content, stop) };
        connect_signal(stop, c"clicked", stop_clicked as *const _);

        set_visible(banner, false);
        // SAFETY: The browser overlay adopts the active-diagnostics banner.
        unsafe { ffi::gtk_overlay_add_overlay(overlay, banner) };

        Self {
            scrim,
            origin,
            detail,
            deny,
            banner,
            banner_label,
        }
    }

    pub(super) fn show_prompt(self, prompt: &ConsentPrompt) {
        set_label_text(self.origin, &prompt.origin);
        let scope_text = match prompt.scope {
            GrantScope::SingleDocument => {
                "An MCP client wants read-only DOM, text, console, and redacted network diagnostics for the current document only."
            }
            GrantScope::SameOriginReload => {
                "An MCP client wants read-only DOM, text, console, and redacted network diagnostics for this document and same-origin reloads."
            }
        };
        set_label_text(self.detail, scope_text);
        set_visible(self.scrim, true);
        // SAFETY: Default focus rests on Deny so accidental Enter/Space never approves access.
        unsafe { ffi::gtk_widget_grab_focus(self.deny) };
    }

    pub(super) fn show_agent_approval(self, prompt: &AgentApprovalPrompt) {
        set_label_text(self.origin, &prompt.summary);
        set_label_text(self.detail, &prompt.detail);
        set_visible(self.scrim, true);
        // SAFETY: Default focus rests on Deny so accidental Enter/Space never approves access.
        unsafe { ffi::gtk_widget_grab_focus(self.deny) };
    }

    pub(super) fn hide_prompt(self) {
        set_visible(self.scrim, false);
    }

    pub(super) fn set_active_origins(self, origins: &[String]) {
        if origins.is_empty() {
            set_visible(self.banner, false);
            return;
        }
        let summary = if origins.len() == 1 {
            format!("Diagnostics active · {}", origins[0])
        } else {
            format!("Diagnostics active · {} tabs", origins.len())
        };
        set_label_text(self.banner_label, &summary);
        set_visible(self.banner, true);
    }
}

pub(super) fn handle_key(key: c_uint, modifiers: c_uint) -> bool {
    match (key, modifiers & SHORTCUT_MODIFIERS) {
        (ffi::KEY_ESCAPE, 0) => super::resolve_consent(false),
        (0x31 | 0xffb1, 0) => super::resolve_consent(true),
        (0xff09 | 0xfe20 | 0x20, 0 | ffi::SHIFT) => return false,
        _ => {}
    }
    true
}

extern "C" fn allow_clicked(_: Object, _: Object) {
    super::resolve_consent(true);
}

extern "C" fn deny_clicked(_: Object, _: Object) {
    super::resolve_consent(false);
}

extern "C" fn stop_clicked(_: Object, _: Object) {
    super::stop_active_grants();
}

fn connect_signal(instance: Object, signal: &CStr, handler: *const std::ffi::c_void) {
    // SAFETY: The widget belongs to the active GTK window and callbacks run on the main thread.
    unsafe {
        ffi::g_signal_connect_data(instance, signal.as_ptr(), handler, ptr::null_mut(), None, 0);
    }
}

fn make_box(orientation: c_int, spacing: c_int) -> Object {
    // SAFETY: GTK is initialized on the main thread.
    unsafe { ffi::gtk_box_new(orientation, spacing) }
}

fn make_label(text: &CStr, style: &CStr) -> Object {
    // SAFETY: GTK is initialized on the main thread and copies the static NUL-terminated text.
    let label = unsafe { ffi::gtk_label_new(text.as_ptr()) };
    // SAFETY: Word-char wrapping is valid on the live label.
    unsafe { gtk_label_set_wrap_mode(label, 2) };
    // SAFETY: Enables line wrapping on the live label.
    unsafe { ffi::gtk_label_set_wrap(label, TRUE) };
    // SAFETY: Left-aligns the live label.
    unsafe { ffi::gtk_label_set_xalign(label, 0.0) };
    add_class(label, style);
    label
}

fn make_button(text: &CStr) -> Object {
    // SAFETY: GTK is initialized on the main thread and copies the static button label.
    let button = unsafe { ffi::gtk_button_new_with_label(text.as_ptr()) };
    add_class(button, c"quickie-privacy-button");
    button
}

fn add_class(widget: Object, name: &CStr) {
    // SAFETY: GTK copies the static CSS class name onto the live widget.
    unsafe { ffi::gtk_widget_add_css_class(widget, name.as_ptr()) };
}

fn set_visible(widget: Object, visible: bool) {
    // SAFETY: Visibility updates occur strictly on the GTK main thread.
    unsafe { ffi::gtk_widget_set_visible(widget, Bool::from(visible)) };
}

fn set_label_text(label: Object, content: &str) {
    let text = CString::new(content.replace('\0', "")).unwrap_or_default();
    // SAFETY: The live label copies the NUL-terminated string.
    unsafe { ffi::gtk_label_set_text(label, text.as_ptr()) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_label_set_max_width_chars(label: Object, width: c_int);
    fn gtk_label_set_wrap_mode(label: Object, mode: c_int);
    fn gtk_overlay_set_measure_overlay(overlay: Object, widget: Object, measure: Bool);
    fn gtk_widget_set_halign(widget: Object, align: c_int);
}
