//! The tabs group: the `⌃⇥` switcher and the tab overview.

use std::cell::RefCell;
use std::rc::Rc;

use quickie_domain_browsing::{Direction, TabCycle};
use quickie_ui_shared::Trigger;
use quickie_ui_shared::stories::{
    Key, LIGHT, LIGHT_DARK, Modifiers, Scene, Story, Theme, WINDOW, page_picture,
};
use quickie_ui_widgets::{Move, OverviewTab, TabOverview, TabSwitcher};

use crate::fixtures::{self, LONG_TITLE, TABS, World};

const fn switcher(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "tabs",
        widget: "tab-switcher",
        state,
        themes,
        window: WINDOW,
        scene,
    }
}

const fn overview(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "tabs",
        widget: "tab-overview",
        state,
        themes,
        window: (1024.0, 720.0),
        scene,
    }
}

pub const STORIES: &[Story] = &[
    switcher("titles", LIGHT_DARK, switcher_titles),
    switcher("long-titles", LIGHT, switcher_long),
    switcher("many", LIGHT, switcher_many),
    switcher("cycle", LIGHT, switcher_cycle),
    overview("thumbnails", LIGHT_DARK, overview_thumbnails),
    overview("no-pictures", LIGHT, overview_no_pictures),
    overview("filtered", LIGHT, overview_filtered),
    overview("no-match", LIGHT, overview_no_match),
];

fn switcher_show(scene: &mut Scene, titles: Vec<String>, selected: usize) -> Rc<TabSwitcher> {
    let switcher = Rc::new(TabSwitcher::new(scene.mtm()));
    switcher.show(scene.host(), titles, selected);
    scene.keep(Rc::clone(&switcher));
    switcher
}

fn switcher_titles(scene: &mut Scene) {
    let titles = World::new(&TABS).titles();
    let switcher = switcher_show(scene, titles, 1);
    scene.check(switcher.is_open(), "the switcher is open");
}

fn switcher_long(scene: &mut Scene) {
    let titles = vec![
        LONG_TITLE.to_owned(),
        TABS[2].1.to_owned(),
        String::new(),
        TABS[4].1.to_owned(),
    ];
    switcher_show(scene, titles, 0);
}

fn switcher_many(scene: &mut Scene) {
    let titles = (1..=24).map(|n| format!("Tab number {n}")).collect();
    switcher_show(scene, titles, 17);
}

/// `⌃⇥` three times, then `⌃⇧⇥` once, over six tabs: the cycle walks the most-recently-used
/// order and wraps; the switcher shows its selection.
fn switcher_cycle(scene: &mut Scene) {
    let world = World::new(&TABS);
    let Some(mru) = world.session.mru(world.window).map(<[_]>::to_vec) else {
        scene.check(false, "the window has an MRU order");
        return;
    };
    let Some(cycle) = TabCycle::begin(&mru, Direction::Forward) else {
        scene.check(false, "a cycle begins over six tabs");
        return;
    };
    let titles: Vec<String> = mru
        .iter()
        .filter_map(|&tab| world.session.tab(tab).map(|tab| tab.title.clone()))
        .collect();
    let switcher = switcher_show(scene, titles, 1);
    let cycle = Rc::new(RefCell::new(cycle));
    let shown = Rc::clone(&switcher);
    let state = Rc::clone(&cycle);
    scene.route(move |stroke| {
        if stroke.key != Key::Tab || !stroke.modifiers.control {
            return false;
        }
        let direction = if stroke.modifiers.shift {
            Direction::Backward
        } else {
            Direction::Forward
        };
        let mut cycle = state.borrow_mut();
        cycle.step(direction);
        let selected = cycle.selected();
        if let Some(index) = cycle.order().iter().position(|&tab| tab == selected) {
            shown.select(index);
        }
        true
    });
    let control = Modifiers::CONTROL;
    for _ in 0..3 {
        scene.press_with(Key::Tab, control);
    }
    scene.check_eq(
        cycle.borrow().selected(),
        mru[4],
        "⌃⇥ ×3 after the first step",
    );
    scene.press_with(
        Key::Tab,
        Modifiers {
            shift: true,
            ..control
        },
    );
    scene.check_eq(cycle.borrow().selected(), mru[3], "⌃⇧⇥ steps back");
    for _ in 0..3 {
        scene.press_with(Key::Tab, control);
    }
    scene.check_eq(cycle.borrow().selected(), mru[0], "⌃⇥ wraps to the front");
    scene.keep(cycle);
}

/// Six tabs with made-up pictures, two of them pinned to slots.
fn cards(pictures: bool) -> Vec<OverviewTab> {
    TABS.iter()
        .enumerate()
        .map(|(index, &(url, title))| {
            #[expect(clippy::cast_precision_loss, reason = "six tabs")]
            let hue = index as f64 / TABS.len() as f64;
            OverviewTab {
                title: title.to_owned(),
                host: fixtures::host(url),
                slot: u8::try_from(index + 1).ok().filter(|&slot| slot <= 2),
                thumbnail: if pictures {
                    page_picture(320, 200, hue)
                } else {
                    None
                },
            }
        })
        .collect()
}

fn overview_show(scene: &mut Scene, tabs: &[OverviewTab], selected: usize) -> Rc<TabOverview> {
    let overview = TabOverview::new(scene.mtm());
    overview.show(
        scene.host(),
        scene.window(),
        tabs,
        selected,
        Trigger::Keyboard,
    );
    let shown = Rc::clone(&overview);
    scene.route(move |stroke| {
        let by = match stroke.key {
            Key::Left => Move::Left,
            Key::Right => Move::Right,
            Key::Up => Move::Up,
            Key::Down => Move::Down,
            _ => return false,
        };
        shown.move_selection(by);
        true
    });
    scene.keep(Rc::clone(&overview));
    overview
}

/// Arrows move in two dimensions over the grid.
fn overview_thumbnails(scene: &mut Scene) {
    let overview = overview_show(scene, &cards(true), 0);
    scene.press(Key::Right);
    scene.press(Key::Down);
    scene.press(Key::Left);
    scene.check(
        overview.selected().is_some_and(|s| s > 1),
        "↓ moves a row down",
    );
}

fn overview_no_pictures(scene: &mut Scene) {
    overview_show(scene, &cards(false), 2);
}

/// Typing filters the cards by title or host.
fn overview_filtered(scene: &mut Scene) {
    let overview = overview_show(scene, &cards(true), 0);
    scene.type_text("rust");
    scene.check_eq(
        overview.selected(),
        Some(5),
        "\"rust\" keeps the Rust docs tab",
    );
}

fn overview_no_match(scene: &mut Scene) {
    let overview = overview_show(scene, &cards(true), 0);
    scene.type_text("qqqq");
    scene.check_eq(overview.selected(), None, "nothing is kept");
}
