use quickie_domain_commands::{
    BindingError, Catalog, Chord, ChordError, CommandId, KeyStroke, KeymapError, Platform,
    TypingStroke,
};

use super::{command, entry, load, runs};
use crate::fakes::FakeKeymapRepo;
use crate::settings::keymap::{RebindError, Rejection, check_binding, rebind, reset_binding};
use crate::{AppError, KeybindingText, StoreError};

fn id(text: &str) -> CommandId {
    CommandId::try_from(text).unwrap()
}

fn chord(text: &str) -> Chord {
    Chord::try_from(text).unwrap()
}

fn repo(entries: Vec<KeybindingText>) -> FakeKeymapRepo {
    FakeKeymapRepo {
        entries,
        ..FakeKeymapRepo::default()
    }
}

fn when(text: KeybindingText, clause: &str) -> KeybindingText {
    KeybindingText {
        when: Some(clause.to_owned()),
        ..text
    }
}

const PAGE: &str = "pageFocused";

fn rebind_on(
    file: &FakeKeymapRepo,
    command: &str,
    keys: &str,
) -> Result<crate::LoadedKeymap, RebindError> {
    rebind(
        file,
        &Catalog::builtin(),
        Platform::MacOs,
        &id(command),
        chord(keys),
    )
}

fn saved(file: &FakeKeymapRepo) -> Option<Vec<KeybindingText>> {
    file.saved.borrow().clone()
}

#[test]
fn a_free_chord_passes_the_check() {
    let keymap = load(Vec::new()).keymap;

    let checked = check_binding(
        &keymap,
        &Catalog::builtin(),
        &id("quickie.tab.reload"),
        chord("cmd+e"),
    );

    assert_eq!(checked, Ok(()));
}

#[test]
fn a_chord_another_command_runs_fails_the_check() {
    let keymap = load(Vec::new()).keymap;

    let checked = check_binding(
        &keymap,
        &Catalog::builtin(),
        &id("quickie.tab.close"),
        chord("cmd+t"),
    );

    let Err(Rejection::Invalid(KeymapError::Conflict(conflict))) = checked else {
        panic!("expected a conflict, got {checked:?}");
    };
    assert_eq!(conflict.first, id("quickie.tab.new"));
    assert_eq!(conflict.second, id("quickie.tab.close"));
}

#[test]
fn the_check_sees_the_users_bindings() {
    let keymap = load(vec![entry("cmd+e", "quickie.page.find")]).keymap;

    let checked = check_binding(
        &keymap,
        &Catalog::builtin(),
        &id("quickie.tab.reload"),
        chord("cmd+e"),
    );

    assert!(checked.is_err());
}

#[test]
fn a_command_may_take_its_own_default_again() {
    let keymap = load(Vec::new()).keymap;

    let checked = check_binding(
        &keymap,
        &Catalog::builtin(),
        &id("quickie.tab.reload"),
        chord("cmd+r"),
    );

    assert_eq!(checked, Ok(()));
}

#[test]
fn rebinding_removes_the_defaults_and_binds_the_chord() {
    let file = repo(Vec::new());

    let loaded = rebind_on(&file, "quickie.tab.reload", "cmd+e").unwrap();

    assert_eq!(
        saved(&file).unwrap(),
        [
            entry("cmd+r", "-quickie.tab.reload"),
            entry("cmd+e", "quickie.tab.reload"),
        ]
    );
    assert_eq!(runs(&loaded.keymap, "cmd+e"), command("quickie.tab.reload"));
    assert_eq!(
        runs(&loaded.keymap, "cmd+r"),
        quickie_domain_commands::Resolution::Unbound
    );
    assert_eq!(loaded.rejected, []);
}

#[test]
fn rebinding_removes_every_default_of_the_command() {
    let file = repo(Vec::new());

    rebind_on(&file, "quickie.palette.openCommands", "cmd+e").unwrap();

    assert_eq!(
        saved(&file).unwrap(),
        [
            entry("shift+cmd+p", "-quickie.palette.openCommands"),
            entry("cmd+k", "-quickie.palette.openCommands"),
            entry("cmd+e", "quickie.palette.openCommands"),
        ]
    );
}

#[test]
fn rebinding_keeps_the_clause_the_command_had() {
    let file = repo(Vec::new());

    let loaded = rebind_on(&file, "quickie.hints.toggle", "alt+g").unwrap();

    assert_eq!(
        saved(&file).unwrap(),
        [
            when(entry("alt+f", "-quickie.hints.toggle"), PAGE),
            when(entry("alt+g", "quickie.hints.toggle"), PAGE),
        ]
    );
    assert_eq!(loaded.rejected, []);
}

#[test]
fn rebinding_takes_the_clause_of_the_first_binding_in_effect() {
    let file = repo(vec![when(
        entry("cmd+y", "quickie.page.zoomReset"),
        "inputFocused",
    )]);
    // The user's binding comes after the defaults, so the default `⌘0` is the first in effect;
    // with the default removed the user's own is.
    let removed = repo(vec![
        entry("cmd+0", "-quickie.page.zoomReset"),
        when(entry("cmd+y", "quickie.page.zoomReset"), "inputFocused"),
    ]);

    rebind_on(&file, "quickie.page.zoomReset", "cmd+u").unwrap();
    rebind_on(&removed, "quickie.page.zoomReset", "cmd+u").unwrap();

    assert_eq!(
        saved(&file).unwrap().last(),
        Some(&entry("cmd+u", "quickie.page.zoomReset"))
    );
    assert_eq!(
        saved(&removed).unwrap().last(),
        Some(&when(
            entry("cmd+u", "quickie.page.zoomReset"),
            "inputFocused"
        ))
    );
}

#[test]
fn a_command_whose_defaults_were_removed_takes_their_clause() {
    let file = repo(vec![when(entry("alt+f", "-quickie.hints.toggle"), PAGE)]);

    rebind_on(&file, "quickie.hints.toggle", "alt+g").unwrap();

    assert_eq!(
        saved(&file).unwrap().last(),
        Some(&when(entry("alt+g", "quickie.hints.toggle"), PAGE))
    );
}

#[test]
fn a_command_without_defaults_is_bound_everywhere() {
    let file = repo(Vec::new());

    rebind_on(&file, "quickie.app.setDefaultBrowser", "cmd+e").unwrap();

    assert_eq!(
        saved(&file).unwrap(),
        [entry("cmd+e", "quickie.app.setDefaultBrowser")]
    );
}

#[test]
fn rebinding_replaces_the_users_entries_for_the_command_and_keeps_the_rest() {
    let bad = entry("", "quickie.tab.new");
    let file = repo(vec![
        entry("cmd+e", "quickie.page.find"),
        entry("cmd+y", "quickie.tab.reload"),
        bad.clone(),
        entry("cmd+r", "-quickie.tab.reload"),
        entry("cmd+i", "quickie.page.zoomIn"),
    ]);

    let loaded = rebind_on(&file, "quickie.tab.reload", "cmd+u").unwrap();

    assert_eq!(
        saved(&file).unwrap(),
        [
            entry("cmd+e", "quickie.page.find"),
            bad.clone(),
            entry("cmd+i", "quickie.page.zoomIn"),
            entry("cmd+r", "-quickie.tab.reload"),
            entry("cmd+u", "quickie.tab.reload"),
        ]
    );
    assert_eq!(loaded.rejected.len(), 1, "the bad entry is still left out");
    assert_eq!(loaded.rejected[0].entry, bad);
    assert_eq!(
        runs(&loaded.keymap, "cmd+y"),
        quickie_domain_commands::Resolution::Unbound
    );
}

#[test]
fn a_refused_chord_is_not_written() {
    let file = repo(vec![entry("cmd+e", "quickie.page.find")]);

    let result = rebind_on(&file, "quickie.tab.close", "cmd+t");

    assert!(matches!(
        result,
        Err(RebindError::Refused(Rejection::Invalid(
            KeymapError::Conflict(_)
        )))
    ));
    assert_eq!(saved(&file), None);
}

#[test]
fn a_recorded_typing_key_is_refused_before_anything_is_written() {
    let keymap = load(Vec::new()).keymap;

    let checked = check_binding(
        &keymap,
        &Catalog::builtin(),
        &id("quickie.hints.toggle"),
        chord("shift+f"),
    );

    assert_eq!(
        checked,
        Err(Rejection::Malformed(BindingError::Typing(TypingStroke(
            KeyStroke::try_from("shift+f").unwrap()
        ))))
    );
}

#[test]
fn a_typing_key_is_not_written() {
    let file = repo(Vec::new());

    let result = rebind_on(&file, "quickie.hints.toggle", "f");

    assert!(matches!(
        result,
        Err(RebindError::Refused(Rejection::Malformed(
            BindingError::Typing(_)
        )))
    ));
    assert_eq!(saved(&file), None);
}

#[test]
fn a_chord_may_end_with_a_typing_key() {
    let file = repo(Vec::new());

    let loaded = rebind_on(&file, "quickie.hints.toggle", "cmd+k f").unwrap();

    assert_eq!(loaded.rejected, []);
}

#[test]
fn a_bad_entry_earlier_in_the_file_does_not_refuse_the_chord() {
    let file = repo(vec![entry("", "quickie.tab.new")]);

    let result = rebind_on(&file, "quickie.tab.reload", "cmd+e");

    assert!(result.is_ok(), "{result:?}");
    assert!(saved(&file).is_some());
}

#[test]
fn a_failed_read_writes_nothing() {
    let file = FakeKeymapRepo {
        load_fails: true,
        ..FakeKeymapRepo::default()
    };

    let result = rebind_on(&file, "quickie.tab.reload", "cmd+e");

    assert_eq!(
        result,
        Err(RebindError::Store(AppError::Store(StoreError(
            "load".to_owned()
        ))))
    );
    assert_eq!(saved(&file), None);
}

#[test]
fn a_failed_write_is_an_error() {
    let file = FakeKeymapRepo {
        save_fails: true,
        ..FakeKeymapRepo::default()
    };

    let result = rebind_on(&file, "quickie.tab.reload", "cmd+e");

    assert_eq!(
        result,
        Err(RebindError::Store(AppError::Store(StoreError(
            "save".to_owned()
        ))))
    );
}

#[test]
fn resetting_drops_the_users_entries_for_the_command() {
    let file = repo(vec![
        entry("cmd+r", "-quickie.tab.reload"),
        entry("cmd+e", "quickie.page.find"),
        entry("cmd+u", "quickie.tab.reload"),
    ]);

    let loaded = reset_binding(
        &file,
        &Catalog::builtin(),
        Platform::MacOs,
        &id("quickie.tab.reload"),
    )
    .unwrap();

    assert_eq!(saved(&file).unwrap(), [entry("cmd+e", "quickie.page.find")]);
    assert_eq!(runs(&loaded.keymap, "cmd+r"), command("quickie.tab.reload"));
    assert_eq!(runs(&loaded.keymap, "cmd+e"), command("quickie.page.find"));
}

#[test]
fn resetting_reads_mod_as_the_platform_primary() {
    let file = repo(vec![entry("mod+e", "quickie.page.find")]);

    let loaded = reset_binding(
        &file,
        &Catalog::builtin(),
        Platform::Linux,
        &id("quickie.tab.reload"),
    )
    .unwrap();

    assert_eq!(runs(&loaded.keymap, "ctrl+e"), command("quickie.page.find"));
}

#[test]
fn a_failed_reset_is_an_error() {
    let unreadable = FakeKeymapRepo {
        load_fails: true,
        ..FakeKeymapRepo::default()
    };
    let unwritable = FakeKeymapRepo {
        save_fails: true,
        ..FakeKeymapRepo::default()
    };
    let reset = |file: &FakeKeymapRepo| {
        reset_binding(
            file,
            &Catalog::builtin(),
            Platform::MacOs,
            &id("quickie.tab.reload"),
        )
    };

    assert_eq!(
        reset(&unreadable),
        Err(AppError::Store(StoreError("load".to_owned())))
    );
    assert_eq!(
        reset(&unwritable),
        Err(AppError::Store(StoreError("save".to_owned())))
    );
    assert_eq!(saved(&unreadable), None);
}

#[test]
fn a_malformed_chord_is_refused_by_its_field() {
    // The editor only records parsed chords, but the reason travels as the keymap gives it.
    let error = Rejection::Malformed(BindingError::Chord(ChordError::Empty));

    assert_eq!(
        RebindError::from(error.clone()).to_string(),
        error.to_string()
    );
}
