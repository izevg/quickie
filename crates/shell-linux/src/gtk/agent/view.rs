//! The persistent native transcript, composer, and endpoint form.

use std::ffi::{CStr, CString, c_char, c_int, c_uint};

use crate::gtk::ffi::{self, Bool, Object};

const PANEL_WIDTH: c_int = 364;
const WRAP_WORD_CHAR: c_int = 2;
const POLICY_AUTOMATIC: c_int = 1;
const POLICY_NEVER: c_int = 2;
const STYLE_APPLICATION: c_uint = 600;
const ELLIPSIZE_END: c_int = 3;

#[derive(Clone, Copy)]
pub(in crate::gtk) struct View {
    pub(super) panel: Object,
    workspace: Object,
    transcript: Object,
    activity: Object,
    source: Object,
    pub(super) composer: Object,
    pub(super) submit: Object,
    pub(super) stop: Object,
    pub(super) close: Object,
    pub(super) settings: Object,
    pub(super) setup: Object,
    pub(super) form: EndpointForm,
}

#[derive(Clone, Copy)]
pub(super) struct EndpointForm {
    panel: Object,
    base: Object,
    model: Object,
    key: Object,
    status: Object,
    pub(super) save: Object,
    pub(super) remove: Object,
    pub(super) back: Object,
}

impl View {
    pub(super) fn new(parent: Object) -> Self {
        install_theme(parent);
        let panel = column(0);
        class(panel, c"quickie-agent");
        let workspace = column(12);
        margins(workspace, 16);
        let header = row(6);
        let title = label(c"Agent");
        class(title, c"agent-heading");
        expand_width(title);
        append(header, title);
        let settings = button(header, c"Settings");
        let close = button(header, c"Close");
        append(workspace, header);
        let source = label(c"Current page");
        class(source, c"agent-source");
        append(workspace, source);

        let transcript = column(10);
        let scroll = scroller(transcript);
        append(workspace, scroll);
        let activity = label(c"");
        append(workspace, activity);
        let setup = button(workspace, c"Agent Settings…");
        let composer = entry(c"Message the agent about this page");
        append(workspace, composer);
        let controls = row(6);
        let submit = button(controls, c"Send");
        class(submit, c"suggested-action");
        let stop = button(controls, c"Stop");
        append(workspace, controls);
        append(panel, workspace);

        let form = EndpointForm::new();
        append(panel, form.panel);
        show(form.panel, false);
        show(panel, false);
        show(stop, false);
        show(setup, false);
        // SAFETY: The live panel leaves extra width to the expanding browser.
        unsafe { ffi::gtk_widget_set_hexpand(panel, ffi::FALSE) };
        // SAFETY: The live panel has a fixed minimum width and its natural minimum height.
        unsafe { gtk_widget_set_size_request(panel, PANEL_WIDTH, -1) };
        append(parent, panel);
        Self {
            panel,
            workspace,
            transcript,
            activity,
            source,
            composer,
            submit,
            stop,
            close,
            settings,
            setup,
            form,
        }
    }

    pub(super) fn focus(self) {
        focus(self.composer);
    }

    pub(super) fn show(self, visible: bool) {
        show(self.panel, visible);
    }

    pub(super) fn show_settings(self, visible: bool) {
        show(self.workspace, !visible);
        show(self.form.panel, visible);
        if visible {
            self.form.focus();
        } else {
            self.form.clear_key();
            self.focus();
        }
    }

    pub(super) fn source(self, source: &str) {
        set_label(self.source, source);
    }

    pub(super) fn activity(self, message: &str, busy: bool, configured: bool) {
        set_label(self.activity, message);
        show(self.activity, !message.is_empty());
        show(self.stop, busy);
        sensitive(self.submit, configured && !busy);
        show(self.setup, !configured);
    }

    pub(super) fn draft(self) -> String {
        editable_text(self.composer)
    }

    pub(super) fn set_draft(self, draft: &str) {
        set_entry(self.composer, draft);
    }

    pub(super) fn clear(self) {
        loop {
            // SAFETY: This is the live GtkBox owning the transcript rows.
            let child = unsafe { gtk_widget_get_first_child(self.transcript) };
            if child.is_null() {
                break;
            }
            // SAFETY: The child still belongs to this live transcript box.
            unsafe { gtk_box_remove(self.transcript, child) };
        }
    }

    pub(super) fn row(self, text: &str, style: &CStr) -> Object {
        let widget = label(c"");
        class(widget, c"agent-row");
        class(widget, style);
        // SAFETY: This label belongs to the transcript and may be selected and copied.
        unsafe { ffi::gtk_label_set_selectable(widget, ffi::TRUE) };
        set_label(widget, text);
        append(self.transcript, widget);
        widget
    }

    pub(super) fn source_chip(self, quote: &str) -> Object {
        let chip = button(self.transcript, c"Source");
        let label = label(c"");
        set_label(label, &format!("“{quote}”"));
        // SAFETY: The live GtkLabel is a single line inside the chip.
        unsafe { ffi::gtk_label_set_wrap(label, ffi::FALSE) };
        // SAFETY: The live label accepts this Pango ellipsize mode.
        unsafe { gtk_label_set_ellipsize(label, ELLIPSIZE_END) };
        // SAFETY: The live label caps the width it requests from the transcript.
        unsafe { gtk_label_set_max_width_chars(label, 36) };
        // SAFETY: The chip adopts the new label.
        unsafe { gtk_button_set_child(chip, label) };
        class(chip, c"agent-citation");
        chip
    }
}

impl EndpointForm {
    fn new() -> Self {
        let panel = column(10);
        margins(panel, 16);
        let header = row(6);
        let title = label(c"Agent Settings");
        expand_width(title);
        class(title, c"agent-heading");
        append(header, title);
        let back = button(header, c"Back");
        append(panel, header);

        let base = field(panel, c"Endpoint", c"https://api.example.com/v1");
        let model = field(panel, c"Model", c"Model ID");
        // SAFETY: GTK is initialized. The password entry obscures text and has no reveal icon.
        let key = unsafe { gtk_password_entry_new() };
        let key_label = label(c"API key (optional)");
        // SAFETY: The newly created label and password entry are live.
        unsafe { gtk_label_set_mnemonic_widget(key_label, key) };
        append(panel, key_label);
        append(panel, key);
        let controls = row(6);
        let save = button(controls, c"Save");
        class(save, c"suggested-action");
        let remove = button(controls, c"Remove saved key");
        append(panel, controls);
        let status = label(c"");
        append(panel, status);
        Self {
            panel,
            base,
            model,
            key,
            status,
            save,
            remove,
            back,
        }
    }

    pub(super) fn set(self, base: &str, model: &str, has_key: bool) {
        set_entry(self.base, base);
        set_entry(self.model, model);
        self.clear_key();
        show(self.remove, has_key);
    }

    pub(super) fn values(self) -> (String, String, String) {
        (
            editable_text(self.base),
            editable_text(self.model),
            editable_text(self.key),
        )
    }

    pub(super) fn clear_key(self) {
        set_entry(self.key, "");
    }

    pub(super) fn focus(self) {
        focus(self.base);
    }

    pub(super) fn status(self, message: &str, busy: bool) {
        set_label(self.status, message);
        for widget in [self.base, self.model, self.key, self.save, self.remove] {
            sensitive(widget, !busy);
        }
    }
}

fn install_theme(widget: Object) {
    let Ok(css) = CString::new(include_str!("panel.css")) else {
        return;
    };
    // SAFETY: GTK is initialized and the live widget has a display.
    let display = unsafe { gtk_widget_get_display(widget) };
    // SAFETY: GTK creates a new provider on its own thread.
    let provider = unsafe { gtk_css_provider_new() };
    // SAFETY: The provider copies the NUL-terminated CSS.
    unsafe { gtk_css_provider_load_from_string(provider, css.as_ptr()) };
    // SAFETY: The live display retains the provider.
    unsafe { gtk_style_context_add_provider_for_display(display, provider, STYLE_APPLICATION) };
    // SAFETY: The display owns its reference and we release ours.
    unsafe { ffi::g_object_unref(provider) };
}

fn row(spacing: c_int) -> Object {
    // SAFETY: GTK has initialized on the current thread; callers adopt the new box.
    unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, spacing) }
}

fn column(spacing: c_int) -> Object {
    // SAFETY: GTK has initialized on the current thread; callers adopt the new box.
    unsafe { ffi::gtk_box_new(ffi::VERTICAL, spacing) }
}

fn append(parent: Object, child: Object) {
    // SAFETY: The live GtkBox adopts the widget.
    unsafe { ffi::gtk_box_append(parent, child) };
}

fn margins(widget: Object, margin: c_int) {
    // SAFETY: The live widget accepts a leading margin on its owning thread.
    unsafe { ffi::gtk_widget_set_margin_start(widget, margin) };
    // SAFETY: The live widget accepts a trailing margin on its owning thread.
    unsafe { ffi::gtk_widget_set_margin_end(widget, margin) };
    // SAFETY: The live widget accepts a top margin on its owning thread.
    unsafe { ffi::gtk_widget_set_margin_top(widget, margin) };
    // SAFETY: The live widget accepts a bottom margin on its owning thread.
    unsafe { ffi::gtk_widget_set_margin_bottom(widget, margin) };
}

fn class(widget: Object, style: &CStr) {
    // SAFETY: The live widget copies the NUL-terminated style name.
    unsafe { ffi::gtk_widget_add_css_class(widget, style.as_ptr()) };
}

fn show(widget: Object, visible: bool) {
    // SAFETY: The live widget's visibility is set on the GTK owner thread.
    unsafe { ffi::gtk_widget_set_visible(widget, Bool::from(visible)) };
}

fn sensitive(widget: Object, value: bool) {
    // SAFETY: The live widget's sensitivity is set on the GTK owner thread.
    unsafe { ffi::gtk_widget_set_sensitive(widget, Bool::from(value)) };
}

fn focus(widget: Object) {
    // SAFETY: The widget is live and is a focusable child of the window.
    unsafe { ffi::gtk_widget_grab_focus(widget) };
}

pub(super) fn widget_focused(widget: Object) -> bool {
    // SAFETY: The live widget may have a root while attached to the GTK window.
    let root = unsafe { ffi::gtk_widget_get_root(widget) };
    if root.is_null() {
        return false;
    }
    // SAFETY: The live root supplies its nullable focused widget on this thread.
    let current = unsafe { gtk_root_get_focus(root) };
    if current.is_null() || current == widget {
        return current == widget;
    }
    // SAFETY: Both widgets are live for this synchronous ancestor lookup.
    let is_descendant = unsafe { gtk_widget_is_ancestor(current, widget) };
    is_descendant != ffi::FALSE
}

fn expand_width(widget: Object) {
    // SAFETY: The live widget accepts extra width in its parent box.
    unsafe { ffi::gtk_widget_set_hexpand(widget, ffi::TRUE) };
}

fn label(text: &CStr) -> Object {
    // SAFETY: GTK copies the initial text. The caller adopts the new label.
    let label = unsafe { ffi::gtk_label_new(text.as_ptr()) };
    // SAFETY: Zero aligns this live GtkLabel with the left edge.
    unsafe { ffi::gtk_label_set_xalign(label, 0.0) };
    // SAFETY: The live label supports wrapping.
    unsafe { ffi::gtk_label_set_wrap(label, ffi::TRUE) };
    // SAFETY: The live label accepts this Pango wrapping mode.
    unsafe { gtk_label_set_wrap_mode(label, WRAP_WORD_CHAR) };
    label
}

pub(super) fn set_label(label: Object, text: &str) {
    let text = CString::new(text.replace('\0', "�")).unwrap_or_default();
    // SAFETY: The live label copies this plain text synchronously.
    unsafe { ffi::gtk_label_set_text(label, text.as_ptr()) };
}

fn button(parent: Object, text: &CStr) -> Object {
    // SAFETY: GTK copies the button label; parent adopts the new button.
    let button = unsafe { ffi::gtk_button_new_with_label(text.as_ptr()) };
    append(parent, button);
    button
}

fn entry(placeholder: &CStr) -> Object {
    // SAFETY: GTK has initialized and the caller adopts its new entry.
    let entry = unsafe { ffi::gtk_entry_new() };
    // SAFETY: The live entry copies the NUL-terminated placeholder.
    unsafe { ffi::gtk_entry_set_placeholder_text(entry, placeholder.as_ptr()) };
    // SAFETY: The live entry is editable and can shrink to its small width request.
    unsafe { ffi::gtk_editable_set_width_chars(entry, 1) };
    entry
}

fn field(parent: Object, text: &CStr, example: &CStr) -> Object {
    let label = label(text);
    let entry = entry(example);
    // SAFETY: The newly created label and entry are live.
    unsafe { gtk_label_set_mnemonic_widget(label, entry) };
    append(parent, label);
    append(parent, entry);
    entry
}

fn editable_text(entry: Object) -> String {
    // SAFETY: The live entry implements GtkEditable.
    let text = unsafe { ffi::gtk_editable_get_text(entry) };
    // SAFETY: The borrowed NUL-terminated UTF-8 text remains valid until the next edit.
    unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned()
}

fn set_entry(entry: Object, text: &str) {
    let text = CString::new(text.replace('\0', "�")).unwrap_or_default();
    // SAFETY: The live GtkEditable copies this text synchronously.
    unsafe { ffi::gtk_editable_set_text(entry, text.as_ptr()) };
}

fn scroller(child: Object) -> Object {
    // SAFETY: GTK is initialized and the caller adopts its new scroller.
    let scroll = unsafe { ffi::gtk_scrolled_window_new() };
    // SAFETY: The live scroller accepts the named GtkPolicyType values.
    unsafe { gtk_scrolled_window_set_policy(scroll, POLICY_NEVER, POLICY_AUTOMATIC) };
    // SAFETY: The live scroller takes the available height in its box.
    unsafe { gtk_widget_set_vexpand(scroll, ffi::TRUE) };
    // SAFETY: The live scroller adopts the new transcript box.
    unsafe { ffi::gtk_scrolled_window_set_child(scroll, child) };
    // SAFETY: The scroller owns its live vertical adjustment.
    let adjustment = unsafe { gtk_scrolled_window_get_vadjustment(scroll) };
    crate::gtk::connect(
        adjustment,
        c"changed",
        scroll_end as *const _,
        std::ptr::null_mut(),
    );
    scroll
}

extern "C" fn scroll_end(adjustment: Object, _: Object) {
    // SAFETY: The live adjustment emits this signal after the transcript bounds change.
    let upper = unsafe { gtk_adjustment_get_upper(adjustment) };
    // SAFETY: The live adjustment reports the visible transcript height.
    let page = unsafe { gtk_adjustment_get_page_size(adjustment) };
    // SAFETY: The live adjustment accepts this value within its updated scroll range.
    unsafe { gtk_adjustment_set_value(adjustment, (upper - page).max(0.0)) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_css_provider_new() -> Object;
    fn gtk_css_provider_load_from_string(provider: Object, css: *const c_char);
    fn gtk_style_context_add_provider_for_display(
        display: Object,
        provider: Object,
        priority: c_uint,
    );
    fn gtk_widget_get_display(widget: Object) -> Object;
    fn gtk_widget_set_size_request(widget: Object, width: c_int, height: c_int);
    fn gtk_widget_set_vexpand(widget: Object, expand: Bool);
    fn gtk_widget_get_first_child(widget: Object) -> Object;
    fn gtk_widget_is_ancestor(widget: Object, ancestor: Object) -> Bool;
    fn gtk_root_get_focus(root: Object) -> Object;
    fn gtk_box_remove(container: Object, child: Object);
    fn gtk_button_set_child(button: Object, child: Object);
    fn gtk_label_set_wrap_mode(label: Object, mode: c_int);
    fn gtk_label_set_ellipsize(label: Object, mode: c_int);
    fn gtk_label_set_max_width_chars(label: Object, width: c_int);
    fn gtk_label_set_mnemonic_widget(label: Object, target: Object);
    fn gtk_password_entry_new() -> Object;
    fn gtk_scrolled_window_set_policy(scroll: Object, horizontal: c_int, vertical: c_int);
    fn gtk_scrolled_window_get_vadjustment(scroll: Object) -> Object;
    fn gtk_adjustment_get_upper(adjustment: Object) -> f64;
    fn gtk_adjustment_get_page_size(adjustment: Object) -> f64;
    fn gtk_adjustment_set_value(adjustment: Object, value: f64);
}
