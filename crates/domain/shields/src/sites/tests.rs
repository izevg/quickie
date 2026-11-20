use super::{CorruptSites, Site, SitePolicies, SitePoliciesSnapshot, SitePolicy};

fn site(text: &str) -> Site {
    Site::try_from(text).unwrap()
}

const UNSHIELDED: SitePolicy = SitePolicy {
    shields: false,
    ..SitePolicy::DEFAULT
};

mod policies {
    use super::*;

    #[test]
    fn every_site_starts_with_everything_on() {
        let policies = SitePolicies::new();

        let policy = policies.policy(&site("example.com"));

        assert_eq!(policy, SitePolicy::DEFAULT);
        assert_eq!(SitePolicy::default(), SitePolicy::DEFAULT);
        assert!(policy.shields && policy.https_only && policy.javascript);
    }

    #[test]
    fn a_site_keeps_the_policy_it_was_given() {
        let mut policies = SitePolicies::new();
        let no_script = SitePolicy {
            javascript: false,
            ..SitePolicy::DEFAULT
        };

        let changed = policies.set(site("example.com"), no_script);

        assert!(changed);
        assert_eq!(policies.policy(&site("example.com")), no_script);
        assert_eq!(policies.policy(&site("other.com")), SitePolicy::DEFAULT);
    }

    #[test]
    fn setting_the_same_policy_changes_nothing() {
        let mut policies = SitePolicies::new();
        policies.set(site("example.com"), UNSHIELDED);

        let changed = policies.set(site("example.com"), UNSHIELDED);

        assert!(!changed);
    }

    #[test]
    fn setting_the_default_on_a_default_site_changes_nothing() {
        let mut policies = SitePolicies::new();

        let changed = policies.set(site("example.com"), SitePolicy::DEFAULT);

        assert!(!changed);
        assert_eq!(policies.iter().count(), 0);
    }

    #[test]
    fn a_site_set_back_to_the_default_is_no_exception() {
        let mut policies = SitePolicies::new();
        policies.set(site("example.com"), UNSHIELDED);

        let changed = policies.set(site("example.com"), SitePolicy::DEFAULT);

        assert!(changed);
        assert_eq!(policies, SitePolicies::new());
    }

    #[test]
    fn toggling_turns_shields_off_and_on_again() {
        let mut policies = SitePolicies::new();

        let off = policies.toggle_shields(site("example.com"));
        let unshielded: Vec<Site> = policies.unshielded().cloned().collect();
        let on = policies.toggle_shields(site("example.com"));

        assert_eq!(off, UNSHIELDED);
        assert_eq!(unshielded, [site("example.com")]);
        assert_eq!(on, SitePolicy::DEFAULT);
        assert_eq!(policies.unshielded().count(), 0);
    }

    #[test]
    fn toggling_keeps_the_rest_of_the_policy() {
        let mut policies = SitePolicies::new();
        let http = SitePolicy {
            https_only: false,
            ..SitePolicy::DEFAULT
        };
        policies.set(site("example.com"), http);

        let toggled = policies.toggle_shields(site("example.com"));

        assert_eq!(
            toggled,
            SitePolicy {
                shields: false,
                https_only: false,
                javascript: true
            }
        );
    }

    #[test]
    fn only_sites_with_shields_off_are_unshielded_in_the_order_set() {
        let mut policies = SitePolicies::new();
        policies.set(site("b.com"), UNSHIELDED);
        policies.set(
            site("c.com"),
            SitePolicy {
                javascript: false,
                ..SitePolicy::DEFAULT
            },
        );
        policies.set(site("a.com"), UNSHIELDED);

        let unshielded: Vec<&str> = policies.unshielded().map(Site::as_str).collect();

        assert_eq!(unshielded, ["b.com", "a.com"]);
    }
}

mod snapshots {
    use super::*;

    #[test]
    fn a_snapshot_restores_to_equal_policies() {
        let mut policies = SitePolicies::new();
        policies.set(site("b.com"), UNSHIELDED);
        policies.toggle_shields(site("a.com"));

        let restored = SitePolicies::restore(policies.snapshot());

        assert_eq!(restored, Ok(policies));
    }

    #[test]
    fn a_snapshot_lists_exceptions_in_the_order_set() {
        let mut policies = SitePolicies::new();
        policies.set(site("b.com"), UNSHIELDED);
        policies.set(site("a.com"), UNSHIELDED);

        let snapshot = policies.snapshot();

        assert_eq!(
            snapshot.sites,
            [(site("b.com"), UNSHIELDED), (site("a.com"), UNSHIELDED)]
        );
    }

    #[test]
    fn a_site_listed_twice_is_corrupt() {
        let snapshot = SitePoliciesSnapshot {
            sites: vec![
                (site("a.com"), UNSHIELDED),
                (site("a.com"), SitePolicy::DEFAULT),
            ],
        };

        let restored = SitePolicies::restore(snapshot);

        assert_eq!(restored, Err(CorruptSites::DuplicateSite(site("a.com"))));
        assert_eq!(
            CorruptSites::DuplicateSite(site("a.com")).to_string(),
            "the site a.com appears twice"
        );
    }

    #[test]
    fn a_stored_default_policy_is_left_out() {
        let snapshot = SitePoliciesSnapshot {
            sites: vec![
                (site("a.com"), SitePolicy::DEFAULT),
                (site("b.com"), UNSHIELDED),
            ],
        };

        let restored = SitePolicies::restore(snapshot).unwrap();

        assert_eq!(restored.snapshot().sites, [(site("b.com"), UNSHIELDED)]);
    }
}
