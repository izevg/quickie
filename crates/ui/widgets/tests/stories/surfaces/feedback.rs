//! The feedback group: toasts and the full-screen notice.

use quickie_ui_shared::stories::{LIGHT, LIGHT_DARK, Scene, Story, Theme, WINDOW};
use quickie_ui_widgets::{FullscreenNotice, ToastStack};

use crate::fixtures;

const fn toasts(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "feedback",
        widget: "toast-stack",
        state,
        themes,
        window: WINDOW,
        scene,
    }
}

pub const STORIES: &[Story] = &[
    toasts("one", LIGHT_DARK, toast_one),
    toasts("stacked", LIGHT, toast_stacked),
    toasts("tagged", LIGHT, toast_tagged),
    Story {
        group: "feedback",
        widget: "fullscreen-notice",
        state: "page",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: fullscreen,
    },
];

fn toast_one(scene: &mut Scene) {
    let stack = ToastStack::new(scene.mtm());
    stack.show(scene.host(), "Opened in a background tab");
    scene.keep(stack);
}

fn toast_stacked(scene: &mut Scene) {
    let stack = ToastStack::new(scene.mtm());
    stack.show(scene.host(), "Copied URL");
    stack.show(scene.host(), "Закладка сохранена");
    stack.show(
        scene.host(),
        "Filter lists are fetched in the background; pages load meanwhile",
    );
    scene.check_eq(stack.texts().len(), 3, "three toasts stack");
    scene.keep(stack);
}

/// A tagged toast replaces the one with its tag: the zoom shows once, never stacked.
fn toast_tagged(scene: &mut Scene) {
    let stack = ToastStack::new(scene.mtm());
    for zoom in ["Zoom 110 %", "Zoom 125 %", "Zoom 150 %"] {
        stack.show_tagged(scene.host(), zoom, "zoom");
    }
    scene.check_eq(
        stack.texts(),
        vec!["Zoom 150 %".to_owned()],
        "one zoom toast",
    );
    scene.keep(stack);
}

fn fullscreen(scene: &mut Scene) {
    let notice = FullscreenNotice::new(scene.mtm());
    notice.show(
        scene.host(),
        &fixtures::url("https://www.youtube.com/watch?v=abc"),
    );
    scene.check(notice.text().is_some(), "the notice shows");
    scene.keep(notice);
}
