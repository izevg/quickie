use super::{
    Chord, ChordError, Digit, FunctionKey, Key, KeyStroke, KeyStrokeError, Letter, Modifiers,
};

fn stroke(text: &str) -> KeyStroke {
    KeyStroke::try_from(text).unwrap()
}

fn letter(c: char) -> Key {
    Key::Letter(Letter::new(c).unwrap())
}

/// Every named key with its `keybindings.json` name and its glyph.
const NAMED: [(Key, &str, &str); 25] = [
    (Key::Tab, "tab", "⇥"),
    (Key::Enter, "enter", "↩"),
    (Key::Escape, "escape", "⎋"),
    (Key::Space, "space", "Space"),
    (Key::Backspace, "backspace", "⌫"),
    (Key::Delete, "delete", "⌦"),
    (Key::Up, "up", "↑"),
    (Key::Down, "down", "↓"),
    (Key::Left, "left", "←"),
    (Key::Right, "right", "→"),
    (Key::Home, "home", "↖"),
    (Key::End, "end", "↘"),
    (Key::PageUp, "pageup", "⇞"),
    (Key::PageDown, "pagedown", "⇟"),
    (Key::BracketLeft, "[", "["),
    (Key::BracketRight, "]", "]"),
    (Key::Minus, "-", "-"),
    (Key::Equal, "=", "="),
    (Key::Comma, ",", ","),
    (Key::Period, ".", "."),
    (Key::Slash, "/", "/"),
    (Key::Backslash, "\\", "\\"),
    (Key::Semicolon, ";", ";"),
    (Key::Quote, "'", "'"),
    (Key::Backquote, "`", "`"),
];

// ---- keys --------------------------------------------------------------------------------------

#[test]
fn every_named_key_has_its_name_and_glyph() {
    for (key, name, glyph) in NAMED {
        assert_eq!(key.name(), name, "{key:?}");
        assert_eq!(key.glyph(), glyph, "{key:?}");
        assert_eq!(key.to_string(), glyph, "{key:?}");
    }
}

#[test]
fn every_named_key_parses_from_its_name() {
    for (key, name, _) in NAMED {
        assert_eq!(Key::from_name(name), Some(key), "{name}");
    }
}

#[test]
fn a_named_key_parses_in_any_case() {
    let key = Key::from_name("PageUp");

    assert_eq!(key, Some(Key::PageUp));
}

#[test]
fn a_letter_is_named_lowercase_and_shown_uppercase() {
    let key = Key::from_name("P").unwrap();

    assert_eq!(key, letter('p'));
    assert_eq!(key.name(), "p");
    assert_eq!(key.glyph(), "P");
}

#[test]
fn a_digit_is_its_own_name_and_glyph() {
    let key = Key::from_name("7").unwrap();

    assert_eq!(key, Key::Digit(Digit::new('7').unwrap()));
    assert_eq!(key.name(), "7");
    assert_eq!(key.glyph(), "7");
}

#[test]
fn a_function_key_is_named_with_a_lowercase_f_and_shown_with_a_capital() {
    let key = Key::from_name("F12").unwrap();

    assert_eq!(key, Key::Function(FunctionKey::new(12).unwrap()));
    assert_eq!(key.name(), "f12");
    assert_eq!(key.glyph(), "F12");
}

#[test]
fn a_function_key_beyond_twelve_is_unknown() {
    let key = Key::from_name("f13");

    assert_eq!(key, None);
}

#[test]
fn a_plain_f_is_the_letter() {
    let key = Key::from_name("f");

    assert_eq!(key, Some(letter('f')));
}

#[test]
fn an_unknown_name_is_no_key() {
    let key = Key::from_name("hyper");

    assert_eq!(key, None);
}

#[test]
fn a_single_symbol_that_is_no_key_is_none() {
    let key = Key::from_name("§");

    assert_eq!(key, None);
}

#[test]
fn a_letter_accepts_both_cases_and_keeps_lowercase() {
    let upper = Letter::new('Q').unwrap();

    assert_eq!(upper.as_char(), 'q');
    assert_eq!(Letter::new('q'), Some(upper));
}

#[test]
fn a_letter_is_only_ascii_alphabetic() {
    assert_eq!(Letter::new('1'), None);
    assert_eq!(Letter::new('ж'), None);
}

#[test]
fn a_digit_is_only_ascii_numeric() {
    assert_eq!(Digit::new('0').map(Digit::as_char), Some('0'));
    assert_eq!(Digit::new('a'), None);
}

#[test]
fn function_keys_run_from_one_to_twelve() {
    assert_eq!(FunctionKey::new(0), None);
    assert_eq!(FunctionKey::new(1).map(FunctionKey::number), Some(1));
    assert_eq!(FunctionKey::new(12).map(FunctionKey::number), Some(12));
    assert_eq!(FunctionKey::new(13), None);
}

// ---- modifiers ---------------------------------------------------------------------------------

#[test]
fn a_set_contains_each_modifier_it_was_built_from() {
    let set = Modifiers::COMMAND | Modifiers::SHIFT;

    assert!(set.contains(Modifiers::COMMAND));
    assert!(set.contains(Modifiers::SHIFT));
    assert!(!set.contains(Modifiers::OPTION));
}

#[test]
fn a_single_modifier_does_not_contain_a_larger_set() {
    let set = Modifiers::COMMAND;

    assert!(!set.contains(Modifiers::COMMAND | Modifiers::SHIFT));
}

#[test]
fn adding_a_held_modifier_again_changes_nothing() {
    let set = Modifiers::COMMAND | Modifiers::COMMAND;

    assert_eq!(set, Modifiers::COMMAND);
}

#[test]
fn modifiers_are_shown_in_macos_order() {
    let set = Modifiers::COMMAND | Modifiers::SHIFT | Modifiers::OPTION | Modifiers::CONTROL;

    assert_eq!(set.to_string(), "⌃⌥⇧⌘");
    assert_eq!(
        set.names().collect::<Vec<_>>(),
        ["ctrl", "alt", "shift", "cmd"]
    );
}

#[test]
fn no_modifiers_show_nothing() {
    let set = Modifiers::default();

    assert_eq!(set, Modifiers::NONE);
    assert_eq!(set.to_string(), "");
    assert_eq!(set.names().count(), 0);
}

#[test]
fn every_modifier_alias_is_understood() {
    let aliases = [
        ("ctrl", Modifiers::CONTROL),
        ("control", Modifiers::CONTROL),
        ("alt", Modifiers::OPTION),
        ("option", Modifiers::OPTION),
        ("opt", Modifiers::OPTION),
        ("shift", Modifiers::SHIFT),
        ("cmd", Modifiers::COMMAND),
        ("command", Modifiers::COMMAND),
        ("meta", Modifiers::COMMAND),
        ("CMD", Modifiers::COMMAND),
    ];

    for (name, modifier) in aliases {
        assert_eq!(Modifiers::from_name(name), Some(modifier), "{name}");
    }
}

#[test]
fn an_unknown_modifier_name_is_none() {
    let modifier = Modifiers::from_name("hyper");

    assert_eq!(modifier, None);
}

// ---- strokes -----------------------------------------------------------------------------------

#[test]
fn a_stroke_parses_modifiers_and_key() {
    let parsed = stroke("cmd+shift+p");

    assert_eq!(
        parsed,
        KeyStroke::new(letter('p'), Modifiers::COMMAND | Modifiers::SHIFT)
    );
}

#[test]
fn a_stroke_is_shown_as_glyphs_in_macos_order() {
    let parsed = stroke("cmd+shift+p");

    assert_eq!(parsed.to_string(), "⇧⌘P");
}

#[test]
fn a_stroke_is_written_back_with_modifiers_in_macos_order() {
    let parsed = stroke("cmd+alt+shift+ctrl+tab");

    assert_eq!(parsed.keybinding_text(), "ctrl+alt+shift+cmd+tab");
    assert_eq!(parsed.to_string(), "⌃⌥⇧⌘⇥");
}

#[test]
fn a_bare_key_is_a_stroke_without_modifiers() {
    let parsed = stroke("f");

    assert_eq!(parsed.modifiers, Modifiers::NONE);
    assert_eq!(parsed.keybinding_text(), "f");
    assert_eq!(parsed.to_string(), "F");
}

#[test]
fn every_stroke_round_trips_through_its_text() {
    let texts = [
        "ctrl+tab",
        "ctrl+shift+tab",
        "cmd+[",
        "alt+cmd+i",
        "cmd+1",
        "f5",
        "shift+space",
    ];

    for text in texts {
        let parsed = stroke(text);

        assert_eq!(parsed.keybinding_text(), text);
        assert_eq!(stroke(&parsed.keybinding_text()), parsed);
    }
}

#[test]
fn an_empty_stroke_is_rejected() {
    let error = KeyStroke::try_from("").unwrap_err();

    assert_eq!(error, KeyStrokeError::EmptyPart);
}

#[test]
fn a_trailing_plus_is_an_empty_part() {
    let error = KeyStroke::try_from("cmd+").unwrap_err();

    assert_eq!(error, KeyStrokeError::EmptyPart);
}

#[test]
fn a_leading_plus_is_an_empty_part() {
    let error = KeyStroke::try_from("+p").unwrap_err();

    assert_eq!(error, KeyStrokeError::EmptyPart);
}

#[test]
fn modifiers_without_a_key_are_rejected() {
    let error = KeyStroke::try_from("cmd+shift").unwrap_err();

    assert_eq!(error, KeyStrokeError::MissingKey);
}

#[test]
fn an_unknown_modifier_is_rejected() {
    let error = KeyStroke::try_from("hyper+p").unwrap_err();

    assert_eq!(error, KeyStrokeError::UnknownModifier("hyper".to_owned()));
}

#[test]
fn a_modifier_held_twice_is_rejected() {
    let error = KeyStroke::try_from("cmd+meta+p").unwrap_err();

    assert_eq!(error, KeyStrokeError::DuplicateModifier("meta".to_owned()));
}

#[test]
fn an_unknown_key_is_rejected() {
    let error = KeyStroke::try_from("cmd+hyper").unwrap_err();

    assert_eq!(error, KeyStrokeError::UnknownKey("hyper".to_owned()));
}

// ---- chords ------------------------------------------------------------------------------------

#[test]
fn a_single_stroke_is_a_chord_of_one() {
    let chord = Chord::try_from("cmd+k").unwrap();

    assert_eq!(chord, Chord::from(stroke("cmd+k")));
    assert_eq!(chord.to_string(), "⌘K");
    assert_eq!(chord.keybinding_text(), "cmd+k");
}

#[test]
fn two_strokes_make_a_chord() {
    let chord = Chord::try_from("cmd+k  cmd+s").unwrap();

    assert_eq!(chord.first, stroke("cmd+k"));
    assert_eq!(chord.second, Some(stroke("cmd+s")));
    assert_eq!(chord.to_string(), "⌘K ⌘S");
    assert_eq!(chord.keybinding_text(), "cmd+k cmd+s");
}

#[test]
fn an_empty_chord_is_rejected() {
    let error = Chord::try_from("   ").unwrap_err();

    assert_eq!(error, ChordError::Empty);
}

#[test]
fn a_chord_of_three_strokes_is_rejected() {
    let error = Chord::try_from("cmd+k cmd+s cmd+d").unwrap_err();

    assert_eq!(error, ChordError::TooManyStrokes);
}

#[test]
fn a_bad_first_stroke_fails_the_chord() {
    let error = Chord::try_from("cmd+ cmd+s").unwrap_err();

    assert_eq!(error, ChordError::Stroke(KeyStrokeError::EmptyPart));
}

#[test]
fn a_bad_second_stroke_fails_the_chord() {
    let error = Chord::try_from("cmd+k shift").unwrap_err();

    assert_eq!(error, ChordError::Stroke(KeyStrokeError::MissingKey));
}

#[test]
fn a_stroke_error_reads_as_itself_inside_a_chord_error() {
    let error = ChordError::from(KeyStrokeError::UnknownKey("hyper".to_owned()));

    assert_eq!(error.to_string(), "`hyper` is not a key");
}
