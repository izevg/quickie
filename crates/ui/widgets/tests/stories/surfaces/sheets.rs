//! The sheets group: E3 decisions a page or an extension asks for, headed by who asks.

use std::cell::RefCell;
use std::rc::Rc;

use quickie_application::{Permission, PermissionAnswer};
use quickie_ui_shared::stories::{LIGHT, Scene, Story, Theme, WINDOW};
use quickie_ui_widgets::{
    DialogChoice, DialogKind, DialogRequest, DialogSheet, ExtensionRequest, ExtensionSheet, Moment,
    PermissionSheet,
};

use crate::fixtures::{self, LONG_TITLE};

const fn permission(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "sheets",
        widget: "permission-sheet",
        state,
        themes,
        window: WINDOW,
        scene,
    }
}

const fn dialog(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "sheets",
        widget: "dialog-sheet",
        state,
        themes,
        window: WINDOW,
        scene,
    }
}

const fn extension(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "sheets",
        widget: "extension-sheet",
        state,
        themes,
        window: WINDOW,
        scene,
    }
}

pub const STORIES: &[Story] = &[
    extension("install", &[Theme::Light, Theme::Dark], extension_install),
    extension("update", LIGHT, extension_update),
    extension("request", LIGHT, extension_request),
    permission("camera", &Theme::ALL, permission_camera),
    permission("microphone", LIGHT, permission_microphone),
    permission("camera-and-microphone", LIGHT, permission_both),
    dialog("alert", &[Theme::Light, Theme::Dark], dialog_alert),
    dialog("confirm-block", LIGHT, dialog_confirm),
    dialog("prompt", LIGHT, dialog_prompt),
    dialog("long-message", LIGHT, dialog_long),
];

/// Clicks the answer button `label` (found by what `VoiceOver` reads) and checks it was the
/// one `answers` recorded last.
fn answer<T: PartialEq + std::fmt::Debug + Copy>(
    scene: &mut Scene,
    answers: &Rc<RefCell<Vec<T>>>,
    label: &str,
    expected: T,
) {
    let Some(button) = scene.view_labelled(label) else {
        scene.check(false, format!("an answer button reads {label:?}"));
        return;
    };
    scene.click(&button);
    let last = answers.borrow().last().copied();
    scene.check_eq(
        last,
        Some(expected),
        format!("a click on {label:?} answers"),
    );
}

fn extension_sheet(
    scene: &mut Scene,
    moment: Moment,
    permissions: &[&str],
    patterns: &[&str],
) -> Rc<RefCell<Vec<bool>>> {
    let sheet = ExtensionSheet::new(scene.mtm());
    let request = ExtensionRequest {
        name: "Bitwarden Password Manager".to_owned(),
        moment,
        permissions: permissions.iter().map(|&name| name.to_owned()).collect(),
        patterns: patterns.iter().map(|&name| name.to_owned()).collect(),
    };
    let answered = Rc::new(RefCell::new(Vec::new()));
    let record = Rc::clone(&answered);
    sheet.show(scene.host(), scene.window(), &request, move |allowed| {
        record.borrow_mut().push(allowed);
    });
    scene.check(sheet.is_open(), "the sheet is open");
    let said = scene.accessibility().contains("Bitwarden Password Manager");
    scene.check(said, "the sheet names the extension");
    scene.keep(sheet);
    answered
}

/// `1` adds, the red default refuses: a click on each answers as its key would.
fn extension_install(scene: &mut Scene) {
    let answered = extension_sheet(
        scene,
        Moment::Install("2025.10.1".to_owned()),
        &["storage", "tabs", "nativeMessaging", "scripting"],
        &["<all_urls>"],
    );
    answer(scene, &answered, "Add Extension", true);
    answer(scene, &answered, "Cancel", false);
}

fn extension_update(scene: &mut Scene) {
    extension_sheet(
        scene,
        Moment::Update("2025.11.0".to_owned()),
        &["clipboardRead"],
        &[],
    );
}

fn extension_request(scene: &mut Scene) {
    extension_sheet(scene, Moment::Running, &[], &["https://*.example.com/*"]);
}

fn ask(scene: &mut Scene, origin: &str, wanted: Permission) -> Rc<RefCell<Vec<PermissionAnswer>>> {
    let sheet = PermissionSheet::new(scene.mtm());
    let answered = Rc::new(RefCell::new(Vec::new()));
    let record = Rc::clone(&answered);
    sheet.show(
        scene.host(),
        scene.window(),
        &fixtures::url(origin),
        wanted,
        move |answer| record.borrow_mut().push(answer),
    );
    scene.check(sheet.is_open(), "the sheet is open");
    scene.keep(sheet);
    answered
}

/// The buttons answer as their keys do: `1` once, `2` on this site, the red Block (`⎋`, the
/// default) denies.
fn permission_camera(scene: &mut Scene) {
    let answered = ask(
        scene,
        "https://meet.example.com/room/42",
        Permission::Camera,
    );
    answer(scene, &answered, "Allow once", PermissionAnswer::AllowOnce);
    answer(
        scene,
        &answered,
        "Allow on this site",
        PermissionAnswer::AllowOnSite,
    );
    answer(scene, &answered, "Block", PermissionAnswer::Deny);
}

fn permission_microphone(scene: &mut Scene) {
    ask(
        scene,
        "https://www.bbc.co.uk/sounds",
        Permission::Microphone,
    );
}

fn permission_both(scene: &mut Scene) {
    ask(
        scene,
        "http://192.168.1.20:8080/",
        Permission::CameraAndMicrophone,
    );
}

type Chosen = Rc<RefCell<Vec<DialogChoice>>>;

fn dialog_show(
    scene: &mut Scene,
    kind: DialogKind,
    message: &str,
    block: bool,
) -> (Rc<DialogSheet>, Chosen) {
    let sheet = Rc::new(DialogSheet::new(scene.mtm()));
    let request = DialogRequest {
        origin: fixtures::url("https://forms.example.org/apply"),
        kind,
        message: message.to_owned(),
        offer_block: block,
    };
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let record = Rc::clone(&chosen);
    sheet.show(scene.host(), scene.window(), &request, move |choice| {
        record.borrow_mut().push(choice);
    });
    scene.keep(Rc::clone(&sheet));
    (sheet, chosen)
}

fn dialog_alert(scene: &mut Scene) {
    dialog_show(
        scene,
        DialogKind::Alert,
        "Your session expires in 5 minutes.",
        false,
    );
}

/// OK, Cancel, and blocking each answer as their keys `⏎`, `⎋`, `⌘B` do.
fn dialog_confirm(scene: &mut Scene) {
    let (_, chosen) = dialog_show(
        scene,
        DialogKind::Confirm,
        "Leave this page? Changes you made may not be saved.",
        true,
    );
    answer(scene, &chosen, "OK", DialogChoice::Ok);
    answer(scene, &chosen, "Cancel", DialogChoice::Cancel);
    answer(scene, &chosen, "Block dialogs", DialogChoice::Block);
}

/// The prompt's field takes the keyboard and keeps what is typed, Cyrillic included.
fn dialog_prompt(scene: &mut Scene) {
    let (sheet, _) = dialog_show(
        scene,
        DialogKind::Prompt {
            default: String::new(),
        },
        "Как вас зовут? What is your name?",
        false,
    );
    scene.type_text("Ада Lovelace");
    scene.check_eq(
        sheet.text(),
        Some("Ада Lovelace".to_owned()),
        "the prompt keeps what is typed",
    );
}

fn dialog_long(scene: &mut Scene) {
    let message = format!("{LONG_TITLE}. {LONG_TITLE}.");
    dialog_show(scene, DialogKind::Confirm, &message, false);
}
