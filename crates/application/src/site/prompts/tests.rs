use quickie_domain_browsing::TabId;
use url::Url;

use super::{DialogGate, Permission, PermissionAnswer, PermissionGate, Prompts};

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn tab(n: u64) -> TabId {
    TabId::new(n)
}

const SHOW: DialogGate = DialogGate::Show { offer_block: false };
const SHOW_WITH_BLOCK: DialogGate = DialogGate::Show { offer_block: true };

mod permissions {
    use super::*;

    #[test]
    fn a_fresh_origin_is_asked() {
        let prompts = Prompts::new();

        assert_eq!(
            prompts.permission(&url("https://meet.example/"), Permission::Camera),
            PermissionGate::Ask
        );
    }

    #[test]
    fn allow_once_grants_without_remembering() {
        let mut prompts = Prompts::new();
        let meet = url("https://meet.example/room");

        let granted =
            prompts.answer_permission(&meet, Permission::Camera, PermissionAnswer::AllowOnce);

        assert!(granted);
        assert_eq!(
            prompts.permission(&meet, Permission::Camera),
            PermissionGate::Ask
        );
    }

    #[test]
    fn deny_refuses_without_remembering() {
        let mut prompts = Prompts::new();
        let meet = url("https://meet.example/");

        let granted = prompts.answer_permission(&meet, Permission::Camera, PermissionAnswer::Deny);

        assert!(!granted);
        assert_eq!(
            prompts.permission(&meet, Permission::Camera),
            PermissionGate::Ask
        );
    }

    #[test]
    fn allow_on_site_is_remembered_for_the_whole_origin_only() {
        let mut prompts = Prompts::new();

        let granted = prompts.answer_permission(
            &url("https://meet.example/a"),
            Permission::Microphone,
            PermissionAnswer::AllowOnSite,
        );

        assert!(granted);
        let gate = |text: &str, permission| prompts.permission(&url(text), permission);
        assert_eq!(
            gate("https://meet.example/b?x", Permission::Microphone),
            PermissionGate::Allow
        );
        assert_eq!(
            gate("https://meet.example/", Permission::Camera),
            PermissionGate::Ask
        );
        assert_eq!(
            gate("http://meet.example/", Permission::Microphone),
            PermissionGate::Ask
        );
        assert_eq!(
            gate("https://meet.example:8443/", Permission::Microphone),
            PermissionGate::Ask
        );
        assert_eq!(
            gate("https://other.example/", Permission::Microphone),
            PermissionGate::Ask
        );
    }

    #[test]
    fn both_devices_allowed_together_cover_each_and_both() {
        let mut prompts = Prompts::new();
        let meet = url("https://meet.example/");

        prompts.answer_permission(
            &meet,
            Permission::CameraAndMicrophone,
            PermissionAnswer::AllowOnSite,
        );

        for permission in [
            Permission::Camera,
            Permission::Microphone,
            Permission::CameraAndMicrophone,
        ] {
            assert_eq!(
                prompts.permission(&meet, permission),
                PermissionGate::Allow,
                "{permission:?}"
            );
        }
    }

    #[test]
    fn one_device_does_not_cover_both() {
        let mut prompts = Prompts::new();
        let meet = url("https://meet.example/");

        prompts.answer_permission(&meet, Permission::Camera, PermissionAnswer::AllowOnSite);

        assert_eq!(
            prompts.permission(&meet, Permission::CameraAndMicrophone),
            PermissionGate::Ask
        );
        prompts.answer_permission(&meet, Permission::Microphone, PermissionAnswer::AllowOnSite);
        assert_eq!(
            prompts.permission(&meet, Permission::CameraAndMicrophone),
            PermissionGate::Allow
        );
    }

    #[test]
    fn a_page_that_is_not_a_website_is_never_remembered() {
        let mut prompts = Prompts::new();
        let page = url("data:text/html,hi");

        let granted =
            prompts.answer_permission(&page, Permission::Camera, PermissionAnswer::AllowOnSite);

        assert!(granted);
        assert_eq!(
            prompts.permission(&page, Permission::Camera),
            PermissionGate::Ask
        );
    }
}

mod dialogs {
    use super::*;

    #[test]
    fn the_first_dialog_shows_without_the_block_option() {
        let mut prompts = Prompts::new();

        assert_eq!(prompts.dialog(tab(1)), SHOW);
    }

    #[test]
    fn a_second_dialog_while_one_is_open_is_cancelled() {
        let mut prompts = Prompts::new();
        prompts.dialog(tab(1));

        assert_eq!(prompts.dialog(tab(1)), DialogGate::Cancel);
        assert_eq!(prompts.dialog(tab(2)), SHOW);
    }

    #[test]
    fn the_second_dialog_in_a_row_offers_to_block() {
        let mut prompts = Prompts::new();
        prompts.dialog(tab(1));
        prompts.dialog_closed(tab(1), false);

        assert_eq!(prompts.dialog(tab(1)), SHOW_WITH_BLOCK);
        prompts.dialog_closed(tab(1), false);
        assert_eq!(prompts.dialog(tab(1)), SHOW_WITH_BLOCK);
    }

    #[test]
    fn a_blocked_page_gets_no_more_dialogs_until_it_navigates() {
        let mut prompts = Prompts::new();
        prompts.dialog(tab(1));
        prompts.dialog_closed(tab(1), false);
        prompts.dialog(tab(1));
        prompts.dialog_closed(tab(1), true);

        assert_eq!(prompts.dialog(tab(1)), DialogGate::Cancel);
        assert_eq!(prompts.dialog(tab(2)), SHOW);

        prompts.navigated(tab(1));
        assert_eq!(prompts.dialog(tab(1)), SHOW);
    }

    #[test]
    fn a_later_answer_does_not_unblock() {
        let mut prompts = Prompts::new();
        prompts.dialog(tab(1));
        prompts.dialog_closed(tab(1), true);
        prompts.dialog_closed(tab(1), false);

        assert_eq!(prompts.dialog(tab(1)), DialogGate::Cancel);
    }

    #[test]
    fn navigating_counts_afresh_but_keeps_an_open_dialog_open() {
        let mut prompts = Prompts::new();
        prompts.dialog(tab(1));
        prompts.dialog_closed(tab(1), false);
        prompts.dialog(tab(1));

        prompts.navigated(tab(1));

        assert_eq!(prompts.dialog(tab(1)), DialogGate::Cancel);
        prompts.dialog_closed(tab(1), false);
        assert_eq!(prompts.dialog(tab(1)), SHOW);
    }

    #[test]
    fn navigating_without_dialogs_or_closing_an_unknown_tab_changes_nothing() {
        let mut prompts = Prompts::new();

        prompts.navigated(tab(1));
        prompts.dialog_closed(tab(1), true);

        assert_eq!(prompts.dialog(tab(1)), SHOW);
    }

    #[test]
    fn a_closed_tab_is_forgotten() {
        let mut prompts = Prompts::new();
        prompts.dialog(tab(1));
        prompts.dialog_closed(tab(1), true);

        prompts.forget(tab(1));

        assert_eq!(prompts.dialog(tab(1)), SHOW);
    }
}
