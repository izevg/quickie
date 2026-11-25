//! The Downloads panel uses the same native surface as the palette and tab panels.

use std::ffi::{CStr, CString, c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use quickie_domain_library::{Download, DownloadState};

use crate::gtk::ffi::{self, Bool, Object};
use crate::gtk::{App, backdrop, connect, palette};

pub(super) const ROWS: usize = 6;
const ALIGN_START: c_int = 1;
const ALIGN_END: c_int = 2;

#[derive(Clone, Copy)]
pub(super) struct View {
    window: Object,
    panel: Object,
    picture: Object,
    list: Object,
    empty: Object,
    close: Object,
    clear: Object,
    rows: [Row; ROWS],
}

#[derive(Clone, Copy)]
struct Row {
    widget: Object,
    title: Object,
    status: Object,
    progress: Object,
    action: Object,
}

impl View {
    pub(super) fn new(window: Object, overlay: Object, data: Object) -> Self {
        let (panel, picture, body) = palette::panel_widgets(ffi::VERTICAL, 10);
        let header = append_box(body, ffi::HORIZONTAL, 8);
        let title = label(header, c"Downloads", c"quickie-panel-section");
        // SAFETY: The live title expands to leave its actions at the right of the header.
        unsafe { ffi::gtk_widget_set_hexpand(title, ffi::TRUE) };
        let clear = button(header, c"Clear Finished");
        let close = button(header, c"Close");
        connect(clear, c"clicked", clear_clicked as *const _, data);
        connect(close, c"clicked", close_clicked as *const _, data);
        let empty = label(body, c"No downloads yet", c"quickie-panel-detail");
        // SAFETY: The label owns a small amount of space when the list is empty.
        unsafe { ffi::gtk_widget_set_margin_top(empty, 18) };
        // SAFETY: The empty label will remain inside the panel.
        unsafe { ffi::gtk_widget_set_margin_bottom(empty, 18) };
        // SAFETY: GTK creates a live list box on this thread.
        let list = unsafe { gtk_list_box_new() };
        class(list, c"quickie-panel-list");
        // SAFETY: The list's focus and selection are managed with the row shortcuts.
        unsafe { gtk_list_box_set_activate_on_single_click(list, ffi::FALSE) };
        connect(list, c"row-selected", row_selected as *const _, data);
        let rows = std::array::from_fn(|_| Row::new(list, data));
        // SAFETY: The panel body adopts the new list after the empty state.
        unsafe { ffi::gtk_box_append(body, list) };
        label(
            body,
            c"↑ ↓  Select    Enter  Use selection    Esc  Close",
            c"quickie-panel-footer",
        );
        // SAFETY: These alignments keep the panel at the top right of the page.
        unsafe { gtk_widget_set_halign(panel, ALIGN_END) };
        // SAFETY: The live panel is sized by its contents.
        unsafe { ffi::gtk_widget_set_valign(panel, ALIGN_START) };
        // SAFETY: The live panel is placed with space below the top of the page.
        unsafe { ffi::gtk_widget_set_margin_top(panel, 20) };
        // SAFETY: The live panel is placed with space inside the right page edge.
        unsafe { ffi::gtk_widget_set_margin_end(panel, 24) };
        // SAFETY: The live panel has room for labels and actions in its normal layout.
        unsafe { gtk_widget_set_size_request(panel, 380, -1) };
        visible(panel, false);
        // SAFETY: The live browser overlay adopts this new panel.
        unsafe { ffi::gtk_overlay_add_overlay(overlay, panel) };
        Self {
            window,
            panel,
            picture,
            list,
            empty,
            close,
            clear,
            rows,
        }
    }

    pub(super) fn show(self, browser: Object, has_rows: bool) {
        backdrop::refresh(self.picture, browser);
        visible(self.panel, true);
        if has_rows {
            self.focus_row(0);
        } else {
            // SAFETY: The now-visible Close button is part of the live Downloads panel.
            unsafe { ffi::gtk_widget_grab_focus(self.close) };
        }
    }

    pub(super) fn hide(self) {
        visible(self.panel, false);
        backdrop::clear(self.picture);
    }

    pub(super) fn focus_row(self, index: usize) {
        if let Some(row) = self.rows.get(index) {
            // SAFETY: This live, visible row belongs to the Downloads list.
            unsafe { ffi::gtk_widget_grab_focus(row.widget) };
        }
    }

    pub(super) fn row_focused(self) -> bool {
        // SAFETY: This is the live parent window while its keyboard controller is active.
        let focus = unsafe { gtk_window_get_focus(self.window) };
        self.rows.iter().any(|row| focus == row.widget)
    }

    pub(super) fn render(self, downloads: &[Download], selected: Option<usize>, clearable: bool) {
        visible(self.empty, downloads.is_empty());
        visible(self.list, !downloads.is_empty());
        // SAFETY: This reflects whether the live list contains finished items.
        unsafe { ffi::gtk_widget_set_sensitive(self.clear, Bool::from(clearable)) };
        for (index, row) in self.rows.into_iter().enumerate() {
            let download = downloads.get(index);
            visible(row.widget, download.is_some());
            if let Some(download) = download {
                row.show(download);
            }
        }
        let row = selected.map_or(ptr::null_mut(), |index| self.rows[index].widget);
        // SAFETY: The live list owns the selected row, if there is one.
        unsafe { gtk_list_box_select_row(self.list, row) };
    }

    pub(super) fn reveal(self, path: &Path) {
        let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
            return;
        };
        // SAFETY: GIO copies the local path into a new file reference and does not open it.
        let file = unsafe { g_file_new_for_path(path.as_ptr()) };
        // SAFETY: GTK creates a new launcher that retains the file object.
        let launcher = unsafe { gtk_file_launcher_new(file) };
        // SAFETY: The live window parents the file-manager request. GTK retains the launcher
        // as needed, and no callback needs to access App to show the containing directory.
        unsafe {
            gtk_file_launcher_open_containing_folder(
                launcher,
                self.window,
                ptr::null_mut(),
                None,
                ptr::null_mut(),
            );
        };
        // SAFETY: The request has its own references. Release those taken by the constructors.
        unsafe { ffi::g_object_unref(launcher) };
        // SAFETY: The launcher already retained the file.
        unsafe { ffi::g_object_unref(file) };
    }
}

impl Row {
    fn new(list: Object, data: Object) -> Self {
        // SAFETY: GTK creates a live row on the GUI thread.
        let row = unsafe { gtk_list_box_row_new() };
        class(row, c"quickie-panel-row");
        // SAFETY: The new row adopts the layout below.
        let layout = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 10) };
        // SAFETY: The live row adopts the new layout.
        unsafe { gtk_list_box_row_set_child(row, layout) };
        let content = append_box(layout, ffi::VERTICAL, 3);
        // SAFETY: This column fills the space left of the action button.
        unsafe { ffi::gtk_widget_set_hexpand(content, ffi::TRUE) };
        let title = label(content, c"", c"quickie-panel-title");
        let status = label(content, c"", c"quickie-panel-detail");
        // SAFETY: This progress meter is new and will be adopted by the row's box.
        let progress = unsafe { gtk_progress_bar_new() };
        // SAFETY: The row's column adopts the new progress meter.
        unsafe { ffi::gtk_box_append(content, progress) };
        let action = button(layout, c"Cancel");
        // SAFETY: The action button sits vertically centered alongside the details.
        unsafe { ffi::gtk_widget_set_valign(action, ffi::ALIGN_CENTER) };
        connect(action, c"clicked", action_clicked as *const _, data);
        // SAFETY: The list takes ownership of its newly constructed row.
        unsafe { gtk_list_box_append(list, row) };
        Self {
            widget: row,
            title,
            status,
            progress,
            action,
        }
    }

    fn show(self, download: &Download) {
        set_label(self.title, &download.file_name);
        let (status, action) = match &download.state {
            DownloadState::InProgress { received, total } => {
                self.update_meter(*received, *total);
                let status = total.map_or_else(
                    || format!("Downloading · {} received", bytes(*received)),
                    |total| format!("Downloading · {} of {}", bytes(*received), bytes(total)),
                );
                (status, c"Cancel")
            }
            DownloadState::Finished => ("Downloaded".to_owned(), c"Show in Files"),
            DownloadState::Failed(reason) => (format!("Failed · {reason}"), c"Retry"),
            DownloadState::Cancelled => ("Cancelled".to_owned(), c"Retry"),
        };
        visible(self.progress, super::running(download));
        set_label(self.status, &status);
        // SAFETY: GTK copies the label into the live native action button.
        unsafe { ffi::gtk_button_set_label(self.action, action.as_ptr()) };
    }

    fn update_meter(self, received: u64, total: Option<u64>) {
        if let Some(total) = total.filter(|&total| total != 0) {
            let thousandths = u128::from(received.min(total)) * 1_000 / u128::from(total);
            let fraction = f64::from(u16::try_from(thousandths).unwrap_or(1_000)) / 1_000.0;
            // SAFETY: This live GTK progress bar accepts a fraction from zero to one.
            unsafe { gtk_progress_bar_set_fraction(self.progress, fraction) };
        } else {
            // SAFETY: The live progress bar uses activity mode when the size is unknown.
            unsafe { gtk_progress_bar_pulse(self.progress) };
        }
    }
}

fn bytes(value: u64) -> String {
    match value {
        0..=1023 => format!("{value} B"),
        1024..=1_048_575 => format!("{} KiB", value / 1024),
        _ => format!("{} MiB", value / 1_048_576),
    }
}

extern "C" fn row_selected(_: Object, row: Object, data: Object) {
    // SAFETY: The list selection callback has the registered GTK App reference.
    let app = unsafe { &*data.cast::<App>() };
    let downloads = &app.browsing.downloads;
    let Some(view) = downloads.view.get() else {
        return;
    };
    if let Some(index) = view.rows.iter().position(|widget| widget.widget == row)
        && let Some(download) = downloads.rows().get(index)
    {
        downloads.selected.set(Some(download.id));
    }
}

extern "C" fn action_clicked(button: Object, data: Object) {
    // SAFETY: The Downloads button callback has the App reference on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let downloads = &app.browsing.downloads;
    let Some(view) = downloads.view.get() else {
        return;
    };
    if let Some(index) = view.rows.iter().position(|row| row.action == button)
        && let Some(download) = downloads.rows().get(index)
    {
        downloads.action(app, download.id, true);
    }
}

extern "C" fn clear_clicked(_: Object, data: Object) {
    // SAFETY: The live Downloads panel and its signal share the App reference.
    let downloads = &unsafe { &*data.cast::<App>() }.browsing.downloads;
    downloads.clear();
    downloads.refresh();
}

extern "C" fn close_clicked(_: Object, data: Object) {
    // SAFETY: The live Downloads panel and its signal share the App reference.
    super::hide(unsafe { &*data.cast::<App>() }, true);
}

fn visible(widget: Object, show: bool) {
    // SAFETY: Widget owners retain every argument for the duration of this GTK call.
    unsafe { ffi::gtk_widget_set_visible(widget, Bool::from(show)) };
}

fn class(widget: Object, class: &CStr) {
    // SAFETY: The widget is live and GTK copies the NUL-terminated CSS class.
    unsafe { ffi::gtk_widget_add_css_class(widget, class.as_ptr()) };
}

fn append_box(parent: Object, axis: c_int, spacing: c_int) -> Object {
    // SAFETY: GTK is initialized on this thread.
    let child = unsafe { ffi::gtk_box_new(axis, spacing) };
    // SAFETY: The live parent box adopts the new widget.
    unsafe { ffi::gtk_box_append(parent, child) };
    child
}

fn label(parent: Object, text: &CStr, role: &CStr) -> Object {
    // SAFETY: GTK creates a label and copies its text.
    let label = unsafe { ffi::gtk_label_new(text.as_ptr()) };
    // SAFETY: The live label aligns to the start of its allocation.
    unsafe { ffi::gtk_label_set_xalign(label, 0.0) };
    // SAFETY: The live label can ellipsize long server-provided names.
    unsafe { gtk_label_set_ellipsize(label, 3) };
    class(label, role);
    // SAFETY: The live parent box adopts the label.
    unsafe { ffi::gtk_box_append(parent, label) };
    label
}

fn set_label(label: Object, text: &str) {
    if let Ok(text) = CString::new(text) {
        // SAFETY: GTK copies this plain text and does not interpret it as markup.
        unsafe { ffi::gtk_label_set_text(label, text.as_ptr()) };
    }
}

fn button(parent: Object, text: &CStr) -> Object {
    // SAFETY: GTK initializes the new button with a copy of the label.
    let button = unsafe { ffi::gtk_button_new_with_label(text.as_ptr()) };
    class(button, c"flat");
    // SAFETY: The live parent box adopts this button.
    unsafe { ffi::gtk_box_append(parent, button) };
    button
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_widget_set_halign(widget: Object, align: c_int);
    fn gtk_widget_set_size_request(widget: Object, width: c_int, height: c_int);
    fn gtk_label_set_ellipsize(label: Object, mode: c_int);
    fn gtk_list_box_new() -> Object;
    fn gtk_list_box_set_activate_on_single_click(list: Object, activate: Bool);
    fn gtk_list_box_select_row(list: Object, row: Object);
    fn gtk_list_box_append(list: Object, child: Object);
    fn gtk_list_box_row_new() -> Object;
    fn gtk_list_box_row_set_child(row: Object, child: Object);
    fn gtk_window_get_focus(window: Object) -> Object;
    fn gtk_progress_bar_new() -> Object;
    fn gtk_progress_bar_set_fraction(bar: Object, fraction: f64);
    fn gtk_progress_bar_pulse(bar: Object);
    fn gtk_file_launcher_new(file: Object) -> Object;
    fn gtk_file_launcher_open_containing_folder(
        launcher: Object,
        parent: Object,
        cancellable: Object,
        callback: Option<extern "C" fn(Object, Object, Object)>,
        data: Object,
    );
}

#[link(name = "gio-2.0")]
unsafe extern "C" {
    fn g_file_new_for_path(path: *const c_char) -> Object;
}
