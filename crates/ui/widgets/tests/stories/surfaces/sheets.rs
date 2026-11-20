//! The sheets group: E3 decisions a page asks for, headed by the origin that asks.

use std::rc::Rc;

use quickie_application::Permission;
use quickie_ui_shared::stories::{LIGHT, Scene, Story, Theme, WINDOW};
use quickie_ui_widgets::{DialogKind, DialogRequest, DialogSheet, PermissionSheet};

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

pub const STORIES: &[Story] = &[
    permission("camera", &Theme::ALL, permission_camera),
    permission("microphone", LIGHT, permission_microphone),
    permission("camera-and-microphone", LIGHT, permission_both),
    dialog("alert", &[Theme::Light, Theme::Dark], dialog_alert),
    dialog("confirm-block", LIGHT, dialog_confirm),
    dialog("prompt", LIGHT, dialog_prompt),
    dialog("long-message", LIGHT, dialog_long),
];

fn ask(scene: &mut Scene, origin: &str, wanted: Permission) {
    let sheet = PermissionSheet::new(scene.mtm());
    sheet.show(
        scene.host(),
        scene.window(),
        &fixtures::url(origin),
        wanted,
        |_| {},
    );
    scene.check(sheet.is_open(), "the sheet is open");
    scene.keep(sheet);
}

fn permission_camera(scene: &mut Scene) {
    ask(
        scene,
        "https://meet.example.com/room/42",
        Permission::Camera,
    );
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

fn dialog_show(scene: &mut Scene, kind: DialogKind, message: &str, block: bool) -> Rc<DialogSheet> {
    let sheet = Rc::new(DialogSheet::new(scene.mtm()));
    let request = DialogRequest {
        origin: fixtures::url("https://forms.example.org/apply"),
        kind,
        message: message.to_owned(),
        offer_block: block,
    };
    sheet.show(scene.host(), scene.window(), &request, |_| {});
    scene.keep(Rc::clone(&sheet));
    sheet
}

fn dialog_alert(scene: &mut Scene) {
    dialog_show(
        scene,
        DialogKind::Alert,
        "Your session expires in 5 minutes.",
        false,
    );
}

fn dialog_confirm(scene: &mut Scene) {
    dialog_show(
        scene,
        DialogKind::Confirm,
        "Leave this page? Changes you made may not be saved.",
        true,
    );
}

/// The prompt's field takes the keyboard and keeps what is typed, Cyrillic included.
fn dialog_prompt(scene: &mut Scene) {
    let sheet = dialog_show(
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
