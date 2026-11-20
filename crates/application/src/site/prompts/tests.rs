use quickie_domain_browsing::{Profile, TabId};
use url::Url;

use super::{
    DialogAnswer, DialogGate, Permission, PermissionAnswer, PermissionGate, Prompts, Waiting,
};
use crate::{Answer, Ask, AskKind, Dialog, PageAsks, ReplyToken};

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn tab(n: u64) -> TabId {
    TabId::new(Profile::Main, n)
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
    fn a_closed_tab_is_forgotten_and_an_open_one_kept() {
        let mut prompts = Prompts::new();
        prompts.dialog(tab(1));
        prompts.dialog_closed(tab(1), true);
        prompts.dialog(tab(2));
        prompts.dialog_closed(tab(2), true);

        prompts.keep_tabs(|open| open == tab(2));

        assert_eq!(prompts.dialog(tab(1)), SHOW);
        assert_eq!(prompts.dialog(tab(2)), DialogGate::Cancel);
    }
}

/// Records each answer a page got, in order.
#[derive(Debug, Default)]
struct FakePages {
    answers: Vec<(ReplyToken, Answer)>,
}

impl PageAsks for FakePages {
    fn answer(&mut self, token: ReplyToken, answer: &Answer) -> bool {
        self.answers.push((token, answer.clone()));
        true
    }
}

fn dialog(token: u64, serial: u64) -> Ask {
    Ask {
        token: ReplyToken::new(token),
        tab: tab(serial),
        origin: url("https://chat.example/"),
        kind: AskKind::Dialog(Dialog::Confirm {
            message: "Sure?".to_owned(),
        }),
    }
}

fn camera(token: u64, serial: u64, origin: &str) -> Ask {
    Ask {
        token: ReplyToken::new(token),
        tab: tab(serial),
        origin: url(origin),
        kind: AskKind::Permission(Permission::Camera),
    }
}

fn waiting(ask: Ask, offer_block: bool) -> Waiting {
    Waiting { ask, offer_block }
}

fn token(raw: u64) -> ReplyToken {
    ReplyToken::new(raw)
}

mod waiting {
    use super::*;

    #[test]
    fn an_ask_waits_for_the_user_without_an_answer() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();

        prompts.asked(dialog(1, 1), &mut pages);
        prompts.asked(camera(2, 2, "https://meet.example/"), &mut pages);

        assert_eq!(pages.answers, []);
        assert_eq!(prompts.waiting(tab(1)), Some(&waiting(dialog(1, 1), false)));
        assert_eq!(
            prompts.waiting(tab(2)),
            Some(&waiting(camera(2, 2, "https://meet.example/"), false))
        );
        assert_eq!(prompts.waiting(tab(3)), None);
    }

    #[test]
    fn a_tab_shows_its_oldest_ask_first() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(camera(1, 1, "https://meet.example/"), &mut pages);
        prompts.asked(dialog(2, 1), &mut pages);

        assert_eq!(
            prompts.waiting(tab(1)).map(|waiting| waiting.ask.token),
            Some(token(1))
        );

        prompts.permission_answered(token(1), PermissionAnswer::Deny, &mut pages);

        assert_eq!(
            prompts.waiting(tab(1)).map(|waiting| waiting.ask.token),
            Some(token(2))
        );
    }

    #[test]
    fn a_dialog_while_one_waits_is_cancelled_at_once() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(dialog(1, 1), &mut pages);

        prompts.asked(dialog(2, 1), &mut pages);

        assert_eq!(pages.answers, [(token(2), Answer::Decline)]);
        assert!(prompts.dialog_answered(token(1), DialogAnswer::Cancel, &mut pages));
        assert!(!prompts.dialog_answered(token(2), DialogAnswer::Cancel, &mut pages));
    }

    #[test]
    fn the_second_dialog_in_a_row_offers_to_block() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(dialog(1, 1), &mut pages);

        assert!(!prompts.offers_block(token(1)));

        prompts.dialog_answered(token(1), DialogAnswer::Cancel, &mut pages);
        prompts.asked(dialog(2, 1), &mut pages);

        assert!(prompts.offers_block(token(2)));
        assert_eq!(prompts.waiting(tab(1)), Some(&waiting(dialog(2, 1), true)));
        assert!(!prompts.offers_block(token(3)));
    }

    #[test]
    fn a_device_allowed_on_the_site_is_granted_without_waiting() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(camera(1, 1, "https://meet.example/a"), &mut pages);
        prompts.permission_answered(token(1), PermissionAnswer::AllowOnSite, &mut pages);

        prompts.asked(camera(2, 2, "https://meet.example/b"), &mut pages);

        assert_eq!(
            pages.answers,
            [
                (token(1), Answer::Accept(None)),
                (token(2), Answer::Accept(None))
            ]
        );
        assert_eq!(prompts.waiting(tab(2)), None);
    }

    #[test]
    fn the_asks_of_closed_tabs_leave() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(dialog(1, 1), &mut pages);
        prompts.asked(dialog(2, 2), &mut pages);

        prompts.keep_tabs(|open| open == tab(2));

        assert_eq!(prompts.waiting(tab(1)), None);
        assert_eq!(prompts.waiting(tab(2)), Some(&waiting(dialog(2, 2), false)));
        assert!(!prompts.dialog_answered(token(1), DialogAnswer::Cancel, &mut pages));
        assert_eq!(pages.answers, []);
    }
}

mod answering {
    use super::*;

    #[test]
    fn ok_hands_the_text_to_the_page_and_ends_the_wait() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(dialog(1, 1), &mut pages);

        let answered = prompts.dialog_answered(
            token(1),
            DialogAnswer::Ok(Some("Grace".to_owned())),
            &mut pages,
        );

        assert!(answered);
        assert_eq!(
            pages.answers,
            [(token(1), Answer::Accept(Some("Grace".to_owned())))]
        );
        assert_eq!(prompts.waiting(tab(1)), None);
        assert_eq!(prompts.dialog(tab(1)), SHOW_WITH_BLOCK);
    }

    #[test]
    fn cancel_declines_and_lets_the_next_dialog_show() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(dialog(1, 1), &mut pages);

        prompts.dialog_answered(token(1), DialogAnswer::Cancel, &mut pages);

        assert_eq!(pages.answers, [(token(1), Answer::Decline)]);
        assert_eq!(prompts.dialog(tab(1)), SHOW_WITH_BLOCK);
    }

    #[test]
    fn block_declines_and_cancels_the_page_s_next_dialogs() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(dialog(1, 1), &mut pages);

        prompts.dialog_answered(token(1), DialogAnswer::Block, &mut pages);
        prompts.asked(dialog(2, 1), &mut pages);

        assert_eq!(
            pages.answers,
            [(token(1), Answer::Decline), (token(2), Answer::Decline)]
        );
        assert_eq!(prompts.waiting(tab(1)), None);
    }

    #[test]
    fn a_dialog_answer_does_not_answer_a_device_request() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(camera(1, 1, "https://meet.example/"), &mut pages);

        let answered = prompts.dialog_answered(token(1), DialogAnswer::Cancel, &mut pages);

        assert!(!answered);
        assert_eq!(pages.answers, []);
        assert!(prompts.waiting(tab(1)).is_some());
    }

    #[test]
    fn a_device_answer_does_not_answer_a_dialog() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(dialog(1, 1), &mut pages);

        let answered = prompts.permission_answered(token(1), PermissionAnswer::Deny, &mut pages);

        assert!(!answered);
        assert_eq!(pages.answers, []);
        assert!(prompts.waiting(tab(1)).is_some());
    }

    #[test]
    fn allowing_once_grants_and_deny_declines() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(camera(1, 1, "https://meet.example/"), &mut pages);
        prompts.asked(camera(2, 2, "https://meet.example/"), &mut pages);

        assert!(prompts.permission_answered(token(1), PermissionAnswer::AllowOnce, &mut pages));
        assert!(prompts.permission_answered(token(2), PermissionAnswer::Deny, &mut pages));

        assert_eq!(
            pages.answers,
            [
                (token(1), Answer::Accept(None)),
                (token(2), Answer::Decline)
            ]
        );
        assert_eq!(
            prompts.permission(&url("https://meet.example/"), Permission::Camera),
            PermissionGate::Ask
        );
        assert!(!prompts.permission_answered(token(1), PermissionAnswer::Deny, &mut pages));
    }

    #[test]
    fn the_right_ask_of_several_is_answered() {
        let mut prompts = Prompts::new();
        let mut pages = FakePages::default();
        prompts.asked(camera(1, 1, "https://meet.example/"), &mut pages);
        prompts.asked(camera(2, 2, "https://call.example/"), &mut pages);
        prompts.asked(dialog(3, 3), &mut pages);
        prompts.asked(dialog(4, 4), &mut pages);

        prompts.permission_answered(token(2), PermissionAnswer::AllowOnSite, &mut pages);
        prompts.dialog_answered(token(4), DialogAnswer::Cancel, &mut pages);

        assert!(prompts.waiting(tab(1)).is_some());
        assert_eq!(prompts.waiting(tab(2)), None);
        assert!(prompts.waiting(tab(3)).is_some());
        assert_eq!(prompts.waiting(tab(4)), None);
        assert_eq!(
            prompts.permission(&url("https://call.example/"), Permission::Camera),
            PermissionGate::Allow
        );
        assert_eq!(
            prompts.permission(&url("https://meet.example/"), Permission::Camera),
            PermissionGate::Ask
        );
    }
}
