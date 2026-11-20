use quickie_domain_commands::{Catalog, CommandId, Platform};

use super::{entry, load};
use crate::KeybindingText;
use crate::keymap::{BindingLine, BindingSource, LoadedKeymap, keymap_lines};

fn id(text: &str) -> CommandId {
    CommandId::try_from(text).unwrap()
}

fn lines_of(loaded: &LoadedKeymap) -> Vec<BindingLine> {
    keymap_lines(&loaded.keymap, &Catalog::builtin(), &loaded.rejected)
}

fn lines_for(lines: &[BindingLine], command: &str) -> Vec<BindingLine> {
    lines
        .iter()
        .filter(|line| line.command.as_str() == command)
        .cloned()
        .collect()
}

fn keys(lines: &[BindingLine]) -> Vec<Option<&str>> {
    lines.iter().map(|line| line.keys.as_deref()).collect()
}

fn line(title: &'static str, command: &str, keys: Option<&str>, when: Option<&str>) -> BindingLine {
    BindingLine {
        command: id(command),
        title,
        keys: keys.map(str::to_owned),
        when: when.map(str::to_owned),
        source: BindingSource::Default,
        conflict: None,
    }
}

#[test]
fn a_default_binding_is_listed_with_its_glyphs_and_source() {
    let lines = lines_of(&load(Vec::new()));

    assert_eq!(
        lines_for(&lines, "quickie.tab.reload"),
        [line("Reload Tab", "quickie.tab.reload", Some("⌘R"), None)]
    );
}

#[test]
fn every_command_is_listed_in_palette_order() {
    let lines = lines_of(&load(Vec::new()));
    let mut commands: Vec<&CommandId> = lines.iter().map(|line| &line.command).collect();
    commands.dedup();

    let catalog = Catalog::builtin();
    let expected: Vec<&CommandId> = catalog.iter().map(|command| &command.id).collect();
    assert_eq!(commands, expected);
}

#[test]
fn a_command_with_two_bindings_has_two_lines_in_order() {
    let lines = lines_of(&load(Vec::new()));

    assert_eq!(
        keys(&lines_for(&lines, "quickie.palette.openCommands")),
        [Some("⇧⌘P"), Some("⌘K")]
    );
}

#[test]
fn a_clause_is_listed_as_written() {
    let lines = lines_of(&load(Vec::new()));

    let hints = lines_for(&lines, "quickie.hints.toggle");
    assert_eq!(
        hints[0].when.as_deref(),
        Some("pageFocused && !inputFocused")
    );
    assert_eq!(hints[0].keys.as_deref(), Some("F"));
}

#[test]
fn a_command_without_keys_has_one_line_without_keys() {
    let lines = lines_of(&load(Vec::new()));

    assert_eq!(
        lines_for(&lines, "quickie.app.setDefaultBrowser"),
        [line(
            "Set as Default Browser",
            "quickie.app.setDefaultBrowser",
            None,
            None
        )]
    );
}

#[test]
fn a_user_binding_is_marked_as_the_users() {
    let lines = lines_of(&load(vec![entry("cmd+e", "quickie.tab.reload")]));

    let reload = lines_for(&lines, "quickie.tab.reload");
    assert_eq!(keys(&reload), [Some("⌘R"), Some("⌘E")]);
    assert_eq!(
        reload.iter().map(|line| line.source).collect::<Vec<_>>(),
        [BindingSource::Default, BindingSource::User]
    );
}

#[test]
fn a_removed_default_leaves_a_user_line_without_keys() {
    let lines = lines_of(&load(vec![entry("cmd+r", "-quickie.tab.reload")]));

    let reload = lines_for(&lines, "quickie.tab.reload");
    assert_eq!(keys(&reload), [None]);
    assert_eq!(reload[0].source, BindingSource::User);
}

#[test]
fn a_removal_of_another_command_leaves_this_one_default() {
    let lines = lines_of(&load(vec![entry("cmd+r", "-quickie.tab.reload")]));

    let browser = lines_for(&lines, "quickie.app.setDefaultBrowser");
    assert_eq!(browser[0].source, BindingSource::Default);
}

#[test]
fn a_conflicting_entry_follows_its_command_and_names_the_other() {
    let lines = lines_of(&load(vec![entry("cmd+t", "quickie.tab.close")]));

    let close = lines_for(&lines, "quickie.tab.close");
    assert_eq!(keys(&close), [Some("⌘W"), Some("⌘T")]);
    assert_eq!(close[0].conflict, None);
    assert_eq!(close[1].source, BindingSource::User);
    assert_eq!(close[1].conflict.as_deref(), Some("Open New Tab"));
    assert_eq!(close[1].when, None);
    let new_tab = lines_for(&lines, "quickie.tab.new");
    assert_eq!(keys(&new_tab), [Some("⌘T")], "listed under its own command");
}

#[test]
fn a_conflict_keeps_its_clause() {
    let lines = lines_of(&load(vec![KeybindingText {
        when: Some("pageFocused && !inputFocused".to_owned()),
        ..entry("f", "quickie.page.find")
    }]));

    let find = lines_for(&lines, "quickie.page.find");
    assert_eq!(find[1].conflict.as_deref(), Some("Toggle Link Hints"));
    assert_eq!(
        find[1].when.as_deref(),
        Some("pageFocused && !inputFocused")
    );
}

#[test]
fn a_malformed_entry_is_not_listed() {
    let plain = lines_of(&load(Vec::new()));
    let with_bad = lines_of(&load(vec![entry("", "quickie.tab.close")]));

    assert_eq!(plain, with_bad);
}

#[test]
fn linux_lists_its_own_glyphs() {
    let loaded = super::load_on(Platform::Linux, Vec::new());
    let lines = lines_of(&loaded);

    assert_eq!(
        keys(&lines_for(&lines, "quickie.tab.reload")),
        [Some("Ctrl+R")]
    );
}

#[test]
fn an_empty_query_keeps_every_line() {
    let reload = line("Reload Tab", "quickie.tab.reload", Some("⌘R"), None);

    assert!(reload.matches(""));
    assert!(reload.matches("   "));
}

#[test]
fn a_query_finds_the_title_the_id_the_keys_and_the_clause_ignoring_case() {
    let hints = line(
        "Toggle Link Hints",
        "quickie.hints.toggle",
        Some("F"),
        Some("pageFocused && !inputFocused"),
    );

    assert!(hints.matches("LINK"), "title");
    assert!(hints.matches("hints.tog"), "id");
    assert!(hints.matches("f"), "keys");
    assert!(hints.matches("pagefocused"), "clause");
    assert!(!hints.matches("reload"));
}

#[test]
fn every_word_of_a_query_must_appear() {
    let reload = line("Reload Tab", "quickie.tab.reload", Some("⌘R"), None);

    assert!(reload.matches("reload ⌘r"));
    assert!(reload.matches("tab RELOAD"));
    assert!(!reload.matches("reload hard"));
}

#[test]
fn a_line_without_keys_or_clause_matches_on_its_title() {
    let browser = line(
        "Set as Default Browser",
        "quickie.app.setDefaultBrowser",
        None,
        None,
    );

    assert!(browser.matches("default"));
    assert!(!browser.matches("⌘"));
}
