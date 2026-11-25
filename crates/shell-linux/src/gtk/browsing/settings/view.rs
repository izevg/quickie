//! Native GTK controls for General and the inset page styling.

use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::path::Path;

use crate::gtk::ffi::{self, Bool, Object};
use crate::gtk::{connect, keys};

use super::{
    desktop::BrowserStatus, downloads_clicked, inset_changed, make_default_clicked,
    settings_closed, settings_key_pressed, updates_clicked,
};

const ACCESSIBLE_LABEL: c_int = 4;
const ELLIPSIZE_MIDDLE: c_int = 2;
const OVERFLOW_VISIBLE: c_int = 0;
const OVERFLOW_HIDDEN: c_int = 1;
const STYLE_APPLICATION: c_uint = 600;

#[derive(Clone, Copy)]
pub(super) struct View {
    window: Object,
    browser: Object,
    make_default: Object,
    browser_status: Object,
    downloads: Object,
    open_folder: Object,
    downloads_status: Object,
    inset: Object,
    appearance_status: Object,
}

impl View {
    pub(super) fn new(parent: Object, keyboard: Object, data: Object) -> Self {
        let (window, content) = window(parent, keyboard, data);
        label(content, c"General", c"title-1");
        let default_section = section(content, c"Default browser");
        let default_row = row(default_section, c"web-browser-symbolic");
        let browser = label(default_row, c"", c"");
        let make_default = button(default_row, c"Make Default");
        let browser_status = status(default_section);

        let downloads_section = section(content, c"Downloads");
        let downloads_row = row(downloads_section, c"folder-download-symbolic");
        let downloads = label(downloads_row, c"", c"quickie-general-path");
        // SAFETY: The path stays on a single line so it leaves room for the adjacent button.
        unsafe { ffi::gtk_label_set_wrap(downloads, ffi::FALSE) };
        // SAFETY: The live label accepts its preferred width in characters.
        unsafe { gtk_label_set_max_width_chars(downloads, 40) };
        // SAFETY: Pango middle ellipsizing preserves the final components of the path.
        unsafe { gtk_label_set_ellipsize(downloads, ELLIPSIZE_MIDDLE) };
        let open_folder = button(downloads_row, c"Show in Files");
        let downloads_status = status(downloads_section);

        let appearance_section = section(content, c"Appearance");
        let appearance_row = row(
            appearance_section,
            c"preferences-desktop-appearance-symbolic",
        );
        let inset_labels = layout(ffi::VERTICAL, 2);
        // SAFETY: The column will occupy the space between the icon and switch.
        unsafe { ffi::gtk_widget_set_hexpand(inset_labels, ffi::TRUE) };
        append(appearance_row, inset_labels);
        label(inset_labels, c"Inset page", c"");
        label(
            inset_labels,
            c"Show the page as a card inside the window, framed by the window's own edge",
            c"dim-label",
        );
        // SAFETY: GTK initializes a new switch on this thread.
        let inset = unsafe { ffi::gtk_switch_new() };
        // SAFETY: The switch is live and belongs in the center of its row.
        unsafe { ffi::gtk_widget_set_valign(inset, ffi::ALIGN_CENTER) };
        // SAFETY: GtkSwitch accepts the string-valued accessible label and copies it.
        unsafe {
            gtk_accessible_update_property(inset, ACCESSIBLE_LABEL, c"Inset page".as_ptr(), -1_i32);
        }
        append(appearance_row, inset);
        let appearance_status = status(appearance_section);

        let updates_section = section(content, c"Software Updates");
        let updates_row = row(updates_section, c"software-update-available-symbolic");
        label(updates_row, c"Manage updates and view release notes", c"");
        let updates = button(updates_row, c"Open Updates");

        for (widget, signal, callback) in [
            (make_default, c"clicked", make_default_clicked as *const _),
            (open_folder, c"clicked", downloads_clicked as *const _),
            (inset, c"notify::active", inset_changed as *const _),
            (updates, c"clicked", updates_clicked as *const _),
        ] {
            connect(widget, signal, callback, data);
        }
        Self {
            window,
            browser,
            make_default,
            browser_status,
            downloads,
            open_folder,
            downloads_status,
            inset,
            appearance_status,
        }
    }

    pub(super) fn window(self) -> Object {
        self.window
    }

    pub(super) fn present(self) {
        // SAFETY: The settings window stays live until the browser window is destroyed.
        unsafe { ffi::gtk_window_present(self.window) };
    }

    pub(super) fn set_browser(self, browser: &BrowserStatus) {
        text(self.browser, &browser.description);
        sensitive(self.make_default, browser.can_make_default);
        let help = if browser.installed {
            ""
        } else {
            "Quickie must be registered with this desktop before it can open links by default."
        };
        message(self.browser_status, help);
    }

    pub(super) fn browser_error(self, error: &str) {
        message(self.browser_status, error);
    }

    pub(super) fn set_downloads(self, folder: Option<&Path>) {
        sensitive(self.open_folder, folder.is_some());
        let path = folder.map_or_else(
            || "Downloads folder unavailable".into(),
            Path::to_string_lossy,
        );
        text(self.downloads, &path);
        let tooltip = c_text(&path);
        // SAFETY: GTK copies the tooltip onto the live label.
        unsafe { gtk_widget_set_tooltip_text(self.downloads, tooltip.as_ptr()) };
        self.clear_downloads_error();
    }

    pub(super) fn downloads_status(self) -> Object {
        self.downloads_status
    }

    pub(super) fn clear_downloads_error(self) {
        message(self.downloads_status, "");
    }

    pub(super) fn downloads_error(self, error: &str) {
        message(self.downloads_status, error);
    }

    pub(super) fn set_inset(self, enabled: bool, available: bool, saving: bool) {
        // SAFETY: The active state of the live GTK switch follows the user's saved appearance.
        unsafe { ffi::gtk_switch_set_active(self.inset, Bool::from(enabled)) };
        if saving {
            self.saving();
            return;
        }
        sensitive(self.inset, available);
        self.appearance_error(if available {
            ""
        } else {
            "Quickie could not load the appearance setting."
        });
    }

    pub(super) fn saving(self) {
        sensitive(self.inset, false);
        self.appearance_error("Saving appearance…");
    }

    pub(super) fn appearance_error(self, error: &str) {
        message(self.appearance_status, error);
    }
}

pub(super) fn install_style(window: Object) {
    let css = c_text(include_str!("style.css"));
    // SAFETY: GTK initialized this live window's display.
    let display = unsafe { gtk_widget_get_display(window) };
    // SAFETY: GTK creates a new provider on this thread.
    let provider = unsafe { gtk_css_provider_new() };
    // SAFETY: The live provider copies the application CSS.
    unsafe { gtk_css_provider_load_from_string(provider, css.as_ptr()) };
    // SAFETY: The live display retains the provider for application styles.
    unsafe { gtk_style_context_add_provider_for_display(display, provider, STYLE_APPLICATION) };
    // SAFETY: The display retains the provider after we release our local reference.
    unsafe { ffi::g_object_unref(provider) };
}

pub(super) fn set_framed(frame: Object, inset: bool) {
    let class = c"quickie-page-frame".as_ptr();
    let overflow = if inset {
        // SAFETY: GTK copies the static class name onto the live frame.
        unsafe { ffi::gtk_widget_add_css_class(frame, class) };
        OVERFLOW_HIDDEN
    } else {
        // SAFETY: The live frame permits removing its inset styling.
        unsafe { gtk_widget_remove_css_class(frame, class) };
        OVERFLOW_VISIBLE
    };
    // SAFETY: The live frame takes a valid GtkOverflow value.
    unsafe { gtk_widget_set_overflow(frame, overflow) };
}

pub(super) fn margins(widget: Object, top: c_int, end: c_int, bottom: c_int, start: c_int) {
    // SAFETY: Each of these margin setters updates the allocation of the live widget.
    unsafe { ffi::gtk_widget_set_margin_top(widget, top) };
    // SAFETY: The widget remains live.
    unsafe { ffi::gtk_widget_set_margin_end(widget, end) };
    // SAFETY: The widget remains live.
    unsafe { ffi::gtk_widget_set_margin_bottom(widget, bottom) };
    // SAFETY: The widget remains live.
    unsafe { ffi::gtk_widget_set_margin_start(widget, start) };
}

fn window(parent: Object, keyboard: Object, data: Object) -> (Object, Object) {
    // SAFETY: GTK is initialized on this thread and retains toplevel windows.
    let window = unsafe { ffi::gtk_window_new() };
    // SAFETY: The live window copies its static title.
    unsafe { ffi::gtk_window_set_title(window, c"Quickie Settings".as_ptr()) };
    // SAFETY: The live window accepts these positive initial dimensions.
    unsafe { ffi::gtk_window_set_default_size(window, 600, 452) };
    // SAFETY: Both windows are live throughout the child window's lifetime.
    unsafe { ffi::gtk_window_set_transient_for(window, parent) };
    // SAFETY: This window remains reusable when the user dismisses it.
    unsafe { ffi::gtk_window_set_hide_on_close(window, ffi::TRUE) };
    // SAFETY: GTK destroys this window before the parent's callback context is released.
    unsafe { ffi::gtk_window_set_destroy_with_parent(window, ffi::TRUE) };

    let content = layout(ffi::VERTICAL, 14);
    // SAFETY: GTK copies the static class name onto this live box.
    unsafe { ffi::gtk_widget_add_css_class(content, c"quickie-general".as_ptr()) };
    margins(content, 18, 24, 20, 24);

    // SAFETY: GTK initializes a new scroller on this thread.
    let scroll = unsafe { ffi::gtk_scrolled_window_new() };
    // SAFETY: The scroller takes valid GtkPolicyType values, never and automatic.
    unsafe { gtk_scrolled_window_set_policy(scroll, 2, 1) };
    // SAFETY: The live scroller adopts the settings content.
    unsafe { ffi::gtk_scrolled_window_set_child(scroll, content) };
    // SAFETY: GTK creates an empty stack for the two views.
    let stack = unsafe { gtk_stack_new() };
    // SAFETY: The new stack adopts the live General scroller.
    unsafe { gtk_stack_add_titled(stack, scroll, c"general".as_ptr(), c"General".as_ptr()) };
    // SAFETY: The live stack adopts the new Keyboard pane.
    unsafe { gtk_stack_add_titled(stack, keyboard, c"keyboard".as_ptr(), c"Keyboard".as_ptr()) };
    // SAFETY: GTK creates a live switcher on this thread.
    let switcher = unsafe { gtk_stack_switcher_new() };
    // SAFETY: The switcher tracks the two panes while their window is live.
    unsafe { gtk_stack_switcher_set_stack(switcher, stack) };
    // SAFETY: The switcher occupies the trailing space beside the pane heading.
    unsafe { gtk_widget_set_halign(switcher, 2) };
    // SAFETY: The switcher is aligned to the top while the panes fill the overlay.
    unsafe { ffi::gtk_widget_set_valign(switcher, 1) };
    margins(switcher, 14, 24, 0, 0);
    // SAFETY: GTK creates an overlay for the native pane switcher.
    let overlay = unsafe { ffi::gtk_overlay_new() };
    // SAFETY: The overlay retains both of its newly created children.
    unsafe { ffi::gtk_overlay_set_child(overlay, stack) };
    // SAFETY: The overlay places the live switcher over the child stack.
    unsafe { ffi::gtk_overlay_add_overlay(overlay, switcher) };
    // SAFETY: The live Settings window adopts the overlay.
    unsafe { ffi::gtk_window_set_child(window, overlay) };
    let controller = keys::capture_controller();
    connect(
        controller,
        c"key-pressed",
        settings_key_pressed as *const _,
        data,
    );
    connect(window, c"close-request", settings_closed as *const _, data);
    // SAFETY: The window adopts the live key controller.
    unsafe { ffi::gtk_widget_add_controller(window, controller) };
    (window, content)
}

fn section(parent: Object, title: &CStr) -> Object {
    let section = layout(ffi::VERTICAL, 5);
    label(section, title, c"heading");
    append(parent, section);
    section
}

fn row(parent: Object, icon: &CStr) -> Object {
    let row = layout(ffi::HORIZONTAL, 12);
    // SAFETY: GTK copies the static class name onto the live row.
    unsafe { ffi::gtk_widget_add_css_class(row, c"quickie-general-row".as_ptr()) };
    // SAFETY: GTK creates this icon on the main thread for the row to adopt.
    let icon = unsafe { gtk_image_new_from_icon_name(icon.as_ptr()) };
    append(row, icon);
    append(parent, row);
    row
}

fn label(parent: Object, title: &CStr, class: &CStr) -> Object {
    // SAFETY: GTK creates the label on this thread and copies its static text.
    let label = unsafe { ffi::gtk_label_new(title.as_ptr()) };
    // SAFETY: The live label permits left alignment.
    unsafe { ffi::gtk_label_set_xalign(label, 0.0) };
    // SAFETY: Long settings descriptions can wrap within the window.
    unsafe { ffi::gtk_label_set_wrap(label, ffi::TRUE) };
    // SAFETY: The live label fills the available space up to the row's action.
    unsafe { ffi::gtk_widget_set_hexpand(label, ffi::TRUE) };
    if !class.is_empty() {
        // SAFETY: GTK copies the static class name onto the live label.
        unsafe { ffi::gtk_widget_add_css_class(label, class.as_ptr()) };
    }
    append(parent, label);
    label
}

fn button(parent: Object, title: &CStr) -> Object {
    // SAFETY: GTK creates the button on the main thread and copies its label.
    let button = unsafe { ffi::gtk_button_new_with_label(title.as_ptr()) };
    // SAFETY: The live button is vertically centered alongside its description.
    unsafe { ffi::gtk_widget_set_valign(button, ffi::ALIGN_CENTER) };
    append(parent, button);
    button
}

fn status(parent: Object) -> Object {
    let status = label(parent, c"", c"dim-label");
    message(status, "");
    status
}

pub(super) fn message(label: Object, value: &str) {
    text(label, value);
    // SAFETY: The live label occupies space only when it has a message.
    unsafe { ffi::gtk_widget_set_visible(label, Bool::from(!value.is_empty())) };
}

fn text(label: Object, value: &str) {
    let value = c_text(value);
    // SAFETY: GTK copies plain text onto the live label.
    unsafe { ffi::gtk_label_set_text(label, value.as_ptr()) };
}

fn c_text(value: &str) -> CString {
    CString::new(value.replace('\0', "�")).unwrap_or_default()
}

fn sensitive(widget: Object, enabled: bool) {
    // SAFETY: The live widget accepts its enabled state from the model.
    unsafe { ffi::gtk_widget_set_sensitive(widget, Bool::from(enabled)) };
}

fn append(parent: Object, child: Object) {
    // SAFETY: The live box adopts the new widget.
    unsafe { ffi::gtk_box_append(parent, child) };
}

fn layout(direction: c_int, spacing: c_int) -> Object {
    // SAFETY: GTK initializes the new box on the main thread.
    unsafe { ffi::gtk_box_new(direction, spacing) }
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_accessible_update_property(accessible: Object, first: c_int, ...);
    fn gtk_css_provider_load_from_string(provider: Object, data: *const c_char);
    fn gtk_css_provider_new() -> Object;
    fn gtk_image_new_from_icon_name(name: *const c_char) -> Object;
    fn gtk_label_set_ellipsize(label: Object, mode: c_int);
    fn gtk_label_set_max_width_chars(label: Object, chars: c_int);
    fn gtk_scrolled_window_set_policy(scroll: Object, horizontal: c_int, vertical: c_int);
    fn gtk_stack_new() -> Object;
    fn gtk_stack_add_titled(
        stack: Object,
        child: Object,
        name: *const c_char,
        title: *const c_char,
    ) -> Object;
    fn gtk_stack_switcher_new() -> Object;
    fn gtk_stack_switcher_set_stack(switcher: Object, stack: Object);
    fn gtk_style_context_add_provider_for_display(
        display: Object,
        provider: Object,
        priority: c_uint,
    );
    fn gtk_widget_get_display(widget: Object) -> Object;
    fn gtk_widget_set_halign(widget: Object, alignment: c_int);
    fn gtk_widget_remove_css_class(widget: Object, class: *const c_char);
    fn gtk_widget_set_overflow(widget: Object, overflow: c_int);
    fn gtk_widget_set_tooltip_text(widget: Object, text: *const c_char);
}
