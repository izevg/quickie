use quickie_domain_kernel::Timestamp;
use url::Url;

use super::{
    ContentHash, CorruptLists, Fetch, FilterList, FilterListId, FilterListSet,
    FilterListSetSnapshot, Refresh, STALE_AFTER_MS, UnknownList,
};

const EASYLIST: FilterListId = FilterListId::EasyList;
const PRIVACY: FilterListId = FilterListId::EasyPrivacy;

fn at(millis: u64) -> Timestamp {
    Timestamp::from_millis(millis)
}

fn hash(n: u64) -> ContentHash {
    ContentHash::from_raw(n)
}

/// A set with EasyList enabled and fetched at `fetched`, refreshed as `refresh` says.
fn fetched_at(fetched: u64, refresh: Refresh) -> FilterListSet {
    let mut set = FilterListSet::new();
    set.enable(EASYLIST);
    set.record_fetch(EASYLIST, at(fetched), hash(1));
    set.set_refresh(refresh);
    set
}

mod sources {
    use super::*;

    #[test]
    fn every_source_is_fetched_over_https() {
        for id in FilterListId::ALL {
            let url = Url::parse(id.source()).unwrap();

            assert_eq!(url.scheme(), "https", "{id}");
        }
    }

    #[test]
    fn every_source_has_its_id_title_and_address() {
        let expected = [
            (
                "easylist",
                "EasyList",
                "https://easylist.to/easylist/easylist.txt",
            ),
            (
                "easyprivacy",
                "EasyPrivacy",
                "https://easylist.to/easylist/easyprivacy.txt",
            ),
            (
                "ublock-filters",
                "uBlock filters",
                "https://ublockorigin.github.io/uAssets/filters/filters.txt",
            ),
            (
                "ublock-quick-fixes",
                "uBlock quick fixes",
                "https://ublockorigin.github.io/uAssets/filters/quick-fixes.txt",
            ),
            (
                "peter-lowe",
                "Peter Lowe's list",
                "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=adblockplus&showintro=0&mimetype=plaintext",
            ),
        ];

        let described: Vec<_> = FilterListId::ALL
            .iter()
            .map(|id| (id.as_str(), id.title(), id.source()))
            .collect();

        assert_eq!(described, expected);
    }

    #[test]
    fn only_ublock_origins_own_lists_are_trusted() {
        let trusted: Vec<_> = FilterListId::ALL
            .into_iter()
            .filter(|id| id.trusted())
            .collect();

        assert_eq!(
            trusted,
            [FilterListId::UblockFilters, FilterListId::UblockQuickFixes]
        );
    }

    #[test]
    fn an_id_parses_back_from_its_text() {
        for id in FilterListId::ALL {
            assert_eq!(FilterListId::try_from(id.as_str()), Ok(id));
            assert_eq!(id.to_string(), id.as_str());
        }
    }

    #[test]
    fn a_list_outside_the_allowed_sources_is_unknown() {
        let parsed = FilterListId::try_from("https://evil.test/list.txt");

        let error = UnknownList("https://evil.test/list.txt".to_owned());
        assert_eq!(parsed, Err(error.clone()));
        assert_eq!(
            error.to_string(),
            "\"https://evil.test/list.txt\" is not an allowed filter list"
        );
    }
}

mod hashing {
    use super::*;

    #[test]
    fn the_hash_is_64_bit_fnv_1a() {
        let vectors = [
            ("", 0xcbf2_9ce4_8422_2325),
            ("a", 0xaf63_dc4c_8601_ec8c),
            ("foobar", 0x8594_4171_f739_67e8),
        ];

        for (text, expected) in vectors {
            assert_eq!(ContentHash::of(text.as_bytes()).get(), expected, "{text:?}");
        }
    }

    #[test]
    fn a_hash_shows_as_sixteen_hex_digits() {
        assert_eq!(hash(0xab).to_string(), "00000000000000ab");
    }
}

mod lists {
    use super::*;

    #[test]
    fn a_fresh_profile_has_every_list_off_and_unfetched() {
        let set = FilterListSet::new();

        let ids: Vec<FilterListId> = set.iter().map(|list| list.id).collect();

        assert_eq!(ids, FilterListId::ALL);
        assert!(
            set.iter()
                .all(|list| !list.enabled && list.fetched.is_none())
        );
        assert_eq!(set.refresh(), Refresh::OnRequest);
        assert_eq!(set.enabled().count(), 0);
    }

    #[test]
    fn a_fresh_profile_has_nothing_to_fetch() {
        let set = FilterListSet::new();

        let due = set.due(at(u64::MAX));

        assert_eq!(due, []);
    }

    #[test]
    fn enabling_turns_on_only_that_list() {
        let mut set = FilterListSet::new();

        let changed = set.enable(PRIVACY);

        assert!(changed);
        assert!(set.get(PRIVACY).enabled);
        let enabled: Vec<FilterListId> = set.enabled().map(|list| list.id).collect();
        assert_eq!(enabled, [PRIVACY]);
    }

    #[test]
    fn enabling_an_enabled_list_changes_nothing() {
        let mut set = FilterListSet::new();
        set.enable(EASYLIST);

        let changed = set.enable(EASYLIST);

        assert!(!changed);
        assert!(set.get(EASYLIST).enabled);
    }

    #[test]
    fn disabling_keeps_what_was_fetched() {
        let mut set = fetched_at(5, Refresh::OnRequest);

        let changed = set.disable(EASYLIST);

        assert!(changed);
        assert_eq!(
            *set.get(EASYLIST),
            FilterList {
                id: EASYLIST,
                enabled: false,
                fetched: Some(Fetch {
                    at: at(5),
                    hash: hash(1)
                }),
            }
        );
    }

    #[test]
    fn disabling_a_disabled_list_changes_nothing() {
        let mut set = FilterListSet::new();

        let changed = set.disable(EASYLIST);

        assert!(!changed);
        assert!(!set.get(EASYLIST).enabled);
    }

    #[test]
    fn a_first_fetch_is_new_content() {
        let mut set = FilterListSet::new();

        let new = set.record_fetch(PRIVACY, at(7), hash(9));

        assert!(new);
        assert_eq!(
            set.get(PRIVACY).fetched,
            Some(Fetch {
                at: at(7),
                hash: hash(9)
            })
        );
        assert_eq!(set.get(EASYLIST).fetched, None);
    }

    #[test]
    fn a_fetch_of_the_same_content_is_not_new_but_is_recorded() {
        let mut set = fetched_at(5, Refresh::OnRequest);

        let new = set.record_fetch(EASYLIST, at(9), hash(1));

        assert!(!new);
        assert_eq!(set.get(EASYLIST).fetched.map(|last| last.at), Some(at(9)));
    }

    #[test]
    fn a_fetch_of_changed_content_is_new() {
        let mut set = fetched_at(5, Refresh::OnRequest);

        let new = set.record_fetch(EASYLIST, at(9), hash(2));

        assert!(new);
        assert_eq!(
            set.get(EASYLIST).fetched.map(|last| last.hash),
            Some(hash(2))
        );
    }

    #[test]
    fn refresh_changes_only_when_it_differs() {
        let mut set = FilterListSet::new();

        let turned_on = set.set_refresh(Refresh::KeepFresh);
        let again = set.set_refresh(Refresh::KeepFresh);

        assert!(turned_on);
        assert!(!again);
        assert_eq!(set.refresh(), Refresh::KeepFresh);
    }
}

mod due {
    use super::*;

    #[test]
    fn an_enabled_list_never_fetched_is_due() {
        let mut set = FilterListSet::new();
        set.enable(PRIVACY);

        let due = set.due(at(0));

        assert_eq!(due, [PRIVACY]);
    }

    #[test]
    fn a_disabled_list_is_never_due() {
        let mut set = fetched_at(0, Refresh::KeepFresh);
        set.disable(EASYLIST);

        let due = set.due(at(STALE_AFTER_MS * 10));

        assert_eq!(due, []);
    }

    #[test]
    fn on_request_a_stale_list_waits_for_the_user() {
        let set = fetched_at(0, Refresh::OnRequest);

        let due = set.due(at(STALE_AFTER_MS * 10));

        assert_eq!(due, []);
    }

    #[test]
    fn kept_fresh_a_list_is_due_once_it_is_stale() {
        let set = fetched_at(1_000, Refresh::KeepFresh);

        let due = set.due(at(1_000 + STALE_AFTER_MS));

        assert_eq!(due, [EASYLIST]);
    }

    #[test]
    fn kept_fresh_a_list_just_short_of_stale_is_not_due() {
        let set = fetched_at(1_000, Refresh::KeepFresh);

        let due = set.due(at(1_000 + STALE_AFTER_MS - 1));

        assert_eq!(due, []);
    }

    #[test]
    fn a_list_goes_stale_after_four_days() {
        assert_eq!(STALE_AFTER_MS, 4 * 86_400_000);
    }
}

mod snapshots {
    use super::*;

    #[test]
    fn a_snapshot_restores_to_equal_lists() {
        let mut set = fetched_at(5, Refresh::KeepFresh);
        set.enable(FilterListId::PeterLowe);

        let restored = FilterListSet::restore(set.snapshot());

        assert_eq!(restored, Ok(set));
    }

    #[test]
    fn a_snapshot_lists_every_source() {
        let snapshot = FilterListSet::new().snapshot();

        let ids: Vec<FilterListId> = snapshot.lists.iter().map(|list| list.id).collect();

        assert_eq!(ids, FilterListId::ALL);
        assert_eq!(snapshot.refresh, Refresh::OnRequest);
    }

    #[test]
    fn a_source_missing_from_a_snapshot_is_off_and_unfetched() {
        let snapshot = FilterListSetSnapshot {
            lists: vec![FilterList {
                id: PRIVACY,
                enabled: true,
                fetched: None,
            }],
            refresh: Refresh::KeepFresh,
        };

        let restored = FilterListSet::restore(snapshot).unwrap();

        assert!(restored.get(PRIVACY).enabled);
        assert!(!restored.get(EASYLIST).enabled);
        assert_eq!(restored.refresh(), Refresh::KeepFresh);
    }

    #[test]
    fn a_list_stated_twice_is_corrupt() {
        let list = FilterList {
            id: PRIVACY,
            enabled: true,
            fetched: None,
        };
        let snapshot = FilterListSetSnapshot {
            lists: vec![list, list],
            refresh: Refresh::OnRequest,
        };

        let restored = FilterListSet::restore(snapshot);

        assert_eq!(restored, Err(CorruptLists::DuplicateList(PRIVACY)));
        assert_eq!(
            CorruptLists::DuplicateList(PRIVACY).to_string(),
            "the filter list easyprivacy appears twice"
        );
    }
}
