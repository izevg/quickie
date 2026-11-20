//! The Settings window's stories (`docs/quality.md`, UI tests): every pane as the browser fills
//! it, reached the way a user reaches it, with `↓` in the sidebar. The window's content (the
//! sidebar and the pane, without the titlebar) is moved onto the story's page to render; the
//! window itself is never ordered in. The sidebar's material does not draw offscreen, so the
//! sidebar shows the page behind it.
//!
//! - [`keyboard`]: the keymap editor: the table, a refused user entry, and recording keys.
//! - [`extensions`]: the installed extensions, none, and a macOS without them.

mod extensions;
mod keyboard;

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2_app_kit::{NSAccessibility, NSColor, NSTableView, NSView};
use objc2_foundation::{NSPoint, NSRect};
use quickie_application::{BindingLine, keymap_lines};
use quickie_domain_commands::{Catalog, Keymap, Platform};
use quickie_domain_shields::FilterListId;
use quickie_ui_screens::{
    GeneralState, KeysState, ListState, Pane, Recorded, SettingsAction, SettingsScreen,
    ShieldsState,
};
use quickie_ui_shared::stories::{self, Key, LIGHT, LIGHT_DARK, Scene, Story, Theme};
use quickie_ui_widgets::{AgentSettings, SettingsSummary, Tone};

/// One story of a Settings pane; each pane is its own widget so its states stay within the rule
/// of four.
const fn settings(
    pane: &'static str,
    state: &'static str,
    themes: &'static [Theme],
    scene: fn(&mut Scene),
) -> Story {
    Story {
        group: "settings",
        widget: pane,
        state,
        themes,
        window: (900.0, 640.0),
        scene,
    }
}

const STORIES: &[Story] = &[
    settings("general", "general", LIGHT_DARK, general),
    settings("general", "general-default", LIGHT, general_default),
    settings("general", "general-inset", LIGHT, general_inset),
    settings("shields", "shields", LIGHT_DARK, shields),
    settings("shields", "shields-fetching", LIGHT, shields_fetching),
    settings("shields", "shields-no-lists", LIGHT_DARK, shields_no_lists),
    settings(
        "extensions",
        "extensions",
        LIGHT_DARK,
        extensions::installed,
    ),
    settings("extensions", "extensions-empty", LIGHT, extensions::empty),
    settings(
        "extensions",
        "extensions-old-macos",
        LIGHT,
        extensions::unavailable,
    ),
    settings("keyboard", "keyboard", LIGHT_DARK, keyboard::table_full),
    settings("keyboard", "keyboard-refused", LIGHT_DARK, keyboard::table),
    settings("keyboard", "keyboard-problems", LIGHT, keyboard::problems),
    settings("keyboard", "keyboard-recording", LIGHT, keyboard::recording),
    settings("keyboard", "keyboard-recorded", LIGHT, keyboard::recorded),
    settings(
        "keyboard",
        "keyboard-conflict",
        LIGHT_DARK,
        keyboard::conflict,
    ),
    settings("keyboard", "keyboard-typing", LIGHT, keyboard::typing),
    settings("agent", "agent", LIGHT_DARK, agent),
    settings("agent", "agent-problem", LIGHT, agent_problem),
    settings("agent", "agent-search", LIGHT, agent_search),
];

/// The sidebar's search keeps the panes that set what is typed, and `⏎` shows the first:
/// "ollama" finds the Agent pane. `VoiceOver` reads the sidebar's rows by their panes.
fn agent_search(scene: &mut Scene) {
    let settings = open(scene, None);
    scene.check(
        row_labels(scene.host()).contains(&"Privacy & Shields".to_owned()),
        "VoiceOver reads a sidebar row by its pane",
    );
    settings.screen.search_panes();
    scene.type_text("ollama");
    scene.press(Key::Return);
    scene.check(
        scene.accessibility().contains("value=\"Agent\""),
        "⏎ in the search shows the first pane it kept",
    );
    scene.check_eq(
        row_labels(scene.host()),
        vec!["Agent".to_owned()],
        "the search keeps only the panes that match",
    );
}

/// What `VoiceOver` reads for each row of the tables on the page, the sidebar's first. Rows
/// are `AppKit`'s proxies in the accessibility tree; their views carry the labels, made here
/// since a window that is never shown has not made them yet.
pub fn row_labels(view: &NSView) -> Vec<String> {
    tables(view)
        .iter()
        .flat_map(|table| {
            (0..table.numberOfRows()).filter_map(|row| {
                table
                    .rowViewAtRow_makeIfNecessary(row, true)
                    .and_then(|row| row.accessibilityLabel())
                    .map(|label| label.to_string())
            })
        })
        .collect()
}

fn main() -> ExitCode {
    if stories::run(STORIES, env!("CARGO_MANIFEST_DIR")) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The screen as the browser holds it: the window, its agent form, the keymap it shows, and
/// what the window asked. A recorded key is answered at once, as the browser answers it.
pub struct Settings {
    pub screen: SettingsScreen,
    pub agent: AgentSettings,
    pub keymap: Keymap,
    pub actions: RefCell<Vec<SettingsAction>>,
    /// The browser's last answer to a recorded press.
    pub recorded: RefCell<Option<Recorded>>,
}

/// The window filled as the browser fills it, its content on the story's page, the sidebar
/// with the keyboard on General.
pub fn open(scene: &mut Scene, lines: Option<Vec<BindingLine>>) -> Rc<Settings> {
    let agent = AgentSettings::new(scene.mtm());
    agent.load(&SettingsSummary {
        base: "http://127.0.0.1:1234/v1".to_owned(),
        model: "qwen3-8b".to_owned(),
        has_key: false,
    });
    let screen = SettingsScreen::new(&agent, scene.mtm());
    let keymap = Keymap::defaults(Platform::MacOs);
    screen.set_general(&GeneralState {
        default_browser: "Safari is your default browser.".to_owned(),
        can_make_default: true,
        downloads_folder: "/Users/ada/Downloads".to_owned(),
        inset_page: false,
    });
    screen.set_shields(&shields_state(""));
    screen.set_keys(&KeysState {
        file: "/Users/ada/Library/Application Support/Quickie/keybindings.json".to_owned(),
        lines: lines.unwrap_or_else(|| keymap_lines(&keymap, &Catalog::builtin(), &[])),
        problems: Vec::new(),
    });
    let settings = Rc::new(Settings {
        screen,
        agent,
        keymap,
        actions: RefCell::new(Vec::new()),
        recorded: RefCell::new(None),
    });
    let weak: Weak<Settings> = Rc::downgrade(&settings);
    settings.screen.on_action(move |action| {
        if let Some(settings) = weak.upgrade() {
            keyboard::answer(&settings, &action);
            settings.actions.borrow_mut().push(action);
        }
    });
    let window = settings.screen.window();
    if let Some(root) = window.contentView() {
        let size = root.frame().size;
        window.setContentView(Some(&NSView::new(scene.mtm())));
        // The window's background, which the window draws behind its content.
        let backdrop = NSView::new(scene.mtm());
        backdrop.setWantsLayer(true);
        if let Some(layer) = backdrop.layer() {
            layer.setBackgroundColor(Some(&NSColor::windowBackgroundColor().CGColor()));
        }
        let page = scene.host().bounds().size;
        backdrop.setFrame(NSRect::new(
            NSPoint::new(
                ((page.width - size.width) / 2.0).round(),
                ((page.height - size.height) / 2.0).round(),
            ),
            size,
        ));
        root.setFrameOrigin(NSPoint::new(0.0, 0.0));
        backdrop.addSubview(&root);
        scene.host().addSubview(&backdrop);
    }
    if let Some(sidebar) = tables(scene.host()).into_iter().next() {
        scene.window().makeFirstResponder(Some(&sidebar));
    }
    scene.keep(Rc::clone(&settings));
    settings
}

/// Goes to `pane` with `↓` in the sidebar, from General, and checks the pane is up.
pub fn go(scene: &mut Scene, pane: Pane, title: &str) {
    let steps = match pane {
        Pane::General => 0,
        Pane::Shields => 1,
        Pane::Extensions => 2,
        Pane::Keys => 3,
        Pane::Agent => 4,
    };
    for _ in 0..steps {
        scene.press(Key::Down);
    }
    // The sidebar's rows are not in the accessibility tree; the pane's title is.
    let named = scene.accessibility().contains(&format!("value={title:?}"));
    scene.check(named, format!("↓ ×{steps} in the sidebar shows {title}"));
}

/// The table views on the page, depth first: the sidebar's, then the keymap's.
pub fn tables(view: &NSView) -> Vec<Retained<NSTableView>> {
    let mut found = Vec::new();
    for child in view.subviews() {
        if let Ok(table) = child.clone().downcast::<NSTableView>() {
            found.push(table);
        }
        found.extend(tables(&child));
    }
    found
}

fn shields_state(status: &str) -> ShieldsState {
    ShieldsState {
        lists: FilterListId::ALL
            .iter()
            .zip(0_u64..)
            .map(|(&id, index)| ListState {
                id,
                enabled: index < 2,
                fetched_ago_ms: (index < 2).then_some(3_600_000 * (index * 20 + 2)),
            })
            .collect(),
        keep_fresh: true,
        site_keys: "⌥⌘S".to_owned(),
        status: status.to_owned(),
        rules: "Rules active: 81,203 rules in 1 list, compiled 2 h ago".to_owned(),
    }
}

/// The default browser button asks the browser.
fn general(scene: &mut Scene) {
    let settings = open(scene, None);
    go(scene, Pane::General, "General");
    match scene.view_labelled("Make Default") {
        Some(button) => scene.click(&button),
        None => scene.check(false, "the default browser button is labelled"),
    }
    scene.check_eq(
        settings.actions.borrow().clone(),
        vec![SettingsAction::MakeDefaultBrowser],
        "the button asks to become the default",
    );
}

/// The Inset page switch asks the browser to put the page in a card.
fn general_inset(scene: &mut Scene) {
    let settings = open(scene, None);
    go(scene, Pane::General, "General");
    match scene.view_labelled("Inset page") {
        Some(switch) => scene.click(&switch),
        None => scene.check(false, "the Inset page switch is labelled"),
    }
    scene.check_eq(
        settings.actions.borrow().clone(),
        vec![SettingsAction::InsetPage(true)],
        "the switch asks for the inset page",
    );
}

fn general_default(scene: &mut Scene) {
    let settings = open(scene, None);
    settings.screen.set_general(&GeneralState {
        default_browser: "Quickie is your default browser.".to_owned(),
        can_make_default: false,
        downloads_folder: "/Users/ada/Library/Mobile Documents/com~apple~CloudDocs/Загрузки"
            .to_owned(),
        inset_page: true,
    });
    go(scene, Pane::General, "General");
}

fn shields(scene: &mut Scene) {
    open(scene, None);
    go(scene, Pane::Shields, "Privacy & Shields");
}

/// No list on: the card says so, and its one button asks to turn them on.
fn shields_no_lists(scene: &mut Scene) {
    let settings = open(scene, None);
    let mut state = shields_state("");
    for list in &mut state.lists {
        list.enabled = false;
    }
    state.rules = String::new();
    settings.screen.set_shields(&state);
    go(scene, Pane::Shields, "Privacy & Shields");
    match scene.view_labelled("Enable") {
        Some(button) => scene.click(&button),
        None => scene.check(false, "the Enable button is labelled"),
    }
    scene.check_eq(
        settings.actions.borrow().clone(),
        vec![SettingsAction::EnableLists],
        "Enable asks to turn the lists on",
    );
}

fn shields_fetching(scene: &mut Scene) {
    let settings = open(scene, None);
    settings
        .screen
        .set_shields(&shields_state("Fetching EasyList and EasyPrivacy…"));
    go(scene, Pane::Shields, "Privacy & Shields");
}

fn agent(scene: &mut Scene) {
    open(scene, None);
    go(scene, Pane::Agent, "Agent");
}

fn agent_problem(scene: &mut Scene) {
    let settings = open(scene, None);
    settings
        .agent
        .set_models(&["qwen3-8b".to_owned(), "llama-3.2-3b".to_owned()]);
    settings.agent.set_status(
        "The endpoint answered 401: Incorrect API key provided. Check the key and try again.",
        Tone::Problem,
    );
    go(scene, Pane::Agent, "Agent");
}
