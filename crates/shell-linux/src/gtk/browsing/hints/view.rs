use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::ptr;

use quickie_domain_commands::link_hint::{HOME_ROW, labels};
use quickie_engine_webkit::link_hints;

use crate::gtk::ffi::{self, Bool, Object};

const ALIGN_END: c_int = 2;
const STYLE_APPLICATION: c_uint = 600;

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Viewport {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

pub(super) struct View {
    overlay: Object,
    layer: Object,
    bar: Object,
    shift: Object,
    mode: Object,
    badges: Vec<Object>,
}

impl View {
    pub(super) fn new(overlay: Object) -> Self {
        install_css(overlay);
        // SAFETY: GTK is initialized on the browser thread.
        let layer = unsafe { gtk_fixed_new() };
        // SAFETY: The new layer is live and fills the native overlay vertically.
        unsafe { gtk_widget_set_vexpand(layer, ffi::TRUE) };
        // SAFETY: The new layer is live and fills the native overlay horizontally.
        unsafe { ffi::gtk_widget_set_hexpand(layer, ffi::TRUE) };
        // SAFETY: GTK is initialized on the browser thread.
        let bar = unsafe { ffi::gtk_box_new(ffi::HORIZONTAL, 6) };
        styled(bar, c"quickie-link-hints-bar");
        // SAFETY: The new bar is a live widget and is centered horizontally.
        unsafe { gtk_widget_set_halign(bar, ffi::ALIGN_CENTER) };
        // SAFETY: The new bar is positioned at the bottom of the overlay.
        unsafe { ffi::gtk_widget_set_valign(bar, ALIGN_END) };
        // SAFETY: The new bar is live and the margin uses GTK logical pixels.
        unsafe { ffi::gtk_widget_set_margin_bottom(bar, 24) };
        let keycap = Some(c"quickie-panel-keycap");
        let shift = label(c"Shift", keycap);
        let mode = label(c"Background tab", None);
        for child in [
            label(c"ASDF…", keycap),
            label(c"Type a hint", None),
            shift,
            mode,
            label(c"Esc", keycap),
            label(c"Cancel", None),
        ] {
            // SAFETY: The live bar adopts each newly constructed label.
            unsafe { ffi::gtk_box_append(bar, child) };
        }
        // Allocate the empty layer while collecting, so layout changes can cancel a request.
        visible(bar, false);
        for widget in [layer, bar] {
            pass_input(widget);
            // SAFETY: The live overlay adopts the constructed widget.
            unsafe { ffi::gtk_overlay_add_overlay(overlay, widget) };
        }
        Self {
            overlay,
            layer,
            bar,
            shift,
            mode,
            badges: Vec::new(),
        }
    }

    pub(super) fn is_layer(&self, widget: Object) -> bool {
        self.layer == widget
    }

    pub(super) fn viewport(&self, source: Object) -> Option<Viewport> {
        let (mut x, mut y) = (0.0, 0.0);
        // SAFETY: The source and overlay are live widgets in this window. The coordinates
        // returned for their shared layout use GTK logical pixels.
        let translated = unsafe {
            gtk_widget_translate_coordinates(source, self.overlay, 0.0, 0.0, &raw mut x, &raw mut y)
        };
        if translated == ffi::FALSE {
            return None;
        }
        // SAFETY: The active web view is live and participates in the overlay's layout.
        let width = f64::from(unsafe { gtk_widget_get_width(source) });
        // SAFETY: The active web view is live and participates in the overlay's layout.
        let height = f64::from(unsafe { gtk_widget_get_height(source) });
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        Some(Viewport {
            x,
            y,
            width,
            height,
        })
    }

    pub(super) fn show(
        &mut self,
        viewport: Viewport,
        answer: &str,
        background: bool,
    ) -> Option<Vec<String>> {
        self.hide();
        let Viewport {
            x,
            y,
            width,
            height,
        } = viewport;
        let targets = link_hints::targets(answer, width, None);
        let labels = labels(targets.len(), &HOME_ROW);
        if labels.is_empty() {
            return None;
        }
        for (target, code) in targets.iter().zip(&labels) {
            let title = CString::new(code.to_ascii_uppercase()).ok()?;
            let frame = target.frame;
            let badge = label(&title, Some(c"quickie-hint-badge"));
            if !target.link {
                styled(badge, c"quickie-control-hint");
            }
            let badge_width = natural_size(badge, ffi::HORIZONTAL);
            let badge_height = natural_size(badge, ffi::VERTICAL);
            let left = (frame.x - 4.0).clamp(0.0, (width - badge_width).max(0.0));
            let top = (frame.y - 4.0).clamp(0.0, (height - badge_height).max(0.0));
            // SAFETY: The live fixed layer adopts the newly constructed badge.
            unsafe { gtk_fixed_put(self.layer, badge, x + left, y + top) };
            self.badges.push(badge);
        }
        let mode = if background {
            c"Opening in background"
        } else {
            c"Background tab"
        };
        visible(self.shift, !background);
        // SAFETY: The status label stays live while the overlay owns it.
        unsafe { ffi::gtk_label_set_text(self.mode, mode.as_ptr()) };
        visible(self.bar, true);
        Some(labels)
    }

    pub(super) fn restyle(&self, codes: &[String], prefix: &str) {
        let prefix_text = prefix.to_ascii_uppercase();
        for (&badge, code) in self.badges.iter().zip(codes) {
            let suffix = code.strip_prefix(prefix);
            visible(badge, suffix.is_some());
            if let Some(suffix) = suffix
                && let Ok(text) = CString::new(format!(
                    r#"<span alpha="58%">{prefix_text}</span>{}"#,
                    suffix.to_ascii_uppercase(),
                ))
            {
                // SAFETY: The badge is a live label. The supplied codes contain only ASCII letters.
                unsafe { ffi::gtk_label_set_markup(badge, text.as_ptr()) };
            }
        }
    }

    pub(super) fn hide(&mut self) {
        visible(self.bar, false);
        for badge in self.badges.drain(..) {
            // SAFETY: Every badge is a live child of the fixed layer until removed.
            unsafe { gtk_fixed_remove(self.layer, badge) };
        }
    }
}

fn label(title: &CStr, style: Option<&CStr>) -> Object {
    // SAFETY: GTK is initialized and copies the valid NUL-terminated title.
    let label = unsafe { ffi::gtk_label_new(title.as_ptr()) };
    pass_input(label);
    if let Some(style) = style {
        styled(label, style);
    }
    label
}

fn styled(widget: Object, style: &CStr) {
    // SAFETY: The caller's live widget copies the valid CSS class.
    unsafe { ffi::gtk_widget_add_css_class(widget, style.as_ptr()) };
}

fn pass_input(widget: Object) {
    // SAFETY: The caller's live widget stays in the overlay without intercepting page input.
    unsafe { gtk_widget_set_can_target(widget, ffi::FALSE) };
}

fn visible(widget: Object, value: bool) {
    // SAFETY: The caller keeps the widget live while this synchronous operation runs.
    unsafe { ffi::gtk_widget_set_visible(widget, Bool::from(value)) };
}

fn natural_size(widget: Object, orientation: c_int) -> f64 {
    let mut natural = 0;
    // SAFETY: The widget is a live label and GTK accepts NULL for the unused outputs.
    unsafe {
        gtk_widget_measure(
            widget,
            orientation,
            -1,
            ptr::null_mut(),
            &raw mut natural,
            ptr::null_mut(),
            ptr::null_mut(),
        );
    }
    f64::from(natural)
}

fn install_css(widget: Object) {
    let Ok(css) = CString::new(include_str!("hints.css")) else {
        return;
    };
    // SAFETY: GTK is initialized on the browser thread.
    let provider = unsafe { gtk_css_provider_new() };
    // SAFETY: The live provider copies the valid stylesheet.
    unsafe { gtk_css_provider_load_from_string(provider, css.as_ptr()) };
    // SAFETY: The live overlay is attached to the browser window's display.
    let display = unsafe { gtk_widget_get_display(widget) };
    // SAFETY: The live display keeps its own reference to the valid provider.
    unsafe {
        gtk_style_context_add_provider_for_display(display, provider, STYLE_APPLICATION);
    }
    // SAFETY: Release the constructor's reference, which the display now retains separately.
    unsafe { ffi::g_object_unref(provider) };
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_fixed_new() -> Object;
    fn gtk_fixed_put(fixed: Object, widget: Object, x: f64, y: f64);
    fn gtk_fixed_remove(fixed: Object, widget: Object);
    fn gtk_widget_set_halign(widget: Object, alignment: c_int);
    fn gtk_widget_set_vexpand(widget: Object, expand: Bool);
    fn gtk_widget_set_can_target(widget: Object, target: Bool);
    fn gtk_widget_get_width(widget: Object) -> c_int;
    fn gtk_widget_get_height(widget: Object) -> c_int;
    fn gtk_widget_translate_coordinates(
        source: Object,
        destination: Object,
        x: f64,
        y: f64,
        out_x: *mut f64,
        out_y: *mut f64,
    ) -> Bool;
    fn gtk_widget_measure(
        widget: Object,
        orientation: c_int,
        for_size: c_int,
        minimum: *mut c_int,
        natural: *mut c_int,
        minimum_baseline: *mut c_int,
        natural_baseline: *mut c_int,
    );
    fn gtk_widget_get_display(widget: Object) -> Object;
    fn gtk_css_provider_new() -> Object;
    fn gtk_css_provider_load_from_string(provider: Object, css: *const c_char);
    fn gtk_style_context_add_provider_for_display(
        display: Object,
        provider: Object,
        priority: c_uint,
    );
}
