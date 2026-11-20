//! Keys that type, and keys an input method is composing with: both go on to the page or the
//! field without the keymap being asked.
//!
//! No binding may start with a stroke that types (the commands context refuses one), so for
//! such a stroke the keymap's answer is known before it is asked: nothing. The browser lets it
//! go on before it builds a `when` context, which reads the focus reports, asks the engine for
//! the page's view, and walks the first responder; typing costs one check.
//!
//! While an input method composes (marked text: Japanese, Chinese, Korean), every key without
//! `⌘` is the field's, `⌥` and `⌃` keys included: the input method reads them. `⌘` stays the
//! browser's, as menu keys stay an app's on macOS during composition.

use quickie_domain_commands::{KeyStroke, Modifiers};

/// Whether the keymap is asked about `stroke`. It is not when no chord waits for a second
/// stroke and `stroke` types, or when a field is `composing` and `⌘` is not held. `composing`
/// is asked only when the answer depends on it, and never for a key that types.
#[must_use]
pub fn asks_keymap(
    stroke: KeyStroke,
    pending: Option<KeyStroke>,
    composing: impl FnOnce() -> bool,
) -> bool {
    if pending.is_none() && stroke.types_text() {
        return false;
    }
    stroke.modifiers.contains(Modifiers::COMMAND) || !composing()
}

#[cfg(test)]
mod tests {
    use quickie_domain_commands::{
        ContextFlag, FocusedView, KeyStroke, Keymap, Modifiers, Resolution, WhenContext,
    };

    use super::asks_keymap;
    use crate::PLATFORM;
    use crate::input::keystroke::{Read, read};

    /// The typing keys by key code, in the order of [`LAYOUTS`]' strings: the letters `a` … `z`,
    /// the digits `1` … `0`, then `` ` - = [ ] \ ; ' , . / `` and the space bar.
    const CODES: [u16; 48] = [
        0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D, 0x1F,
        0x23, 0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06, // a … z
        0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19, 0x1D, // 1 … 0
        0x32, 0x1B, 0x18, 0x21, 0x1E, 0x2A, 0x29, 0x27, 0x2B, 0x2F, 0x2C, // punctuation
        0x31, // space
    ];

    /// What each key of [`CODES`] types unshifted, one character each, on layouts whose keys
    /// type no Latin letter or type one where a US keyboard has another, as `AppKit` reports the
    /// characters ignoring modifiers; each is read bare and with `⇧`. `\0` stands for a dead key
    /// that types nothing yet. A pinyin or romaji input method reads Latin letters.
    const LAYOUTS: [(&str, &str); 11] = [
        ("US", "abcdefghijklmnopqrstuvwxyz1234567890`-=[]\\;',./ "),
        (
            "Russian",
            "фисвуапршолдьтщзйкыегмцчня1234567890ё-=хъ\\жэбю. ",
        ),
        ("Arabic", "شﻻؤيثبلاهتنمةىخحضقسفعرصءغئ١٢٣٤٥٦٧٨٩٠ذ-=جد\\كطوزظ "),
        (
            "Hebrew",
            "שנבגקכעיןחלךצמםפ/רדאוה'סטז1234567890;-=][\\ף,תץ. ",
        ),
        ("Greek", "αβψδεφγηιξκλμνοπ;ρστθωςχυζ1234567890`-=[]\\΄',./ "),
        ("Thai", "ฟิแกำดเ้รา่สทืนยๆพหะีอไปัผๅ/-ภถุึคตจ_ขชบลฃวงมใฝ "),
        (
            "Devanagari InScript",
            "ोवम्ािुपगरकतसलदजौीेूहनैंबॆ१२३४५६७८९०ॊ-ृड़ॉचट,.य ",
        ),
        (
            "Japanese kana",
            "ちこそしいはきくにまのりもみらせたすとかなひてさんつぬふあうえおやゆよわろほへ゛゜むれけねるめ ",
        ),
        (
            "Korean 2-set",
            "ㅁㅠㅊㅇㄷㄹㅎㅗㅑㅓㅏㅣㅡㅜㅐㅔㅂㄱㄴㅅㅕㅍㅈㅌㅛㅋ1234567890`-=[]\\;',./ ",
        ),
        (
            "German",
            "abcdefghijklmnopqrstuvwxzy1234567890^ß\u{0}ü+#öä,.- ",
        ),
        (
            "Chinese pinyin",
            "abcdefghijklmnopqrstuvwxyz1234567890·-=【】、；'，。/ ",
        ),
    ];

    /// Where a key that types can be pressed: on a page, in its field, and with each of the
    /// browser's overlays up.
    fn contexts() -> Vec<WhenContext> {
        let page = WhenContext::new().with(ContextFlag::PageFocused);
        let overlay = |flag, view| {
            WhenContext::new()
                .with(flag)
                .with(ContextFlag::InputFocused)
                .in_view(view)
        };
        vec![
            page.clone().in_view(FocusedView::Page),
            page.clone()
                .with(ContextFlag::InputFocused)
                .in_view(FocusedView::Page),
            page.with(ContextFlag::HintsOpen).in_view(FocusedView::Page),
            overlay(ContextFlag::PaletteOpen, FocusedView::Palette),
            overlay(ContextFlag::AddressOpen, FocusedView::Address),
            overlay(ContextFlag::OverviewOpen, FocusedView::Overview),
            WhenContext::new().in_view(FocusedView::Settings),
        ]
    }

    /// Every typing key of every layout, bare and with `⇧`, as the browser reads it.
    fn typing_strokes() -> Vec<(String, KeyStroke)> {
        let mut strokes = Vec::new();
        for (layout, typed) in LAYOUTS {
            let typed: Vec<char> = typed.chars().collect();
            assert_eq!(typed.len(), CODES.len(), "{layout}");
            for (&code, &c) in CODES.iter().zip(&typed) {
                let typed = if c == '\u{0}' {
                    String::new()
                } else {
                    c.to_string()
                };
                for held in [Modifiers::NONE, Modifiers::SHIFT] {
                    let Read::Stroke(stroke) = read(code, &typed, held, false) else {
                        panic!("{layout}: {code:#x} {typed:?} reads as a stroke");
                    };
                    strokes.push((format!("{layout}: {code:#x} {typed:?} {held:?}"), stroke));
                }
            }
        }
        strokes
    }

    fn stroke(text: &str) -> KeyStroke {
        KeyStroke::try_from(text).unwrap()
    }

    fn unasked() -> bool {
        panic!("a key that types never asks whether a field composes")
    }

    #[test]
    fn every_typing_key_of_every_layout_goes_on_without_the_keymap() {
        for (what, stroke) in typing_strokes() {
            assert!(!asks_keymap(stroke, None, unasked), "{what}");
        }
    }

    #[test]
    fn the_default_keymap_binds_no_typing_key_anywhere() {
        let keymap = Keymap::defaults(PLATFORM);
        let contexts = contexts();

        for (what, stroke) in typing_strokes() {
            for context in &contexts {
                let resolution = keymap.resolve(stroke, None, context);

                assert_eq!(resolution, Resolution::Unbound, "{what} in {context:?}");
            }
        }
    }

    #[test]
    fn a_typing_key_completes_a_waiting_chord() {
        let asked = asks_keymap(stroke("s"), Some(stroke("cmd+k")), || false);

        assert!(asked);
    }

    #[test]
    fn a_shortcut_asks_the_keymap() {
        for text in ["alt+f", "shift+alt+f", "ctrl+tab", "cmd+t", "escape", "f5"] {
            assert!(asks_keymap(stroke(text), None, || false), "{text}");
        }
    }

    #[test]
    fn option_f_is_read_by_its_key_code() {
        // ⌥F types `ƒ` on a US layout; `AppKit` reports `f` ignoring modifiers, and either
        // way the key code names the key.
        for typed in ["f", "ƒ", "а"] {
            let read = read(0x03, typed, Modifiers::OPTION, false);

            assert_eq!(read, Read::Stroke(stroke("alt+f")), "{typed}");
        }
    }

    #[test]
    fn while_a_field_composes_option_and_control_keys_are_the_fields() {
        for text in [
            "alt+f",
            "ctrl+k",
            "ctrl+tab",
            "shift+alt+f",
            "escape",
            "enter",
        ] {
            assert!(!asks_keymap(stroke(text), None, || true), "{text}");
        }
    }

    #[test]
    fn while_a_field_composes_command_keys_stay_the_browsers() {
        for text in ["cmd+t", "cmd+shift+p", "alt+cmd+i"] {
            assert!(asks_keymap(stroke(text), None, || true), "{text}");
        }
    }

    #[test]
    fn while_a_field_composes_a_waiting_chord_gives_way() {
        let asked = asks_keymap(stroke("s"), Some(stroke("cmd+k")), || true);

        assert!(!asked);
    }

    #[test]
    fn a_dead_key_with_option_is_unbound_by_default() {
        // ⌥E starts an accent on a US layout; then `E` types, as any letter does.
        let keymap = Keymap::defaults(PLATFORM);
        let page = WhenContext::new()
            .with(ContextFlag::PageFocused)
            .in_view(FocusedView::Page);
        let Read::Stroke(accent) = read(0x0E, "e", Modifiers::OPTION, false) else {
            panic!("⌥E reads as a stroke");
        };

        assert_eq!(keymap.resolve(accent, None, &page), Resolution::Unbound);
        assert!(!asks_keymap(stroke("e"), None, unasked));
    }
}
