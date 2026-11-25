use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::ptr;

use quickie_domain_shields::FilterListId;

use crate::gtk::{backdrop, ffi, palette};
use ffi::{Bool, Object, TRUE};

const WIDTH: c_int = 420;
const SETTINGS_WIDTH: c_int = 480;
const POLICY_AUTOMATIC: c_int = 1;
const POLICY_NEVER: c_int = 2;
const STYLE_PRIORITY: c_uint = 600;
const ACCESSIBLE_LABEL: c_int = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Panel {
    Shields,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Reload {
    Hidden,
    Pending,
    Ready,
}

#[derive(Clone, Copy)]
pub(in crate::gtk) struct View {
    pub(super) panel: Object,
    pub(super) permission: super::permissions::View,
    picture: Object,
    focus: Object,
    site: Object,
    protection: Object,
    site_rows: Object,
    pub(super) site_switches: [Object; 3],
    no_lists: Object,
    enabled_count: Object,
    list_headings: [Object; 5],
    list_details: [Object; 5],
    pub(super) list_switches: [Object; 5],
    refresh_row: Object,
    pub(super) keep_fresh: Object,
    details: Object,
    pub(super) close: Object,
    pub(super) settings: Object,
    pub(super) enable: Object,
    pub(super) refresh: Object,
    pub(super) reload: Object,
    hint: Object,
}

pub(super) struct Summary {
    pub(super) panel: Panel,
    pub(super) site: Option<(String, [bool; 3])>,
    pub(super) lists: [(bool, String); 5],
    pub(super) keep_fresh: bool,
    pub(super) status: String,
    pub(super) reload: Reload,
    pub(super) busy: bool,
    pub(super) available: bool,
}

impl View {
    pub(super) fn new(overlay: Object, data: Object) -> Self {
        install_style(overlay);
        let (panel, picture, content) = palette::panel_widgets(ffi::VERTICAL, 10);
        class(panel, c"quickie-privacy");
        let header = row(content, 8);
        let heading = column(header, 3);
        expand(heading);
        let site = label(heading, c"Shields", c"quickie-privacy-title");
        let protection = label(heading, c"", c"quickie-privacy-muted");
        let close = button(header, c"Close");

        let body = scroll_body(content);

        let stats = row(body, 8);
        let blocked = tile(stats, c"Blocked");
        set_label(blocked, "—");
        let enabled_count = tile(stats, c"Filter lists");
        let site_rows = column(body, 5);
        label(
            site_rows,
            c"Protections for this site",
            c"quickie-panel-section",
        );
        let site_switches = [
            toggle(
                site_rows,
                c"Shields",
                c"Block trackers and ads with the filter lists",
            )
            .0,
            toggle(
                site_rows,
                c"HTTPS only",
                c"Saved; enforced in a later version",
            )
            .0,
            toggle(
                site_rows,
                c"JavaScript",
                c"Saved; enforced in a later version",
            )
            .0,
        ];
        let filter_group = column(body, 6);
        label(filter_group, c"Filter lists", c"quickie-panel-section");
        let list_names: [CString; 5] = FilterListId::ALL.map(|id| text(id.title()));
        let lists = list_names.each_ref().map(|name| {
            let line = row(filter_group, 8);
            let names = column(line, 2);
            expand(names);
            label(names, name, c"quickie-privacy-row-title");
            let detail = label(names, c"", c"quickie-privacy-muted");
            (line, detail, switch(line, name))
        });
        let no_lists = label(
            filter_group,
            c"Enable filter lists to block ads and trackers.",
            c"quickie-privacy-muted",
        );
        let refresh_row = column(body, 5);
        label(refresh_row, c"Updates", c"quickie-panel-section");
        let keep_fresh = toggle(
            refresh_row,
            c"Keep lists fresh",
            c"Refresh enabled lists once they become stale",
        )
        .0;
        let details = label(body, c"", c"quickie-privacy-muted");
        let actions = row(content, 7);
        let settings = button(actions, c"Settings");
        let enable = button(actions, c"Enable");
        let refresh = button(actions, c"Refresh");
        let reload = button(actions, c"Reload to Apply");
        class(reload, c"suggested-action");
        let hint = label(content, c"Esc  Close", c"quickie-panel-footer");
        visible(panel, false);
        // SAFETY: The live browser overlay adopts this panel.
        unsafe { ffi::gtk_overlay_add_overlay(overlay, panel) };
        Self {
            panel,
            permission: super::permissions::View::new(overlay, data),
            picture,
            focus: close,
            site,
            protection,
            site_rows,
            site_switches,
            no_lists,
            enabled_count,
            list_headings: lists.map(|(line, _, _)| line),
            list_details: lists.map(|(_, detail, _)| detail),
            list_switches: lists.map(|(_, _, toggle)| toggle),
            refresh_row,
            keep_fresh,
            details,
            close,
            settings,
            enable,
            refresh,
            reload,
            hint,
        }
    }

    pub(super) fn render(self, summary: &Summary) {
        let settings = summary.panel == Panel::Settings;
        let count = summary
            .lists
            .iter()
            .filter(|&&(enabled, _)| enabled)
            .count();
        let (site, policy) = summary
            .site
            .as_ref()
            .map_or(("Shields", [false; 3]), |(site, policy)| {
                (site.as_str(), *policy)
            });
        set_label(
            self.site,
            if settings {
                "Settings · Shields"
            } else {
                site
            },
        );
        let protection = if settings {
            "Tracker and ad blocking"
        } else if summary.site.is_none() {
            "Off the web"
        } else if !policy[0] {
            "Shields are down on this site"
        } else if count == 0 {
            "No filter lists enabled"
        } else {
            "Shields are up"
        };
        set_label(self.protection, protection);
        set_label(self.enabled_count, &count.to_string());
        visible(self.site_rows, !settings && summary.site.is_some());
        for (toggle, active) in self.site_switches.into_iter().zip(policy) {
            set_switch(toggle, active, summary.available);
        }
        for (index, (enabled, detail)) in summary.lists.iter().enumerate() {
            visible(self.list_headings[index], settings || *enabled);
            visible(self.list_switches[index], settings);
            set_switch(self.list_switches[index], *enabled, summary.available);
            set_label(self.list_details[index], detail);
        }
        visible(self.refresh_row, settings);
        set_switch(self.keep_fresh, summary.keep_fresh, summary.available);
        set_label(self.details, &summary.status);
        visible(self.details, !summary.status.is_empty());
        visible(self.no_lists, count == 0 && !settings);
        visible(self.settings, !settings);
        visible(self.enable, count == 0 && summary.available);
        visible(self.refresh, count != 0 && summary.available);
        sensitive(self.refresh, !summary.busy);
        visible(self.reload, summary.reload != Reload::Hidden && !settings);
        sensitive(self.reload, summary.reload == Reload::Ready);
        set_label(
            self.hint,
            if summary.reload == Reload::Ready && !settings {
                "↵  Reload     Esc  Close"
            } else {
                "Esc  Close"
            },
        );
    }

    pub(super) fn show(self, web_view: Object) {
        visible(self.panel, true);
        backdrop::refresh(self.picture, web_view);
        // SAFETY: The panel's close control is visible and accepts keyboard focus.
        unsafe { ffi::gtk_widget_grab_focus(self.focus) };
    }

    pub(super) fn hide(self, web_view: Object, restore_focus: bool) {
        backdrop::clear(self.picture);
        visible(self.panel, false);
        if restore_focus {
            // SAFETY: The live web view remains attached to the window while dismissing its surface.
            unsafe { ffi::gtk_widget_grab_focus(web_view) };
        }
    }

    pub(super) fn layout(self, host: Object, panel: Panel, allocation: &mut palette::Allocation) {
        // SAFETY: GTK emits layout for this live overlay with an allocated size.
        let width = unsafe { gtk_widget_get_width(host) };
        // SAFETY: GTK emits layout for this live overlay with an allocated size.
        let height = unsafe { gtk_widget_get_height(host) };
        let margin = 20.min(width / 10);
        let settings = panel == Panel::Settings;
        allocation.width =
            (width - margin * 2).clamp(1, if settings { SETTINGS_WIDTH } else { WIDTH });
        allocation.x = if settings {
            (width - allocation.width) / 2
        } else {
            width - margin - allocation.width
        };
        let mut natural = 0;
        // SAFETY: The panel is live. GTK writes its measured height to the supplied out-pointer.
        unsafe {
            gtk_widget_measure(
                self.panel,
                ffi::VERTICAL,
                allocation.width,
                ptr::null_mut(),
                &raw mut natural,
                ptr::null_mut(),
                ptr::null_mut(),
            );
        }
        allocation.height = natural.clamp(1, (height - 32).max(1));
        allocation.y = if settings {
            ((height - allocation.height) / 2).max(0)
        } else {
            16
        };
    }
}

fn scroll_body(parent: Object) -> Object {
    // SAFETY: GTK is initialized and the live box adopts the scroller below.
    let scroll = unsafe { ffi::gtk_scrolled_window_new() };
    // SAFETY: These policies are valid on the live scroller.
    unsafe { gtk_scrolled_window_set_policy(scroll, POLICY_NEVER, POLICY_AUTOMATIC) };
    // SAFETY: The live scroller should request its natural content height.
    unsafe { gtk_scrolled_window_set_propagate_natural_height(scroll, TRUE) };
    let body = column_widget(10);
    // SAFETY: The native scroller adopts the newly created body.
    unsafe { ffi::gtk_scrolled_window_set_child(scroll, body) };
    append(parent, scroll);
    body
}

fn tile(parent: Object, caption: &CStr) -> Object {
    let section = column(parent, 2);
    class(section, c"quickie-privacy-tile");
    expand(section);
    let value = label(section, c"", c"quickie-privacy-value");
    label(section, caption, c"quickie-privacy-muted");
    value
}

fn toggle(parent: Object, title: &CStr, detail: &CStr) -> (Object, Object) {
    let line = row(parent, 6);
    let names = column(line, 2);
    expand(names);
    label(names, title, c"quickie-privacy-row-title");
    label(names, detail, c"quickie-privacy-muted");
    (switch(line, title), line)
}

fn switch(parent: Object, title: &CStr) -> Object {
    // SAFETY: GTK is initialized and the row adopts the new switch below.
    let toggle = unsafe { ffi::gtk_switch_new() };
    // SAFETY: The live switch is centered vertically inside its row.
    unsafe { ffi::gtk_widget_set_valign(toggle, ffi::ALIGN_CENTER) };
    // SAFETY: GtkSwitch implements GtkAccessible. GTK copies the accessible label.
    unsafe {
        gtk_accessible_update_property(toggle, ACCESSIBLE_LABEL, title.as_ptr(), -1_i32);
    }
    append(parent, toggle);
    toggle
}

pub(super) fn set_label(label: Object, content: &str) {
    let content = text(content);
    // SAFETY: The live native label copies the NUL-terminated text.
    unsafe { ffi::gtk_label_set_text(label, content.as_ptr()) };
}

fn set_switch(toggle: Object, active: bool, sensitive_value: bool) {
    // SAFETY: The live GtkSwitch can update its modeled state.
    unsafe { ffi::gtk_switch_set_active(toggle, Bool::from(active)) };
    sensitive(toggle, sensitive_value);
}

pub(super) fn label(parent: Object, content: &CStr, style: &CStr) -> Object {
    // SAFETY: GTK is initialized and the parent adopts the label below.
    let label = unsafe { ffi::gtk_label_new(content.as_ptr()) };
    // SAFETY: Word-char wrapping is valid for this live GtkLabel.
    unsafe { gtk_label_set_wrap_mode(label, 2) };
    // SAFETY: This live GtkLabel can wrap its text.
    unsafe { ffi::gtk_label_set_wrap(label, TRUE) };
    // SAFETY: The live label supports left-aligned text.
    unsafe { ffi::gtk_label_set_xalign(label, 0.0) };
    class(label, style);
    append(parent, label);
    label
}

pub(super) fn button(parent: Object, title: &CStr) -> Object {
    // SAFETY: GTK is initialized. The native button copies its label.
    let button = unsafe { ffi::gtk_button_new_with_label(title.as_ptr()) };
    class(button, c"quickie-privacy-button");
    append(parent, button);
    button
}

pub(super) fn row(parent: Object, spacing: c_int) -> Object {
    // SAFETY: GTK is initialized and parent adopts the new row below.
    let row = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, spacing) };
    append(parent, row);
    row
}

pub(super) fn column(parent: Object, spacing: c_int) -> Object {
    let column = column_widget(spacing);
    append(parent, column);
    column
}

fn column_widget(spacing: c_int) -> Object {
    // SAFETY: GTK is initialized and transfers the floating widget to the caller.
    unsafe { ffi::gtk_box_new(ffi::VERTICAL, spacing) }
}

fn append(parent: Object, child: Object) {
    // SAFETY: The live box adopts the new native child.
    unsafe { ffi::gtk_box_append(parent, child) };
}

fn expand(widget: Object) {
    // SAFETY: The live widget can expand to occupy available row width.
    unsafe { ffi::gtk_widget_set_hexpand(widget, TRUE) };
}

pub(super) fn class(widget: Object, name: &CStr) {
    // SAFETY: GTK copies the CSS class to the live widget.
    unsafe { ffi::gtk_widget_add_css_class(widget, name.as_ptr()) };
}

pub(super) fn visible(widget: Object, visible: bool) {
    // SAFETY: Visibility changes on the live widget take place on its GTK thread.
    unsafe { ffi::gtk_widget_set_visible(widget, Bool::from(visible)) };
}

fn sensitive(widget: Object, sensitive: bool) {
    // SAFETY: Sensitivity changes on the live widget take place on its GTK thread.
    unsafe { ffi::gtk_widget_set_sensitive(widget, Bool::from(sensitive)) };
}

fn text(content: &str) -> CString {
    CString::new(content.replace('\0', "�")).unwrap_or_default()
}

fn install_style(widget: Object) {
    let Ok(css) = CString::new(include_str!("privacy.css")) else {
        return;
    };
    // SAFETY: The live widget belongs to the display that will own the provider.
    let display = unsafe { gtk_widget_get_display(widget) };
    // SAFETY: GTK is initialized and the caller owns this provider.
    let provider = unsafe { gtk_css_provider_new() };
    // SAFETY: GTK copies the NUL-terminated CSS to the live provider.
    unsafe { gtk_css_provider_load_from_string(provider, css.as_ptr()) };
    // SAFETY: The live display retains a reference to the provider.
    unsafe { gtk_style_context_add_provider_for_display(display, provider, STYLE_PRIORITY) };
    // SAFETY: This releases the caller's reference.
    unsafe { ffi::g_object_unref(provider) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_accessible_update_property(accessible: Object, first: c_int, ...);
    fn gtk_css_provider_new() -> Object;
    fn gtk_css_provider_load_from_string(provider: Object, data: *const c_char);
    fn gtk_style_context_add_provider_for_display(
        display: Object,
        provider: Object,
        priority: c_uint,
    );
    fn gtk_label_set_wrap_mode(label: Object, mode: c_int);
    fn gtk_scrolled_window_set_policy(scroll: Object, horizontal: c_int, vertical: c_int);
    fn gtk_scrolled_window_set_propagate_natural_height(scroll: Object, propagate: Bool);
    fn gtk_widget_get_display(widget: Object) -> Object;
    fn gtk_widget_get_width(widget: Object) -> c_int;
    fn gtk_widget_get_height(widget: Object) -> c_int;
    fn gtk_widget_measure(
        widget: Object,
        orientation: c_int,
        for_size: c_int,
        minimum: *mut c_int,
        natural: *mut c_int,
        minimum_baseline: *mut c_int,
        natural_baseline: *mut c_int,
    );
}
