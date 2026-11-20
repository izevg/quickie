use std::sync::Arc;

use quickie_domain_browsing::{Profile, TabId};
use quickie_domain_shields::{
    ContentHash, CorruptLists, CorruptSites, Fetch, FilterList, FilterListId, FilterListSet,
    FilterListSetSnapshot, Refresh, STALE_AFTER_MS, Site, SiteError, SitePolicies,
    SitePoliciesSnapshot, SitePolicy, Timestamp,
};
use url::Url;

use super::fakes::{FakeBlocker, FakeHttp, FakeShieldsRepo};
use super::{CorruptShields, FetchedList, ListFetch, Refreshed, Shields, fetch_lists};
use crate::fakes::FakeClock;
use crate::{
    AppError, BlockerError, BlockerRules, ContentBlockerPort, HttpError, ListRules,
    ShieldsSnapshot, StoreError,
};

type Subject = Shields<FakeShieldsRepo, FakeClock, FakeBlocker>;

const EASYLIST: FilterListId = FilterListId::EasyList;
const PRIVACY: FilterListId = FilterListId::EasyPrivacy;
const BODY: &[u8] = b"||tracker.test^";

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn page() -> Url {
    url("https://www.example.com/article")
}

fn site(text: &str) -> Site {
    Site::try_from(text).unwrap()
}

fn unshielded() -> SitePolicy {
    SitePolicy {
        shields: false,
        ..SitePolicy::DEFAULT
    }
}

fn no_script() -> SitePolicy {
    SitePolicy {
        javascript: false,
        ..SitePolicy::DEFAULT
    }
}

fn fetch_at(millis: u64, body: &[u8]) -> Fetch {
    Fetch {
        at: Timestamp::from_millis(millis),
        hash: ContentHash::of(body),
    }
}

/// A saved list, enabled or not, fetched at 0 with `body` as its content.
fn saved_list(list: FilterListId, enabled: bool, body: &[u8]) -> ShieldsSnapshot {
    ShieldsSnapshot {
        lists: FilterListSetSnapshot {
            lists: vec![FilterList {
                id: list,
                enabled,
                fetched: Some(fetch_at(0, body)),
            }],
            refresh: Refresh::OnRequest,
        },
        bodies: vec![(list, body.to_vec())],
        ..ShieldsSnapshot::default()
    }
}

fn start_with(saved: ShieldsSnapshot) -> Subject {
    let repo = FakeShieldsRepo {
        saved,
        ..FakeShieldsRepo::default()
    };
    Shields::start(repo, FakeClock::default(), FakeBlocker::default()).unwrap()
}

fn started() -> Subject {
    start_with(ShieldsSnapshot::default())
}

fn rules(lists: &[(FilterListId, &[u8])], unshielded: &[&str]) -> BlockerRules {
    BlockerRules {
        lists: lists
            .iter()
            .map(|&(list, body)| ListRules {
                list,
                hash: ContentHash::of(body),
                body: Arc::from(body),
            })
            .collect(),
        unshielded: unshielded.iter().map(|text| site(text)).collect(),
    }
}

fn fetched(list: FilterListId, body: &[u8]) -> FetchedList {
    FetchedList {
        list,
        body: Ok(body.to_vec()),
    }
}

fn failed(list: FilterListId) -> FetchedList {
    FetchedList {
        list,
        body: Err(HttpError("offline".to_owned())),
    }
}

fn store_failure(kind: &str) -> AppError {
    AppError::Store(StoreError(kind.to_owned()))
}

fn refused() -> AppError {
    AppError::Blocker(BlockerError("refused".to_owned()))
}

mod starting {
    use super::*;

    #[test]
    fn a_fresh_profile_starts_with_everything_off_and_installs_nothing() {
        let shields = started();

        assert_eq!(shields.sites(), &SitePolicies::new());
        assert_eq!(shields.lists(), &FilterListSet::new());
        assert_eq!(shields.corrupt_on_start(), []);
        assert_eq!(shields.blocker.installed, []);
        assert_eq!(shields.repo.saves, 0);
    }

    #[test]
    fn a_fresh_profile_has_nothing_to_fetch() {
        let shields = started();

        assert_eq!(shields.due_fetches(), []);
        assert_eq!(shields.requested_fetches(), []);
    }

    #[test]
    fn an_unreadable_store_fails_the_start() {
        let repo = FakeShieldsRepo {
            load_fails: true,
            ..FakeShieldsRepo::default()
        };

        let started = Shields::start(repo, FakeClock::default(), FakeBlocker::default());

        assert_eq!(started.err(), Some(store_failure("load")));
    }

    #[test]
    fn an_enabled_list_with_content_is_installed_at_once() {
        let shields = start_with(saved_list(EASYLIST, true, BODY));

        assert_eq!(shields.blocker.installed, [rules(&[(EASYLIST, BODY)], &[])]);
        assert_eq!(shields.due_fetches(), []);
    }

    #[test]
    fn the_rules_at_start_let_unshielded_sites_through() {
        let mut saved = saved_list(EASYLIST, true, BODY);
        saved.sites = SitePoliciesSnapshot {
            sites: vec![(site("example.com"), unshielded())],
        };

        let shields = start_with(saved);

        assert_eq!(
            shields.blocker.installed,
            [rules(&[(EASYLIST, BODY)], &["example.com"])]
        );
    }

    #[test]
    fn a_disabled_list_is_not_installed() {
        let shields = start_with(saved_list(EASYLIST, false, BODY));

        assert_eq!(shields.blocker.installed, []);
    }

    #[test]
    fn content_that_does_not_match_its_fetch_is_dropped() {
        let mut saved = saved_list(EASYLIST, true, BODY);
        saved.bodies = vec![(EASYLIST, b"tampered".to_vec())];

        let shields = start_with(saved);

        assert_eq!(shields.corrupt_on_start(), [CorruptShields::Body(EASYLIST)]);
        assert_eq!(shields.blocker.installed, []);
    }

    #[test]
    fn content_of_a_list_never_fetched_is_dropped() {
        let saved = ShieldsSnapshot {
            bodies: vec![(PRIVACY, BODY.to_vec())],
            ..ShieldsSnapshot::default()
        };

        let shields = start_with(saved);

        assert_eq!(shields.corrupt_on_start(), [CorruptShields::Body(PRIVACY)]);
    }

    #[test]
    fn corrupt_sites_start_fresh_and_are_reported() {
        let saved = ShieldsSnapshot {
            sites: SitePoliciesSnapshot {
                sites: vec![(site("a.com"), unshielded()), (site("a.com"), unshielded())],
            },
            ..ShieldsSnapshot::default()
        };

        let shields = start_with(saved);

        assert_eq!(shields.sites(), &SitePolicies::new());
        assert_eq!(
            shields.corrupt_on_start(),
            [CorruptShields::Sites(CorruptSites::DuplicateSite(site(
                "a.com"
            )))]
        );
    }

    #[test]
    fn corrupt_lists_start_fresh_and_are_reported() {
        let list = FilterList {
            id: PRIVACY,
            enabled: true,
            fetched: None,
        };
        let saved = ShieldsSnapshot {
            lists: FilterListSetSnapshot {
                lists: vec![list, list],
                refresh: Refresh::KeepFresh,
            },
            ..ShieldsSnapshot::default()
        };

        let shields = start_with(saved);

        assert_eq!(shields.lists(), &FilterListSet::new());
        assert_eq!(
            shields.corrupt_on_start(),
            [CorruptShields::Lists(CorruptLists::DuplicateList(PRIVACY))]
        );
    }

    #[test]
    fn a_blocker_that_refuses_the_saved_rules_fails_the_start() {
        let repo = FakeShieldsRepo {
            saved: saved_list(EASYLIST, true, BODY),
            ..FakeShieldsRepo::default()
        };
        let blocker = FakeBlocker {
            fails: true,
            ..FakeBlocker::default()
        };

        let started = Shields::start(repo, FakeClock::default(), blocker);

        assert_eq!(started.err(), Some(refused()));
    }

    #[test]
    fn corruption_is_explained() {
        let messages = [
            CorruptShields::Body(EASYLIST).to_string(),
            CorruptShields::Sites(CorruptSites::DuplicateSite(site("a.com"))).to_string(),
            CorruptShields::Lists(CorruptLists::DuplicateList(EASYLIST)).to_string(),
        ];

        assert_eq!(
            messages,
            [
                "the saved content of easylist does not match its last fetch",
                "saved site shields are corrupt: the site a.com appears twice",
                "saved filter lists are corrupt: the filter list easylist appears twice",
            ]
        );
    }
}

mod sites {
    use super::*;

    #[test]
    fn a_web_page_has_its_site_s_policy() {
        let mut shields = started();
        shields.toggle_site(&page()).unwrap();

        let policy = shields.policy(&url("https://example.com/other"));

        assert_eq!(policy, Some(unshielded()));
        assert_eq!(
            shields.policy(&url("https://other.test/")),
            Some(SitePolicy::DEFAULT)
        );
    }

    #[test]
    fn a_page_off_the_web_has_no_policy() {
        let shields = started();

        assert_eq!(shields.policy(&url("quickie://start/")), None);
    }

    #[test]
    fn toggling_a_site_saves_it_and_lets_its_pages_through() {
        let mut shields = start_with(saved_list(EASYLIST, true, BODY));

        let policy = shields.toggle_site(&page()).unwrap();

        assert_eq!(policy, unshielded());
        assert_eq!(shields.sites().policy(&site("example.com")), unshielded());
        assert_eq!(
            shields.repo.saved.sites.sites,
            [(site("example.com"), unshielded())]
        );
        assert_eq!(
            shields.blocker.installed.last(),
            Some(&rules(&[(EASYLIST, BODY)], &["example.com"]))
        );
    }

    #[test]
    fn toggling_twice_shields_the_site_again() {
        let mut shields = started();
        shields.toggle_site(&page()).unwrap();

        let policy = shields.toggle_site(&page()).unwrap();

        assert_eq!(policy, SitePolicy::DEFAULT);
        assert_eq!(shields.repo.saved.sites.sites, []);
        assert_eq!(shields.blocker.installed.last(), Some(&rules(&[], &[])));
    }

    #[test]
    fn a_page_off_the_web_cannot_be_toggled() {
        let mut shields = started();

        let toggled = shields.toggle_site(&url("about:blank"));

        assert_eq!(
            toggled,
            Err(AppError::NoSite(SiteError::NotWeb("about".to_owned())))
        );
        assert_eq!(shields.repo.saves, 0);
    }

    #[test]
    fn a_failed_save_leaves_the_site_shielded() {
        let mut shields = started();
        shields.repo.failing = vec!["sites"];

        let toggled = shields.toggle_site(&page());

        assert_eq!(toggled, Err(store_failure("sites")));
        assert_eq!(shields.sites(), &SitePolicies::new());
        assert_eq!(shields.blocker.installed, []);
    }

    #[test]
    fn a_refused_install_keeps_the_toggle() {
        let mut shields = started();
        shields.blocker.fails = true;

        let toggled = shields.toggle_site(&page());

        assert_eq!(toggled, Err(refused()));
        assert_eq!(shields.policy(&page()), Some(unshielded()));
    }

    #[test]
    fn a_policy_without_a_shields_change_compiles_nothing() {
        let mut shields = started();

        let changed = shields.set_policy(&page(), no_script()).unwrap();

        assert!(changed);
        assert_eq!(shields.policy(&page()), Some(no_script()));
        assert_eq!(
            shields.repo.saved.sites.sites,
            [(site("example.com"), no_script())]
        );
        assert_eq!(shields.blocker.installed, []);
    }

    #[test]
    fn a_policy_that_drops_shields_compiles_the_rules() {
        let mut shields = started();

        let changed = shields.set_policy(&page(), unshielded()).unwrap();

        assert!(changed);
        assert_eq!(shields.blocker.installed, [rules(&[], &["example.com"])]);
    }

    #[test]
    fn the_same_policy_again_saves_nothing() {
        let mut shields = started();

        let changed = shields.set_policy(&page(), SitePolicy::DEFAULT).unwrap();

        assert!(!changed);
        assert_eq!(shields.repo.saves, 0);
    }

    #[test]
    fn a_policy_needs_a_web_page() {
        let mut shields = started();

        let set = shields.set_policy(&url("about:blank"), no_script());

        assert!(matches!(set, Err(AppError::NoSite(_))));
    }

    #[test]
    fn a_failed_policy_save_changes_nothing() {
        let mut shields = started();
        shields.repo.failing = vec!["sites"];

        let set = shields.set_policy(&page(), unshielded());

        assert_eq!(set, Err(store_failure("sites")));
        assert_eq!(shields.policy(&page()), Some(SitePolicy::DEFAULT));
        assert_eq!(shields.blocker.installed, []);
    }

    #[test]
    fn a_refused_install_keeps_the_policy() {
        let mut shields = started();
        shields.blocker.fails = true;

        let set = shields.set_policy(&page(), unshielded());

        assert_eq!(set, Err(refused()));
        assert_eq!(shields.policy(&page()), Some(unshielded()));
    }
}

mod lists {
    use super::*;

    #[test]
    fn enabling_a_list_never_fetched_makes_it_due_without_compiling() {
        let mut shields = started();

        let enabled = shields.enable_lists(&[EASYLIST, PRIVACY]).unwrap();

        assert_eq!(enabled, [EASYLIST, PRIVACY]);
        assert_eq!(shields.repo.saves, 1);
        assert_eq!(shields.repo.saved.lists, shields.lists().snapshot());
        assert_eq!(shields.blocker.installed, []);
        assert_eq!(
            shields.due_fetches(),
            [
                ListFetch {
                    list: EASYLIST,
                    url: url("https://easylist.to/easylist/easylist.txt")
                },
                ListFetch {
                    list: PRIVACY,
                    url: url("https://easylist.to/easylist/easyprivacy.txt")
                },
            ]
        );
    }

    #[test]
    fn enabling_a_list_with_content_compiles_it() {
        let mut shields = start_with(saved_list(EASYLIST, false, BODY));

        let enabled = shields.enable_lists(&[EASYLIST]).unwrap();

        assert_eq!(enabled, [EASYLIST]);
        assert_eq!(shields.blocker.installed, [rules(&[(EASYLIST, BODY)], &[])]);
    }

    #[test]
    fn enabling_enabled_lists_saves_nothing() {
        let mut shields = start_with(saved_list(EASYLIST, true, BODY));

        let enabled = shields.enable_lists(&[EASYLIST]).unwrap();

        assert_eq!(enabled, []);
        assert_eq!(shields.repo.saves, 0);
        assert_eq!(shields.blocker.installed.len(), 1);
    }

    #[test]
    fn a_failed_save_leaves_the_lists_off() {
        let mut shields = started();
        shields.repo.failing = vec!["lists"];

        let enabled = shields.enable_lists(&[EASYLIST]);

        assert_eq!(enabled, Err(store_failure("lists")));
        assert_eq!(shields.lists(), &FilterListSet::new());
    }

    #[test]
    fn a_refused_install_keeps_the_list_on() {
        let mut shields = start_with(saved_list(EASYLIST, false, BODY));
        shields.blocker.fails = true;

        let enabled = shields.enable_lists(&[EASYLIST]);

        assert_eq!(enabled, Err(refused()));
        assert!(shields.lists().get(EASYLIST).enabled);
    }

    #[test]
    fn disabling_a_list_with_content_takes_its_rules_away() {
        let mut shields = start_with(saved_list(EASYLIST, true, BODY));

        let disabled = shields.disable_lists(&[EASYLIST]).unwrap();

        assert_eq!(disabled, [EASYLIST]);
        assert_eq!(shields.blocker.installed.last(), Some(&rules(&[], &[])));
        assert!(
            !shields
                .repo
                .saved
                .lists
                .lists
                .iter()
                .any(|list| list.enabled)
        );
    }

    #[test]
    fn disabling_a_list_without_content_compiles_nothing() {
        let mut shields = started();
        shields.enable_lists(&[PRIVACY]).unwrap();

        let disabled = shields.disable_lists(&[PRIVACY]).unwrap();

        assert_eq!(disabled, [PRIVACY]);
        assert_eq!(shields.blocker.installed, []);
    }

    #[test]
    fn keeping_lists_fresh_is_saved() {
        let mut shields = started();

        let changed = shields.set_refresh(Refresh::KeepFresh).unwrap();

        assert!(changed);
        assert_eq!(shields.lists().refresh(), Refresh::KeepFresh);
        assert_eq!(shields.repo.saved.lists.refresh, Refresh::KeepFresh);
    }

    #[test]
    fn the_same_refresh_again_saves_nothing() {
        let mut shields = started();

        let changed = shields.set_refresh(Refresh::OnRequest).unwrap();

        assert!(!changed);
        assert_eq!(shields.repo.saves, 0);
    }

    #[test]
    fn a_failed_refresh_save_changes_nothing() {
        let mut shields = started();
        shields.repo.failing = vec!["lists"];

        let changed = shields.set_refresh(Refresh::KeepFresh);

        assert_eq!(changed, Err(store_failure("lists")));
        assert_eq!(shields.lists().refresh(), Refresh::OnRequest);
    }
}

mod fetching {
    use super::*;

    #[test]
    fn a_requested_refresh_fetches_every_enabled_list() {
        let mut saved = saved_list(EASYLIST, true, BODY);
        saved.lists.lists.push(FilterList {
            id: FilterListId::PeterLowe,
            enabled: false,
            fetched: None,
        });
        let shields = start_with(saved);

        let fetches: Vec<FilterListId> = shields
            .requested_fetches()
            .into_iter()
            .map(|fetch| fetch.list)
            .collect();

        assert_eq!(fetches, [EASYLIST]);
        assert_eq!(shields.due_fetches(), []);
    }

    #[test]
    fn a_stale_list_is_due_only_when_kept_fresh() {
        let mut saved = saved_list(EASYLIST, true, BODY);
        saved.lists.refresh = Refresh::KeepFresh;
        let shields = start_with(saved);
        shields.clock.0.set(STALE_AFTER_MS);

        let due: Vec<FilterListId> = shields.due_fetches().into_iter().map(|f| f.list).collect();

        assert_eq!(due, [EASYLIST]);
    }

    #[test]
    fn fetching_asks_the_network_for_each_list() {
        let requests = ListFetch::of([EASYLIST, PRIVACY]);
        let http = FakeHttp {
            bodies: [(EASYLIST.source().to_owned(), BODY.to_vec())].into(),
        };

        let answers = fetch_lists(&http, &requests);

        assert_eq!(
            answers,
            [
                fetched(EASYLIST, BODY),
                FetchedList {
                    list: PRIVACY,
                    body: Err(HttpError(format!("404 {}", PRIVACY.source()))),
                },
            ]
        );
    }

    #[test]
    fn a_first_fetch_saves_the_content_and_compiles_it() {
        let mut shields = started();
        shields.enable_lists(&[EASYLIST]).unwrap();
        shields.clock.0.set(42);

        let refreshed = shields
            .lists_fetched(vec![fetched(EASYLIST, BODY)])
            .unwrap();

        assert_eq!(
            refreshed,
            Refreshed {
                updated: vec![EASYLIST],
                ..Refreshed::default()
            }
        );
        assert_eq!(shields.repo.saved.bodies, [(EASYLIST, BODY.to_vec())]);
        assert_eq!(
            shields.lists().get(EASYLIST).fetched,
            Some(fetch_at(42, BODY))
        );
        assert_eq!(shields.repo.saved.lists, shields.lists().snapshot());
        assert_eq!(shields.blocker.installed, [rules(&[(EASYLIST, BODY)], &[])]);
        assert_eq!(shields.due_fetches(), []);
    }

    #[test]
    fn unchanged_content_records_the_fetch_only() {
        let mut shields = start_with(saved_list(EASYLIST, true, BODY));
        shields.clock.0.set(42);

        let refreshed = shields
            .lists_fetched(vec![fetched(EASYLIST, BODY)])
            .unwrap();

        assert_eq!(refreshed.unchanged, [EASYLIST]);
        assert_eq!(refreshed.updated, []);
        assert_eq!(shields.repo.saves, 1);
        assert_eq!(
            shields
                .repo
                .saved
                .lists
                .lists
                .first()
                .and_then(|list| list.fetched),
            Some(fetch_at(42, BODY))
        );
        assert_eq!(shields.blocker.installed.len(), 1);
    }

    #[test]
    fn content_lost_at_start_comes_back_with_an_unchanged_fetch() {
        let mut saved = saved_list(EASYLIST, true, BODY);
        saved.bodies = vec![(EASYLIST, b"tampered".to_vec())];
        let mut shields = start_with(saved);

        let refreshed = shields
            .lists_fetched(vec![fetched(EASYLIST, BODY)])
            .unwrap();

        assert_eq!(refreshed.updated, [EASYLIST]);
        assert_eq!(shields.repo.saved.bodies, [(EASYLIST, BODY.to_vec())]);
        assert_eq!(shields.blocker.installed, [rules(&[(EASYLIST, BODY)], &[])]);
    }

    #[test]
    fn content_lost_at_start_comes_back_even_within_the_same_millisecond() {
        let mut saved = saved_list(EASYLIST, true, BODY);
        saved.bodies = Vec::new();
        let mut shields = start_with(saved);

        shields
            .lists_fetched(vec![fetched(EASYLIST, BODY)])
            .unwrap();

        assert_eq!(shields.repo.saved.bodies, [(EASYLIST, BODY.to_vec())]);
        assert_eq!(shields.blocker.installed.len(), 1);
    }

    #[test]
    fn changed_content_replaces_the_old() {
        let mut shields = start_with(saved_list(EASYLIST, true, BODY));

        shields
            .lists_fetched(vec![fetched(EASYLIST, b"||ads.test^")])
            .unwrap();

        assert_eq!(
            shields.blocker.installed.last(),
            Some(&rules(&[(EASYLIST, b"||ads.test^")], &[]))
        );
    }

    #[test]
    fn a_failed_fetch_keeps_the_previous_content_and_saves_nothing() {
        let mut shields = start_with(saved_list(EASYLIST, true, BODY));

        let refreshed = shields.lists_fetched(vec![failed(EASYLIST)]).unwrap();

        assert_eq!(
            refreshed.failed,
            [(EASYLIST, HttpError("offline".to_owned()))]
        );
        assert_eq!(shields.repo.saves, 0);
        assert_eq!(shields.blocker.installed.len(), 1);
    }

    #[test]
    fn new_content_of_a_disabled_list_is_kept_without_compiling() {
        let mut shields = started();

        let refreshed = shields.lists_fetched(vec![fetched(PRIVACY, BODY)]).unwrap();

        assert_eq!(refreshed.updated, [PRIVACY]);
        assert_eq!(shields.repo.saved.bodies, [(PRIVACY, BODY.to_vec())]);
        assert_eq!(shields.blocker.installed, []);
        shields.enable_lists(&[PRIVACY]).unwrap();
        assert_eq!(shields.blocker.installed, [rules(&[(PRIVACY, BODY)], &[])]);
    }

    #[test]
    fn a_failed_content_save_changes_nothing() {
        let mut shields = started();
        shields.enable_lists(&[EASYLIST]).unwrap();
        shields.repo.failing = vec!["body"];

        let refreshed = shields.lists_fetched(vec![fetched(EASYLIST, BODY)]);

        assert_eq!(refreshed, Err(store_failure("body")));
        assert_eq!(shields.lists().get(EASYLIST).fetched, None);
        assert_eq!(shields.blocker.installed, []);
    }

    #[test]
    fn a_failed_list_save_keeps_the_content_out() {
        let mut shields = started();
        shields.enable_lists(&[EASYLIST]).unwrap();
        shields.repo.failing = vec!["lists"];

        let refreshed = shields.lists_fetched(vec![fetched(EASYLIST, BODY)]);

        assert_eq!(refreshed, Err(store_failure("lists")));
        assert_eq!(shields.lists().get(EASYLIST).fetched, None);
        shields.repo.failing = vec![];
        shields.disable_lists(&[EASYLIST]).unwrap();
        shields.enable_lists(&[EASYLIST]).unwrap();
        assert_eq!(shields.blocker.installed, []);
    }

    #[test]
    fn a_refused_install_keeps_the_content() {
        let mut shields = started();
        shields.enable_lists(&[EASYLIST]).unwrap();
        shields.blocker.fails = true;

        let refreshed = shields.lists_fetched(vec![fetched(EASYLIST, BODY)]);

        assert_eq!(refreshed, Err(refused()));
        assert_eq!(
            shields.lists().get(EASYLIST).fetched,
            Some(fetch_at(0, BODY))
        );
    }
}

mod blocked {
    use super::*;

    #[test]
    fn blocked_requests_count_per_page() {
        let mut shields = started();
        shields.record_blocked(TabId::new(Profile::Main, 1), 2);

        let count = shields.record_blocked(TabId::new(Profile::Main, 1), 1);

        assert_eq!(count, 3);
        assert_eq!(shields.blocked_count(TabId::new(Profile::Main, 1)), 3);
        assert_eq!(shields.blocked_count(TabId::new(Profile::Main, 2)), 0);
    }

    #[test]
    fn a_page_left_behind_counts_from_zero() {
        let mut shields = started();
        shields.record_blocked(TabId::new(Profile::Main, 1), 2);

        shields.page_left(TabId::new(Profile::Main, 1));

        assert_eq!(shields.blocked_count(TabId::new(Profile::Main, 1)), 0);
    }
}

#[test]
fn the_blocker_adapter_is_reachable() {
    let mut shields = started();

    shields.blocker_mut().install(&rules(&[], &[])).unwrap();

    assert_eq!(shields.blocker.installed, [rules(&[], &[])]);
}
