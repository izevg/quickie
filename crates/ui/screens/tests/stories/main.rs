//! The Settings window's stories (`docs/quality.md`, UI tests): each pane as the browser fills
//! it. The selected pane's view is moved onto the story's page, on the window background, to
//! render; the toolbar of panes belongs to the window's frame and is not drawn.

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;

use objc2_app_kit::{NSColor, NSTabViewController, NSView};
use objc2_foundation::{NSPoint, NSRect};
use quickie_domain_shields::FilterListId;
use quickie_ui_screens::{
    GeneralState, KeysState, ListState, Pane, SettingsAction, SettingsScreen, ShieldsState,
};
use quickie_ui_shared::stories::{self, LIGHT, LIGHT_DARK, Scene, Story, Theme};
use quickie_ui_widgets::{AgentSettings, SettingsSummary, Tone};

const fn pane(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "settings",
        widget: "settings",
        state,
        themes,
        window: (720.0, 760.0),
        scene,
    }
}

const STORIES: &[Story] = &[
    pane("general", LIGHT_DARK, general),
    pane("general-default", LIGHT, general_default),
    pane("shields", LIGHT_DARK, shields),
    pane("shields-fetching", LIGHT, shields_fetching),
    pane("agent", LIGHT_DARK, agent),
    pane("agent-problem", LIGHT, agent_problem),
    pane("keys", LIGHT, keys),
    pane("keys-problems", LIGHT_DARK, keys_problems),
];

fn main() -> ExitCode {
    if stories::run(STORIES, env!("CARGO_MANIFEST_DIR")) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The screen with its agent pane, filled as the browser fills it, showing `shown`.
struct Settings {
    screen: SettingsScreen,
    agent: AgentSettings,
    actions: Rc<RefCell<Vec<SettingsAction>>>,
}

fn open(scene: &mut Scene, shown: Pane) -> Rc<Settings> {
    let agent = AgentSettings::new(scene.mtm());
    agent.load(&SettingsSummary {
        base: "http://127.0.0.1:1234/v1".to_owned(),
        model: "qwen3-8b".to_owned(),
        has_key: false,
    });
    let screen = SettingsScreen::new(agent.view(), scene.mtm());
    let actions = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&actions);
    screen.on_action(move |action| log.borrow_mut().push(action));
    screen.set_general(&GeneralState {
        default_browser: "Safari is your default browser.".to_owned(),
        can_make_default: true,
        downloads_folder: "/Users/ada/Downloads".to_owned(),
    });
    screen.set_shields(&shields_state(""));
    screen.set_keys(&KeysState {
        file: "/Users/ada/Library/Application Support/Quickie/keybindings.json".to_owned(),
        problems: Vec::new(),
    });
    screen.fit_agent();
    let shown_screen = Rc::new(Settings {
        screen,
        agent,
        actions,
    });
    select(scene, &shown_screen, shown);
    scene.keep(Rc::clone(&shown_screen));
    shown_screen
}

/// Selects `pane` and moves the window's content, sized to the pane, onto the story's page.
fn select(scene: &mut Scene, screen: &Settings, pane: Pane) {
    let window = screen.screen.window();
    let Some(tabs) = window
        .contentViewController()
        .and_then(|controller| controller.downcast::<NSTabViewController>().ok())
    else {
        scene.check(false, "the window holds the panes' tab view controller");
        return;
    };
    let index = [Pane::General, Pane::Shields, Pane::Agent, Pane::Keys]
        .iter()
        .position(|&each| each == pane)
        .unwrap_or_default();
    tabs.setSelectedTabViewItemIndex(isize::try_from(index).unwrap_or_default());
    let children = tabs.childViewControllers();
    if index >= children.count() {
        scene.check(false, "every pane has a view controller");
        return;
    }
    let controller = children.objectAtIndex(index);
    let size = controller.preferredContentSize();
    let view = controller.view();
    view.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
    // The pane on the window's background, as the window draws it.
    let backdrop = NSView::new(scene.mtm());
    backdrop.setWantsLayer(true);
    if let Some(layer) = backdrop.layer() {
        layer.setBackgroundColor(Some(&NSColor::windowBackgroundColor().CGColor()));
    }
    let host = scene.host().bounds().size;
    backdrop.setFrame(NSRect::new(
        NSPoint::new(
            ((host.width - size.width) / 2.0).round(),
            (host.height - size.height - 32.0).round(),
        ),
        size,
    ));
    backdrop.addSubview(&view);
    for old in scene.host().subviews() {
        old.removeFromSuperview();
    }
    scene.host().addSubview(&backdrop);
}

fn shields_state(status: &str) -> ShieldsState {
    ShieldsState {
        lists: FilterListId::ALL
            .iter()
            .enumerate()
            .map(|(index, &id)| ListState {
                id,
                enabled: index < 2,
                fetched_ago_ms: (index < 2).then_some(3_600_000 * (index as u64 * 20 + 2)),
            })
            .collect(),
        keep_fresh: true,
        site_keys: "⌥⌘S".to_owned(),
        status: status.to_owned(),
    }
}

/// The default browser button asks the browser.
fn general(scene: &mut Scene) {
    let screen = open(scene, Pane::General);
    match scene.view_labelled("Make Quickie the Default Browser") {
        Some(button) => scene.click(&button),
        None => scene.check(false, "the default browser button is labelled"),
    }
    scene.check_eq(
        screen.actions.borrow().clone(),
        vec![SettingsAction::MakeDefaultBrowser],
        "the button asks to become the default",
    );
}

fn general_default(scene: &mut Scene) {
    let screen = open(scene, Pane::General);
    screen.screen.set_general(&GeneralState {
        default_browser: "Quickie is your default browser.".to_owned(),
        can_make_default: false,
        downloads_folder: "/Users/ada/Library/Mobile Documents/com~apple~CloudDocs/Загрузки"
            .to_owned(),
    });
    select(scene, &screen, Pane::General);
}

fn shields(scene: &mut Scene) {
    open(scene, Pane::Shields);
}

fn shields_fetching(scene: &mut Scene) {
    let screen = open(scene, Pane::Shields);
    screen
        .screen
        .set_shields(&shields_state("Fetching EasyList and EasyPrivacy…"));
    select(scene, &screen, Pane::Shields);
}

fn agent(scene: &mut Scene) {
    open(scene, Pane::Agent);
}

fn agent_problem(scene: &mut Scene) {
    let screen = open(scene, Pane::Agent);
    screen
        .agent
        .set_models(&["qwen3-8b".to_owned(), "llama-3.2-3b".to_owned()]);
    screen.agent.set_status(
        "The endpoint answered 401: Incorrect API key provided. Check the key and try again.",
        Tone::Problem,
    );
    screen.screen.fit_agent();
    select(scene, &screen, Pane::Agent);
}

fn keys(scene: &mut Scene) {
    open(scene, Pane::Keys);
}

fn keys_problems(scene: &mut Scene) {
    let screen = open(scene, Pane::Keys);
    screen.screen.set_keys(&KeysState {
        file: "/Users/ada/Library/Application Support/Quickie/keybindings.json".to_owned(),
        problems: vec![
            "entry 3 (\"cmd+shift+k\" → quickie.tab.close): the key is bound twice".to_owned(),
            "entry 7 (\"ctrl+ж\" → quickie.palette.open): not a key on the ASCII layout".to_owned(),
        ],
    });
    select(scene, &screen, Pane::Keys);
}
