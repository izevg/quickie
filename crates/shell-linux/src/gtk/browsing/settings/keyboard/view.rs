//! GTK controls for the shortcut table.

use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

use quickie_application::{BindingLine, BindingSource};

use crate::gtk::ffi::{self, Bool, Object};

const ALIGN_START: c_int = 1;
const ACCESSIBLE_LABEL: c_int = 4;
const ELLIPSIZE_END: c_int = 3;
const COLUMN_WIDTHS: [c_int; 4] = [180, 112, 106, 94];

pub(super) struct View {
    pub(super) root: Object,
    pub(super) search: Object,
    pub(super) list: Object,
    pub(super) open: Object,
    pub(super) reload: Object,
    pub(super) hint: Object,
    problems: Object,
}

impl View {
    pub(super) fn new() -> Self {
        let root = layout(ffi::VERTICAL, 9);
        super::super::view::margins(root, 13, 22, 13, 22);
        append(root, heading(c"Keyboard", c"title-1"));

        let toolbar = layout(ffi::HORIZONTAL, 8);
        // SAFETY: GTK returns a new search entry on its main thread.
        let search = unsafe { gtk_search_entry_new() };
        // SAFETY: The search entry copies the static placeholder.
        unsafe {
            gtk_search_entry_set_placeholder_text(search, c"Search commands and keys".as_ptr());
        };
        // SAFETY: The search widget fills the available width before the file actions.
        unsafe { ffi::gtk_widget_set_hexpand(search, ffi::TRUE) };
        // SAFETY: GtkSearchEntry implements GtkEditable and accepts this width in characters.
        unsafe { ffi::gtk_editable_set_width_chars(search, 16) };
        append(toolbar, search);
        let open = button(toolbar, c"Open keybindings.json");
        let reload = button(toolbar, c"Reload");
        tooltip(reload, "Read keybindings.json again and apply it");
        append(root, toolbar);

        let headers = [c"Command", c"Shortcut", c"When", c"Source"];
        append(root, columns(headers.map(|name| heading(name, c"heading"))));

        // SAFETY: GTK creates a new selection list on this thread.
        let list = unsafe { gtk_list_box_new() };
        // SAFETY: Pointer activation in the table requires a double-click.
        unsafe { gtk_list_box_set_activate_on_single_click(list, ffi::FALSE) };
        // SAFETY: The live list exposes the table's name to assistive technology.
        unsafe {
            gtk_accessible_update_property(
                list,
                ACCESSIBLE_LABEL,
                c"Keyboard shortcuts".as_ptr(),
                -1_i32,
            );
        };
        let empty = heading(c"No matching shortcuts", c"dim-label");
        // SAFETY: The live GtkListBox adopts the new empty-state label.
        unsafe { gtk_list_box_set_placeholder(list, empty) };
        // SAFETY: GTK creates a scroller on this thread.
        let scroll = unsafe { ffi::gtk_scrolled_window_new() };
        // SAFETY: The scroller takes horizontal-never and vertical-automatic policies.
        unsafe { gtk_scrolled_window_set_policy(scroll, 2, 1) };
        // SAFETY: The live scroller fills space between the headers and footer.
        unsafe { gtk_widget_set_vexpand(scroll, ffi::TRUE) };
        // SAFETY: The scroller adopts the live list.
        unsafe { ffi::gtk_scrolled_window_set_child(scroll, list) };
        append(root, scroll);

        let hint = caption(root, c"dim-label");
        let problems = caption(root, c"error");
        Self {
            root,
            search,
            list,
            open,
            reload,
            hint,
            problems,
        }
    }

    pub(super) fn mapped(&self) -> bool {
        // SAFETY: The root remains live while its Settings window receives keys.
        unsafe { gtk_widget_get_mapped(self.root) != ffi::FALSE }
    }

    pub(super) fn list_focus(&self) -> Object {
        // SAFETY: GTK creates a focus controller on this thread.
        let controller = unsafe { gtk_event_controller_focus_new() };
        // SAFETY: The live list adopts and retains the new controller.
        unsafe { ffi::gtk_widget_add_controller(self.list, controller) };
        controller
    }

    pub(super) fn has_focus(&self, widget: Object) -> bool {
        // SAFETY: The pane is mapped to a live GtkRoot when handling a key.
        let root = unsafe { ffi::gtk_widget_get_root(self.root) };
        if root.is_null() {
            return false;
        }
        // SAFETY: The live root can report its borrowed currently focused widget.
        let focus = unsafe { gtk_root_get_focus(root) };
        if focus.is_null() {
            return false;
        }
        if focus == widget {
            return true;
        }
        // SAFETY: Both arguments are live widgets in this hierarchy.
        unsafe { gtk_widget_is_ancestor(focus, widget) != ffi::FALSE }
    }

    pub(super) fn query(&self) -> String {
        // SAFETY: The live search entry implements GtkEditable and owns its text.
        let text = unsafe { ffi::gtk_editable_get_text(self.search) };
        // SAFETY: GtkEditable owns a non-null NUL-terminated string until the next change.
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    }

    pub(super) fn focus_search(&self) {
        // SAFETY: This search entry belongs to the visible Keyboard pane.
        unsafe { ffi::gtk_widget_grab_focus(self.search) };
    }

    pub(super) fn selected(&self) -> Option<usize> {
        // SAFETY: The selected row is borrowed from the live list.
        let row = unsafe { gtk_list_box_get_selected_row(self.list) };
        index(row)
    }

    pub(super) fn focus_row(&self, position: usize) {
        let Some(row) = self.row(position) else {
            return;
        };
        // SAFETY: The live row belongs to this selection list.
        unsafe { gtk_list_box_select_row(self.list, row) };
        // SAFETY: This makes the live row the target for arrow and recording keys.
        unsafe { ffi::gtk_widget_grab_focus(row) };
    }

    fn row(&self, position: usize) -> Option<Object> {
        let position = c_int::try_from(position).ok()?;
        // SAFETY: The list returns its live row for this visible index, or NULL.
        let row = unsafe { gtk_list_box_get_row_at_index(self.list, position) };
        (!row.is_null()).then_some(row)
    }

    pub(super) fn row_keys(&self, position: usize, keys: &str, error: bool) {
        let Some(row) = self.row(position) else {
            return;
        };
        // SAFETY: Every live row in this list owns the columns constructed in append_row.
        let columns = unsafe { gtk_list_box_row_get_child(row) };
        // SAFETY: The columns start with the command label and the shortcut beside it.
        let command = unsafe { gtk_widget_get_first_child(columns) };
        // SAFETY: The command label's following sibling is the live shortcut label.
        let shortcut = unsafe { gtk_widget_get_next_sibling(command) };
        text(shortcut, keys);
        tooltip(shortcut, keys);
        set_error(shortcut, error);
    }

    pub(super) fn clear_rows(&self) {
        while let Some(row) = self.row(0) {
            // SAFETY: This row belongs to the live list and is removed once.
            unsafe { gtk_list_box_remove(self.list, row) };
        }
    }

    pub(super) fn append_row(&self, line: &BindingLine, available: bool) {
        let keys = line.keys.as_deref().unwrap_or("—");
        let condition = line.when.as_deref().unwrap_or("—");
        let source = match (&line.conflict, line.source, available) {
            (Some(_), _, _) => "Conflict",
            (_, _, false) => "Unavailable",
            (_, BindingSource::Default, true) => "Default",
            (_, BindingSource::User, true) => "User",
        };
        let detail = line
            .conflict
            .as_ref()
            .map(|command| format!("Already runs {command}"));
        let fields = [line.title, keys, condition, source];
        let cells = fields.map(|field| {
            // SAFETY: GTK constructs a new blank label for the row.
            let label = unsafe { ffi::gtk_label_new(ptr::null()) };
            text(label, field);
            configure_label(label);
            tooltip(label, field);
            label
        });
        // SAFETY: The live keycap label can adopt GTK's monospace style.
        unsafe { ffi::gtk_widget_add_css_class(cells[1], c"monospace".as_ptr()) };
        if detail.is_some() {
            // SAFETY: This classes the live source label with GTK's error tone.
            unsafe { ffi::gtk_widget_add_css_class(cells[3], c"error".as_ptr()) };
        }
        // SAFETY: GTK initializes a new row on the main thread.
        let row = unsafe { gtk_list_box_row_new() };
        // SAFETY: The row adopts the newly constructed columns.
        unsafe { gtk_list_box_row_set_child(row, columns(cells)) };
        let accessible = detail.as_ref().map_or_else(
            || fields.join(", "),
            |detail| format!("{}, {detail}", fields.join(", ")),
        );
        let accessible = c_text(&accessible);
        // SAFETY: GTK copies the accessible text onto the live row.
        unsafe {
            gtk_accessible_update_property(row, ACCESSIBLE_LABEL, accessible.as_ptr(), -1_i32);
        };
        if let Some(detail) = detail {
            tooltip(row, &detail);
        }
        // SAFETY: The live list adopts its new row.
        unsafe { gtk_list_box_append(self.list, row) };
    }

    pub(super) fn hint(&self, label: &str, error: bool) {
        text(self.hint, label);
        set_error(self.hint, error);
    }

    pub(super) fn problems(&self, file: &str, entries: &[String]) {
        tooltip(self.open, file);
        let summary = match entries.len() {
            0 => String::new(),
            1 => "1 entry of keybindings.json is left out".to_owned(),
            count => format!("{count} entries of keybindings.json are left out"),
        };
        text(self.problems, &summary);
        tooltip(self.problems, &entries.join("\n"));
        // SAFETY: The live label occupies space only when it displays a problem.
        unsafe { ffi::gtk_widget_set_visible(self.problems, Bool::from(!entries.is_empty())) };
    }

    pub(super) fn working(&self, active: bool) {
        for button in [self.open, self.reload] {
            // SAFETY: The live buttons can only request one file operation at a time.
            unsafe { ffi::gtk_widget_set_sensitive(button, Bool::from(!active)) };
        }
    }
}

fn set_error(label: Object, error: bool) {
    if error {
        // SAFETY: GTK applies its error tone to the live label.
        unsafe { ffi::gtk_widget_add_css_class(label, c"error".as_ptr()) };
    } else {
        // SAFETY: The live label returns to its normal tone.
        unsafe { gtk_widget_remove_css_class(label, c"error".as_ptr()) };
    }
}

pub(super) fn index(row: Object) -> Option<usize> {
    if row.is_null() {
        return None;
    }
    // SAFETY: GTK passes a live row of this selection list.
    usize::try_from(unsafe { gtk_list_box_row_get_index(row) }).ok()
}

fn columns(labels: [Object; 4]) -> Object {
    let row = layout(ffi::HORIZONTAL, 10);
    super::super::view::margins(row, 4, 8, 4, 8);
    for (label, width) in labels.into_iter().zip(COLUMN_WIDTHS) {
        // SAFETY: A small natural width lets each column keep its explicit size request.
        unsafe { gtk_label_set_max_width_chars(label, 1) };
        // SAFETY: The new label accepts a positive width.
        unsafe { gtk_widget_set_size_request(label, width, 24) };
        append(row, label);
    }
    row
}

fn heading(name: &CStr, class: &CStr) -> Object {
    // SAFETY: GTK initializes the label and copies its text.
    let label = unsafe { ffi::gtk_label_new(name.as_ptr()) };
    configure_label(label);
    // SAFETY: GTK copies the static style class onto the new label.
    unsafe { ffi::gtk_widget_add_css_class(label, class.as_ptr()) };
    label
}

fn configure_label(label: Object) {
    // SAFETY: The live label aligns its text to the leading edge.
    unsafe { ffi::gtk_label_set_xalign(label, 0.0) };
    // SAFETY: GTK uses the valid Pango ellipsize-end mode.
    unsafe { gtk_label_set_ellipsize(label, ELLIPSIZE_END) };
}

fn caption(parent: Object, class: &CStr) -> Object {
    let label = heading(c"", class);
    // SAFETY: The live label can wrap the longer recording and refusal messages.
    unsafe { ffi::gtk_label_set_wrap(label, ffi::TRUE) };
    // SAFETY: This caption is left aligned within the pane.
    unsafe { gtk_widget_set_halign(label, ALIGN_START) };
    append(parent, label);
    label
}

fn button(parent: Object, name: &CStr) -> Object {
    // SAFETY: GTK creates the control on its main thread and copies its label.
    let button = unsafe { ffi::gtk_button_new_with_label(name.as_ptr()) };
    append(parent, button);
    button
}

fn text(label: Object, value: &str) {
    let value = c_text(value);
    // SAFETY: GTK copies plain text onto the live native label.
    unsafe { ffi::gtk_label_set_text(label, value.as_ptr()) };
}

fn tooltip(widget: Object, value: &str) {
    let value = c_text(value);
    // SAFETY: GTK copies the tooltip or clears it for an empty value.
    unsafe { gtk_widget_set_tooltip_text(widget, value.as_ptr()) };
}

fn c_text(value: &str) -> CString {
    CString::new(value.replace('\0', "�")).unwrap_or_default()
}

fn layout(direction: c_int, gap: c_int) -> Object {
    // SAFETY: GTK constructs the native box on this thread.
    unsafe { ffi::gtk_box_new(direction, gap) }
}

fn append(parent: Object, widget: Object) {
    // SAFETY: The live parent adopts the newly created child.
    unsafe { ffi::gtk_box_append(parent, widget) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_accessible_update_property(accessible: Object, first: c_int, ...);
    fn gtk_event_controller_focus_new() -> Object;
    fn gtk_label_set_ellipsize(label: Object, mode: c_int);
    fn gtk_label_set_max_width_chars(label: Object, chars: c_int);
    fn gtk_list_box_append(list: Object, row: Object);
    fn gtk_list_box_get_row_at_index(list: Object, index: c_int) -> Object;
    fn gtk_list_box_get_selected_row(list: Object) -> Object;
    fn gtk_list_box_new() -> Object;
    fn gtk_list_box_remove(list: Object, child: Object);
    fn gtk_list_box_row_get_child(row: Object) -> Object;
    fn gtk_list_box_row_get_index(row: Object) -> c_int;
    fn gtk_list_box_row_new() -> Object;
    fn gtk_list_box_row_set_child(row: Object, child: Object);
    fn gtk_list_box_select_row(list: Object, row: Object);
    fn gtk_list_box_set_activate_on_single_click(list: Object, enabled: Bool);
    fn gtk_list_box_set_placeholder(list: Object, placeholder: Object);
    fn gtk_root_get_focus(root: Object) -> Object;
    fn gtk_scrolled_window_set_policy(scroll: Object, horizontal: c_int, vertical: c_int);
    fn gtk_search_entry_new() -> Object;
    fn gtk_search_entry_set_placeholder_text(search: Object, text: *const c_char);
    fn gtk_widget_get_first_child(widget: Object) -> Object;
    fn gtk_widget_get_next_sibling(widget: Object) -> Object;
    fn gtk_widget_get_mapped(widget: Object) -> Bool;
    fn gtk_widget_is_ancestor(widget: Object, ancestor: Object) -> Bool;
    fn gtk_widget_remove_css_class(widget: Object, name: *const c_char);
    fn gtk_widget_set_halign(widget: Object, align: c_int);
    fn gtk_widget_set_size_request(widget: Object, width: c_int, height: c_int);
    fn gtk_widget_set_tooltip_text(widget: Object, text: *const c_char);
    fn gtk_widget_set_vexpand(widget: Object, expand: Bool);
}
