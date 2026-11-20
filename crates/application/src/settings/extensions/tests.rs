use quickie_domain_browsing::{TabId, Timestamp};
use quickie_domain_commands::{KeyStroke, Platform};
use quickie_domain_settings::{
    CorruptExtensions, ExtensionError, ExtensionId, ExtensionSnapshot, ExtensionSource,
    ExtensionVersion, Grants, Installed, InstalledExtensionsSnapshot, Site, SiteAccess,
};
use url::Url;

use super::fakes::{Call, FakeExtensionsRepo, FakeHost, FakeHttp, FakePackages};
use super::*;
use crate::fakes::FakeClock;
use crate::{
    Access, AppError, ExtensionCommand, ExtensionInfo, PackageError, PackageSource, ReplyToken,
    StoreError, Unpacked,
};

const BITWARDEN: &str = "nngceckbapebfimnlniiiahkandclblb";
const UBLOCK: &str = "ddkjiahejlhfcafbddmgiahcphecmpfh";
const DAY: u64 = Timestamp::DAY_MS;

type Uses = Extensions<FakeExtensionsRepo, FakeHost, FakeClock>;

fn id(text: &str) -> ExtensionId {
    ExtensionId::try_from(text).unwrap()
}

fn version(text: &str) -> ExtensionVersion {
    ExtensionVersion::try_from(text).unwrap()
}

fn grants(permissions: &[&str], patterns: &[&str]) -> Grants {
    Grants {
        permissions: permissions.iter().map(|&name| name.to_owned()).collect(),
        patterns: patterns.iter().map(|&name| name.to_owned()).collect(),
    }
}

fn info(at: &str, requested: Grants) -> ExtensionInfo {
    ExtensionInfo {
        name: "Bitwarden".to_owned(),
        version: version(at),
        description: "A password manager".to_owned(),
        requested,
        action: Some("Open".to_owned()),
        commands: vec![
            ExtensionCommand {
                name: "_execute_action".to_owned(),
                title: String::new(),
                shortcut: Some("cmd+shift+y".to_owned()),
            },
            ExtensionCommand {
                name: "autofill_login".to_owned(),
                title: "Autofill".to_owned(),
                shortcut: Some("cmd+shift+l".to_owned()),
            },
            ExtensionCommand {
                name: "generate_password".to_owned(),
                title: String::new(),
                shortcut: Some("not a key".to_owned()),
            },
        ],
    }
}

fn entry(text: &str, at: &str, source: ExtensionSource, enabled: bool) -> ExtensionSnapshot {
    ExtensionSnapshot {
        id: text.to_owned(),
        name: "Bitwarden".to_owned(),
        version: at.to_owned(),
        source,
        location: format!("/kept/{text}/{at}.zip"),
        enabled,
        granted: grants(&["storage"], &[]),
        sites: Vec::new(),
    }
}

struct World {
    uses: Uses,
    host: FakeHost,
    clock: FakeClock,
}

fn start_with(saved: InstalledExtensionsSnapshot) -> World {
    let host = FakeHost::default();
    let clock = FakeClock::default();
    clock.0.set(10 * DAY);
    let repo = FakeExtensionsRepo {
        saved,
        ..FakeExtensionsRepo::default()
    };
    let uses = Extensions::start(repo, host.clone(), clock.clone(), Platform::MacOs).unwrap();
    World { uses, host, clock }
}

fn empty() -> World {
    start_with(InstalledExtensionsSnapshot::default())
}

/// Bitwarden installed from the store at 2025.1.0, read and running.
fn installed() -> World {
    let mut world = start_with(InstalledExtensionsSnapshot {
        extensions: vec![entry(
            BITWARDEN,
            "2025.1.0",
            ExtensionSource::ChromeWebStore,
            true,
        )],
        ..InstalledExtensionsSnapshot::default()
    });
    world
        .uses
        .read(
            &id(BITWARDEN),
            Ok(info("2025.1.0", grants(&["storage"], &[]))),
        )
        .unwrap();
    world.host.clear();
    world
}

fn arrive(world: &mut World, text: &str, source: ExtensionSource) {
    world
        .uses
        .unpacked(
            Unpacked {
                id: id(text),
                location: format!("/kept/{text}/new.zip"),
            },
            source,
        )
        .unwrap();
}

fn access(granted: Grants) -> Access {
    Access {
        granted,
        sites: Vec::new(),
    }
}

fn saves(world: &World) -> usize {
    world.uses.repo.saves
}

mod starting {
    use super::*;

    #[test]
    fn every_installed_package_is_read_and_enabled_ones_run_once_read() {
        let mut world = start_with(InstalledExtensionsSnapshot {
            extensions: vec![
                entry(BITWARDEN, "1.0", ExtensionSource::ChromeWebStore, true),
                entry(UBLOCK, "1.0", ExtensionSource::File, false),
            ],
            ..InstalledExtensionsSnapshot::default()
        });

        assert_eq!(
            world.host.calls(),
            [
                Call::Read(id(BITWARDEN), format!("/kept/{BITWARDEN}/1.0.zip")),
                Call::Read(id(UBLOCK), format!("/kept/{UBLOCK}/1.0.zip")),
            ]
        );
        world.host.clear();

        let loaded = world
            .uses
            .read(&id(BITWARDEN), Ok(info("1.0", Grants::new())));
        let disabled = world.uses.read(&id(UBLOCK), Ok(info("1.0", Grants::new())));

        assert_eq!(loaded, Ok(ReadOutcome::Loaded));
        assert_eq!(disabled, Ok(ReadOutcome::Loaded));
        assert_eq!(
            world.host.calls(),
            [Call::Start(
                id(BITWARDEN),
                access(grants(&["storage"], &[]))
            )]
        );
        assert_eq!(world.uses.info(&id(UBLOCK)).unwrap().name, "Bitwarden");
        assert_eq!(world.uses.list().iter().count(), 2);
        assert!(world.uses.corrupt_on_start().is_none());
    }

    #[test]
    fn a_package_that_cannot_be_read_is_reported_and_runs_not() {
        let mut world = installed();

        let outcome = world.uses.read(&id(BITWARDEN), Err("gone".to_owned()));

        assert_eq!(
            outcome,
            Ok(ReadOutcome::Unreadable {
                id: id(BITWARDEN),
                error: "gone".to_owned()
            })
        );
        assert_eq!(world.host.calls(), []);
    }

    #[test]
    fn a_read_nobody_waits_for_is_ignored() {
        let mut world = empty();

        let outcome = world.uses.read(&id(UBLOCK), Ok(info("1", Grants::new())));

        assert_eq!(outcome, Ok(ReadOutcome::Ignored));
        assert!(world.uses.info(&id(UBLOCK)).is_none());
        assert_eq!(world.host.calls(), []);
    }

    #[test]
    fn a_corrupt_list_starts_empty() {
        let mut bad = entry(BITWARDEN, "1.0", ExtensionSource::File, true);
        bad.version = "x".to_owned();
        let world = start_with(InstalledExtensionsSnapshot {
            extensions: vec![bad],
            ..InstalledExtensionsSnapshot::default()
        });

        assert_eq!(
            world.uses.corrupt_on_start(),
            Some(&CorruptExtensions::Version(id(BITWARDEN), "x".to_owned()))
        );
        assert_eq!(world.uses.list().iter().count(), 0);
        assert_eq!(world.host.calls(), []);
    }

    #[test]
    fn a_store_or_engine_failure_stops_the_start() {
        let repo = FakeExtensionsRepo {
            fail_load: true,
            ..FakeExtensionsRepo::default()
        };
        let failed = Uses::start(
            repo,
            FakeHost::default(),
            FakeClock::default(),
            Platform::MacOs,
        );
        assert_eq!(
            failed.err(),
            Some(AppError::Store(StoreError("unreadable".to_owned())))
        );

        let repo = FakeExtensionsRepo {
            saved: InstalledExtensionsSnapshot {
                extensions: vec![entry(BITWARDEN, "1", ExtensionSource::File, true)],
                ..InstalledExtensionsSnapshot::default()
            },
            ..FakeExtensionsRepo::default()
        };
        let host = FakeHost {
            refuse: true,
            ..FakeHost::default()
        };
        let failed = Uses::start(repo, host, FakeClock::default(), Platform::MacOs);
        assert!(matches!(failed, Err(AppError::Engine(_))));
    }
}

mod installing {
    use super::*;

    #[test]
    fn a_new_extension_waits_for_approval_then_runs_with_what_it_asked() {
        let mut world = empty();
        arrive(&mut world, BITWARDEN, ExtensionSource::ChromeWebStore);
        let asked = grants(&["storage", "tabs"], &["<all_urls>"]);

        let outcome = world
            .uses
            .read(&id(BITWARDEN), Ok(info("1.0", asked.clone())));

        assert_eq!(
            outcome,
            Ok(ReadOutcome::Approve(Approval {
                id: id(BITWARDEN),
                name: "Bitwarden".to_owned(),
                version: version("1.0"),
                requested: asked.clone(),
                update: false,
            }))
        );
        assert_eq!(saves(&world), 0);
        assert!(world.uses.list().get(&id(BITWARDEN)).is_none());

        let installed = world.uses.approve(&id(BITWARDEN));

        assert_eq!(installed, Ok(Installed::New));
        let extension = world.uses.list().get(&id(BITWARDEN)).unwrap();
        assert_eq!(extension.granted(), &asked);
        assert_eq!(extension.source(), ExtensionSource::ChromeWebStore);
        assert_eq!(extension.location(), format!("/kept/{BITWARDEN}/new.zip"));
        assert_eq!(saves(&world), 1);
        assert_eq!(
            world.uses.info(&id(BITWARDEN)).unwrap().version,
            version("1.0")
        );
        assert_eq!(
            world.host.calls(),
            [
                Call::Read(id(BITWARDEN), format!("/kept/{BITWARDEN}/new.zip")),
                Call::Start(id(BITWARDEN), access(asked)),
            ]
        );
        assert_eq!(
            world.uses.approve(&id(BITWARDEN)),
            Err(AppError::Extension(ExtensionError::NotInstalled(id(
                BITWARDEN
            ))))
        );
    }

    #[test]
    fn the_kept_packages_are_the_installed_and_the_waiting_ones() {
        let mut world = installed();
        arrive(&mut world, UBLOCK, ExtensionSource::File);

        assert_eq!(
            world.uses.locations(),
            [
                format!("/kept/{BITWARDEN}/2025.1.0.zip"),
                format!("/kept/{UBLOCK}/new.zip"),
            ]
        );
    }

    #[test]
    fn approving_before_the_engine_read_the_package_is_refused() {
        let mut world = empty();
        arrive(&mut world, BITWARDEN, ExtensionSource::File);

        assert!(world.uses.approve(&id(BITWARDEN)).is_err());
        assert_eq!(saves(&world), 0);
    }

    #[test]
    fn declining_drops_the_package() {
        let mut world = empty();
        arrive(&mut world, BITWARDEN, ExtensionSource::File);
        world
            .uses
            .read(&id(BITWARDEN), Ok(info("1.0", Grants::new())))
            .unwrap();
        world.host.clear();

        world.uses.decline(&id(BITWARDEN));
        world.uses.decline(&id(BITWARDEN));

        assert_eq!(world.host.calls(), [Call::Discard(id(BITWARDEN))]);
        assert!(world.uses.approve(&id(BITWARDEN)).is_err());
        assert_eq!(saves(&world), 0);
    }

    #[test]
    fn an_unreadable_package_is_dropped() {
        let mut world = empty();
        arrive(&mut world, BITWARDEN, ExtensionSource::File);
        world.host.clear();

        let outcome = world
            .uses
            .read(&id(BITWARDEN), Err("no manifest".to_owned()));

        assert_eq!(
            outcome,
            Ok(ReadOutcome::Unreadable {
                id: id(BITWARDEN),
                error: "no manifest".to_owned()
            })
        );
        assert_eq!(world.host.calls(), [Call::Discard(id(BITWARDEN))]);
    }

    #[test]
    fn a_package_the_list_refuses_is_dropped() {
        let mut world = empty();
        arrive(&mut world, BITWARDEN, ExtensionSource::File);
        let mut nameless = info("1.0", Grants::new());
        nameless.name = String::new();
        world.uses.read(&id(BITWARDEN), Ok(nameless)).unwrap();
        world.host.clear();

        let approved = world.uses.approve(&id(BITWARDEN));

        assert_eq!(
            approved,
            Err(AppError::Extension(ExtensionError::NoName(id(BITWARDEN))))
        );
        assert_eq!(world.host.calls(), [Call::Discard(id(BITWARDEN))]);
        assert_eq!(saves(&world), 0);
    }

    #[test]
    fn an_engine_that_cannot_read_refuses_the_package() {
        let mut world = empty();
        world.host.refuse = true;
        let host = world.host.clone();
        world.uses.host = host;

        let unpacked = world.uses.unpacked(
            Unpacked {
                id: id(BITWARDEN),
                location: "/kept".to_owned(),
            },
            ExtensionSource::File,
        );

        assert!(matches!(unpacked, Err(AppError::Engine(_))));
        world.host.refuse = false;
        world.uses.host = world.host.clone();
        assert_eq!(
            world
                .uses
                .read(&id(BITWARDEN), Ok(info("1", Grants::new()))),
            Ok(ReadOutcome::Ignored)
        );
    }

    #[test]
    fn a_failed_save_installs_nothing() {
        let mut world = empty();
        arrive(&mut world, BITWARDEN, ExtensionSource::File);
        world
            .uses
            .read(&id(BITWARDEN), Ok(info("1.0", Grants::new())))
            .unwrap();
        world.uses.repo.fail = true;
        world.host.clear();

        let approved = world.uses.approve(&id(BITWARDEN));

        assert!(matches!(approved, Err(AppError::Store(_))));
        assert!(world.uses.list().get(&id(BITWARDEN)).is_none());
        assert!(world.uses.info(&id(BITWARDEN)).is_none());
        assert_eq!(world.host.calls(), []);
    }
}

mod updating {
    use super::*;

    #[test]
    fn an_update_that_asks_nothing_new_replaces_the_running_version() {
        let mut world = installed();
        arrive(&mut world, BITWARDEN, ExtensionSource::ChromeWebStore);

        let outcome = world.uses.read(
            &id(BITWARDEN),
            Ok(info("2025.2.0", grants(&["storage"], &[]))),
        );

        assert_eq!(
            outcome,
            Ok(ReadOutcome::Updated {
                name: "Bitwarden".to_owned(),
                version: version("2025.2.0")
            })
        );
        let extension = world.uses.list().get(&id(BITWARDEN)).unwrap();
        assert_eq!(extension.version(), &version("2025.2.0"));
        assert_eq!(extension.location(), format!("/kept/{BITWARDEN}/new.zip"));
        assert_eq!(
            world.uses.info(&id(BITWARDEN)).unwrap().version,
            version("2025.2.0")
        );
        assert_eq!(
            world.host.calls().last(),
            Some(&Call::Start(
                id(BITWARDEN),
                access(grants(&["storage"], &[]))
            ))
        );
    }

    #[test]
    fn an_update_that_asks_for_more_waits_for_approval_of_what_is_new() {
        let mut world = installed();
        arrive(&mut world, BITWARDEN, ExtensionSource::ChromeWebStore);

        let outcome = world.uses.read(
            &id(BITWARDEN),
            Ok(info("2025.2.0", grants(&["storage", "tabs"], &[]))),
        );

        assert_eq!(
            outcome,
            Ok(ReadOutcome::Approve(Approval {
                id: id(BITWARDEN),
                name: "Bitwarden".to_owned(),
                version: version("2025.2.0"),
                requested: grants(&["tabs"], &[]),
                update: true,
            }))
        );
        assert_eq!(
            world.uses.approve(&id(BITWARDEN)),
            Ok(Installed::Replaced {
                previous: version("2025.1.0")
            })
        );
        assert_eq!(
            world.uses.list().get(&id(BITWARDEN)).unwrap().granted(),
            &grants(&["storage", "tabs"], &[])
        );
    }

    #[test]
    fn a_store_package_not_newer_is_dropped() {
        let mut world = installed();
        arrive(&mut world, BITWARDEN, ExtensionSource::ChromeWebStore);
        world.host.clear();

        let outcome = world
            .uses
            .read(&id(BITWARDEN), Ok(info("2025.1.0", Grants::new())));

        assert_eq!(
            outcome,
            Ok(ReadOutcome::UpToDate {
                name: "Bitwarden".to_owned()
            })
        );
        assert_eq!(world.host.calls(), [Call::Discard(id(BITWARDEN))]);
        assert_eq!(saves(&world), 0);
    }

    #[test]
    fn a_file_installs_again_at_the_same_version() {
        let mut world = installed();
        arrive(&mut world, BITWARDEN, ExtensionSource::File);

        let outcome = world
            .uses
            .read(&id(BITWARDEN), Ok(info("2025.1.0", Grants::new())));

        assert!(matches!(outcome, Ok(ReadOutcome::Updated { .. })));
        assert_eq!(
            world.uses.list().get(&id(BITWARDEN)).unwrap().source(),
            ExtensionSource::File
        );
    }

    #[test]
    fn a_disabled_extension_updates_without_running() {
        let mut world = installed();
        world.uses.set_enabled(&id(BITWARDEN), false).unwrap();
        arrive(&mut world, BITWARDEN, ExtensionSource::ChromeWebStore);
        world.host.clear();

        world
            .uses
            .read(&id(BITWARDEN), Ok(info("2025.2.0", Grants::new())))
            .unwrap();

        assert_eq!(world.host.calls(), []);
        assert!(!world.uses.list().get(&id(BITWARDEN)).unwrap().enabled());
    }

    #[test]
    fn checks_ask_the_store_for_its_extensions_at_their_versions() {
        let mut world = installed();
        arrive(&mut world, UBLOCK, ExtensionSource::File);
        world
            .uses
            .read(&id(UBLOCK), Ok(info("1", Grants::new())))
            .unwrap();
        world.uses.approve(&id(UBLOCK)).unwrap();

        let checks = world.uses.update_checks();

        assert_eq!(
            checks,
            [UpdateCheck::new(id(BITWARDEN), Some(&version("2025.1.0"))).unwrap()]
        );
    }

    #[test]
    fn checks_are_due_only_with_auto_update_once_a_day() {
        let mut world = installed();

        assert_eq!(world.uses.due_update_checks(), []);
        assert_eq!(world.uses.set_auto_update(true), Ok(true));
        assert_eq!(world.uses.set_auto_update(true), Ok(false));
        assert_eq!(world.uses.due_update_checks().len(), 1);
        world.uses.checked().unwrap();
        assert_eq!(world.uses.due_update_checks(), []);
        assert_eq!(
            world.uses.repo.saved.last_checked,
            Some(Timestamp::from_millis(10 * DAY))
        );
        assert!(world.uses.repo.saved.auto_update);
        world.clock.0.set(11 * DAY);
        assert_eq!(world.uses.due_update_checks().len(), 1);
    }

    #[test]
    fn a_failed_save_changes_no_setting() {
        let mut world = installed();
        world.uses.repo.fail = true;

        assert!(world.uses.set_auto_update(true).is_err());
        assert!(world.uses.checked().is_err());
        assert!(!world.uses.list().auto_update());
        assert_eq!(world.uses.list().last_checked(), None);
    }

    #[test]
    fn a_store_page_installs_from_version_zero_or_updates_the_installed_one() {
        let world = installed();
        let page = |text: &str| {
            Url::parse(&format!(
                "https://chromewebstore.google.com/detail/x/{text}"
            ))
            .unwrap()
        };

        assert_eq!(
            world.uses.store_install(&page(UBLOCK)),
            Ok(UpdateCheck::new(id(UBLOCK), None).unwrap())
        );
        assert_eq!(
            world.uses.store_install(&page(BITWARDEN)),
            Ok(UpdateCheck::new(id(BITWARDEN), Some(&version("2025.1.0"))).unwrap())
        );
        let elsewhere = Url::parse("https://example.com/").unwrap();
        assert_eq!(
            world.uses.store_install(&elsewhere),
            Err(AppError::NotAStorePage("https://example.com/".to_owned()))
        );
    }
}

mod choices {
    use super::*;

    #[test]
    fn disabling_stops_and_enabling_runs() {
        let mut world = installed();

        assert_eq!(world.uses.set_enabled(&id(BITWARDEN), false), Ok(true));
        assert_eq!(world.uses.set_enabled(&id(BITWARDEN), false), Ok(false));
        assert_eq!(world.uses.set_enabled(&id(BITWARDEN), true), Ok(true));

        assert_eq!(
            world.host.calls(),
            [
                Call::Stop(id(BITWARDEN)),
                Call::Start(id(BITWARDEN), access(grants(&["storage"], &[]))),
            ]
        );
        assert_eq!(saves(&world), 2);
        assert!(world.uses.set_enabled(&id(UBLOCK), true).is_err());
    }

    #[test]
    fn a_failed_save_leaves_it_running() {
        let mut world = installed();
        world.uses.repo.fail = true;

        assert!(world.uses.set_enabled(&id(BITWARDEN), false).is_err());
        assert!(world.uses.list().get(&id(BITWARDEN)).unwrap().enabled());
        assert_eq!(world.host.calls(), []);
    }

    #[test]
    fn removing_deletes_it_and_says_its_name() {
        let mut world = installed();

        assert_eq!(
            world.uses.remove(&id(BITWARDEN)),
            Ok("Bitwarden".to_owned())
        );

        assert!(world.uses.list().get(&id(BITWARDEN)).is_none());
        assert!(world.uses.info(&id(BITWARDEN)).is_none());
        assert_eq!(world.host.calls(), [Call::Remove(id(BITWARDEN))]);
        assert_eq!(
            world.uses.remove(&id(BITWARDEN)),
            Err(AppError::Extension(ExtensionError::NotInstalled(id(
                BITWARDEN
            ))))
        );
    }

    #[test]
    fn a_failed_save_removes_nothing() {
        let mut world = installed();
        world.uses.repo.fail = true;

        assert!(world.uses.remove(&id(BITWARDEN)).is_err());
        assert!(world.uses.info(&id(BITWARDEN)).is_some());
        assert_eq!(world.host.calls(), []);
    }

    #[test]
    fn a_site_decision_reaches_the_running_extension() {
        let mut world = installed();
        let page = Url::parse("https://www.example.com/login").unwrap();

        let set = world
            .uses
            .set_site_access(&id(BITWARDEN), &page, Some(SiteAccess::Blocked));
        let again = world
            .uses
            .set_site_access(&id(BITWARDEN), &page, Some(SiteAccess::Blocked));

        assert_eq!((set, again), (Ok(true), Ok(false)));
        assert_eq!(
            world.host.calls(),
            [Call::Start(
                id(BITWARDEN),
                Access {
                    granted: grants(&["storage"], &[]),
                    sites: vec![(Site::try_from("example.com").unwrap(), SiteAccess::Blocked)],
                }
            )]
        );
        let start = Url::parse("quickie://start/").unwrap();
        assert!(matches!(
            world.uses.set_site_access(&id(BITWARDEN), &start, None),
            Err(AppError::NoSite(_))
        ));
    }

    #[test]
    fn an_allowed_request_is_saved_and_granted() {
        let mut world = installed();
        let token = ReplyToken::new(7);
        let tabs = grants(&["tabs"], &[]);

        world
            .uses
            .answer(token, &id(BITWARDEN), &tabs, true)
            .unwrap();

        assert_eq!(world.host.calls(), [Call::Answer(token, tabs)]);
        assert_eq!(
            world.uses.list().get(&id(BITWARDEN)).unwrap().granted(),
            &grants(&["storage", "tabs"], &[])
        );
        assert_eq!(saves(&world), 1);
    }

    #[test]
    fn a_refused_or_unknown_request_gets_nothing() {
        let mut world = installed();
        let tabs = grants(&["tabs"], &[]);

        world
            .uses
            .answer(ReplyToken::new(1), &id(BITWARDEN), &tabs, false)
            .unwrap();
        world
            .uses
            .answer(ReplyToken::new(2), &id(UBLOCK), &tabs, true)
            .unwrap();

        assert_eq!(
            world.host.calls(),
            [
                Call::Answer(ReplyToken::new(1), Grants::new()),
                Call::Answer(ReplyToken::new(2), Grants::new()),
            ]
        );
        assert_eq!(saves(&world), 0);
    }

    #[test]
    fn a_request_that_cannot_be_saved_gets_nothing() {
        let mut world = installed();
        world.uses.repo.fail = true;

        let answered = world.uses.answer(
            ReplyToken::new(3),
            &id(BITWARDEN),
            &grants(&["tabs"], &[]),
            true,
        );

        assert!(answered.is_err());
        assert_eq!(
            world.host.calls(),
            [Call::Answer(ReplyToken::new(3), Grants::new())]
        );
    }
}

mod triggers {
    use super::*;

    fn stroke(text: &str) -> KeyStroke {
        KeyStroke::parse(text, Platform::MacOs).unwrap()
    }

    fn trigger(run: ExtensionRun) -> ExtensionTrigger {
        ExtensionTrigger {
            id: id(BITWARDEN),
            run,
        }
    }

    #[test]
    fn a_running_extension_offers_its_action_then_its_commands() {
        let world = installed();

        let offers = world.uses.offers();

        assert_eq!(
            offers,
            [
                ExtensionOffer {
                    trigger: trigger(ExtensionRun::Action),
                    title: "Bitwarden: Open".to_owned(),
                    stroke: Some(stroke("cmd+shift+y")),
                },
                ExtensionOffer {
                    trigger: trigger(ExtensionRun::Command("autofill_login".to_owned())),
                    title: "Bitwarden: Autofill".to_owned(),
                    stroke: Some(stroke("cmd+shift+l")),
                },
                ExtensionOffer {
                    trigger: trigger(ExtensionRun::Command("generate_password".to_owned())),
                    title: "Bitwarden: generate_password".to_owned(),
                    stroke: None,
                },
            ]
        );
    }

    #[test]
    fn an_action_without_a_key_and_no_action_at_all() {
        let mut world = installed();
        let mut plain = info("2025.1.0", Grants::new());
        plain.commands.remove(0);
        world.uses.read(&id(BITWARDEN), Ok(plain.clone())).unwrap();
        assert_eq!(world.uses.offers()[0].stroke, None);

        plain.action = None;
        world.uses.read(&id(BITWARDEN), Ok(plain)).unwrap();
        assert_eq!(
            world.uses.offers()[0].trigger,
            trigger(ExtensionRun::Command("autofill_login".to_owned()))
        );
    }

    #[test]
    fn stopped_or_unread_extensions_offer_nothing() {
        let mut world = installed();
        world.uses.set_enabled(&id(BITWARDEN), false).unwrap();
        assert_eq!(world.uses.offers(), []);

        let unread = start_with(InstalledExtensionsSnapshot {
            extensions: vec![entry(BITWARDEN, "1", ExtensionSource::File, true)],
            ..InstalledExtensionsSnapshot::default()
        });
        assert_eq!(unread.uses.offers(), []);
    }

    #[test]
    fn an_extension_key_that_types_is_no_shortcut() {
        let mut world = installed();
        let mut typing = info("2025.1.0", Grants::new());
        typing.commands[0].shortcut = Some("shift+y".to_owned());
        typing.commands[1].shortcut = Some("alt+l".to_owned());
        world.uses.read(&id(BITWARDEN), Ok(typing)).unwrap();

        let offers = world.uses.offers();

        assert_eq!(offers[0].stroke, None);
        assert_eq!(offers[1].stroke, Some(stroke("alt+l")));
        assert_eq!(world.uses.trigger_for(stroke("shift+y")), None);
    }

    #[test]
    fn a_key_finds_what_it_runs() {
        let world = installed();

        assert_eq!(
            world.uses.trigger_for(stroke("cmd+shift+l")),
            Some(trigger(ExtensionRun::Command("autofill_login".to_owned())))
        );
        assert_eq!(world.uses.trigger_for(stroke("cmd+shift+k")), None);
    }

    #[test]
    fn running_a_trigger_asks_the_engine() {
        let mut world = installed();
        let tab = TabId::new(quickie_domain_browsing::Profile::Main, 4);

        world
            .uses
            .trigger(&trigger(ExtensionRun::Action), Some(tab))
            .unwrap();
        world
            .uses
            .trigger(
                &trigger(ExtensionRun::Command("autofill_login".to_owned())),
                None,
            )
            .unwrap();

        assert_eq!(
            world.host.calls(),
            [
                Call::Action(id(BITWARDEN), Some(tab)),
                Call::Command(id(BITWARDEN), "autofill_login".to_owned()),
            ]
        );
        world.uses.host.refuse = true;
        assert!(
            world
                .uses
                .trigger(&trigger(ExtensionRun::Action), None)
                .is_err()
        );
    }
}

mod store {
    use super::*;

    const CODEBASE: &str = "https://clients2.googleusercontent.com/crx/blobs/abc/bitwarden.crx";
    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn answer(app: &str, check: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?><gupdate protocol=\"2.0\">\
             <app appid=\"{UBLOCK}\" status=\"ok\"><updatecheck status=\"noupdate\"/></app>\
             <app appid=\"{app}\" status=\"ok\"><updatecheck {check}/></app></gupdate>"
        )
    }

    fn offer(check: &str) -> Result<Option<UpdateOffer>, PackageError> {
        parse_answer(&answer(BITWARDEN, check), &id(BITWARDEN))
    }

    #[test]
    fn a_check_names_the_extension_and_its_version() {
        let fresh = UpdateCheck::new(id(BITWARDEN), None).unwrap();
        let installed = UpdateCheck::new(id(BITWARDEN), Some(&version("1.2"))).unwrap();

        assert_eq!(
            fresh.url.as_str(),
            format!(
                "https://clients2.google.com/service/update2/crx?response=updatecheck&\
                 acceptformat=crx3&prodversion=140.0.0.0&x=id%3D{BITWARDEN}%26v%3D0%26\
                 installsource%3Dondemand%26uc"
            )
        );
        assert!(installed.url.as_str().contains("%26v%3D1.2%26"));
        assert_eq!(installed.id, id(BITWARDEN));
    }

    #[test]
    fn an_offer_has_a_version_a_download_and_maybe_a_hash() {
        let check = format!(
            "codebase=\"{CODEBASE}?a=1&amp;b=2\" hash_sha256=\"{HASH}\" status=\"ok\" \
             version=\"2025.2.0\""
        );

        assert_eq!(
            offer(&check),
            Ok(Some(UpdateOffer {
                version: version("2025.2.0"),
                download: Url::parse(&format!("{CODEBASE}?a=1&b=2")).unwrap(),
                sha256: Some(HASH.to_owned()),
            }))
        );
    }

    #[test]
    fn a_malformed_hash_is_no_hash() {
        for hash in [
            &HASH[1..],
            &HASH.to_uppercase(),
            &format!("{}g", &HASH[1..]),
        ] {
            let check = format!(
                "codebase=\"{CODEBASE}\" hash_sha256=\"{hash}\" status=\"ok\" version=\"1\""
            );
            assert_eq!(offer(&check).unwrap().unwrap().sha256, None, "{hash}");
        }
    }

    #[test]
    fn no_update_is_no_offer() {
        assert_eq!(offer("status=\"noupdate\""), Ok(None));
    }

    #[test]
    fn a_refusal_or_an_incomplete_offer_is_an_error() {
        let refused = offer("status=\"error-unknownApplication\"");
        assert_eq!(
            refused,
            Err(PackageError(format!(
                "the store's answer for {BITWARDEN} is status error-unknownApplication"
            )))
        );
        assert_eq!(
            offer("codebase=\"x\""),
            Err(PackageError(format!(
                "the store's answer for {BITWARDEN} is status none"
            )))
        );
        assert_eq!(
            offer(&format!("codebase=\"{CODEBASE}\" status=\"ok\"")),
            Err(PackageError(format!(
                "the store's answer for {BITWARDEN} has no version"
            )))
        );
        assert_eq!(
            offer(&format!(
                "codebase=\"{CODEBASE}\" status=\"ok\" version=\"v1\""
            )),
            Err(PackageError(format!(
                "the store's answer for {BITWARDEN} has no version"
            )))
        );
        assert_eq!(
            offer("status=\"ok\" version=\"1\""),
            Err(PackageError(format!(
                "the store's answer for {BITWARDEN} has no download on the store's hosts"
            )))
        );
    }

    #[test]
    fn downloads_stay_on_the_store_s_hosts_over_https() {
        let at = |codebase: &str| {
            offer(&format!(
                "codebase=\"{codebase}\" status=\"ok\" version=\"1\""
            ))
            .map(|offer| offer.map(|offer| offer.download.to_string()))
        };

        assert!(at("https://google.com/x.crx").is_ok());
        assert!(at("https://edgedl.me.gvt1.com/x.crx").is_ok());
        assert!(at("http://clients2.googleusercontent.com/x.crx").is_err());
        assert!(at("https://evilgoogle.com/x.crx").is_err());
        assert!(at("https://example.com/x.crx").is_err());
        assert!(at("https://googleusercontent.com.example.com/x.crx").is_err());
        assert!(at("not a url").is_err());
    }

    #[test]
    fn an_answer_without_the_extension_or_its_check_is_an_error() {
        assert_eq!(
            parse_answer(&answer(UBLOCK, "status=\"ok\""), &id(BITWARDEN)),
            Err(PackageError(format!(
                "the store's answer for {BITWARDEN} is missing"
            )))
        );
        let no_check = format!("<gupdate><app appid=\"{BITWARDEN}\" status=\"ok\"></app>");
        assert_eq!(
            parse_answer(&no_check, &id(BITWARDEN)),
            Err(PackageError(format!(
                "the store's answer for {BITWARDEN} has no update check"
            )))
        );
        assert!(parse_answer("", &id(BITWARDEN)).is_err());
    }

    fn check() -> UpdateCheck {
        UpdateCheck::new(id(BITWARDEN), Some(&version("1"))).unwrap()
    }

    fn fetch(http: &FakeHttp, packages: &FakePackages) -> Fetched {
        fetch_packages(http, packages, &[check()]).remove(0)
    }

    #[test]
    fn a_newer_package_is_downloaded_and_kept() {
        let mut http = FakeHttp::default();
        let check =
            format!("codebase=\"{CODEBASE}\" hash_sha256=\"{HASH}\" status=\"ok\" version=\"2\"");
        http.answers
            .insert(self::check().url, answer(BITWARDEN, &check).into_bytes());
        http.answers
            .insert(Url::parse(CODEBASE).unwrap(), b"Cr24 package".to_vec());
        let packages = FakePackages::default();

        let fetched = fetch(&http, &packages);

        assert_eq!(fetched.id, id(BITWARDEN));
        assert_eq!(
            fetched.package,
            Ok(Some(Unpacked {
                id: id(BITWARDEN),
                location: format!("/kept/{BITWARDEN}.zip"),
            }))
        );
        assert_eq!(
            packages.given.borrow().as_slice(),
            [(
                PackageSource::Crx(b"Cr24 package".to_vec()),
                Some(crate::Expected {
                    id: id(BITWARDEN),
                    sha256: Some(HASH.to_owned()),
                })
            )]
        );
    }

    #[test]
    fn no_update_downloads_nothing() {
        let mut http = FakeHttp::default();
        http.answers.insert(
            check().url,
            answer(BITWARDEN, "status=\"noupdate\"").into_bytes(),
        );
        let packages = FakePackages::default();

        assert_eq!(fetch(&http, &packages).package, Ok(None));
        assert_eq!(http.asked.borrow().len(), 1);
        assert!(packages.given.borrow().is_empty());
    }

    #[test]
    fn a_failure_on_the_way_is_reported() {
        let http = FakeHttp::default();
        let packages = FakePackages::default();
        assert_eq!(
            fetch(&http, &packages).package,
            Err(PackageError(format!(
                "the store did not answer: http: no answer at {}",
                check().url
            )))
        );

        let mut http = FakeHttp::default();
        let offer = format!("codebase=\"{CODEBASE}\" status=\"ok\" version=\"2\"");
        http.answers
            .insert(check().url, answer(BITWARDEN, &offer).into_bytes());
        assert_eq!(
            fetch(&http, &packages).package,
            Err(PackageError(format!(
                "the download failed: http: no answer at {CODEBASE}"
            )))
        );

        http.answers
            .insert(Url::parse(CODEBASE).unwrap(), b"Cr24".to_vec());
        let refusing = FakePackages {
            refuse: true,
            ..FakePackages::default()
        };
        assert_eq!(
            fetch(&http, &refusing).package,
            Err(PackageError("bad signature".to_owned()))
        );
    }
}
