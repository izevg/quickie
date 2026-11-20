use quickie_domain_kernel::{Site, Timestamp};
use url::Url;

use super::*;

const BITWARDEN: &str = "nngceckbapebfimnlniiiahkandclblb";
const UBLOCK: &str = "ddkjiahejlhfcafbddmgiahcphecmpfh";

fn id(text: &str) -> ExtensionId {
    ExtensionId::try_from(text).unwrap()
}

fn version(text: &str) -> ExtensionVersion {
    ExtensionVersion::try_from(text).unwrap()
}

fn site(text: &str) -> Site {
    Site::try_from(text).unwrap()
}

fn package(text: &str, at: &str) -> Package {
    Package {
        id: id(text),
        name: "Bitwarden".to_owned(),
        version: version(at),
        source: ExtensionSource::ChromeWebStore,
        location: format!("/extensions/{text}/{at}.zip"),
    }
}

fn grants(permissions: &[&str], patterns: &[&str]) -> Grants {
    Grants {
        permissions: permissions.iter().map(|&name| name.to_owned()).collect(),
        patterns: patterns.iter().map(|&name| name.to_owned()).collect(),
    }
}

fn with_bitwarden() -> InstalledExtensions {
    let mut list = InstalledExtensions::new();
    list.install(package(BITWARDEN, "2025.1.0"), grants(&["storage"], &[]))
        .unwrap();
    list
}

mod ids {
    use super::*;

    #[test]
    fn an_id_is_32_letters_a_to_p() {
        assert_eq!(id(BITWARDEN).as_str(), BITWARDEN);
        assert_eq!(id(BITWARDEN).to_string(), BITWARDEN);
        assert!(ExtensionId::try_from("a".repeat(31).as_str()).is_err());
        assert!(ExtensionId::try_from("a".repeat(33).as_str()).is_err());
        assert!(ExtensionId::try_from("q".repeat(32).as_str()).is_err());
        assert!(ExtensionId::try_from("A".repeat(32).as_str()).is_err());
        assert_eq!(
            ExtensionId::try_from("p".repeat(32).as_str())
                .unwrap()
                .as_str(),
            "p".repeat(32)
        );
    }

    #[test]
    fn a_digest_gives_two_letters_per_byte_high_half_first() {
        let mut digest = [0_u8; 16];
        digest[0] = 0x0f;
        digest[1] = 0xf0;
        digest[15] = 0x5a;

        let made = ExtensionId::from_digest(&digest);

        assert_eq!(made.as_str(), format!("appa{}fk", "a".repeat(26)));
    }

    fn store(text: &str) -> Option<ExtensionId> {
        ExtensionId::from_store_url(&Url::parse(text).unwrap())
    }

    #[test]
    fn the_store_names_the_id_last_in_its_detail_pages() {
        let expected = Some(id(BITWARDEN));
        assert_eq!(
            store(&format!(
                "https://chromewebstore.google.com/detail/bitwarden/{BITWARDEN}"
            )),
            expected
        );
        assert_eq!(
            store(&format!(
                "https://chromewebstore.google.com/detail/{BITWARDEN}?hl=en"
            )),
            expected
        );
        assert_eq!(
            store(&format!(
                "https://chrome.google.com/webstore/detail/bitwarden/{BITWARDEN}/"
            )),
            expected
        );
    }

    #[test]
    fn other_pages_name_no_extension() {
        for text in [
            format!("http://chromewebstore.google.com/detail/x/{BITWARDEN}"),
            format!("https://example.com/detail/x/{BITWARDEN}"),
            format!("https://chromewebstore.google.com/category/{BITWARDEN}"),
            format!("https://chrome.google.com/detail/x/{BITWARDEN}"),
            format!("https://chrome.google.com/extensions/detail/{BITWARDEN}"),
            "https://chromewebstore.google.com/detail/bitwarden/not-an-id".to_owned(),
            "https://chromewebstore.google.com/detail".to_owned(),
        ] {
            assert_eq!(store(&text), None, "{text}");
        }
    }
}

mod versions {
    use super::*;

    #[test]
    fn a_version_is_one_to_four_numbers() {
        for text in ["0", "1", "1.2", "1.2.3", "1.2.3.4", "65535.0.10.100"] {
            assert_eq!(version(text).as_str(), text);
            assert_eq!(version(text).to_string(), text);
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for text in [
            "",
            "1.",
            ".1",
            "1..2",
            "01",
            "1.02",
            "1.2.3.4.5",
            "65536",
            "a",
            "+1",
            "-1",
            "1 2",
        ] {
            assert_eq!(
                ExtensionVersion::try_from(text),
                Err(VersionError(text.to_owned())),
                "{text:?}"
            );
        }
    }

    #[test]
    fn versions_compare_number_by_number() {
        assert!(version("1.10") > version("1.9"));
        assert!(version("2") > version("1.65535"));
        assert!(version("1.2.3.5") > version("1.2.3.4"));
        assert_eq!(version("1.2"), version("1.2.0.0"));
        assert_eq!(
            version("1.2").cmp(&version("1.2.0")),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            version("1.2").partial_cmp(&version("1.3")),
            Some(std::cmp::Ordering::Less)
        );
        assert_ne!(version("1.2"), version("1.3"));
    }
}

mod sources_and_access {
    use super::*;

    #[test]
    fn sources_round_trip_through_their_stored_names() {
        for source in [
            ExtensionSource::ChromeWebStore,
            ExtensionSource::File,
            ExtensionSource::Folder,
        ] {
            assert_eq!(ExtensionSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(ExtensionSource::ChromeWebStore.as_str(), "chrome_web_store");
        assert_eq!(ExtensionSource::File.as_str(), "file");
        assert_eq!(ExtensionSource::Folder.as_str(), "folder");
        assert_eq!(ExtensionSource::parse("store"), None);
    }

    #[test]
    fn sources_have_labels() {
        assert_eq!(ExtensionSource::ChromeWebStore.label(), "Chrome Web Store");
        assert_eq!(ExtensionSource::File.label(), "Package file");
        assert_eq!(ExtensionSource::Folder.label(), "Folder");
    }

    #[test]
    fn access_round_trips_through_its_stored_names() {
        assert_eq!(SiteAccess::Allowed.as_str(), "allowed");
        assert_eq!(SiteAccess::Blocked.as_str(), "blocked");
        for access in [SiteAccess::Allowed, SiteAccess::Blocked] {
            assert_eq!(SiteAccess::parse(access.as_str()), Some(access));
        }
        assert_eq!(SiteAccess::parse("denied"), None);
    }

    #[test]
    fn a_site_is_covered_with_its_subdomains() {
        assert_eq!(
            site_patterns(&site("example.com")),
            [
                "*://example.com/*".to_owned(),
                "*://*.example.com/*".to_owned()
            ]
        );
    }
}

mod grants {
    use super::*;

    #[test]
    fn beyond_is_what_is_not_granted_yet() {
        let asked = grants(&["storage", "tabs"], &["<all_urls>", "*://a.com/*"]);
        let granted = grants(&["storage"], &["*://a.com/*", "*://b.com/*"]);

        assert_eq!(asked.beyond(&granted), grants(&["tabs"], &["<all_urls>"]));
        assert!(asked.beyond(&asked).is_empty());
    }

    #[test]
    fn adding_says_whether_anything_was_new() {
        let mut held = grants(&["storage"], &["*://a.com/*"]);

        assert!(!held.add(&grants(&["storage"], &["*://a.com/*"])));
        assert!(held.add(&grants(&["tabs"], &[])));
        assert!(held.add(&grants(&[], &["*://b.com/*"])));
        assert_eq!(
            held,
            grants(&["storage", "tabs"], &["*://a.com/*", "*://b.com/*"])
        );
    }

    #[test]
    fn removing_says_whether_anything_was_there() {
        let mut held = grants(&["storage", "tabs"], &["*://a.com/*"]);

        assert!(!held.remove(&grants(&["cookies"], &["*://b.com/*"])));
        assert!(held.remove(&grants(&["tabs"], &[])));
        assert!(held.remove(&grants(&[], &["*://a.com/*"])));
        assert_eq!(held, grants(&["storage"], &[]));
    }

    #[test]
    fn empty_means_neither_permissions_nor_patterns() {
        assert!(Grants::new().is_empty());
        assert!(!grants(&["storage"], &[]).is_empty());
        assert!(!grants(&[], &["<all_urls>"]).is_empty());
    }

    #[test]
    fn a_name_must_be_one_word() {
        assert_eq!(grants(&["storage"], &["<all_urls>"]).malformed(), None);
        assert_eq!(grants(&[""], &[]).malformed(), Some(""));
        assert_eq!(grants(&["st orage"], &[]).malformed(), Some("st orage"));
        assert_eq!(
            grants(&[], &["*://a.com/*\n"]).malformed(),
            Some("*://a.com/*\n")
        );
    }
}

mod installing {
    use super::*;

    #[test]
    fn a_new_extension_runs_with_what_was_granted() {
        let mut list = InstalledExtensions::new();

        let installed = list.install(package(BITWARDEN, "1.0"), grants(&["storage"], &[]));

        assert_eq!(installed, Ok(Installed::New));
        let extension = list.get(&id(BITWARDEN)).unwrap();
        assert!(extension.enabled());
        assert_eq!(extension.id(), &id(BITWARDEN));
        assert_eq!(extension.name(), "Bitwarden");
        assert_eq!(extension.version(), &version("1.0"));
        assert_eq!(extension.source(), ExtensionSource::ChromeWebStore);
        assert_eq!(
            extension.location(),
            format!("/extensions/{BITWARDEN}/1.0.zip")
        );
        assert_eq!(extension.granted(), &grants(&["storage"], &[]));
        assert_eq!(extension.sites().count(), 0);
    }

    #[test]
    fn a_new_version_keeps_the_user_s_choices_and_takes_its_grants() {
        let mut list = with_bitwarden();
        list.set_enabled(&id(BITWARDEN), false).unwrap();
        list.set_access(&id(BITWARDEN), site("a.com"), Some(SiteAccess::Blocked))
            .unwrap();

        let installed = list.install(package(BITWARDEN, "2025.2.0"), grants(&["tabs"], &[]));

        assert_eq!(
            installed,
            Ok(Installed::Replaced {
                previous: version("2025.1.0")
            })
        );
        let extension = list.get(&id(BITWARDEN)).unwrap();
        assert!(!extension.enabled());
        assert_eq!(extension.version(), &version("2025.2.0"));
        assert_eq!(extension.granted(), &grants(&["tabs"], &[]));
        assert_eq!(extension.access(&site("a.com")), Some(SiteAccess::Blocked));
        assert_eq!(list.iter().count(), 1);
    }

    #[test]
    fn a_package_needs_a_name_a_location_and_well_formed_grants() {
        let mut list = InstalledExtensions::new();
        let mut nameless = package(BITWARDEN, "1");
        nameless.name = "  ".to_owned();
        let mut nowhere = package(BITWARDEN, "1");
        nowhere.location = String::new();

        assert_eq!(
            list.install(nameless, Grants::new()),
            Err(ExtensionError::NoName(id(BITWARDEN)))
        );
        assert_eq!(
            list.install(nowhere, Grants::new()),
            Err(ExtensionError::NoLocation(id(BITWARDEN)))
        );
        assert_eq!(
            list.install(package(BITWARDEN, "1"), grants(&["a b"], &[])),
            Err(ExtensionError::BadGrant(id(BITWARDEN), "a b".to_owned()))
        );
        assert_eq!(list, InstalledExtensions::new());
    }

    #[test]
    fn extensions_keep_their_install_order() {
        let mut list = with_bitwarden();
        let mut ublock = package(UBLOCK, "1.0");
        ublock.source = ExtensionSource::File;
        list.install(ublock, Grants::new()).unwrap();

        let ids: Vec<&str> = list.iter().map(|e| e.id().as_str()).collect();
        assert_eq!(ids, [BITWARDEN, UBLOCK]);
        let from_store: Vec<&str> = list.from_store().map(|e| e.id().as_str()).collect();
        assert_eq!(from_store, [BITWARDEN]);
    }

    #[test]
    fn removing_gives_the_extension_back_once() {
        let mut list = with_bitwarden();

        let removed = list.remove(&id(BITWARDEN));

        assert_eq!(
            removed.map(|e| e.name().to_owned()),
            Some("Bitwarden".to_owned())
        );
        assert!(list.remove(&id(BITWARDEN)).is_none());
        assert!(list.get(&id(BITWARDEN)).is_none());
    }

    #[test]
    fn only_a_later_version_is_newer() {
        let list = with_bitwarden();

        assert!(list.is_newer(&id(BITWARDEN), &version("2025.1.1")));
        assert!(!list.is_newer(&id(BITWARDEN), &version("2025.1.0")));
        assert!(!list.is_newer(&id(BITWARDEN), &version("2024.12")));
        assert!(list.is_newer(&id(UBLOCK), &version("0.1")));
    }
}

mod choices {
    use super::*;

    #[test]
    fn enabling_says_whether_it_changed() {
        let mut list = with_bitwarden();

        assert_eq!(list.set_enabled(&id(BITWARDEN), true), Ok(false));
        assert_eq!(list.set_enabled(&id(BITWARDEN), false), Ok(true));
        assert!(!list.get(&id(BITWARDEN)).unwrap().enabled());
        assert_eq!(list.set_enabled(&id(BITWARDEN), false), Ok(false));
        assert_eq!(
            list.set_enabled(&id(UBLOCK), true),
            Err(ExtensionError::NotInstalled(id(UBLOCK)))
        );
    }

    #[test]
    fn granting_and_revoking_say_whether_they_changed_anything() {
        let mut list = with_bitwarden();
        let tabs = grants(&["tabs"], &[]);

        assert_eq!(list.grant(&id(BITWARDEN), &tabs), Ok(true));
        assert_eq!(list.grant(&id(BITWARDEN), &tabs), Ok(false));
        assert_eq!(
            list.get(&id(BITWARDEN)).unwrap().granted(),
            &grants(&["storage", "tabs"], &[])
        );
        assert_eq!(list.revoke(&id(BITWARDEN), &tabs), Ok(true));
        assert_eq!(list.revoke(&id(BITWARDEN), &tabs), Ok(false));
        assert_eq!(
            list.get(&id(BITWARDEN)).unwrap().granted(),
            &grants(&["storage"], &[])
        );
    }

    #[test]
    fn grants_need_an_installed_extension_and_good_names() {
        let mut list = with_bitwarden();

        assert_eq!(
            list.grant(&id(UBLOCK), &grants(&["tabs"], &[])),
            Err(ExtensionError::NotInstalled(id(UBLOCK)))
        );
        assert_eq!(
            list.grant(&id(BITWARDEN), &grants(&[], &[""])),
            Err(ExtensionError::BadGrant(id(BITWARDEN), String::new()))
        );
        assert_eq!(
            list.revoke(&id(UBLOCK), &grants(&["tabs"], &[])),
            Err(ExtensionError::NotInstalled(id(UBLOCK)))
        );
        assert_eq!(
            list.get(&id(BITWARDEN)).unwrap().granted(),
            &grants(&["storage"], &[])
        );
    }

    #[test]
    fn a_site_decision_is_set_changed_and_forgotten() {
        let mut list = with_bitwarden();
        let bitwarden = id(BITWARDEN);

        assert_eq!(
            list.set_access(&bitwarden, site("a.com"), Some(SiteAccess::Allowed)),
            Ok(true)
        );
        assert_eq!(
            list.set_access(&bitwarden, site("a.com"), Some(SiteAccess::Allowed)),
            Ok(false)
        );
        assert_eq!(
            list.set_access(&bitwarden, site("b.com"), Some(SiteAccess::Blocked)),
            Ok(true)
        );
        assert_eq!(
            list.set_access(&bitwarden, site("a.com"), Some(SiteAccess::Blocked)),
            Ok(true)
        );
        let sites: Vec<(String, SiteAccess)> = list
            .get(&bitwarden)
            .unwrap()
            .sites()
            .map(|(site, access)| (site.to_string(), access))
            .collect();
        assert_eq!(
            sites,
            [
                ("a.com".to_owned(), SiteAccess::Blocked),
                ("b.com".to_owned(), SiteAccess::Blocked)
            ]
        );
        assert_eq!(list.set_access(&bitwarden, site("a.com"), None), Ok(true));
        assert_eq!(list.set_access(&bitwarden, site("a.com"), None), Ok(false));
        assert_eq!(list.get(&bitwarden).unwrap().access(&site("a.com")), None);
        assert_eq!(
            list.set_access(&id(UBLOCK), site("a.com"), None),
            Err(ExtensionError::NotInstalled(id(UBLOCK)))
        );
    }
}

mod updates {
    use super::*;

    const NOW: u64 = 10 * Timestamp::DAY_MS;

    #[test]
    fn auto_update_is_off_until_the_user_turns_it_on() {
        let mut list = InstalledExtensions::new();

        assert!(!list.auto_update());
        assert!(list.set_auto_update(true));
        assert!(!list.set_auto_update(true));
        assert!(list.auto_update());
        assert!(list.set_auto_update(false));
    }

    #[test]
    fn a_look_is_due_only_with_auto_update_and_a_day_after_the_last() {
        let mut list = InstalledExtensions::new();
        let now = Timestamp::from_millis(NOW);

        assert!(!list.update_due(now));
        list.set_auto_update(true);
        assert!(list.update_due(now));
        list.checked(Timestamp::from_millis(NOW - Timestamp::DAY_MS + 1));
        assert!(!list.update_due(now));
        list.checked(Timestamp::from_millis(NOW - Timestamp::DAY_MS));
        assert!(list.update_due(now));
        assert_eq!(
            list.last_checked(),
            Some(Timestamp::from_millis(NOW - Timestamp::DAY_MS))
        );
    }
}

mod snapshots {
    use super::*;

    fn snapshot() -> InstalledExtensionsSnapshot {
        let mut list = with_bitwarden();
        list.set_access(&id(BITWARDEN), site("a.com"), Some(SiteAccess::Allowed))
            .unwrap();
        list.set_auto_update(true);
        list.checked(Timestamp::from_millis(5));
        list.snapshot()
    }

    #[test]
    fn a_snapshot_restores_an_equal_list() {
        let mut list = with_bitwarden();
        list.set_access(&id(BITWARDEN), site("a.com"), Some(SiteAccess::Allowed))
            .unwrap();
        list.set_enabled(&id(BITWARDEN), false).unwrap();
        list.set_auto_update(true);
        list.checked(Timestamp::from_millis(5));

        let restored = InstalledExtensions::restore(list.snapshot()).unwrap();

        assert_eq!(restored, list);
        let entry = &snapshot().extensions[0];
        assert_eq!(entry.id, BITWARDEN);
        assert_eq!(entry.version, "2025.1.0");
        assert_eq!(entry.sites, [(site("a.com"), SiteAccess::Allowed)]);
    }

    #[test]
    fn a_corrupt_snapshot_is_refused() {
        let mut bad_id = snapshot();
        bad_id.extensions[0].id = "nope".to_owned();
        let mut bad_version = snapshot();
        bad_version.extensions[0].version = "x".to_owned();
        let mut twice = snapshot();
        twice.extensions.push(twice.extensions[0].clone());
        let mut site_twice = snapshot();
        let first = site_twice.extensions[0].sites[0].clone();
        site_twice.extensions[0].sites.push(first);
        let mut nameless = snapshot();
        nameless.extensions[0].name = String::new();

        assert_eq!(
            InstalledExtensions::restore(bad_id),
            Err(CorruptExtensions::Id("nope".to_owned()))
        );
        assert_eq!(
            InstalledExtensions::restore(bad_version),
            Err(CorruptExtensions::Version(id(BITWARDEN), "x".to_owned()))
        );
        assert_eq!(
            InstalledExtensions::restore(twice),
            Err(CorruptExtensions::Duplicate(id(BITWARDEN)))
        );
        assert_eq!(
            InstalledExtensions::restore(site_twice),
            Err(CorruptExtensions::DuplicateSite(
                id(BITWARDEN),
                site("a.com")
            ))
        );
        assert_eq!(
            InstalledExtensions::restore(nameless),
            Err(CorruptExtensions::Entry(ExtensionError::NoName(id(
                BITWARDEN
            ))))
        );
    }
}
