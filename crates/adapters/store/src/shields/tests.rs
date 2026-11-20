//! The shields tables against real SQLite in memory.

use quickie_application::{ShieldsRepo, ShieldsSnapshot, StoreError};
use quickie_domain_shields::{
    ContentHash, Fetch, FilterList, FilterListId, FilterListSet, FilterListSetSnapshot, Refresh,
    Site, SitePolicies, SitePolicy, Timestamp,
};

use crate::SqliteStore;

fn site(text: &str) -> Site {
    Site::try_from(text).unwrap()
}

fn store() -> SqliteStore {
    SqliteStore::in_memory().unwrap()
}

/// Exceptions with every flag off somewhere, in an order that is not alphabetical.
fn sites() -> SitePolicies {
    let mut sites = SitePolicies::new();
    sites.toggle_shields(site("b.example"));
    sites.set(
        site("a.example"),
        SitePolicy {
            https_only: false,
            ..SitePolicy::DEFAULT
        },
    );
    sites.set(
        site("127.0.0.1"),
        SitePolicy {
            shields: false,
            https_only: false,
            javascript: false,
        },
    );
    sites
}

/// One list enabled and fetched with a hash above `i64::MAX`, one fetched but off, the rest
/// fresh; kept fresh.
fn lists() -> FilterListSet {
    let mut lists = FilterListSet::new();
    lists.enable(FilterListId::EasyPrivacy);
    lists.record_fetch(
        FilterListId::EasyPrivacy,
        Timestamp::from_millis(1_700_000_000_000),
        ContentHash::from_raw(u64::MAX - 1),
    );
    lists.record_fetch(
        FilterListId::PeterLowe,
        Timestamp::from_millis(3),
        ContentHash::from_raw(7),
    );
    lists.set_refresh(Refresh::KeepFresh);
    lists
}

#[test]
fn a_new_database_has_a_fresh_profile_s_shields() {
    let store = store();

    let loaded = store.load_shields();

    assert_eq!(loaded, Ok(ShieldsSnapshot::default()));
}

#[test]
fn site_policies_come_back_in_their_order() {
    let mut store = store();

    store.save_sites(sites().snapshot()).unwrap();

    assert_eq!(store.load_shields().unwrap().sites, sites().snapshot());
}

#[test]
fn saving_sites_replaces_the_previous_ones() {
    let mut store = store();
    store.save_sites(sites().snapshot()).unwrap();

    store.save_sites(SitePolicies::new().snapshot()).unwrap();

    assert_eq!(store.load_shields().unwrap().sites.sites, []);
}

#[test]
fn list_states_and_their_refresh_come_back() {
    let mut store = store();

    store.save_lists(lists().snapshot()).unwrap();

    let loaded = store.load_shields().unwrap().lists;
    assert_eq!(FilterListSet::restore(loaded), Ok(lists()));
}

#[test]
fn refresh_on_request_comes_back_after_keep_fresh() {
    let mut store = store();
    store.save_lists(lists().snapshot()).unwrap();

    store.save_lists(FilterListSet::new().snapshot()).unwrap();

    let loaded = store.load_shields().unwrap().lists;
    assert_eq!(loaded, FilterListSet::new().snapshot());
}

#[test]
fn a_list_stated_twice_is_not_saved_and_the_previous_states_stay() {
    let mut store = store();
    store.save_lists(lists().snapshot()).unwrap();
    let list = FilterList {
        id: FilterListId::EasyList,
        enabled: true,
        fetched: None,
    };
    let twice = FilterListSetSnapshot {
        lists: vec![list, list],
        refresh: Refresh::OnRequest,
    };

    let saved = store.save_lists(twice);

    assert!(saved.is_err());
    let loaded = store.load_shields().unwrap().lists;
    assert_eq!(FilterListSet::restore(loaded), Ok(lists()));
}

#[test]
fn list_content_is_kept_per_list_and_replaced() {
    let mut store = store();
    store
        .save_list_body(FilterListId::PeterLowe, b"old")
        .unwrap();
    store
        .save_list_body(FilterListId::EasyList, &[0, 159, 255])
        .unwrap();

    store
        .save_list_body(FilterListId::PeterLowe, b"new")
        .unwrap();

    assert_eq!(
        store.load_shields().unwrap().bodies,
        [
            (FilterListId::EasyList, vec![0, 159, 255]),
            (FilterListId::PeterLowe, b"new".to_vec()),
        ]
    );
}

#[test]
fn a_fetch_needs_both_its_time_and_its_hash() {
    let store = store();

    let inserted = store.db.execute(
        "INSERT INTO filter_lists (id, enabled, fetched_at, content_hash)
         VALUES ('easylist', 1, 5, NULL)",
        [],
    );

    assert!(inserted.is_err());
}

#[test]
fn a_fetched_list_loads_its_fetch() {
    let mut store = store();

    store.save_lists(lists().snapshot()).unwrap();

    let loaded = store.load_shields().unwrap().lists;
    let privacy = loaded
        .lists
        .iter()
        .find(|list| list.id == FilterListId::EasyPrivacy)
        .unwrap();
    assert_eq!(
        privacy.fetched,
        Some(Fetch {
            at: Timestamp::from_millis(1_700_000_000_000),
            hash: ContentHash::from_raw(u64::MAX - 1),
        })
    );
}

#[test]
fn an_unknown_list_in_the_file_fails_the_load() {
    let store = store();
    store
        .db
        .execute(
            "INSERT INTO filter_list_bodies (id, body) VALUES ('evil', x'00')",
            [],
        )
        .unwrap();

    let loaded = store.load_shields();

    assert_eq!(
        loaded,
        Err(StoreError(
            "\"evil\" is not an allowed filter list".to_owned()
        ))
    );
}

#[test]
fn an_unknown_list_state_fails_the_load() {
    let store = store();
    store
        .db
        .execute(
            "INSERT INTO filter_lists (id, enabled) VALUES ('evil', 1)",
            [],
        )
        .unwrap();

    let loaded = store.load_shields();

    assert!(loaded.is_err());
}

#[test]
fn a_site_that_is_no_host_fails_the_load() {
    let store = store();
    store
        .db
        .execute(
            "INSERT INTO site_policies (position, site, shields, https_only, javascript)
             VALUES (0, 'not a host', 0, 1, 1)",
            [],
        )
        .unwrap();

    let loaded = store.load_shields();

    assert_eq!(
        loaded,
        Err(StoreError("\"not a host\" is not a host".to_owned()))
    );
}
