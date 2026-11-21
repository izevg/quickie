//! Play: synthetic keys and clicks against a live widget, and its accessibility tree.
//!
//! Events are real `NSEvent`s handed to the window as `AppKit` would after taking them off the
//! queue, so a text field's editor, its delegate, and buttons react as they do on screen. Keys
//! are named by their place on a US keyboard (`kVK_ANSI_*`), as the keymap names them.

use std::fmt::Write as _;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSControl, NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow,
};
use objc2_foundation::{NSArray, NSPoint, NSString};

/// A key, by its place on a US keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A key that types this character: a letter, a digit, a space, punctuation. Characters a
    /// US keyboard has no key for (Cyrillic, accents) are typed as by an input source whose
    /// key code the harness does not know (code 0).
    Char(char),
    Return,
    Tab,
    Escape,
    Backspace,
    Up,
    Down,
    Left,
    Right,
}

/// Modifier keys held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four modifier keys, each up or down"
)]
pub struct Modifiers {
    pub command: bool,
    pub shift: bool,
    pub option: bool,
    pub control: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        command: false,
        shift: false,
        option: false,
        control: false,
    };
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };
    pub const COMMAND: Self = Self {
        command: true,
        ..Self::NONE
    };
    pub const CONTROL: Self = Self {
        control: true,
        ..Self::NONE
    };

    fn flags(self) -> NSEventModifierFlags {
        let mut flags = NSEventModifierFlags::empty();
        for (on, flag) in [
            (self.command, NSEventModifierFlags::Command),
            (self.shift, NSEventModifierFlags::Shift),
            (self.option, NSEventModifierFlags::Option),
            (self.control, NSEventModifierFlags::Control),
        ] {
            if on {
                flags |= flag;
            }
        }
        flags
    }
}

/// One key press: what the shell's key routing sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keystroke {
    pub key: Key,
    pub modifiers: Modifiers,
}

/// US-layout key codes of the letters, `a` first.
const LETTER_CODES: [u16; 26] = [
    0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D, 0x1F, 0x23,
    0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06,
];
/// US-layout key codes of the digits, `0` first.
const DIGIT_CODES: [u16; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];

impl Key {
    /// The key code, and what the key types (`characters`) as `AppKit` reports them.
    fn code_and_characters(self) -> (u16, String) {
        let special = |code: u16, unit: u32| {
            (
                code,
                char::from_u32(unit).map_or_else(String::new, String::from),
            )
        };
        match self {
            Self::Char(c) => (char_code(c), c.to_string()),
            Self::Return => special(0x24, 0x0D),
            Self::Tab => special(0x30, 0x09),
            Self::Escape => special(0x35, 0x1B),
            Self::Backspace => special(0x33, 0x7F),
            Self::Up => special(0x7E, 0xF700),
            Self::Down => special(0x7D, 0xF701),
            Self::Left => special(0x7B, 0xF702),
            Self::Right => special(0x7C, 0xF703),
        }
    }

    /// True for the keys `AppKit` flags as function keys (the arrows).
    fn is_function(self) -> bool {
        matches!(self, Self::Up | Self::Down | Self::Left | Self::Right)
    }
}

fn char_code(c: char) -> u16 {
    let lower = c.to_ascii_lowercase();
    if lower.is_ascii_lowercase() {
        return LETTER_CODES[usize::from(lower as u8 - b'a')];
    }
    if c.is_ascii_digit() {
        return DIGIT_CODES[usize::from(c as u8 - b'0')];
    }
    match c {
        ' ' => 0x31,
        '-' => 0x1B,
        '=' => 0x18,
        '.' => 0x2F,
        ',' => 0x2B,
        '/' => 0x2C,
        ';' => 0x29,
        '\'' => 0x27,
        '[' => 0x21,
        ']' => 0x1E,
        _ => 0,
    }
}

/// Sends `stroke` to `window` as a key down and a key up: first as a key equivalent when a
/// modifier other than shift is held, then to the first responder.
pub(super) fn send_key(window: &NSWindow, stroke: Keystroke) {
    let (code, characters) = stroke.key.code_and_characters();
    let mut flags = stroke.modifiers.flags();
    if stroke.key.is_function() {
        flags |= NSEventModifierFlags::Function | NSEventModifierFlags::NumericPad;
    }
    let unmodified = if stroke.modifiers.shift {
        characters.to_lowercase()
    } else {
        characters.clone()
    };
    let characters = if stroke.modifiers.shift {
        characters.to_uppercase()
    } else {
        characters
    };
    let event = |kind: NSEventType| {
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            kind,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            window.windowNumber(),
            None,
            &NSString::from_str(&characters),
            &NSString::from_str(&unmodified),
            false,
            code,
        )
    };
    let Some(down) = event(NSEventType::KeyDown) else {
        return;
    };
    let equivalent = stroke.modifiers.command || stroke.modifiers.control;
    if !(equivalent && window.performKeyEquivalent(&down)) {
        window.sendEvent(&down);
    }
    if let Some(up) = event(NSEventType::KeyUp) {
        window.sendEvent(&up);
    }
}

/// Clicks `view` at `point` in its own coordinates, on whatever the window would hit there.
///
/// A control (a button, a switch) is clicked through `performClick:`, the path a pointer click
/// takes once tracking ends: highlight, state, target and action. Real mouse events do not
/// reach a control's tracking in a window that was never key, which a headless app's never is.
/// Any other view gets a mouse down and then the mouse up, both handed to it, as a window
/// hands the mouse up to the view that took the mouse down. Not through the app's event queue:
/// how soon a posted event can be taken back out of it differs between macOS releases (on
/// macOS 26 not at once), and a mouse up left behind would land in the next click. No view of
/// ours runs a tracking loop of its own in `mouseDown:` that would wait for it there.
pub(super) fn send_click(window: &NSWindow, view: &NSView, point: NSPoint) {
    let location = view.convertPoint_toView(point, None);
    let Some(content) = window.contentView() else {
        return;
    };
    let target = content.hitTest(location).unwrap_or_else(|| content.clone());
    if let Ok(control) = target.clone().downcast::<NSControl>() {
        if control.isEnabled() {
            // SAFETY: `performClick:` accepts any sender, including none.
            unsafe { control.performClick(None) };
        }
        return;
    }
    let event = |kind: NSEventType| {
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            location,
            NSEventModifierFlags::empty(),
            0.0,
            window.windowNumber(),
            None,
            0,
            1,
            1.0,
        )
    };
    let (Some(down), Some(up)) = (
        event(NSEventType::LeftMouseDown),
        event(NSEventType::LeftMouseUp),
    ) else {
        return;
    };
    target.mouseDown(&down);
    target.mouseUp(&up);
}

/// The first visible view under `view`, depth first, whose accessibility title or label is
/// `label`: a button by its title, a chip, an image by its description.
pub(super) fn labelled(view: &NSView, label: &str) -> Option<Retained<NSView>> {
    for child in view.subviews() {
        if child.isHidden() {
            continue;
        }
        let named = [child.accessibilityTitle(), child.accessibilityLabel()]
            .into_iter()
            .flatten()
            .any(|name| name.to_string() == label);
        if named {
            return Some(child);
        }
        if let Some(found) = labelled(&child, label) {
            return Some(found);
        }
    }
    None
}

/// The accessibility tree under `view`, one element per line, indented by depth: role, then
/// title, label and value where the element has them.
pub(super) fn accessibility(view: &NSView) -> String {
    let mut out = String::new();
    if let Some(children) = view.accessibilityChildren() {
        walk(&children, 0, &mut out);
    }
    out
}

fn walk(children: &NSArray, depth: usize, out: &mut String) {
    for child in children {
        let mut line = "  ".repeat(depth);
        line.push_str(
            &attribute(&child, sel!(accessibilityRole), "AXRole").unwrap_or_else(|| "?".to_owned()),
        );
        for (name, selector, legacy) in [
            ("title", sel!(accessibilityTitle), "AXTitle"),
            ("label", sel!(accessibilityLabel), "AXDescription"),
            ("value", sel!(accessibilityValue), "AXValue"),
        ] {
            if let Some(value) =
                attribute(&child, selector, legacy).filter(|value| !value.is_empty())
            {
                let _ = write!(line, " {name}={value:?}");
            }
        }
        out.push_str(&line);
        out.push('\n');
        let grandchildren: Option<Retained<NSArray>> =
            if responds(&child, sel!(accessibilityChildren)) {
                // SAFETY: the element answers `accessibilityChildren`, which returns an array of
                // elements or nil.
                unsafe { msg_send![&*child, accessibilityChildren] }
            } else {
                legacy(&child, "AXChildren").and_then(|value| value.downcast::<NSArray>().ok())
            };
        if let Some(grandchildren) = grandchildren {
            walk(&grandchildren, depth + 1, out);
        }
    }
}

/// What `selector` answers on `object`, or, for an element of the older informal protocol
/// (a table's rows), its attribute `legacy`, as text.
fn attribute(object: &AnyObject, selector: Sel, legacy_name: &str) -> Option<String> {
    if responds(object, selector) {
        return text(object, selector);
    }
    legacy(object, legacy_name).map(|value| describe(&value))
}

/// The attribute `name` of an element of the older informal protocol, if it answers it.
fn legacy(object: &AnyObject, name: &str) -> Option<Retained<AnyObject>> {
    if !responds(object, sel!(accessibilityAttributeValue:)) {
        return None;
    }
    let name = NSString::from_str(name);
    // SAFETY: `accessibilityAttributeValue:` takes an attribute name and returns an object or
    // nil; the object answers it.
    unsafe { msg_send![object, accessibilityAttributeValue: &*name] }
}

fn responds(object: &AnyObject, selector: Sel) -> bool {
    // SAFETY: `respondsToSelector:` is defined on every `NSObject`, which every accessibility
    // element is.
    unsafe { msg_send![object, respondsToSelector: selector] }
}

/// What `selector` answers on `object`, as text: an `NSString` itself, anything else by its
/// description.
fn text(object: &AnyObject, selector: Sel) -> Option<String> {
    if !responds(object, selector) {
        return None;
    }
    // SAFETY: the accessibility getters return an object or nil, and the object responds.
    let value: Option<Retained<AnyObject>> =
        unsafe { msg_send![object, performSelector: selector] };
    value.map(|value| describe(&value))
}

/// `value` as text: an `NSString` itself, anything else by its description.
fn describe(value: &AnyObject) -> String {
    if let Some(string) = value.downcast_ref::<NSString>() {
        return string.to_string();
    }
    // SAFETY: every object answers `description` with a string.
    let description: Retained<NSString> = unsafe { msg_send![value, description] };
    description.to_string()
}
