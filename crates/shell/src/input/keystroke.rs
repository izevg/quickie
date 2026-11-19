//! Key presses as the keymap reads them: a hardware key code becomes a [`Key`] named as on a US
//! layout, so `⌘L` opens the address overlay on every layout (a Russian layout types `д` on the
//! `L` key). Also the fixed keys of an open overlay, which are not commands and cannot be
//! rebound: `↑` `↓` `⌃N` `⌃P` move, `⏎` accepts, `⌘⏎` accepts into a new tab, `⇧⏎` goes to
//! the previous find match, `⎋` closes.

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

/// The key at hardware key `code`, or `None` for a key the keymap cannot name.
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

/// The stroke of hardware key `code` with `modifiers` held.
#[must_use]
pub fn stroke(code: u16, modifiers: Modifiers) -> Option<KeyStroke> {
    key(code).map(|key| KeyStroke::new(key, modifiers))
}

/// What a fixed key does in an open overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKey {
    /// Move the selection by this many rows.
    Move(isize),
    /// Go with the selection; `new_tab` with `⌘`.
    Accept { new_tab: bool },
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
        (Key::Enter, m) if m == none => Some(OverlayKey::Accept { new_tab: false }),
        (Key::Enter, m) if m == Modifiers::COMMAND => Some(OverlayKey::Accept { new_tab: true }),
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
    stroke.modifiers == Modifiers::COMMAND
        && matches!(stroke.key, Key::Letter(letter) if letter.as_char() == 'q')
}

#[cfg(test)]
mod tests {
    use quickie_domain_commands::{Key, KeyStroke, Keymap, Modifiers};

    use super::{OverlayKey, SheetKey, is_quit, key, overlay_key, sheet_key, stroke};

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
    fn strokes_carry_their_modifiers_into_the_default_keymap() {
        let palette = stroke(0x23, Modifiers::COMMAND | Modifiers::SHIFT).unwrap();

        assert_eq!(palette, parsed("cmd+shift+p"));
        assert!(Keymap::is_reserved(palette));
        assert_eq!(stroke(0x0A, Modifiers::COMMAND), None);
    }

    #[test]
    fn overlay_keys_move_accept_and_dismiss() {
        let cases = [
            ("up", Some(OverlayKey::Move(-1))),
            ("down", Some(OverlayKey::Move(1))),
            ("ctrl+p", Some(OverlayKey::Move(-1))),
            ("ctrl+n", Some(OverlayKey::Move(1))),
            ("enter", Some(OverlayKey::Accept { new_tab: false })),
            ("cmd+enter", Some(OverlayKey::Accept { new_tab: true })),
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
    fn only_command_q_quits() {
        assert!(is_quit(parsed("cmd+q")));
        assert!(!is_quit(parsed("cmd+shift+q")));
        assert!(!is_quit(parsed("q")));
        assert!(!is_quit(parsed("cmd+w")));
    }
}
