//! The Keyboard pane: the keymap table, a refused user entry, the footer's problems, and
//! recording new keys for a command. The browser's part (reading a press as the keymap does,
//! checking it, answering with what was recorded) is [`answer`].

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use quickie_application::{BindingLine, BindingSource, Rejection, check_binding, keymap_lines};
use quickie_domain_commands::{Catalog, Chord, CommandId, Keymap, KeymapError, Platform};
use quickie_ui_screens::{KeysState, Pane, Recorded, SettingsAction};
use quickie_ui_shared::stories::{Key, Modifiers, Scene};

use crate::{Settings, go, open, tables};

/// What the browser does with a recorded press: the story knows the keys it pressed, so it
/// reads the press by its key code, checks the chord against the keymap as the browser does,
/// and answers the window.
pub fn answer(settings: &Settings, action: &SettingsAction) {
    let SettingsAction::Record(command, press) = action else {
        return;
    };
    let text = match press.code {
        0x0E => "shift+cmd+e",
        0x11 => "cmd+t",
        _ => return,
    };
    let Ok(chord) = Chord::parse(text, Platform::MacOs) else {
        return;
    };
    let keys = chord.glyphs(Platform::MacOs);
    let catalog = Catalog::builtin();
    let problem = check_binding(&settings.keymap, &catalog, command, chord)
        .err()
        .map(|refused| refusal(&keys, &refused, &catalog));
    let recorded = Recorded {
        chord: Some(chord),
        keys,
        problem,
    };
    settings.screen.show_recorded(command, recorded.clone());
    *settings.recorded.borrow_mut() = Some(recorded);
}

/// The browser's words for a refused chord.
fn refusal(keys: &str, refused: &Rejection, catalog: &Catalog) -> String {
    match refused {
        Rejection::Invalid(KeymapError::Conflict(conflict)) => {
            let other = catalog.get(&conflict.first).map_or_else(
                || conflict.first.to_string(),
                |other| other.title.to_owned(),
            );
            format!("{keys} already runs {other}")
        }
        other => other.to_string(),
    }
}

/// The defaults, and a user entry the keymap refused: `⌘T` for Bookmark Page, which Open New
/// Tab already has.
fn lines_with_conflict() -> Vec<BindingLine> {
    let keymap = Keymap::defaults(Platform::MacOs);
    let mut lines = keymap_lines(&keymap, &Catalog::builtin(), &[]);
    if let Some(bookmark) = lines.iter().position(|line| line.title == "Bookmark Page") {
        let refused = BindingLine {
            keys: Some("⌘T".to_owned()),
            when: None,
            source: BindingSource::User,
            conflict: Some("Open New Tab".to_owned()),
            ..lines[bookmark].clone()
        };
        lines.insert(bookmark + 1, refused);
    }
    lines
}

/// The table narrowed by its search to the bookmark commands: the default line, and the
/// refused user entry as a red row.
pub fn table(scene: &mut Scene) {
    let settings = open(scene, Some(lines_with_conflict()));
    go(scene, Pane::Keys, "Keyboard");
    scene.check(
        tables(scene.host()).len() == 2,
        "the sidebar and the keymap table are on the page",
    );
    settings.screen.search_keymap();
    scene.type_text("bookmark");
    // `↓` ends the search's editing, which applies it, and goes down to the table.
    scene.press(Key::Down);
}

/// The whole table, as the pane opens.
pub fn table_full(scene: &mut Scene) {
    open(scene, Some(lines_with_conflict()));
    go(scene, Pane::Keys, "Keyboard");
}

pub fn problems(scene: &mut Scene) {
    let settings = open(scene, None);
    settings.screen.set_keys(&KeysState {
        file: "/Users/ada/Library/Application Support/Quickie/keybindings.json".to_owned(),
        lines: keymap_lines(&settings.keymap, &Catalog::builtin(), &[]),
        problems: vec![
            "entry 3 (\"cmd+shift+k\" → quickie.tab.close): the key is bound twice".to_owned(),
            "entry 7 (\"ctrl+ж\" → quickie.palette.open): not a key on the ASCII layout".to_owned(),
        ],
    });
    go(scene, Pane::Keys, "Keyboard");
}

/// `⌘F` (the browser calls `search_keymap`), typing narrows the table, `↓` goes down to it,
/// and `⏎` records the selected command's keys. Returns that command.
fn start(scene: &mut Scene) -> (std::rc::Rc<Settings>, Option<CommandId>) {
    let settings = open(scene, None);
    go(scene, Pane::Keys, "Keyboard");
    settings.screen.search_keymap();
    scene.type_text("bookmark page");
    scene.press(Key::Down);
    let table = tables(scene.host()).into_iter().nth(1);
    let focused = table.as_ref().is_some_and(|table| {
        scene.window().firstResponder().is_some_and(|responder| {
            Retained::as_ptr(&responder).cast::<AnyObject>()
                == Retained::as_ptr(table).cast::<AnyObject>()
        })
    });
    scene.check(focused, "↓ in the search goes down to the table");
    scene.press(Key::Return);
    scene.check(
        scene.accessibility().contains("Type the new keys"),
        "⏎ records: the hint says how",
    );
    let command = CommandId::try_from("quickie.page.bookmark").ok();
    (settings, command)
}

pub fn recording(scene: &mut Scene) {
    start(scene);
}

/// `⇧⌘E` is free: the row shows it, and `⏎` asks to bind it.
pub fn recorded(scene: &mut Scene) {
    let (settings, _) = start(scene);
    scene.press_with(
        Key::Char('e'),
        Modifiers {
            shift: true,
            ..Modifiers::COMMAND
        },
    );
    let recorded = settings
        .actions
        .borrow()
        .iter()
        .find_map(|action| match action {
            SettingsAction::Record(command, _) => Some(command.clone()),
            _ => None,
        });
    scene.check(recorded.is_some(), "the press is handed to the browser");
    scene.check_eq(
        settings
            .recorded
            .borrow()
            .as_ref()
            .map(|r| r.problem.clone()),
        Some(None),
        "⇧⌘E is free",
    );
    scene.press(Key::Return);
    let rebind = settings
        .actions
        .borrow()
        .iter()
        .find_map(|action| match action {
            SettingsAction::Rebind(command, chord) => Some((command.clone(), *chord)),
            _ => None,
        });
    scene.check_eq(
        rebind.map(|(command, chord)| (command, chord.glyphs(Platform::MacOs))),
        recorded.map(|command| (command, "⇧⌘E".to_owned())),
        "⏎ asks to bind the recorded keys",
    );
    // Rendered recording again, with the keys shown.
    scene.press(Key::Return);
    scene.press_with(
        Key::Char('e'),
        Modifiers {
            shift: true,
            ..Modifiers::COMMAND
        },
    );
}

/// `⌘T` already runs New Tab: the row turns red and says so.
pub fn conflict(scene: &mut Scene) {
    let (settings, _) = start(scene);
    scene.press_with(Key::Char('t'), Modifiers::COMMAND);
    scene.check_eq(
        settings
            .recorded
            .borrow()
            .as_ref()
            .and_then(|recorded| recorded.problem.clone()),
        Some("⌘T already runs Open New Tab".to_owned()),
        "the browser refuses ⌘T and the row says why",
    );
}
