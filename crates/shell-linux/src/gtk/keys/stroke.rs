//! The GDK stroke read by shortcut recording and browser dispatch.

use std::ffi::{c_int, c_uint, c_void};
use std::ptr;
use std::slice;

use quickie_domain_commands::{Digit, FunctionKey, Key, KeyStroke, Letter, Modifiers};

use crate::gtk::ffi::{self, Bool, Object};

pub(in crate::gtk) fn read(
    controller: Object,
    key: c_uint,
    hardware: c_uint,
    state: c_uint,
) -> Option<KeyStroke> {
    if state & ffi::HYPER != 0 {
        return None;
    }
    let key = plain_key(controller, hardware).or_else(|| key_of(key))?;
    let mut modifiers = Modifiers::NONE;
    for (mask, modifier) in [
        (ffi::CONTROL, Modifiers::CONTROL),
        (ffi::ALT, Modifiers::OPTION),
        (ffi::SHIFT, Modifiers::SHIFT),
        (ffi::SUPER | ffi::META, Modifiers::COMMAND),
    ] {
        if state & mask != 0 {
            modifiers = modifiers | modifier;
        }
    }
    Some(KeyStroke::new(key, modifiers))
}

pub(in crate::gtk) fn is_modifier(key: c_uint) -> bool {
    matches!(key, 0xffe1..=0xffee | 0xfe03 | 0xfe11)
}

pub(in crate::gtk) fn input_focused(view: Object) -> bool {
    // SAFETY: The current view belongs to the live browser GtkRoot.
    let root = unsafe { ffi::gtk_widget_get_root(view) };
    if root.is_null() {
        return false;
    }
    // SAFETY: The live root returns its borrowed focus widget or NULL.
    let focused = unsafe { gtk_root_get_focus(root) };
    if focused.is_null() {
        return false;
    }
    // SAFETY: GTK provides the GtkEditable interface type on this thread.
    let editable = unsafe { gtk_editable_get_type() };
    // SAFETY: The focused GtkWidget is a live GTypeInstance.
    if unsafe { g_type_check_instance_is_a(focused, editable) } != ffi::FALSE {
        return true;
    }
    // SAFETY: The web view and current native focus are both live widgets.
    let within_view = unsafe { gtk_widget_is_ancestor(focused, view) } != ffi::FALSE;
    let within_page = focused == view || within_view;
    within_page && super::focus::editable(view)
}

fn plain_key(controller: Object, hardware: c_uint) -> Option<Key> {
    // SAFETY: This is the live native controller within its own key event signal.
    let event = unsafe { gtk_event_controller_get_current_event(controller) };
    if event.is_null() {
        return None;
    }
    // SAFETY: GTK supplies its current keyboard event and a borrowed display.
    let display = unsafe { gdk_event_get_display(event) };
    // SAFETY: The event is a live key event and reports its active layout.
    let layout = unsafe { gdk_key_event_get_layout(event) };
    let mut raw = 0;
    // SAFETY: The live display maps the hardware key in the active layout without Shift.
    // GDK accepts NULL for the translation outputs we don't need.
    let translated = unsafe {
        gdk_display_translate_key(
            display,
            hardware,
            0,
            c_int::try_from(layout).ok()?,
            &raw mut raw,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if translated != ffi::FALSE
        && let Some(key) = key_of(raw)
    {
        return Some(key);
    }
    alternate_layout(display, hardware)
}

#[repr(C)]
struct KeymapKey {
    hardware: c_uint,
    group: c_int,
    level: c_int,
}

fn alternate_layout(display: Object, hardware: c_uint) -> Option<Key> {
    let mut keys = ptr::null_mut();
    let mut values = ptr::null_mut();
    let mut count = 0;
    // SAFETY: GDK fills the three writable outputs for the live display and keycode.
    let found = unsafe {
        gdk_display_map_keycode(
            display,
            hardware,
            &raw mut keys,
            &raw mut values,
            &raw mut count,
        )
    };
    let mut result = None;
    if found != ffi::FALSE
        && !keys.is_null()
        && !values.is_null()
        && let Ok(count) = usize::try_from(count)
    {
        // SAFETY: A successful mapping returns count entries in the owned key array.
        let mapped_keys = unsafe { slice::from_raw_parts(keys, count) };
        // SAFETY: GDK gives the parallel owned value array the same count.
        let mapped_values = unsafe { slice::from_raw_parts(values, count) };
        result = mapped_keys
            .iter()
            .zip(mapped_values)
            .filter(|(key, _)| key.level == 0)
            .find_map(|(_, &value)| key_of(value));
    }
    // SAFETY: The returned arrays belong to the caller and g_free accepts NULL.
    unsafe { g_free(keys.cast()) };
    // SAFETY: The second array belongs to the caller even if none of its layouts had a key.
    unsafe { g_free(values.cast()) };
    result
}

fn key_of(key: c_uint) -> Option<Key> {
    Some(match key {
        0x41..=0x5a | 0x61..=0x7a => Key::Letter(Letter::new(char::from_u32(key)?)?),
        0x30..=0x39 => Key::Digit(Digit::new(char::from_u32(key)?)?),
        0xffbe..=0xffc9 => Key::Function(FunctionKey::new(u8::try_from(key - 0xffbd).ok()?)?),
        0x20 => Key::Space,
        0x5b | 0x7b => Key::BracketLeft,
        0x5d | 0x7d => Key::BracketRight,
        0x2d | 0x5f => Key::Minus,
        0x3d | 0x2b => Key::Equal,
        0x2c | 0x3c => Key::Comma,
        0x2e | 0x3e => Key::Period,
        0x2f | 0x3f => Key::Slash,
        0x5c | 0x7c => Key::Backslash,
        0x3b | 0x3a => Key::Semicolon,
        0x27 | 0x22 => Key::Quote,
        0x60 | 0x7e => Key::Backquote,
        0xff09 | 0xfe20 => Key::Tab,
        0xff0d => Key::Enter,
        0xff1b => Key::Escape,
        0xff08 => Key::Backspace,
        0xffff => Key::Delete,
        0xff52 => Key::Up,
        0xff54 => Key::Down,
        0xff51 => Key::Left,
        0xff53 => Key::Right,
        0xff50 => Key::Home,
        0xff57 => Key::End,
        0xff55 => Key::PageUp,
        0xff56 => Key::PageDown,
        _ => return None,
    })
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_event_controller_get_current_event(controller: Object) -> Object;
    fn gtk_root_get_focus(root: Object) -> Object;
    fn gtk_editable_get_type() -> usize;
    fn gtk_widget_is_ancestor(widget: Object, ancestor: Object) -> Bool;
    fn gdk_event_get_display(event: Object) -> Object;
    fn gdk_key_event_get_layout(event: Object) -> c_uint;
    fn gdk_display_translate_key(
        display: Object,
        hardware: c_uint,
        state: c_uint,
        group: c_int,
        key: *mut c_uint,
        effective_group: *mut c_int,
        level: *mut c_int,
        consumed: *mut c_uint,
    ) -> Bool;
    fn gdk_display_map_keycode(
        display: Object,
        hardware: c_uint,
        keys: *mut *mut KeymapKey,
        keyvals: *mut *mut c_uint,
        count: *mut c_int,
    ) -> Bool;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_type_check_instance_is_a(instance: Object, kind: usize) -> Bool;
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(allocation: *mut c_void);
}
