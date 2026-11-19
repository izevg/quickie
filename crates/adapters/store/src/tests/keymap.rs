use quickie_application::{KeybindingText, KeymapRepo};
use rusqlite::params;

use crate::SqliteStore;

fn entry(key: &str, command: &str, when: Option<&str>) -> KeybindingText {
    KeybindingText {
        key: key.to_owned(),
        command: command.to_owned(),
        when: when.map(str::to_owned),
    }
}

fn insert(store: &SqliteStore, position: i64, entry: &KeybindingText) {
    store
        .db
        .execute(
            "INSERT INTO keybindings (position, key, command, when_clause)
             VALUES (?1, ?2, ?3, ?4)",
            params![position, entry.key, entry.command, entry.when],
        )
        .unwrap();
}

#[test]
fn a_new_store_has_no_user_bindings() {
    let store = SqliteStore::in_memory().unwrap();

    assert_eq!(store.load_keybindings(), Ok(Vec::new()));
}

#[test]
fn entries_load_in_file_order_as_written() {
    let store = SqliteStore::in_memory().unwrap();
    let first = entry("cmd+k cmd+s", "quickie.palette.openCommands", None);
    let second = entry("f", "-quickie.hints.toggle", Some("pageFocused"));
    let broken = entry("cmd+", "not a command", Some("&&"));
    insert(&store, 2, &broken);
    insert(&store, 0, &first);
    insert(&store, 1, &second);

    let loaded = store.load_keybindings();

    assert_eq!(loaded, Ok(vec![first, second, broken]));
}
