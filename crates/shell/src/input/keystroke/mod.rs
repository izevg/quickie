//! Key presses as the keymap reads them. A letter is the letter the layout types when that is a
//! Latin letter, so `⌘Z` follows the `Z` keycap on QWERTZ and AZERTY too; any other key, and a
//! letter key on a layout that types no Latin letter, is named by its place on a US layout, so
//! `⌘L` opens the address overlay on a Russian layout too (it types `д` on the `L` key). A key
//! held with `fn`/Globe that does not carry `fn` by itself is never the keymap's: those strokes
//! belong to the system (Globe+F full screen, Globe+⌃F fill, a remapped key).
//!
//! Also the fixed keys of an open overlay, which are not commands and cannot be rebound: `↑`
//! `↓` `⌃N` `⌃P` move, `⏎` accepts, `⌘⏎` accepts into a new tab, `⌥⏎` into a new throwaway
//! tab, `⇧⏎` goes to the previous find match, `⎋` closes.
//!
//! - [`chord`]: the first stroke of a two-stroke chord, and when it stops waiting.
//! - [`log`]: the last key presses and what became of them, for the key debug overlay.
//! - [`typing`]: keys that type, and keys an input method composes with, which go on without
//!   the keymap.

pub mod chord;
pub mod log;
pub mod typing;

use quickie_domain_commands::{Digit, FunctionKey, Key, KeyStroke, Letter, Modifiers};

/// Letter keys by key code (`kVK_ANSI_*` in Carbon's `Events.h`); the codes follow the keys'
/// physical positions, not the alphabet.
const LETTERS: [(u16, char); 26] = [
    (0x00, 'a'),
    (0x01, 's'),
    (0x02, 'd'),
    (0x03, 'f'),
    (0x04, 'h'),
    (0x05, 'g'),
    (0x06, 'z'),
    (0x07, 'x'),
    (0x08, 'c'),
    (0x09, 'v'),
    (0x0B, 'b'),
    (0x0C, 'q'),
    (0x0D, 'w'),
    (0x0E, 'e'),
    (0x0F, 'r'),
    (0x10, 'y'),
    (0x11, 't'),
    (0x1F, 'o'),
    (0x20, 'u'),
    (0x22, 'i'),
    (0x23, 'p'),
    (0x25, 'l'),
    (0x26, 'j'),
    (0x28, 'k'),
    (0x2D, 'n'),
    (0x2E, 'm'),
];

/// Digit keys of the main row by key code.
const DIGITS: [(u16, char); 10] = [
    (0x12, '1'),
    (0x13, '2'),
    (0x14, '3'),
    (0x15, '4'),
    (0x17, '5'),
    (0x16, '6'),
    (0x1A, '7'),
    (0x1C, '8'),
    (0x19, '9'),
    (0x1D, '0'),
];

/// Function keys by key code, `F1` first.
const FUNCTION_KEYS: [u16; 12] = [
    0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F,
];

/// Keys whose presses carry `AppKit`'s function-key flag by themselves: the arrows, Home, End,
/// Page Up and Down, Forward Delete, Help, and `F1` … `F20`. Held with `fn`, any other key
/// is a system stroke.
const CARRY_FN: [u16; 30] = [
    0x7B, 0x7C, 0x7D, 0x7E, 0x73, 0x77, 0x74, 0x79, 0x75, 0x72, 0x7A, 0x78, 0x63, 0x76, 0x60, 0x61,
    0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A,
];

/// Every other key by key code.
const NAMED: [(u16, Key); 26] = [
    (0x18, Key::Equal),
    (0x1B, Key::Minus),
    (0x1E, Key::BracketRight),
    (0x21, Key::BracketLeft),
    (0x24, Key::Enter),
    (0x27, Key::Quote),
    (0x29, Key::Semicolon),
    (0x2A, Key::Backslash),
    (0x2B, Key::Comma),
    (0x2C, Key::Slash),
    (0x2F, Key::Period),
    (0x30, Key::Tab),
    (0x31, Key::Space),
    (0x32, Key::Backquote),
    (0x33, Key::Backspace),
    (0x35, Key::Escape),
    // The keypad's Enter is the same key to the keymap.
    (0x4C, Key::Enter),
    (0x73, Key::Home),
    (0x74, Key::PageUp),
    (0x75, Key::Delete),
    (0x77, Key::End),
    (0x79, Key::PageDown),
    (0x7B, Key::Left),
    (0x7C, Key::Right),
    (0x7D, Key::Down),
    (0x7E, Key::Up),
];

/// The key at hardware key `code` on a US layout, or `None` for a key the keymap cannot name.
#[must_use]
pub fn key(code: u16) -> Option<Key> {
    let find = |table: &[(u16, char)]| table.iter().find(|&&(at, _)| at == code).map(|&(_, c)| c);
    if let Some(letter) = find(&LETTERS).and_then(Letter::new) {
        return Some(Key::Letter(letter));
    }
    if let Some(digit) = find(&DIGITS).and_then(Digit::new) {
        return Some(Key::Digit(digit));
    }
    if let Some(function) = FUNCTION_KEYS
        .iter()
        .position(|&at| at == code)
        .and_then(|index| u8::try_from(index + 1).ok())
        .and_then(FunctionKey::new)
    {
        return Some(Key::Function(function));
    }
    NAMED
        .iter()
        .find(|&&(at, _)| at == code)
        .map(|&(_, key)| key)
}

/// The key pressed: the Latin letter `typed` names when it is one (the event's characters
/// ignoring modifiers), else the key at `code` on a US layout.
#[must_use]
pub fn key_of(code: u16, typed: &str) -> Option<Key> {
    let mut chars = typed.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_ascii_alphabetic()
        && let Some(letter) = Letter::new(c.to_ascii_lowercase())
    {
        return Some(Key::Letter(letter));
    }
    key(code)
}

/// What the keymap makes of a key press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// A stroke the keymap can look up.
    Stroke(KeyStroke),
    /// A key the keymap cannot name (the keypad, a media key).
    Unnamed,
    /// A key held with `fn`/Globe that does not carry `fn` by itself: the system's.
    System,
}

/// Reads a key press: hardware `code`, the characters it types ignoring modifiers, the
/// modifiers the keymap knows, and whether `fn`/Globe is down.
#[must_use]
pub fn read(code: u16, typed: &str, modifiers: Modifiers, function: bool) -> Read {
    if function && !CARRY_FN.contains(&code) {
        return Read::System;
    }
    key_of(code, typed).map_or(Read::Unnamed, |key| {
        Read::Stroke(KeyStroke::new(key, modifiers))
    })
}

/// One key press, as `AppKit` stamps it: its timestamp's bits and its key code. The same press
/// handed back by the page has the same stamp.
pub type PressId = (u64, u16);

/// The last presses the browser passed on. `WebKit` answers asynchronously, so a press the
/// page did not take can come back after a few newer ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Passed {
    ids: [Option<PressId>; Self::LENGTH],
    next: usize,
}

impl Passed {
    const LENGTH: usize = 8;

    /// Remembers `id`, forgetting the oldest once there are [`Self::LENGTH`].
    pub fn remember(&mut self, id: PressId) {
        self.ids[self.next] = Some(id);
        self.next = (self.next + 1) % Self::LENGTH;
    }

    /// Whether `id` was passed on lately: it is back from the page.
    #[must_use]
    pub fn contains(&self, id: PressId) -> bool {
        self.ids.contains(&Some(id))
    }
}

/// What a fixed key does in an open overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKey {
    /// Move the selection by this many rows.
    Move(isize),
    /// Go with the selection; `new_tab` with `⌘`, `throwaway` (a new tab in a new throwaway
    /// jar) with `⌥`.
    Accept { new_tab: bool, throwaway: bool },
    /// `⇧⏎`: the previous one (a find match).
    Previous,
    /// Close the overlay.
    Dismiss,
}

/// The fixed meaning of `stroke` in an overlay; `None` leaves it to the keymap and the field.
#[must_use]
pub fn overlay_key(stroke: KeyStroke) -> Option<OverlayKey> {
    let none = Modifiers::NONE;
    let control = Modifiers::CONTROL;
    match (stroke.key, stroke.modifiers) {
        (Key::Up, m) if m == none => Some(OverlayKey::Move(-1)),
        (Key::Down, m) if m == none => Some(OverlayKey::Move(1)),
        (Key::Letter(letter), m) if m == control && letter.as_char() == 'p' => {
            Some(OverlayKey::Move(-1))
        }
        (Key::Letter(letter), m) if m == control && letter.as_char() == 'n' => {
            Some(OverlayKey::Move(1))
        }
        (Key::Enter, m) if m == none => Some(OverlayKey::Accept {
            new_tab: false,
            throwaway: false,
        }),
        (Key::Enter, m) if m == Modifiers::COMMAND => Some(OverlayKey::Accept {
            new_tab: true,
            throwaway: false,
        }),
        (Key::Enter, m) if m == Modifiers::OPTION => Some(OverlayKey::Accept {
            new_tab: false,
            throwaway: true,
        }),
        (Key::Enter, m) if m == Modifiers::SHIFT => Some(OverlayKey::Previous),
        (Key::Escape, m) if m == none => Some(OverlayKey::Dismiss),
        _ => None,
    }
}

/// What a fixed key does on an E3 sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetKey {
    /// `⏎`: OK.
    Accept,
    /// `⎋`: cancel, deny; the safe default.
    Dismiss,
    /// `1` … `9`: the numbered answer.
    Number(u8),
    /// `⌘B`: block the page's dialogs.
    Block,
    /// Any other key without `⌘` or `⌃`: it belongs to the sheet's field, or to nobody, never
    /// to the page under the sheet.
    Typing,
}

/// The meaning of `stroke` on a sheet; `None` leaves a `⌘` or `⌃` shortcut to the keymap.
#[must_use]
pub fn sheet_key(stroke: KeyStroke) -> Option<SheetKey> {
    let none = Modifiers::NONE;
    let reaching = |m: Modifiers| m.contains(Modifiers::COMMAND) || m.contains(Modifiers::CONTROL);
    let number = |key: Key| match key {
        Key::Digit(digit) => digit
            .as_char()
            .to_digit(10)
            .and_then(|n| u8::try_from(n).ok())
            .filter(|&n| n > 0),
        _ => None,
    };
    match (stroke.key, stroke.modifiers) {
        (Key::Enter, m) if m == none => Some(SheetKey::Accept),
        (Key::Escape, m) if m == none => Some(SheetKey::Dismiss),
        (key, m) if m == none && number(key).is_some() => number(key).map(SheetKey::Number),
        (Key::Letter(letter), m) if m == Modifiers::COMMAND && letter.as_char() == 'b' => {
            Some(SheetKey::Block)
        }
        (_, m) if reaching(m) => None,
        _ => Some(SheetKey::Typing),
    }
}

/// `⌘Q`: quits. It is reserved but no command, so the keymap cannot run it.
#[must_use]
pub fn is_quit(stroke: KeyStroke) -> bool {
    is_command_letter(stroke, 'q')
}

/// `⌘W` in a window that is not a browser window (Settings): closes it.
#[must_use]
pub fn is_close(stroke: KeyStroke) -> bool {
    is_command_letter(stroke, 'w')
}

fn is_command_letter(stroke: KeyStroke, letter: char) -> bool {
    stroke.modifiers == Modifiers::COMMAND
        && matches!(stroke.key, Key::Letter(held) if held.as_char() == letter)
}

#[cfg(test)]
mod tests {
    use quickie_domain_commands::{Key, KeyStroke, Keymap, Modifiers, Resolution, WhenContext};

    use super::{
        OverlayKey, Passed, Read, SheetKey, is_close, is_quit, key, key_of, overlay_key, read,
        sheet_key,
    };
    use crate::PLATFORM;

    fn parsed(text: &str) -> KeyStroke {
        KeyStroke::try_from(text).unwrap()
    }

    #[test]
    fn letters_are_named_by_their_place_on_a_us_layout() {
        let row: String = [0x0C, 0x0D, 0x0E, 0x0F, 0x11, 0x10, 0x20, 0x22, 0x1F, 0x23]
            .into_iter()
            .map(|code| key(code).unwrap().name())
            .collect();

        assert_eq!(row, "qwertyuiop");
        assert_eq!(key(0x25).map(Key::name).as_deref(), Some("l"));
        assert_eq!(key(0x2D).map(Key::name).as_deref(), Some("n"));
    }

    #[test]
    fn every_letter_and_digit_has_one_key() {
        let mut names: Vec<String> = (0..=0x7F_u16).filter_map(key).map(Key::name).collect();
        names.sort();
        names.dedup();

        for c in ('a'..='z').chain('0'..='9') {
            assert!(names.contains(&c.to_string()), "{c}");
        }
    }

    #[test]
    fn digits_follow_the_top_row() {
        let digits: String = [0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19, 0x1D]
            .into_iter()
            .map(|code| key(code).unwrap().name())
            .collect();

        assert_eq!(digits, "1234567890");
    }

    #[test]
    fn function_and_named_keys_are_found() {
        let cases = [
            (0x7A, "f1"),
            (0x6F, "f12"),
            (0x60, "f5"),
            (0x30, "tab"),
            (0x24, "enter"),
            (0x4C, "enter"),
            (0x35, "escape"),
            (0x21, "["),
            (0x1E, "]"),
            (0x7E, "up"),
            (0x7D, "down"),
        ];
        for (code, name) in cases {
            assert_eq!(key(code).map(Key::name).as_deref(), Some(name), "{code:#x}");
        }
        assert_eq!(key(0x0A), None);
        assert_eq!(key(0x3A), None);
    }

    #[test]
    fn a_latin_layout_names_letters_as_its_keycaps_say() {
        // QWERTZ: the key at the US `Y` types `z`, so `⌘Z` is undo there too.
        assert_eq!(key_of(0x10, "z").map(Key::name).as_deref(), Some("z"));
        // AZERTY: the key at the US `Q` types `a`, and `M` sits at the US `;`.
        assert_eq!(key_of(0x0C, "a").map(Key::name).as_deref(), Some("a"));
        assert_eq!(key_of(0x29, "m").map(Key::name).as_deref(), Some("m"));
        // Shift reaches the characters too; the letter is the same key.
        assert_eq!(key_of(0x25, "L").map(Key::name).as_deref(), Some("l"));
    }

    #[test]
    fn a_non_latin_layout_falls_back_to_the_us_position() {
        // Russian: the `L` key types `д`; Greek: the `F` key types `φ`.
        assert_eq!(key_of(0x25, "д").map(Key::name).as_deref(), Some("l"));
        assert_eq!(key_of(0x03, "φ").map(Key::name).as_deref(), Some("f"));
        // AZERTY's top row types `&é"'…` unshifted: the digits stay where they are.
        assert_eq!(key_of(0x12, "&").map(Key::name).as_deref(), Some("1"));
        assert_eq!(key_of(0x13, "é").map(Key::name).as_deref(), Some("2"));
        // Nothing typed (a dead key's first press), or several characters: by position.
        assert_eq!(key_of(0x0E, "").map(Key::name).as_deref(), Some("e"));
        assert_eq!(key_of(0x0E, "ab").map(Key::name).as_deref(), Some("e"));
        // Punctuation is always read by position.
        assert_eq!(key_of(0x18, "+").map(Key::name).as_deref(), Some("="));
    }

    #[test]
    fn fn_and_globe_strokes_belong_to_the_system() {
        let none = Modifiers::NONE;
        // Globe+F (full screen), Globe+⌃F (fill), fn+⌥F: never the keymap's, never link hints.
        assert_eq!(read(0x03, "f", none, true), Read::System);
        assert_eq!(read(0x03, "f", Modifiers::CONTROL, true), Read::System);
        assert_eq!(read(0x03, "f", Modifiers::OPTION, true), Read::System);
        assert_eq!(read(0x31, " ", none, true), Read::System);
        // Keys that carry `fn` by themselves stay ordinary strokes.
        assert_eq!(read(0x7E, "", none, true), Read::Stroke(parsed("up")));
        assert_eq!(read(0x7A, "", none, true), Read::Stroke(parsed("f1")));
        assert_eq!(read(0x75, "", none, true), Read::Stroke(parsed("delete")));
        assert_eq!(read(0x73, "", none, true), Read::Stroke(parsed("home")));
        // Without `fn`: as usual.
        assert_eq!(read(0x03, "f", none, false), Read::Stroke(parsed("f")));
        assert_eq!(
            read(0x25, "д", Modifiers::COMMAND, false),
            Read::Stroke(parsed("cmd+l"))
        );
        assert_eq!(read(0x0A, "§", none, false), Read::Unnamed);
    }

    #[test]
    fn strokes_carry_their_modifiers_into_the_default_keymap() {
        let shift_command = Modifiers::COMMAND | Modifiers::SHIFT;
        let Read::Stroke(palette) = read(0x23, "P", shift_command, false) else {
            panic!("⇧⌘P reads as a stroke");
        };

        assert_eq!(palette, parsed("cmd+shift+p"));
        assert!(Keymap::defaults(PLATFORM).is_reserved(palette));
        assert_eq!(read(0x0A, "", Modifiers::COMMAND, false), Read::Unnamed);
    }

    #[test]
    fn f12_opens_the_inspector_from_a_page() {
        let keymap = Keymap::defaults(PLATFORM);
        let context = WhenContext::new()
            .with_flag("pageFocused")
            .with_flag("inputFocused");

        for function in [false, true] {
            let Read::Stroke(stroke) = read(0x6F, "\u{F70F}", Modifiers::NONE, function) else {
                panic!("AppKit F12 must reach the keymap");
            };
            let resolution = keymap.resolve(stroke, None, &context);
            assert!(
                matches!(resolution, Resolution::Command(id) if id.as_str() == "quickie.devtools.toggle"),
                "F12 with the function flag set to {function} did not resolve to Web Inspector",
            );
        }
    }

    #[test]
    fn a_press_passed_on_is_known_when_it_comes_back() {
        let mut passed = Passed::default();
        assert!(!passed.contains((1, 0x03)));

        passed.remember((1, 0x03));
        for later in 2..=8 {
            passed.remember((later, 0x11));
        }

        assert!(passed.contains((1, 0x03)), "seven newer presses later");
        assert!(!passed.contains((1, 0x11)), "another key at the same time");
        assert!(!passed.contains((9, 0x03)), "the same key pressed again");

        passed.remember((9, 0x11));
        assert!(!passed.contains((1, 0x03)), "eight newer presses later");
        assert!(passed.contains((9, 0x11)));
    }

    #[test]
    fn overlay_keys_move_accept_and_dismiss() {
        let cases = [
            ("up", Some(OverlayKey::Move(-1))),
            ("down", Some(OverlayKey::Move(1))),
            ("ctrl+p", Some(OverlayKey::Move(-1))),
            ("ctrl+n", Some(OverlayKey::Move(1))),
            (
                "enter",
                Some(OverlayKey::Accept {
                    new_tab: false,
                    throwaway: false,
                }),
            ),
            (
                "cmd+enter",
                Some(OverlayKey::Accept {
                    new_tab: true,
                    throwaway: false,
                }),
            ),
            (
                "alt+enter",
                Some(OverlayKey::Accept {
                    new_tab: false,
                    throwaway: true,
                }),
            ),
            ("escape", Some(OverlayKey::Dismiss)),
            ("shift+enter", Some(OverlayKey::Previous)),
            ("shift+up", None),
            ("shift+cmd+enter", None),
            ("ctrl+escape", None),
            ("ctrl+k", None),
            ("p", None),
        ];
        for (text, expected) in cases {
            assert_eq!(overlay_key(parsed(text)), expected, "{text}");
        }
    }

    #[test]
    fn sheet_keys_answer_and_keep_the_page_out() {
        let cases = [
            ("enter", Some(SheetKey::Accept)),
            ("escape", Some(SheetKey::Dismiss)),
            ("1", Some(SheetKey::Number(1))),
            ("2", Some(SheetKey::Number(2))),
            ("9", Some(SheetKey::Number(9))),
            ("cmd+b", Some(SheetKey::Block)),
            ("0", Some(SheetKey::Typing)),
            ("a", Some(SheetKey::Typing)),
            ("shift+1", Some(SheetKey::Typing)),
            ("shift+enter", Some(SheetKey::Typing)),
            ("alt+escape", Some(SheetKey::Typing)),
            ("cmd+w", None),
            ("ctrl+tab", None),
            ("cmd+shift+b", None),
            ("cmd+enter", None),
            ("ctrl+b", None),
        ];
        for (text, expected) in cases {
            assert_eq!(sheet_key(parsed(text)), expected, "{text}");
        }
    }

    #[test]
    fn only_command_q_quits_and_only_command_w_closes() {
        assert!(is_quit(parsed("cmd+q")));
        assert!(!is_quit(parsed("cmd+shift+q")));
        assert!(!is_quit(parsed("q")));
        assert!(!is_quit(parsed("cmd+w")));
        assert!(is_close(parsed("cmd+w")));
        assert!(!is_close(parsed("cmd+shift+w")));
        assert!(!is_close(parsed("w")));
    }
}
