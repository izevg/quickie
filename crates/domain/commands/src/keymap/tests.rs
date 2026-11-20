use quickie_domain_kernel::{CommandId, CommandIdError};

use super::defaults::{self, BINDINGS};
use super::{
    BindingError, Conflict, KeyBinding, Keymap, KeymapEntry, KeymapError, Resolution, Unbinding,
    WhenContext, WhenError, WhenExpr,
};
use crate::command::Catalog;
use crate::key::{Chord, ChordError, KeyStroke, KeyStrokeError, Platform, TypingStroke};

const MAC: Platform = Platform::MacOs;
const LINUX: Platform = Platform::Linux;

fn stroke(text: &str) -> KeyStroke {
    KeyStroke::try_from(text).unwrap()
}

fn id(text: &str) -> CommandId {
    CommandId::try_from(text).unwrap()
}

fn bind(key: &str, command: &str) -> KeyBinding {
    KeyBinding::parse(key, command, None, MAC).unwrap()
}

fn bind_when(key: &str, command: &str, when: &str) -> KeyBinding {
    KeyBinding::parse(key, command, Some(when), MAC).unwrap()
}

fn entry(key: &str, command: &str, when: Option<&str>) -> KeymapEntry {
    KeymapEntry::parse(key, command, when, MAC).unwrap()
}

fn keymap(defaults: Vec<KeyBinding>, entries: Vec<KeymapEntry>) -> Keymap {
    Keymap::new(&Catalog::builtin(), MAC, defaults, entries).unwrap()
}

fn glyphs(keymap: &Keymap, command: &str) -> Vec<String> {
    let command = id(command);
    keymap
        .bindings_for(&command)
        .map(|binding| binding.chord.glyphs(keymap.platform()))
        .collect()
}

fn resolve(keymap: &Keymap, key: &str, context: &WhenContext) -> Resolution {
    keymap.resolve(stroke(key), None, context)
}

fn command(text: &str) -> Resolution {
    Resolution::Command(id(text))
}

const CLOSE: &str = "quickie.tab.close";
const FIND: &str = "quickie.page.find";
const PALETTE: &str = "quickie.palette.openCommands";
const HINTS: &str = "quickie.hints.toggle";

// ---- defaults ----------------------------------------------------------------------------------

/// The macOS defaults as they were written before `mod` existed, row for row, plus the rows
/// added since in their `cmd` form: the keymap of `mod` must stay exactly this on macOS.
const MACOS_BEFORE_MOD: [(&str, &str, Option<&str>); 54] = [
    ("cmd+shift+p", "quickie.palette.openCommands", None),
    ("cmd+k", "quickie.palette.openCommands", None),
    ("cmd+p", "quickie.palette.goto", None),
    ("cmd+l", "quickie.address.open", None),
    ("cmd+t", "quickie.tab.new", None),
    ("cmd+shift+n", "quickie.tab.newThrowaway", None),
    ("cmd+w", "quickie.tab.close", None),
    ("cmd+n", "quickie.window.new", None),
    ("cmd+shift+w", "quickie.window.close", None),
    ("cmd+shift+t", "quickie.tab.reopenClosed", None),
    ("ctrl+tab", "quickie.tab.nextRecent", None),
    ("ctrl+shift+tab", "quickie.tab.previousRecent", None),
    ("cmd+1", "quickie.tab.goToSlot1", None),
    ("cmd+2", "quickie.tab.goToSlot2", None),
    ("cmd+3", "quickie.tab.goToSlot3", None),
    ("cmd+4", "quickie.tab.goToSlot4", None),
    ("cmd+5", "quickie.tab.goToSlot5", None),
    ("cmd+6", "quickie.tab.goToSlot6", None),
    ("cmd+7", "quickie.tab.goToSlot7", None),
    ("cmd+8", "quickie.tab.goToSlot8", None),
    ("cmd+9", "quickie.tab.goToSlot9", None),
    ("alt+cmd+1", "quickie.tab.pinToSlot1", None),
    ("alt+cmd+2", "quickie.tab.pinToSlot2", None),
    ("alt+cmd+3", "quickie.tab.pinToSlot3", None),
    ("alt+cmd+4", "quickie.tab.pinToSlot4", None),
    ("alt+cmd+5", "quickie.tab.pinToSlot5", None),
    ("alt+cmd+6", "quickie.tab.pinToSlot6", None),
    ("alt+cmd+7", "quickie.tab.pinToSlot7", None),
    ("alt+cmd+8", "quickie.tab.pinToSlot8", None),
    ("alt+cmd+9", "quickie.tab.pinToSlot9", None),
    ("cmd+r", "quickie.tab.reload", None),
    ("cmd+shift+r", "quickie.tab.hardReload", None),
    ("cmd+=", "quickie.page.zoomIn", None),
    ("cmd+shift+=", "quickie.page.zoomIn", None),
    ("cmd+-", "quickie.page.zoomOut", None),
    ("cmd+0", "quickie.page.zoomReset", None),
    ("alt+cmd+p", "quickie.page.print", None),
    ("cmd+[", "quickie.navigation.back", None),
    ("cmd+]", "quickie.navigation.forward", None),
    ("cmd+f", "quickie.page.find", None),
    ("cmd+g", "quickie.page.findNext", None),
    ("cmd+shift+g", "quickie.page.findPrevious", None),
    ("alt+f", "quickie.hints.toggle", Some("pageFocused")),
    (
        "shift+alt+f",
        "quickie.hints.openInBackground",
        Some("pageFocused"),
    ),
    ("cmd+shift+\\", "quickie.tab.overview", None),
    ("cmd+shift+c", "quickie.page.copyUrl", None),
    ("alt+cmd+c", "quickie.page.copyAsMarkdownLink", None),
    ("cmd+d", "quickie.page.bookmark", None),
    ("cmd+j", "quickie.downloads.show", None),
    ("alt+cmd+s", "quickie.shields.open", None),
    ("cmd+.", "quickie.agent.stop", None),
    ("alt+cmd+i", "quickie.devtools.toggle", None),
    ("alt+cmd+j", "quickie.devtools.console", None),
    ("cmd+,", "quickie.app.settings", None),
];

#[test]
fn the_macos_defaults_are_the_same_strokes_as_before_mod() {
    let before: Vec<KeyBinding> = MACOS_BEFORE_MOD
        .into_iter()
        .map(|(key, command, when)| KeyBinding {
            chord: Chord::try_from(key).unwrap(),
            command: id(command),
            when: when.map(|when| WhenExpr::try_from(when).unwrap()),
        })
        .collect();

    let after: Vec<KeyBinding> = Keymap::defaults(MAC).bindings().cloned().collect();

    assert_eq!(after, before);
}

#[test]
fn every_default_row_parses_on_every_platform() {
    for platform in [MAC, LINUX] {
        let bindings = defaults::bindings(platform);

        assert_eq!(bindings.len(), BINDINGS.len(), "{platform:?}");
    }
}

#[test]
fn every_reserved_row_parses_on_every_platform() {
    for platform in [MAC, LINUX] {
        let reserved = defaults::reserved(platform);

        assert_eq!(reserved.len(), defaults::RESERVED.len(), "{platform:?}");
    }
}

#[test]
fn the_defaults_make_a_valid_keymap() {
    let validated = Keymap::new(
        &Catalog::builtin(),
        MAC,
        defaults::bindings(MAC),
        Vec::new(),
    );

    assert_eq!(validated, Ok(Keymap::defaults(MAC)));
}

#[test]
fn the_linux_defaults_make_a_valid_keymap() {
    let validated = Keymap::new(
        &Catalog::builtin(),
        LINUX,
        defaults::bindings(LINUX),
        Vec::new(),
    );

    assert_eq!(validated, Ok(Keymap::defaults(LINUX)));
}

#[test]
fn a_keymap_keeps_the_platform_it_was_made_for() {
    let mac = Keymap::defaults(MAC);
    let linux = Keymap::defaults(LINUX);

    assert_eq!(mac.platform(), MAC);
    assert_eq!(linux.platform(), LINUX);
}

#[test]
fn the_default_keymap_has_no_user_entries() {
    let keymap = Keymap::defaults(MAC);

    assert_eq!(keymap.user_entries(), []);
    assert_eq!(keymap.bindings().count(), BINDINGS.len());
}

#[test]
fn the_linux_defaults_use_ctrl_where_macos_uses_command() {
    let keymap = Keymap::defaults(LINUX);
    let context = WhenContext::new().with_flag("pageFocused");
    let table = [
        ("ctrl+shift+p", PALETTE),
        ("ctrl+t", "quickie.tab.new"),
        ("ctrl+w", CLOSE),
        ("ctrl+tab", "quickie.tab.nextRecent"),
        ("ctrl+shift+tab", "quickie.tab.previousRecent"),
        ("ctrl+1", "quickie.tab.goToSlot1"),
        ("alt+ctrl+1", "quickie.tab.pinToSlot1"),
        ("alt+ctrl+i", "quickie.devtools.toggle"),
        ("alt+f", HINTS),
    ];

    for (key, expected) in table {
        assert_eq!(resolve(&keymap, key, &context), command(expected), "{key}");
    }
}

#[test]
fn the_linux_defaults_leave_super_to_the_page() {
    let keymap = Keymap::defaults(LINUX);

    let resolution = resolve(&keymap, "super+t", &WhenContext::new());

    assert_eq!(resolution, Resolution::Unbound);
}

#[test]
fn the_default_keymap_binds_the_readme_keys() {
    let keymap = Keymap::defaults(MAC);
    let context = WhenContext::new().with_flag("pageFocused");
    let table = [
        ("cmd+shift+p", PALETTE),
        ("cmd+k", PALETTE),
        ("cmd+p", "quickie.palette.goto"),
        ("cmd+l", "quickie.address.open"),
        ("cmd+t", "quickie.tab.new"),
        ("ctrl+tab", "quickie.tab.nextRecent"),
        ("ctrl+shift+tab", "quickie.tab.previousRecent"),
        ("cmd+1", "quickie.tab.goToSlot1"),
        ("cmd+9", "quickie.tab.goToSlot9"),
        ("cmd+[", "quickie.navigation.back"),
        ("cmd+]", "quickie.navigation.forward"),
        ("cmd+f", FIND),
        ("alt+f", HINTS),
        ("shift+alt+f", "quickie.hints.openInBackground"),
        ("cmd+shift+c", "quickie.page.copyUrl"),
        ("alt+cmd+c", "quickie.page.copyAsMarkdownLink"),
        ("cmd+shift+t", "quickie.tab.reopenClosed"),
        ("cmd+w", CLOSE),
        ("cmd+j", "quickie.downloads.show"),
        ("alt+cmd+s", "quickie.shields.open"),
        ("alt+cmd+i", "quickie.devtools.toggle"),
        ("alt+cmd+j", "quickie.devtools.console"),
        ("cmd+r", "quickie.tab.reload"),
        ("cmd+d", "quickie.page.bookmark"),
        ("cmd+shift+n", "quickie.tab.newThrowaway"),
        ("alt+cmd+1", "quickie.tab.pinToSlot1"),
        ("alt+cmd+5", "quickie.tab.pinToSlot5"),
        ("alt+cmd+9", "quickie.tab.pinToSlot9"),
    ];

    for (key, expected) in table {
        assert_eq!(resolve(&keymap, key, &context), command(expected), "{key}");
    }
}

#[test]
fn link_hints_open_from_a_text_field_too() {
    let keymap = Keymap::defaults(MAC);
    let typing = WhenContext::new()
        .with_flag("pageFocused")
        .with_flag("inputFocused");

    let resolution = resolve(&keymap, "alt+f", &typing);

    assert_eq!(resolution, command(HINTS));
}

#[test]
fn a_bare_f_types_into_the_page() {
    let keymap = Keymap::defaults(MAC);
    let page = WhenContext::new().with_flag("pageFocused");

    for key in ["f", "shift+f"] {
        assert_eq!(resolve(&keymap, key, &page), Resolution::Unbound, "{key}");
    }
}

#[test]
fn no_default_starts_with_a_typing_stroke() {
    for platform in [MAC, LINUX] {
        let keymap = Keymap::defaults(platform);

        for binding in keymap.bindings() {
            assert_eq!(binding.chord.check_shortcut(), Ok(()), "{binding:?}");
        }
    }
}

// ---- reserved strokes --------------------------------------------------------------------------

#[test]
fn the_browser_keys_are_reserved() {
    let keymap = Keymap::defaults(MAC);

    for key in [
        "cmd+q",
        "cmd+w",
        "cmd+t",
        "cmd+l",
        "cmd+shift+p",
        "cmd+p",
        "ctrl+tab",
    ] {
        assert!(keymap.is_reserved(stroke(key)), "{key}");
    }
}

#[test]
fn other_keys_are_not_reserved() {
    let keymap = Keymap::defaults(MAC);
    let keys = [
        "cmd+a",
        "cmd+shift+q",
        "cmd+shift+t",
        "ctrl+p",
        "p",
        "ctrl+shift+tab",
        "cmd+tab",
        "tab",
        "cmd+1",
    ];

    for key in keys {
        assert!(!keymap.is_reserved(stroke(key)), "{key}");
    }
}

#[test]
fn the_browser_keys_are_reserved_with_ctrl_on_linux() {
    let keymap = Keymap::defaults(LINUX);

    for key in [
        "ctrl+q",
        "ctrl+w",
        "ctrl+t",
        "ctrl+l",
        "ctrl+shift+p",
        "ctrl+p",
        "ctrl+tab",
    ] {
        assert!(keymap.is_reserved(stroke(key)), "{key}");
    }
}

#[test]
fn the_command_keys_are_not_reserved_on_linux() {
    let keymap = Keymap::defaults(LINUX);

    for key in ["cmd+q", "cmd+w", "cmd+t", "cmd+shift+p", "ctrl+shift+tab"] {
        assert!(!keymap.is_reserved(stroke(key)), "{key}");
    }
}

// ---- resolution --------------------------------------------------------------------------------

#[test]
fn an_unbound_stroke_goes_to_the_page() {
    let keymap = keymap(vec![bind("cmd+w", CLOSE)], Vec::new());

    let resolution = resolve(&keymap, "cmd+e", &WhenContext::new());

    assert_eq!(resolution, Resolution::Unbound);
}

#[test]
fn a_bound_stroke_runs_its_command() {
    let keymap = keymap(vec![bind("cmd+w", CLOSE)], Vec::new());

    let resolution = resolve(&keymap, "cmd+w", &WhenContext::new());

    assert_eq!(resolution, command(CLOSE));
}

#[test]
fn a_binding_whose_clause_fails_is_skipped() {
    let keymap = keymap(
        vec![
            bind("cmd+e", CLOSE),
            bind_when("cmd+e", FIND, "pageFocused"),
        ],
        Vec::new(),
    );

    let resolution = resolve(&keymap, "cmd+e", &WhenContext::new());

    assert_eq!(resolution, command(CLOSE));
}

#[test]
fn the_later_binding_wins_when_both_clauses_hold() {
    let keymap = keymap(
        vec![
            bind("cmd+e", CLOSE),
            bind_when("cmd+e", FIND, "pageFocused"),
        ],
        Vec::new(),
    );

    let resolution = resolve(
        &keymap,
        "cmd+e",
        &WhenContext::new().with_flag("pageFocused"),
    );

    assert_eq!(resolution, command(FIND));
}

#[test]
fn a_user_binding_wins_over_a_default() {
    let keymap = keymap(
        vec![bind("cmd+e", CLOSE)],
        vec![entry("cmd+e", FIND, Some("pageFocused"))],
    );

    let resolution = resolve(
        &keymap,
        "cmd+e",
        &WhenContext::new().with_flag("pageFocused"),
    );

    assert_eq!(resolution, command(FIND));
}

#[test]
fn the_first_stroke_of_a_chord_is_pending() {
    let keymap = keymap(vec![bind("cmd+k cmd+s", FIND)], Vec::new());

    let resolution = resolve(&keymap, "cmd+k", &WhenContext::new());

    assert_eq!(resolution, Resolution::Pending(stroke("cmd+k")));
}

#[test]
fn the_second_stroke_completes_a_chord() {
    let keymap = keymap(vec![bind("cmd+k cmd+s", FIND)], Vec::new());

    let resolution = keymap.resolve(stroke("cmd+s"), Some(stroke("cmd+k")), &WhenContext::new());

    assert_eq!(resolution, command(FIND));
}

#[test]
fn a_wrong_second_stroke_ends_the_chord_unbound() {
    let keymap = keymap(
        vec![bind("cmd+k cmd+s", FIND), bind("cmd+d", CLOSE)],
        Vec::new(),
    );

    let resolution = keymap.resolve(stroke("cmd+d"), Some(stroke("cmd+k")), &WhenContext::new());

    assert_eq!(resolution, Resolution::Unbound);
}

#[test]
fn a_second_stroke_after_another_first_stroke_is_unbound() {
    let keymap = keymap(vec![bind("cmd+k cmd+s", FIND)], Vec::new());

    let resolution = keymap.resolve(stroke("cmd+s"), Some(stroke("cmd+j")), &WhenContext::new());

    assert_eq!(resolution, Resolution::Unbound);
}

#[test]
fn a_chord_whose_clause_fails_does_not_complete() {
    let keymap = keymap(
        vec![bind_when("cmd+k cmd+s", FIND, "pageFocused")],
        Vec::new(),
    );

    let resolution = keymap.resolve(stroke("cmd+s"), Some(stroke("cmd+k")), &WhenContext::new());

    assert_eq!(resolution, Resolution::Unbound);
}

#[test]
fn a_single_stroke_binding_is_not_a_chord_completion() {
    let keymap = keymap(vec![bind("cmd+s", FIND)], Vec::new());

    let resolution = keymap.resolve(stroke("cmd+s"), Some(stroke("cmd+k")), &WhenContext::new());

    assert_eq!(resolution, Resolution::Unbound);
}

#[test]
fn a_later_chord_takes_over_its_first_stroke() {
    let keymap = keymap(
        vec![bind("cmd+k", PALETTE)],
        vec![entry("cmd+k cmd+s", FIND, None)],
    );

    let resolution = resolve(&keymap, "cmd+k", &WhenContext::new());

    assert_eq!(resolution, Resolution::Pending(stroke("cmd+k")));
}

#[test]
fn a_later_completion_of_the_same_chord_wins() {
    let keymap = keymap(
        vec![bind("cmd+k cmd+s", FIND)],
        vec![entry("cmd+k cmd+s", CLOSE, Some("pageFocused"))],
    );
    let context = WhenContext::new().with_flag("pageFocused");

    let resolution = keymap.resolve(stroke("cmd+s"), Some(stroke("cmd+k")), &context);

    assert_eq!(resolution, command(CLOSE));
}

// ---- user entries ------------------------------------------------------------------------------

#[test]
fn a_minus_entry_removes_the_default() {
    let keymap = keymap(
        vec![bind("cmd+w", CLOSE)],
        vec![entry("cmd+w", &format!("-{CLOSE}"), None)],
    );

    let resolution = resolve(&keymap, "cmd+w", &WhenContext::new());

    assert_eq!(resolution, Resolution::Unbound);
    assert_eq!(keymap.bindings().count(), 0);
}

#[test]
fn a_removal_names_the_chord_it_removes() {
    let keymap = keymap(
        vec![bind("cmd+shift+p", PALETTE), bind("cmd+k", PALETTE)],
        vec![entry("cmd+k", &format!("-{PALETTE}"), None)],
    );

    let left = glyphs(&keymap, PALETTE);

    assert_eq!(left, ["⇧⌘P"]);
}

#[test]
fn a_removal_names_the_command_it_removes() {
    let keymap = keymap(
        vec![
            bind("cmd+e", CLOSE),
            bind_when("cmd+e", FIND, "pageFocused"),
        ],
        vec![entry("cmd+e", &format!("-{CLOSE}"), None)],
    );

    let left: Vec<&str> = keymap
        .bindings()
        .map(|binding| binding.command.as_str())
        .collect();

    assert_eq!(left, [FIND]);
}

#[test]
fn a_removal_with_a_clause_removes_only_that_clause() {
    let keymap = keymap(
        vec![bind("cmd+e", FIND), bind_when("cmd+e", FIND, "pageFocused")],
        vec![entry("cmd+e", &format!("-{FIND}"), Some("pageFocused"))],
    );

    let left: Vec<Option<WhenExpr>> = keymap
        .bindings()
        .map(|binding| binding.when.clone())
        .collect();

    assert_eq!(left, [None]);
}

#[test]
fn a_removal_without_a_clause_removes_every_clause() {
    let keymap = keymap(
        vec![bind("cmd+e", FIND), bind_when("cmd+e", FIND, "pageFocused")],
        vec![entry("cmd+e", &format!("-{FIND}"), None)],
    );

    assert_eq!(keymap.bindings().count(), 0);
}

#[test]
fn a_removal_does_not_touch_user_bindings() {
    let keymap = keymap(
        Vec::new(),
        vec![
            entry("cmd+e", FIND, None),
            entry("cmd+e", &format!("-{FIND}"), None),
        ],
    );

    assert_eq!(keymap.bindings().count(), 1);
}

#[test]
fn a_removed_default_frees_its_chord_for_another_command() {
    let keymap = keymap(
        vec![bind("cmd+shift+t", "quickie.tab.reopenClosed")],
        vec![
            entry("cmd+shift+t", "-quickie.tab.reopenClosed", None),
            entry("cmd+shift+t", FIND, None),
        ],
    );

    let resolution = resolve(&keymap, "cmd+shift+t", &WhenContext::new());

    assert_eq!(resolution, command(FIND));
}

#[test]
fn user_entries_replace_the_previous_ones() {
    let first = keymap(vec![bind("cmd+w", CLOSE)], vec![entry("cmd+e", FIND, None)]);

    let second = first
        .with_user_entries(&Catalog::builtin(), vec![entry("cmd+d", FIND, None)])
        .unwrap();

    assert_eq!(second.user_entries(), [entry("cmd+d", FIND, None)]);
    assert_eq!(
        resolve(&second, "cmd+w", &WhenContext::new()),
        command(CLOSE)
    );
    assert_eq!(
        resolve(&second, "cmd+e", &WhenContext::new()),
        Resolution::Unbound
    );
}

#[test]
fn replaced_user_entries_are_validated() {
    let first = keymap(vec![bind("cmd+w", CLOSE)], Vec::new());

    let error = first
        .with_user_entries(&Catalog::builtin(), vec![entry("cmd+w", FIND, None)])
        .unwrap_err();

    assert!(matches!(error, KeymapError::Conflict { .. }));
}

#[test]
fn the_bindings_of_a_command_are_listed_for_the_palette() {
    let keymap = Keymap::defaults(MAC);

    let keys = glyphs(&keymap, PALETTE);

    assert_eq!(keys, ["⇧⌘P", "⌘K"]);
}

#[test]
fn the_palette_keys_read_as_linux_writes_them() {
    let keymap = Keymap::defaults(LINUX);

    let keys = glyphs(&keymap, PALETTE);

    assert_eq!(keys, ["Ctrl+Shift+P", "Ctrl+K"]);
}

#[test]
fn a_user_entry_reads_mod_as_the_platform_does() {
    let parsed = KeymapEntry::parse("mod+e", FIND, None, LINUX).unwrap();

    assert_eq!(parsed, entry("ctrl+e", FIND, None));
}

#[test]
fn a_user_removal_reads_mod_as_the_platform_does() {
    let parsed = KeymapEntry::parse("mod+w", &format!("-{CLOSE}"), None, LINUX).unwrap();

    assert_eq!(parsed, entry("ctrl+w", &format!("-{CLOSE}"), None));
}

#[test]
fn replaced_user_entries_keep_the_platform() {
    let linux = Keymap::defaults(LINUX);

    let replaced = linux
        .with_user_entries(&Catalog::builtin(), vec![entry("ctrl+e", FIND, None)])
        .unwrap();

    assert_eq!(replaced.platform(), LINUX);
    assert!(replaced.is_reserved(stroke("ctrl+q")));
}

#[test]
fn an_entry_names_its_command() {
    let added = entry("cmd+e", FIND, None);
    let removed = entry("cmd+e", &format!("-{CLOSE}"), None);

    assert_eq!(added.command(), &id(FIND));
    assert_eq!(removed.command(), &id(CLOSE));
}

#[test]
fn a_minus_entry_parses_as_an_unbinding() {
    let parsed = entry(
        "cmd+k",
        "-quickie.palette.openCommands",
        Some("pageFocused"),
    );

    assert_eq!(
        parsed,
        KeymapEntry::Unbind(Unbinding {
            chord: Chord::try_from("cmd+k").unwrap(),
            command: id(PALETTE),
            when: Some(WhenExpr::try_from("pageFocused").unwrap()),
        })
    );
}

// ---- validation --------------------------------------------------------------------------------

#[test]
fn a_default_bound_to_an_unknown_command_is_rejected() {
    let error = Keymap::new(
        &Catalog::builtin(),
        MAC,
        vec![bind("cmd+e", "quickie.tab.explode")],
        Vec::new(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        KeymapError::UnknownCommand(id("quickie.tab.explode"))
    );
}

#[test]
fn a_user_binding_to_an_unknown_command_is_rejected() {
    let error = Keymap::new(
        &Catalog::builtin(),
        MAC,
        Vec::new(),
        vec![entry("cmd+e", "quickie.tab.explode", None)],
    )
    .unwrap_err();

    assert_eq!(
        error,
        KeymapError::UnknownCommand(id("quickie.tab.explode"))
    );
}

#[test]
fn removing_an_unknown_command_is_rejected() {
    let error = Keymap::new(
        &Catalog::builtin(),
        MAC,
        Vec::new(),
        vec![entry("cmd+e", "-quickie.tab.explode", None)],
    )
    .unwrap_err();

    assert_eq!(
        error,
        KeymapError::UnknownCommand(id("quickie.tab.explode"))
    );
}

#[test]
fn two_commands_on_one_chord_and_clause_conflict() {
    let error = Keymap::new(
        &Catalog::builtin(),
        MAC,
        vec![bind_when("cmd+e", CLOSE, "a&&b")],
        vec![entry("cmd+e", FIND, Some("a && b"))],
    )
    .unwrap_err();

    assert_eq!(
        error,
        KeymapError::Conflict(Box::new(Conflict {
            chord: Chord::try_from("cmd+e").unwrap(),
            when: Some(WhenExpr::try_from("a && b").unwrap()),
            first: id(CLOSE),
            second: id(FIND),
        }))
    );
}

#[test]
fn two_unconditional_commands_on_one_chord_conflict() {
    let error = Keymap::new(
        &Catalog::builtin(),
        MAC,
        vec![bind("cmd+e", CLOSE), bind("cmd+e", FIND)],
        Vec::new(),
    )
    .unwrap_err();

    assert!(matches!(error, KeymapError::Conflict(_)));
}

#[test]
fn the_same_binding_twice_is_no_conflict() {
    let keymap = Keymap::new(
        &Catalog::builtin(),
        MAC,
        vec![bind("cmd+e", CLOSE)],
        vec![entry("cmd+e", CLOSE, None)],
    );

    assert!(keymap.is_ok());
}

#[test]
fn one_chord_under_different_clauses_is_no_conflict() {
    let keymap = Keymap::new(
        &Catalog::builtin(),
        MAC,
        vec![
            bind("cmd+e", CLOSE),
            bind_when("cmd+e", FIND, "pageFocused"),
        ],
        Vec::new(),
    );

    assert!(keymap.is_ok());
}

#[test]
fn a_conflict_reads_as_keybinding_text_and_ids() {
    let error = KeymapError::Conflict(Box::new(Conflict {
        chord: Chord::try_from("cmd+shift+t").unwrap(),
        when: None,
        first: id("quickie.tab.reopenClosed"),
        second: id(FIND),
    }));

    assert_eq!(
        error.to_string(),
        "`shift+cmd+t` runs both quickie.tab.reopenClosed and quickie.page.find under the same \
         condition"
    );
}

#[test]
fn a_user_ctrl_key_conflicts_with_a_linux_default() {
    let error = Keymap::new(
        &Catalog::builtin(),
        LINUX,
        defaults::bindings(LINUX),
        vec![KeymapEntry::parse("ctrl+t", FIND, None, LINUX).unwrap()],
    )
    .unwrap_err();

    assert_eq!(
        error,
        KeymapError::Conflict(Box::new(Conflict {
            chord: Chord::try_from("ctrl+t").unwrap(),
            when: None,
            first: id("quickie.tab.new"),
            second: id(FIND),
        }))
    );
}

#[test]
fn the_same_ctrl_key_is_free_on_macos() {
    let keymap = Keymap::new(
        &Catalog::builtin(),
        MAC,
        defaults::bindings(MAC),
        vec![entry("ctrl+t", FIND, None)],
    );

    assert!(keymap.is_ok());
}

// ---- parsing entries ---------------------------------------------------------------------------

#[test]
fn a_bad_key_is_reported_as_the_key_field() {
    let error = KeyBinding::parse("cmd+", CLOSE, None, MAC).unwrap_err();

    assert_eq!(
        error,
        BindingError::Chord(ChordError::Stroke(KeyStrokeError::EmptyPart))
    );
    assert_eq!(error.to_string(), "key: a key stroke has an empty part");
}

#[test]
fn a_bad_command_is_reported_as_the_command_field() {
    let error = KeymapEntry::parse("cmd+e", "-tab.close", None, MAC).unwrap_err();

    assert_eq!(
        error,
        BindingError::Command(CommandIdError::ForeignNamespace)
    );
}

#[test]
fn a_bad_clause_is_reported_as_the_when_field() {
    let error = KeymapEntry::parse("cmd+e", CLOSE, Some("a &&"), MAC).unwrap_err();

    assert_eq!(
        error,
        BindingError::When(WhenError::UnexpectedEnd { offset: 4 })
    );
}

// ---- shortcuts never type ----------------------------------------------------------------------

#[test]
fn a_binding_to_a_typing_key_is_refused() {
    for key in ["f", "shift+f", "5", "shift+5", "space", "/", "shift+/", "`"] {
        let error = KeyBinding::parse(key, HINTS, None, MAC).unwrap_err();

        assert_eq!(
            error,
            BindingError::Typing(TypingStroke(stroke(key))),
            "{key}"
        );
    }
}

#[test]
fn a_typing_key_is_refused_as_the_key_field() {
    let error = KeymapEntry::parse("shift+f", HINTS, Some("pageFocused"), MAC).unwrap_err();

    assert_eq!(
        error.to_string(),
        "key: a shortcut needs cmd, alt or ctrl: `shift+f` would type into text fields"
    );
}

#[test]
fn a_typing_key_with_command_option_or_control_is_a_shortcut() {
    for key in [
        "cmd+f",
        "alt+f",
        "ctrl+f",
        "shift+alt+f",
        "mod+5",
        "ctrl+space",
    ] {
        let parsed = KeyBinding::parse(key, HINTS, None, MAC);

        assert!(parsed.is_ok(), "{key}");
    }
}

#[test]
fn a_key_that_types_nothing_needs_no_modifier() {
    for key in [
        "f5",
        "escape",
        "up",
        "shift+tab",
        "enter",
        "pagedown",
        "delete",
    ] {
        let parsed = KeyBinding::parse(key, HINTS, None, MAC);

        assert!(parsed.is_ok(), "{key}");
    }
}

#[test]
fn the_second_stroke_of_a_chord_may_type() {
    let parsed = KeyBinding::parse("cmd+k f", HINTS, None, MAC);

    assert!(parsed.is_ok());
}

#[test]
fn a_chord_that_starts_typing_is_refused() {
    let error = KeyBinding::parse("f cmd+k", HINTS, None, MAC).unwrap_err();

    assert_eq!(error, BindingError::Typing(TypingStroke(stroke("f"))));
}

#[test]
fn an_old_removal_of_a_bare_key_still_reads() {
    let parsed = entry(
        "f",
        &format!("-{HINTS}"),
        Some("pageFocused && !inputFocused"),
    );

    assert_eq!(
        parsed,
        KeymapEntry::Unbind(Unbinding {
            chord: Chord::try_from("f").unwrap(),
            command: id(HINTS),
            when: Some(WhenExpr::try_from("pageFocused && !inputFocused").unwrap()),
        })
    );
}

#[test]
fn an_old_removal_of_a_bare_key_removes_nothing() {
    let keymap = keymap(
        defaults::bindings(MAC),
        vec![entry("f", &format!("-{HINTS}"), None)],
    );

    assert_eq!(glyphs(&keymap, HINTS), ["⌥F"]);
}

#[test]
fn a_default_that_types_is_refused() {
    let typing = KeyBinding {
        chord: Chord::try_from("f").unwrap(),
        command: id(HINTS),
        when: None,
    };

    let error = Keymap::new(&Catalog::builtin(), MAC, vec![typing], Vec::new()).unwrap_err();

    assert_eq!(error, KeymapError::Typing(TypingStroke(stroke("f"))));
}

#[test]
fn a_user_binding_that_types_is_refused() {
    let typing = KeymapEntry::Bind(KeyBinding {
        chord: Chord::try_from("shift+f").unwrap(),
        command: id(HINTS),
        when: None,
    });

    let error = Keymap::defaults(MAC)
        .with_user_entries(&Catalog::builtin(), vec![typing])
        .unwrap_err();

    assert_eq!(error, KeymapError::Typing(TypingStroke(stroke("shift+f"))));
    assert_eq!(
        error.to_string(),
        "a shortcut needs cmd, alt or ctrl: `shift+f` would type into text fields"
    );
}
