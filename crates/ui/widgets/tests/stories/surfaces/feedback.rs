//! The feedback group: toasts, the debug keys overlay, and the full-screen notice.

use quickie_ui_shared::stories::{LIGHT, LIGHT_DARK, Scene, Story, Theme, WINDOW};
use quickie_ui_widgets::{FullscreenNotice, KeyLogPanel, ToastStack};

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
        widget: "key-log",
        state: "presses",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: key_log,
    },
    Story {
        group: "feedback",
        widget: "key-log",
        state: "long",
        themes: LIGHT,
        window: WINDOW,
        scene: key_log_long,
    },
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

/// The debug keys overlay as the shell's key log words its lines: key code, characters, held
/// modifiers, what the keymap read, and what became of the press. A Russian layout's `л` on
/// the `K` key still reads as `⌘K`.
const PRESSES: [&str; 6] = [
    "0x28 \"л\" ⌘ → ⌘K → quickie.palette.openCommands",
    "0x7d \"U+F701\" – → ↓ → overlay key",
    "0x24 \"U+000D\" – → ↩ → overlay key",
    "0x25 \"д\" ⌘ → ⌘L → quickie.address.focus",
    "0x03 \"f\" – → F → passed through",
    "0x31 \" \" fn → Space → passed through: fn/Globe is the system's",
];

fn key_log(scene: &mut Scene) {
    let panel = KeyLogPanel::new(scene.mtm());
    let lines: Vec<String> = PRESSES.iter().map(|&line| line.to_owned()).collect();
    panel.show(scene.host(), &lines[..3]);
    panel.update(&lines);
    scene.check(panel.is_open(), "the panel shows");
    scene.check(
        scene.accessibility().contains("quickie.address.focus"),
        "an update shows the newest press",
    );
    scene.keep(panel);
}

/// Lines past the panel's width are cut, not wrapped.
fn key_log_long(scene: &mut Scene) {
    let panel = KeyLogPanel::new(scene.mtm());
    let lines = vec![
        PRESSES[0].to_owned(),
        format!("0x0e \"e\" ⇧⌘ → ⇧⌘E → pending: waits for the second stroke · {LONG}"),
    ];
    panel.show(scene.host(), &lines);
    scene.keep(panel);
}

const LONG: &str = "and a note far longer than the panel is wide, so its end is cut off";

fn fullscreen(scene: &mut Scene) {
    let notice = FullscreenNotice::new(scene.mtm());
    notice.show(
        scene.host(),
        &fixtures::url("https://www.youtube.com/watch?v=abc"),
    );
    scene.check(notice.text().is_some(), "the notice shows");
    scene.keep(notice);
}
