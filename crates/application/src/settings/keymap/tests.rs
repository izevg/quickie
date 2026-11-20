mod edit;
mod listing;

use quickie_domain_commands::{
    BindingError, Catalog, ChordError, CommandId, KeyStroke, Keymap, KeymapError, Platform,
    Resolution, WhenContext,
};

use super::{LoadedKeymap, RejectedEntry, Rejection, load_keymap};
use crate::fakes::FakeKeymapRepo;
use crate::{AppError, KeybindingText, StoreError};

fn entry(key: &str, command: &str) -> KeybindingText {
    KeybindingText {
        key: key.to_owned(),
        command: command.to_owned(),
        when: None,
    }
}

fn load_on(platform: Platform, entries: Vec<KeybindingText>) -> LoadedKeymap {
    let repo = FakeKeymapRepo {
        entries,
        ..FakeKeymapRepo::default()
    };
    load_keymap(&repo, &Catalog::builtin(), platform).unwrap()
}

fn load(entries: Vec<KeybindingText>) -> LoadedKeymap {
    load_on(Platform::MacOs, entries)
}

fn runs(keymap: &Keymap, key: &str) -> Resolution {
    let stroke = KeyStroke::try_from(key).unwrap();
    keymap.resolve(stroke, None, &WhenContext::new())
}

fn command(id: &str) -> Resolution {
    Resolution::Command(CommandId::try_from(id).unwrap())
}

#[test]
fn without_user_entries_the_defaults_apply() {
    let loaded = load(Vec::new());

    assert_eq!(loaded.keymap, Keymap::defaults(Platform::MacOs));
    assert_eq!(loaded.rejected, []);
}

#[test]
fn on_linux_the_linux_defaults_apply() {
    let loaded = load_on(Platform::Linux, Vec::new());

    assert_eq!(loaded.keymap, Keymap::defaults(Platform::Linux));
}

#[test]
fn a_user_entry_reads_mod_as_the_platform_primary() {
    let linux = load_on(Platform::Linux, vec![entry("mod+e", "quickie.page.find")]);
    let mac = load_on(Platform::MacOs, vec![entry("mod+e", "quickie.page.find")]);

    assert_eq!(runs(&linux.keymap, "ctrl+e"), command("quickie.page.find"));
    assert_eq!(runs(&mac.keymap, "cmd+e"), command("quickie.page.find"));
}

#[test]
fn a_user_entry_on_a_linux_default_key_conflicts() {
    let loaded = load_on(Platform::Linux, vec![entry("ctrl+t", "quickie.tab.close")]);

    assert_eq!(loaded.rejected.len(), 1);
    assert_eq!(runs(&loaded.keymap, "ctrl+t"), command("quickie.tab.new"));
}

#[test]
fn a_user_binding_is_applied() {
    let loaded = load(vec![entry("cmd+e", "quickie.page.find")]);

    assert_eq!(runs(&loaded.keymap, "cmd+e"), command("quickie.page.find"));
}

#[test]
fn a_user_entry_keeps_its_when_clause() {
    let loaded = load(vec![KeybindingText {
        when: Some("inputFocused".to_owned()),
        ..entry("cmd+e", "quickie.page.find")
    }]);

    assert_eq!(runs(&loaded.keymap, "cmd+e"), Resolution::Unbound);
}

#[test]
fn a_user_entry_can_remove_a_default() {
    let loaded = load(vec![entry("cmd+f", "-quickie.page.find")]);

    assert_eq!(runs(&loaded.keymap, "cmd+f"), Resolution::Unbound);
}

#[test]
fn a_malformed_entry_is_reported_and_the_rest_still_apply() {
    let bad = entry("", "quickie.page.find");

    let loaded = load(vec![bad.clone(), entry("cmd+e", "quickie.page.find")]);

    assert_eq!(
        loaded.rejected,
        [RejectedEntry {
            position: 0,
            entry: bad,
            reason: Rejection::Malformed(BindingError::Chord(ChordError::Empty)),
        }]
    );
    assert_eq!(runs(&loaded.keymap, "cmd+e"), command("quickie.page.find"));
}

#[test]
fn an_unknown_command_is_reported_and_left_out() {
    let loaded = load(vec![
        entry("cmd+e", "quickie.page.find"),
        entry("cmd+g", "quickie.page.nothing"),
    ]);

    let unknown = CommandId::try_from("quickie.page.nothing").unwrap();
    assert_eq!(loaded.rejected.len(), 1);
    assert_eq!(loaded.rejected[0].position, 1);
    assert_eq!(
        loaded.rejected[0].reason,
        Rejection::Invalid(KeymapError::UnknownCommand(unknown))
    );
    assert_eq!(loaded.keymap.user_entries().len(), 1);
}

#[test]
fn a_conflicting_entry_is_left_out_and_the_default_keeps_its_key() {
    let loaded = load(vec![entry("cmd+t", "quickie.tab.close")]);

    assert!(matches!(
        loaded.rejected.as_slice(),
        [RejectedEntry {
            reason: Rejection::Invalid(KeymapError::Conflict(_)),
            ..
        }]
    ));
    assert_eq!(runs(&loaded.keymap, "cmd+t"), command("quickie.tab.new"));
}

#[test]
fn a_bare_typing_key_from_an_old_file_is_left_out() {
    let typing = entry("g", "quickie.hints.toggle");

    let loaded = load(vec![typing.clone(), entry("cmd+e", "quickie.page.find")]);

    assert!(matches!(
        loaded.rejected.as_slice(),
        [RejectedEntry {
            position: 0,
            reason: Rejection::Malformed(BindingError::Typing(_)),
            ..
        }]
    ));
    assert_eq!(runs(&loaded.keymap, "g"), Resolution::Unbound);
    assert_eq!(runs(&loaded.keymap, "cmd+e"), command("quickie.page.find"));
}

#[test]
fn an_old_removal_of_bare_f_loads_and_removes_nothing() {
    let old = KeybindingText {
        when: Some("pageFocused && !inputFocused".to_owned()),
        ..entry("f", "-quickie.hints.toggle")
    };

    let loaded = load(vec![old]);

    assert_eq!(loaded.rejected, []);
    let page = WhenContext::new().with_flag("pageFocused");
    let stroke = KeyStroke::try_from("alt+f").unwrap();
    assert_eq!(
        loaded.keymap.resolve(stroke, None, &page),
        command("quickie.hints.toggle")
    );
}

#[test]
fn a_failed_read_is_an_error() {
    let repo = FakeKeymapRepo {
        load_fails: true,
        ..FakeKeymapRepo::default()
    };

    let result = load_keymap(&repo, &Catalog::builtin(), Platform::MacOs);

    assert_eq!(result, Err(AppError::Store(StoreError("load".to_owned()))));
}
