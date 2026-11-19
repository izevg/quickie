use quickie_domain_kernel::{CommandId, CommandIdError};

use super::defaults::{self, BINDINGS};
use super::{
    BindingError, Conflict, KeyBinding, Keymap, KeymapEntry, KeymapError, Resolution, Unbinding,
    WhenContext, WhenError, WhenExpr,
};
use crate::command::Catalog;
use crate::key::{Chord, ChordError, KeyStroke, KeyStrokeError};

fn stroke(text: &str) -> KeyStroke {
    KeyStroke::try_from(text).unwrap()
}

fn id(text: &str) -> CommandId {
    CommandId::try_from(text).unwrap()
}

fn bind(key: &str, command: &str) -> KeyBinding {
    KeyBinding::parse(key, command, None).unwrap()
}

fn bind_when(key: &str, command: &str, when: &str) -> KeyBinding {
    KeyBinding::parse(key, command, Some(when)).unwrap()
}

fn entry(key: &str, command: &str, when: Option<&str>) -> KeymapEntry {
    KeymapEntry::parse(key, command, when).unwrap()
}

fn keymap(defaults: Vec<KeyBinding>, entries: Vec<KeymapEntry>) -> Keymap {
    Keymap::new(&Catalog::builtin(), defaults, entries).unwrap()
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

#[test]
fn every_default_row_parses() {
    let bindings = defaults::bindings();

    assert_eq!(bindings.len(), BINDINGS.len());
}

#[test]
fn the_defaults_make_a_valid_keymap() {
    let validated = Keymap::new(&Catalog::builtin(), defaults::bindings(), Vec::new());

    assert_eq!(validated, Ok(Keymap::default()));
}

#[test]
fn the_default_keymap_has_no_user_entries() {
    let keymap = Keymap::default();

    assert_eq!(keymap.user_entries(), []);
    assert_eq!(keymap.bindings().count(), BINDINGS.len());
}

#[test]
fn the_default_keymap_binds_the_readme_keys() {
    let keymap = Keymap::default();
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
        ("f", HINTS),
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
fn link_hints_stay_away_from_text_fields() {
    let keymap = Keymap::default();
    let typing = WhenContext::new()
        .with_flag("pageFocused")
        .with_flag("inputFocused");

    let resolution = resolve(&keymap, "f", &typing);

    assert_eq!(resolution, Resolution::Unbound);
}

// ---- reserved strokes --------------------------------------------------------------------------

#[test]
fn the_browser_keys_are_reserved() {
    for key in [
        "cmd+q",
        "cmd+w",
        "cmd+t",
        "cmd+l",
        "cmd+shift+p",
        "cmd+p",
        "ctrl+tab",
    ] {
        assert!(Keymap::is_reserved(stroke(key)), "{key}");
    }
}

#[test]
fn other_keys_are_not_reserved() {
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
        assert!(!Keymap::is_reserved(stroke(key)), "{key}");
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

    let left: Vec<String> = keymap
        .bindings()
        .map(|binding| binding.chord.to_string())
        .collect();

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
    let keymap = Keymap::default();
    let palette = id(PALETTE);

    let keys: Vec<String> = keymap
        .bindings_for(&palette)
        .map(|binding| binding.chord.to_string())
        .collect();

    assert_eq!(keys, ["⇧⌘P", "⌘K"]);
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
        vec![bind("cmd+e", CLOSE)],
        vec![entry("cmd+e", CLOSE, None)],
    );

    assert!(keymap.is_ok());
}

#[test]
fn one_chord_under_different_clauses_is_no_conflict() {
    let keymap = Keymap::new(
        &Catalog::builtin(),
        vec![
            bind("cmd+e", CLOSE),
            bind_when("cmd+e", FIND, "pageFocused"),
        ],
        Vec::new(),
    );

    assert!(keymap.is_ok());
}

#[test]
fn a_conflict_reads_as_glyphs_and_ids() {
    let error = KeymapError::Conflict(Box::new(Conflict {
        chord: Chord::try_from("cmd+shift+t").unwrap(),
        when: None,
        first: id("quickie.tab.reopenClosed"),
        second: id(FIND),
    }));

    assert_eq!(
        error.to_string(),
        "⇧⌘T runs both quickie.tab.reopenClosed and quickie.page.find under the same condition"
    );
}

// ---- parsing entries ---------------------------------------------------------------------------

#[test]
fn a_bad_key_is_reported_as_the_key_field() {
    let error = KeyBinding::parse("cmd+", CLOSE, None).unwrap_err();

    assert_eq!(
        error,
        BindingError::Chord(ChordError::Stroke(KeyStrokeError::EmptyPart))
    );
    assert_eq!(error.to_string(), "key: a key stroke has an empty part");
}

#[test]
fn a_bad_command_is_reported_as_the_command_field() {
    let error = KeymapEntry::parse("cmd+e", "-tab.close", None).unwrap_err();

    assert_eq!(
        error,
        BindingError::Command(CommandIdError::ForeignNamespace)
    );
}

#[test]
fn a_bad_clause_is_reported_as_the_when_field() {
    let error = KeymapEntry::parse("cmd+e", CLOSE, Some("a &&")).unwrap_err();

    assert_eq!(
        error,
        BindingError::When(WhenError::UnexpectedEnd { offset: 4 })
    );
}
