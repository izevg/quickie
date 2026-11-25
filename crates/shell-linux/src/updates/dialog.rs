//! The native software updates window, owned by the browser window.

#![expect(unsafe_code, reason = "the update dialog uses GTK's native widget API")]

mod notes;

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr;

use quickie_updater::{UpdateAsset, Version};

use crate::gtk::ffi::{self, Bool, Object};

use super::{Phase, Status, Updates, installed_notes_path};

pub struct Dialog {
    window: Object,
    toggle: Object,
    check: Object,
    status: Object,
    change: ChangeNotes,
    footer: Object,
    cancel: Object,
    action: Object,
}

struct ChangeNotes {
    card: Object,
    title: Object,
    preview: Object,
    all: Object,
    expander: Object,
    link: Object,
    rendered: RefCell<Option<(Version, String)>>,
}

#[derive(Clone, Copy)]
pub struct Actions {
    pub check: extern "C" fn(Object, Object),
    pub toggle: extern "C" fn(Object, Object, Object),
    pub cancel: extern "C" fn(Object, Object),
    pub action: extern "C" fn(Object, Object),
}

pub(crate) fn install_error(error: &str) {
    let Ok(detail) = CString::new(error) else {
        return;
    };
    // SAFETY: The browser initialized GTK on this thread. The loop uses its default context.
    let main_loop = unsafe { ffi::g_main_loop_new(ptr::null_mut(), ffi::FALSE) };
    // SAFETY: GTK copies this static format string, which has no substitutions.
    let alert = unsafe { ffi::gtk_alert_dialog_new(c"Quickie Could Not Update".as_ptr()) };
    // SAFETY: The live alert copies the NUL-terminated detail.
    unsafe { ffi::gtk_alert_dialog_set_detail(alert, detail.as_ptr()) };
    // SAFETY: The alert and loop stay alive until the GTK callback quits the loop.
    unsafe {
        ffi::gtk_alert_dialog_choose(
            alert,
            ptr::null_mut(),
            ptr::null_mut(),
            dismiss_error,
            main_loop,
        );
    }
    // SAFETY: This thread owns the live loop and drives the alert through dismissal.
    unsafe { ffi::g_main_loop_run(main_loop) };
    // SAFETY: The callback returned, and this releases the caller's alert reference.
    unsafe { ffi::g_object_unref(alert) };
    // SAFETY: The callback returned, and this releases the caller's loop reference.
    unsafe { ffi::g_main_loop_unref(main_loop) };
}

extern "C" fn dismiss_error(alert: Object, result: Object, main_loop: Object) {
    // SAFETY: GTK supplies the live alert and its async result.
    unsafe { ffi::gtk_alert_dialog_choose_finish(alert, result, ptr::null_mut()) };
    // SAFETY: The caller retains the loop until this callback returns.
    unsafe { ffi::g_main_loop_quit(main_loop) };
}

impl Dialog {
    pub fn new(
        parent: Object,
        prefix: &Path,
        version: &str,
        data: Object,
        actions: Actions,
    ) -> Self {
        let (window, content) = window(parent);
        let version_row = row(content);
        let version_label = label(version_row);
        set_text(version_label, &format!("Quickie {version}"));
        let check = button(version_row, c"Check for Updates");

        let preference_row = row(content);
        let preference_label = label(preference_row);
        set_text(preference_label, "Automatically update Quickie");
        // SAFETY: GTK is initialized on this thread.
        let toggle = unsafe { ffi::gtk_switch_new() };
        // SAFETY: The new switch is a live widget.
        unsafe { ffi::gtk_widget_set_valign(toggle, ffi::ALIGN_CENTER) };
        append(preference_row, toggle);

        let installed_notes = notes::read(&installed_notes_path(prefix));
        let (installed, _) = expanded_notes(content, c"What's New in Quickie");
        if installed_notes.is_empty() {
            set_text(
                installed,
                "Release notes are unavailable for this installation.",
            );
        } else {
            set_markup(installed, &notes::markup(&installed_notes, false));
        }

        let status = label(content);
        let change = ChangeNotes::new(content);
        let footer = row(content);
        append(footer, change.link);
        let cancel = button(footer, c"Cancel");
        let action = button(footer, c"Download Update");
        // SAFETY: The action is a live GtkButton retained by the footer.
        unsafe { ffi::gtk_widget_add_css_class(action, c"suggested-action".as_ptr()) };
        for (widget, signal, callback) in [
            (check, c"clicked", actions.check as *const _),
            (toggle, c"notify::active", actions.toggle as *const _),
            (cancel, c"clicked", actions.cancel as *const _),
            (action, c"clicked", actions.action as *const _),
        ] {
            crate::gtk::connect(widget, signal, callback, data);
        }
        Self {
            window,
            toggle,
            check,
            status,
            change,
            footer,
            cancel,
            action,
        }
    }

    pub fn present(&self) {
        // SAFETY: The dialog is a live GtkWindow owned by the browser window.
        unsafe { ffi::gtk_window_present(self.window) };
    }

    pub fn render(&self, updates: &Updates) {
        let candidate = match updates.status() {
            Status::Available(candidate) => Some(&candidate.asset),
            Status::Staged(staged) => Some(&staged.candidate.asset),
            _ => None,
        };
        let phase = updates.phase();
        let staged = matches!(updates.status(), Status::Staged(_));
        let status = match (phase, updates.status()) {
            (Some(Phase::Checking), _) => "Checking for updates…",
            (Some(Phase::Downloading), _) => "Downloading and verifying the update…",
            (Some(Phase::Cancelling), _) => "Cancelling the update…",
            (None, Status::Idle) if updates.version().is_none() => {
                "Updates are available for supported release packages."
            }
            (None, Status::Idle) => "",
            (None, Status::Available(_)) => "An update is available.",
            (None, Status::Current) => "Quickie is up to date.",
            (None, Status::Staged(_)) => "The verified update will install when Quickie quits.",
            (None, Status::Cancelled) => "The update was cancelled.",
            (None, Status::Failed(error)) => error.as_str(),
        };
        set_text(self.status, status);
        visible(self.status, !status.is_empty());

        if let Some(asset) = candidate {
            self.change.render(asset);
        }
        visible(self.change.card, candidate.is_some());
        let has_action = candidate.is_some() && phase.is_none();
        let can_cancel = matches!(phase, Some(Phase::Checking | Phase::Downloading));
        visible(self.action, has_action);
        visible(self.change.link, has_action);
        visible(self.cancel, can_cancel);
        visible(self.footer, has_action || can_cancel);
        // SAFETY: The dialog retains the live GtkSwitch during rendering.
        unsafe { ffi::gtk_switch_set_active(self.toggle, flag(updates.automatic())) };
        sensitive(self.toggle, updates.can_set_automatic());
        sensitive(
            self.check,
            updates.version().is_some() && phase.is_none() && !staged,
        );
        let action = if staged {
            c"Update and Restart"
        } else {
            c"Download Update"
        };
        // SAFETY: The live GtkButton copies the static label.
        unsafe { ffi::gtk_button_set_label(self.action, action.as_ptr()) };
    }
}

impl ChangeNotes {
    fn new(parent: Object) -> Self {
        // SAFETY: GTK is initialized on this thread.
        let card = unsafe { ffi::gtk_box_new(ffi::VERTICAL, 9) };
        append(parent, card);
        let title = label(card);
        let preview = label(card);
        let (all, expander) = expanded_notes(card, c"All Changes");
        // SAFETY: The new link widget copies the constant URI and label.
        let link = unsafe {
            ffi::gtk_link_button_new_with_label(
                c"https://github.com/izevg/quickie/releases".as_ptr(),
                c"View on GitHub".as_ptr(),
            )
        };
        Self {
            card,
            title,
            preview,
            all,
            expander,
            link,
            rendered: RefCell::new(None),
        }
    }

    fn render(&self, asset: &UpdateAsset) {
        if self
            .rendered
            .borrow()
            .as_ref()
            .is_some_and(|(version, notes)| version == asset.version() && notes == asset.notes())
        {
            return;
        }
        set_markup(self.title, &format!("<b>Quickie {}</b>", asset.version()));
        set_markup(self.preview, &notes::markup(asset.notes(), true));
        set_markup(self.all, &notes::markup(asset.notes(), false));
        visible(self.preview, !asset.notes().is_empty());
        visible(self.expander, !asset.notes().is_empty());
        if let Ok(uri) = CString::new(format!(
            "https://github.com/izevg/quickie/releases/tag/v{}",
            asset.version()
        )) {
            // SAFETY: The live link button copies the generated URL.
            unsafe { ffi::gtk_link_button_set_uri(self.link, uri.as_ptr()) };
        }
        self.rendered
            .replace(Some((asset.version().clone(), asset.notes().to_owned())));
    }
}

fn window(parent: Object) -> (Object, Object) {
    // SAFETY: GTK is initialized on this thread.
    let window = unsafe { ffi::gtk_window_new() };
    // SAFETY: The live window copies its static title.
    unsafe { ffi::gtk_window_set_title(window, c"Software Updates".as_ptr()) };
    // SAFETY: The live window accepts its default width and default natural height.
    unsafe { ffi::gtk_window_set_default_size(window, 480, -1) };
    // SAFETY: Both windows are live on the same GTK thread.
    unsafe { ffi::gtk_window_set_transient_for(window, parent) };
    // SAFETY: The live window may be made modal for its parent.
    unsafe { ffi::gtk_window_set_modal(window, ffi::TRUE) };
    // SAFETY: This live window remains reusable after the user dismisses it.
    unsafe { ffi::gtk_window_set_hide_on_close(window, ffi::TRUE) };
    // SAFETY: The dialog is destroyed before its parent's callback context is released.
    unsafe { ffi::gtk_window_set_destroy_with_parent(window, ffi::TRUE) };
    // SAFETY: GTK is initialized on this thread.
    let content = unsafe { ffi::gtk_box_new(ffi::VERTICAL, 14) };
    // SAFETY: The new box is a live widget.
    unsafe { ffi::gtk_widget_set_margin_top(content, 20) };
    // SAFETY: The new box is a live widget.
    unsafe { ffi::gtk_widget_set_margin_bottom(content, 20) };
    // SAFETY: The new box is a live widget.
    unsafe { ffi::gtk_widget_set_margin_start(content, 20) };
    // SAFETY: The new box is a live widget.
    unsafe { ffi::gtk_widget_set_margin_end(content, 20) };
    // SAFETY: The live dialog adopts the new box.
    unsafe { ffi::gtk_window_set_child(window, content) };
    crate::gtk::keys::dismiss_on_escape(window);
    (window, content)
}

fn row(parent: Object) -> Object {
    // SAFETY: GTK is initialized on this thread.
    let row = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 10) };
    append(parent, row);
    row
}

fn append(parent: Object, child: Object) {
    // SAFETY: Callers pass a live GtkBox and an unattached child widget, adopted by GTK.
    unsafe { ffi::gtk_box_append(parent, child) };
}

fn label(parent: Object) -> Object {
    let label = new_label();
    append(parent, label);
    label
}

fn new_label() -> Object {
    // SAFETY: The new GtkLabel copies the static empty text.
    let label = unsafe { ffi::gtk_label_new(c"".as_ptr()) };
    // SAFETY: The new label is a live GtkLabel.
    unsafe { ffi::gtk_label_set_wrap(label, ffi::TRUE) };
    // SAFETY: The new label is a live GtkLabel.
    unsafe { ffi::gtk_label_set_xalign(label, 0.0) };
    // SAFETY: The new label is a live widget.
    unsafe { ffi::gtk_widget_set_hexpand(label, ffi::TRUE) };
    label
}

fn button(parent: Object, text: &CStr) -> Object {
    // SAFETY: The new button copies the NUL-terminated label.
    let button = unsafe { ffi::gtk_button_new_with_label(text.as_ptr()) };
    append(parent, button);
    button
}

fn expanded_notes(parent: Object, title: &CStr) -> (Object, Object) {
    let notes = new_label();
    // SAFETY: The newly created notes widget is a live GtkLabel.
    unsafe { ffi::gtk_label_set_selectable(notes, ffi::TRUE) };
    // SAFETY: GTK is initialized on this thread.
    let scroll = unsafe { ffi::gtk_scrolled_window_new() };
    // SAFETY: This is a live GtkScrolledWindow.
    unsafe { ffi::gtk_scrolled_window_set_min_content_height(scroll, 180) };
    // SAFETY: The live scrolled window adopts the new label.
    unsafe { ffi::gtk_scrolled_window_set_child(scroll, notes) };
    // SAFETY: The new expander copies the NUL-terminated title.
    let expander = unsafe { ffi::gtk_expander_new(title.as_ptr()) };
    // SAFETY: The live expander adopts the new scrolled window.
    unsafe { ffi::gtk_expander_set_child(expander, scroll) };
    append(parent, expander);
    (notes, expander)
}

fn set_text(widget: Object, text: &str) {
    if let Ok(text) = CString::new(text) {
        // SAFETY: The live label copies the text from this NUL-terminated string.
        unsafe { ffi::gtk_label_set_text(widget, text.as_ptr()) };
    }
}

fn set_markup(widget: Object, markup: &str) {
    if let Ok(markup) = CString::new(markup) {
        // SAFETY: The live label copies the sanitized and escaped Pango markup.
        unsafe { ffi::gtk_label_set_markup(widget, markup.as_ptr()) };
    }
}

fn visible(widget: Object, state: bool) {
    // SAFETY: All callers use a live widget retained by the dialog.
    unsafe { ffi::gtk_widget_set_visible(widget, flag(state)) };
}

fn sensitive(widget: Object, state: bool) {
    // SAFETY: All callers use a live widget retained by the dialog.
    unsafe { ffi::gtk_widget_set_sensitive(widget, flag(state)) };
}

fn flag(value: bool) -> Bool {
    if value { ffi::TRUE } else { ffi::FALSE }
}
