//! The palette group: the command palette and the address overlay.

use std::cell::RefCell;
use std::rc::Rc;

use quickie_application::{PaletteItem, Target};
use quickie_ui_shared::stories::{Key, LIGHT, LIGHT_DARK, Scene, Story, Theme, WINDOW};
use quickie_ui_widgets::{AddressOverlay, CommandPalette};

use crate::fixtures::{LONG_TITLE, TABS, World};

const fn palette(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "palette",
        widget: "command-palette",
        state,
        themes,
        window: WINDOW,
        scene,
    }
}

pub const STORIES: &[Story] = &[
    palette("go-to", LIGHT_DARK, go_to),
    palette("commands", &Theme::ALL, commands),
    palette("highlights", LIGHT_DARK, highlights),
    palette("empty", LIGHT_DARK, empty),
    palette("cyrillic", LIGHT, cyrillic),
    palette("long-titles", LIGHT, long_titles),
    palette("run-command", LIGHT, run_command),
    Story {
        window: (560.0, 480.0),
        ..palette("narrow", LIGHT, narrow)
    },
    Story {
        group: "palette",
        widget: "address-overlay",
        state: "current-url",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: address_current,
    },
    Story {
        group: "palette",
        widget: "address-overlay",
        state: "typed",
        themes: LIGHT,
        window: WINDOW,
        scene: address_typed,
    },
];

/// The palette as the browser drives it: every edit asks the palette use case, `↑` `↓` move,
/// `⏎` runs the selected row's target, which the story records.
struct Host {
    palette: CommandPalette,
    world: World,
    results: RefCell<Vec<PaletteItem>>,
    ran: RefCell<Option<Target>>,
}

impl Host {
    fn open(scene: &mut Scene, world: World, text: &str) -> Rc<Self> {
        let host = Rc::new(Self {
            palette: CommandPalette::new(scene.mtm()),
            world,
            results: RefCell::new(Vec::new()),
            ran: RefCell::new(None),
        });
        let weak = Rc::downgrade(&host);
        host.palette.on_change(move || {
            if let Some(host) = weak.upgrade() {
                host.requery();
            }
        });
        host.palette.show(scene.host(), scene.window(), text);
        host.requery();
        let weak = Rc::downgrade(&host);
        scene.route(move |stroke| {
            let Some(host) = weak.upgrade() else {
                return false;
            };
            match stroke.key {
                Key::Down => host.palette.move_selection(1),
                Key::Up => host.palette.move_selection(-1),
                Key::Return => host.run(),
                _ => return false,
            }
            true
        });
        scene.keep(Rc::clone(&host));
        host
    }

    fn requery(&self) {
        let items = self.world.answer(&self.palette.text());
        self.palette.set_items(&items);
        *self.results.borrow_mut() = items;
    }

    fn run(&self) {
        let target = self.palette.selected().and_then(|row| {
            self.results
                .borrow()
                .get(row)
                .map(|item| item.target.clone())
        });
        if matches!(target, Some(Target::Command(_))) {
            self.palette.hide();
        }
        *self.ran.borrow_mut() = target;
    }

    fn titles(&self) -> Vec<String> {
        self.results
            .borrow()
            .iter()
            .map(|item| item.title.clone())
            .collect()
    }
}

fn go_to(scene: &mut Scene) {
    let host = Host::open(scene, World::new(&TABS), "");
    scene.check(
        host.titles().len() >= TABS.len(),
        "every open tab is offered",
    );
}

fn commands(scene: &mut Scene) {
    let host = Host::open(scene, World::new(&TABS), ">");
    scene.check(host.titles().len() > 8, "the catalog fills the list");
}

fn highlights(scene: &mut Scene) {
    let host = Host::open(scene, World::new(&TABS), ">");
    scene.type_text("tab");
    scene.check_eq(host.palette.text(), ">tab".to_owned(), "the typed query");
    let first = host.results.borrow().first().cloned();
    scene.check(
        first.is_some_and(|item| !item.title_matches.is_empty()),
        "the best row highlights what matched",
    );
}

fn empty(scene: &mut Scene) {
    let host = Host::open(scene, World::new(&TABS), ">");
    scene.type_text("qqqqzz");
    scene.check_eq(host.titles().len(), 0, "nothing matches");
}

fn cyrillic(scene: &mut Scene) {
    let host = Host::open(scene, World::new(&TABS), "");
    scene.type_text("сувер");
    scene.check_eq(
        host.titles().first().cloned(),
        Some("Суверенитет — Википедия".to_owned()),
        "a Russian title is found by Russian typing",
    );
}

fn long_titles(scene: &mut Scene) {
    let tabs = [
        (
            "https://example.com/a-very/long/path/that/goes/on/and/on/forever",
            LONG_TITLE,
        ),
        TABS[0],
        TABS[2],
    ];
    Host::open(scene, World::new(&tabs), "");
}

/// Typing narrows to the reload commands, `↓` picks the second, `⏎` runs it and closes.
fn run_command(scene: &mut Scene) {
    let host = Host::open(scene, World::new(&TABS), ">");
    scene.type_text("rel");
    let wanted = host.results.borrow().get(1).map(|item| item.target.clone());
    scene.check(wanted.is_some(), "\">rel\" offers at least two commands");
    scene.press(Key::Down);
    scene.check_eq(host.palette.selected(), Some(1), "↓ selects the second row");
    scene.press(Key::Return);
    scene.check_eq(
        host.ran.borrow().clone(),
        wanted,
        "⏎ runs the selected command",
    );
    scene.check(
        !host.palette.is_open(),
        "running a command closes the palette",
    );
    if let Some(Target::Command(id)) = host.ran.borrow().clone() {
        scene.check_eq(id.as_str(), "quickie.tab.hardReload", "the command run");
    }
    // Rendered open again on the answer, for the picture.
    host.palette.show(scene.host(), scene.window(), ">rel");
    host.requery();
    host.palette.move_selection(1);
}

fn narrow(scene: &mut Scene) {
    Host::open(scene, World::new(&TABS), ">");
}

fn address(scene: &mut Scene, text: &str) -> Rc<AddressOverlay> {
    let overlay = Rc::new(AddressOverlay::new(scene.mtm()));
    overlay.show(scene.host(), scene.window(), text);
    scene.keep(Rc::clone(&overlay));
    overlay
}

fn address_current(scene: &mut Scene) {
    let overlay = address(
        scene,
        "https://developer.apple.com/documentation/appkit/nswindow",
    );
    scene.check(overlay.is_open(), "the overlay is open");
}

fn address_typed(scene: &mut Scene) {
    let overlay = address(scene, "https://developer.apple.com/");
    scene.type_text("rust-lang.org/learn");
    scene.check_eq(
        overlay.text(),
        "rust-lang.org/learn".to_owned(),
        "typing replaces the selected address",
    );
}
