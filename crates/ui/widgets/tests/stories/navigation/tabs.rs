//! The tabs group: the `⌃⇥` switcher and the tab overview.

use std::cell::RefCell;
use std::rc::Rc;

use quickie_domain_browsing::{Direction, TabCycle, TabId};
use quickie_ui_shared::Trigger;
use quickie_ui_shared::stories::{
    Key, Keystroke, LIGHT, LIGHT_DARK, Modifiers, Scene, Story, Theme, WINDOW, page_picture,
};
use quickie_ui_shared::tab_card::TabCard;
use quickie_ui_widgets::{Move, TabOverview, TabSwitcher};

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
    switcher("filtered", LIGHT_DARK, switcher_filtered),
    switcher("no-match", LIGHT, switcher_no_match),
    overview("thumbnails", LIGHT_DARK, overview_thumbnails),
    overview("no-pictures", LIGHT, overview_no_pictures),
    overview("filtered", LIGHT, overview_filtered),
    overview("no-match", LIGHT, overview_no_match),
];

/// A made-up picture for the tab at `index` of `count`.
fn picture(index: usize, count: usize) -> Option<objc2::rc::Retained<objc2_app_kit::NSImage>> {
    #[expect(clippy::cast_precision_loss, reason = "a handful of tabs")]
    let hue = index as f64 / count as f64;
    page_picture(320, 200, hue)
}

/// The window's tabs in most-recently-used order, as the switcher shows them, with pictures.
fn mru_cards(world: &World) -> (Vec<TabId>, Vec<TabCard>) {
    let mru = world
        .session
        .mru(world.window)
        .map_or_else(Vec::new, <[_]>::to_vec);
    let cards = mru
        .iter()
        .enumerate()
        .filter_map(|(index, &id)| {
            let tab = world.session.tab(id)?;
            Some(TabCard {
                title: tab.title.clone(),
                host: tab.url.host_str().unwrap_or_default().to_owned(),
                thumbnail: picture(index, mru.len()),
                ..TabCard::default()
            })
        })
        .collect();
    (mru, cards)
}

fn switcher_show(scene: &mut Scene, tabs: Vec<TabCard>, selected: usize) -> Rc<TabSwitcher> {
    let switcher = Rc::new(TabSwitcher::new(scene.mtm()));
    switcher.show(scene.host(), tabs, Some(selected));
    scene.keep(Rc::clone(&switcher));
    switcher
}

fn switcher_titles(scene: &mut Scene) {
    let (_, cards) = mru_cards(&World::new(&TABS));
    let switcher = switcher_show(scene, cards, 1);
    scene.check(switcher.is_open(), "the switcher is open");
}

/// A title that never fits, an empty one, and tabs without pictures: the host stands in.
fn switcher_long(scene: &mut Scene) {
    let tabs = [
        (LONG_TITLE, TABS[0].0),
        (TABS[2].1, TABS[2].0),
        ("", TABS[3].0),
        (TABS[4].1, TABS[4].0),
    ]
    .iter()
    .map(|&(title, url)| TabCard {
        title: title.to_owned(),
        host: fixtures::host(url),
        ..TabCard::default()
    })
    .collect();
    switcher_show(scene, tabs, 0);
}

/// More tabs than fit: the strip shows the ones around the selection.
fn switcher_many(scene: &mut Scene) {
    let tabs = (1..=24)
        .map(|n| TabCard {
            title: format!("Tab number {n}"),
            host: format!("site{n}.example"),
            thumbnail: picture(n, 24),
            ..TabCard::default()
        })
        .collect();
    switcher_show(scene, tabs, 17);
}

/// What the browser does with the keys while `⌃` is held, over one cycle: `⇥` and `⇧⇥` step,
/// letters filter, `⌫` takes a letter back.
fn hold(switcher: &Rc<TabSwitcher>, cycle: &Rc<RefCell<TabCycle>>, stroke: Keystroke) -> bool {
    if !stroke.modifiers.control {
        return false;
    }
    let mut cycle = cycle.borrow_mut();
    match stroke.key {
        Key::Tab => {
            let direction = if stroke.modifiers.shift {
                Direction::Backward
            } else {
                Direction::Forward
            };
            cycle.step(direction);
        }
        Key::Char(typed) => cycle.narrow(&switcher.type_text(&typed.to_string())),
        Key::Backspace => cycle.narrow(&switcher.erase()),
        _ => return false,
    }
    switcher.select(cycle.cursor());
    true
}

/// Shows the switcher over a cycle of the window's tabs and routes keys as the browser does.
fn switcher_cycle_show(scene: &mut Scene) -> Option<(Vec<TabId>, Rc<RefCell<TabCycle>>)> {
    let world = World::new(&TABS);
    let (mru, cards) = mru_cards(&world);
    let Some(cycle) = TabCycle::begin(&mru, Direction::Forward) else {
        scene.check(false, "a cycle begins over six tabs");
        return None;
    };
    let switcher = switcher_show(scene, cards, 1);
    let cycle = Rc::new(RefCell::new(cycle));
    let state = Rc::clone(&cycle);
    scene.route(move |stroke| hold(&switcher, &state, stroke));
    scene.keep(Rc::clone(&cycle));
    Some((mru, cycle))
}

const CONTROL: Modifiers = Modifiers::CONTROL;

/// `⌃⇥` three times, then `⌃⇧⇥` once, over six tabs: the cycle walks the most-recently-used
/// order and wraps; the switcher shows its selection.
fn switcher_cycle(scene: &mut Scene) {
    let Some((mru, cycle)) = switcher_cycle_show(scene) else {
        return;
    };
    for _ in 0..3 {
        scene.press_with(Key::Tab, CONTROL);
    }
    scene.check_eq(
        cycle.borrow().selected(),
        Some(mru[4]),
        "⌃⇥ ×3 after the first step",
    );
    scene.press_with(
        Key::Tab,
        Modifiers {
            shift: true,
            ..CONTROL
        },
    );
    scene.check_eq(cycle.borrow().selected(), Some(mru[3]), "⌃⇧⇥ steps back");
    for _ in 0..3 {
        scene.press_with(Key::Tab, CONTROL);
    }
    scene.check_eq(
        cycle.borrow().selected(),
        Some(mru[0]),
        "⌃⇥ wraps to the front",
    );
}

/// `⌃` held, "docs" typed and one letter taken back: "doc" keeps the Rust docs and the Apple
/// documentation, and `⌃⇥` steps between those two only.
fn switcher_filtered(scene: &mut Scene) {
    let Some((mru, cycle)) = switcher_cycle_show(scene) else {
        return;
    };
    for typed in "docs".chars() {
        scene.press_with(Key::Char(typed), CONTROL);
    }
    scene.check_eq(cycle.borrow().selected(), None, "\"docs\" keeps nothing");
    scene.press_with(Key::Backspace, CONTROL);
    scene.check_eq(
        cycle.borrow().shown().to_vec(),
        vec![0, 5],
        "\"doc\" keeps two",
    );
    scene.check_eq(
        cycle.borrow().selected(),
        Some(mru[5]),
        "the kept tab behind the active one is selected",
    );
    scene.press_with(Key::Tab, CONTROL);
    scene.check_eq(
        cycle.borrow().selected(),
        Some(mru[0]),
        "⌃⇥ steps to the other kept tab",
    );
    scene.press_with(Key::Tab, CONTROL);
    scene.check_eq(
        cycle.borrow().selected(),
        Some(mru[5]),
        "and wraps among the kept tabs",
    );
}

/// Typing what no tab has: no cards, and nothing to switch to.
fn switcher_no_match(scene: &mut Scene) {
    let Some((_, cycle)) = switcher_cycle_show(scene) else {
        return;
    };
    for typed in "qqq".chars() {
        scene.press_with(Key::Char(typed), CONTROL);
    }
    scene.check_eq(cycle.borrow().selected(), None, "nothing is kept");
}

/// Six tabs with made-up pictures, two of them pinned to slots and two with favicons.
fn cards(pictures: bool) -> Vec<TabCard> {
    TABS.iter()
        .enumerate()
        .map(|(index, &(url, title))| TabCard {
            title: title.to_owned(),
            host: fixtures::host(url),
            slot: u8::try_from(index + 1).ok().filter(|&slot| slot <= 2),
            thumbnail: if pictures {
                picture(index, TABS.len())
            } else {
                None
            },
            favicon: if index % 3 == 0 {
                page_picture(16, 16, 0.6)
            } else {
                None
            },
        })
        .collect()
}

fn overview_show(scene: &mut Scene, tabs: Vec<TabCard>, selected: usize) -> Rc<TabOverview> {
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

/// Arrows move in two dimensions over the grid; the selected card's close button closes it.
fn overview_thumbnails(scene: &mut Scene) {
    let overview = overview_show(scene, cards(true), 0);
    let closed = Rc::new(RefCell::new(None));
    let seen = Rc::clone(&closed);
    overview.on_close(move |index| *seen.borrow_mut() = Some(index));
    scene.press(Key::Right);
    scene.press(Key::Down);
    scene.press(Key::Left);
    scene.check(
        overview.selected().is_some_and(|s| s > 1),
        "↓ moves a row down",
    );
    scene.check(
        scene.view_labelled("Close tab").is_some(),
        "the selected card has a close button",
    );
    scene.keep(closed);
}

fn overview_no_pictures(scene: &mut Scene) {
    overview_show(scene, cards(false), 2);
}

/// Typing filters the cards by title or host.
fn overview_filtered(scene: &mut Scene) {
    let overview = overview_show(scene, cards(true), 0);
    scene.type_text("rust");
    scene.check_eq(
        overview.selected(),
        Some(5),
        "\"rust\" keeps the Rust docs tab",
    );
}

fn overview_no_match(scene: &mut Scene) {
    let overview = overview_show(scene, cards(true), 0);
    scene.type_text("qqqq");
    scene.check_eq(overview.selected(), None, "nothing is kept");
}
