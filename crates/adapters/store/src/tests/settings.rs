use quickie_application::{AgentRepo, KeybindingText, KeymapRepo, StoreError, ZoomRepo};
use quickie_domain_agents::EndpointSnapshot;
use quickie_domain_settings::{Site, SiteZoomsSnapshot};
use rusqlite::params;

use crate::SqliteStore;

fn endpoint(key_ref: Option<&str>) -> EndpointSnapshot {
    EndpointSnapshot {
        base_url: "https://api.example.com/v1/".to_owned(),
        model: "small".to_owned(),
        key_ref: key_ref.map(str::to_owned),
    }
}

#[test]
fn a_new_store_has_no_agent_endpoint() {
    let store = SqliteStore::in_memory().unwrap();

    assert_eq!(store.load_endpoint(), Ok(None));
}

#[test]
fn an_agent_endpoint_round_trips_with_its_key_reference() {
    let mut store = SqliteStore::in_memory().unwrap();
    let saved = endpoint(Some("https://api.example.com"));

    store.save_endpoint(Some(saved.clone())).unwrap();

    assert_eq!(store.load_endpoint(), Ok(Some(saved)));
}

#[test]
fn saving_another_agent_endpoint_replaces_the_first() {
    let mut store = SqliteStore::in_memory().unwrap();
    store
        .save_endpoint(Some(endpoint(Some("https://api.example.com"))))
        .unwrap();

    store.save_endpoint(Some(endpoint(None))).unwrap();

    assert_eq!(store.load_endpoint(), Ok(Some(endpoint(None))));
}

#[test]
fn saving_no_agent_endpoint_forgets_it() {
    let mut store = SqliteStore::in_memory().unwrap();
    store.save_endpoint(Some(endpoint(None))).unwrap();

    store.save_endpoint(None).unwrap();

    assert_eq!(store.load_endpoint(), Ok(None));
}

#[test]
fn the_agent_endpoint_table_holds_one_row_at_most() {
    let store = SqliteStore::in_memory().unwrap();

    let second = store.db.execute(
        "INSERT INTO agent_endpoint (id, base_url, model) VALUES (2, 'x', 'y')",
        [],
    );

    assert!(second.is_err());
}

#[test]
fn the_agent_endpoint_table_has_no_column_for_the_key_itself() {
    let store = SqliteStore::in_memory().unwrap();

    let columns: Vec<String> = store
        .db
        .prepare("SELECT name FROM pragma_table_info('agent_endpoint')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(columns, ["id", "base_url", "model", "key_ref"]);
}

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

fn site(text: &str) -> Site {
    Site::try_from(text).unwrap()
}

#[test]
fn a_new_store_has_no_site_zooms() {
    let store = SqliteStore::in_memory().unwrap();

    assert_eq!(store.load_site_zooms(), Ok(SiteZoomsSnapshot::default()));
}

#[test]
fn site_zooms_round_trip_in_order_and_replace_what_was_saved() {
    let mut store = SqliteStore::in_memory().unwrap();
    let first = SiteZoomsSnapshot {
        sites: vec![(site("b.test"), 150), (site("a.test"), 75)],
    };
    let second = SiteZoomsSnapshot {
        sites: vec![(site("a.test"), 300)],
    };

    store.save_site_zooms(first.clone()).unwrap();
    assert_eq!(store.load_site_zooms(), Ok(first));

    store.save_site_zooms(second.clone()).unwrap();
    assert_eq!(store.load_site_zooms(), Ok(second));
}

#[test]
fn a_zoom_row_that_names_no_host_is_a_store_error() {
    let store = SqliteStore::in_memory().unwrap();
    store
        .db
        .execute(
            "INSERT INTO site_zooms (position, site, percent) VALUES (0, 'exa mple', 150)",
            [],
        )
        .unwrap();

    let loaded = store.load_site_zooms();

    assert_eq!(
        loaded,
        Err(StoreError("\"exa mple\" is not a host".to_owned()))
    );
}
