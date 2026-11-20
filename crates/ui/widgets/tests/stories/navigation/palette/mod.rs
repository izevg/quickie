//! The palette group: the command palette here, the address overlay in [`address`].

pub mod address;

use std::cell::RefCell;
use std::rc::Rc;

use objc2_app_kit::NSImage;
use objc2_foundation::NSString;
use quickie_application::{PaletteItem, Target};
use quickie_domain_browsing::{PinnedSlot, Timestamp};
use quickie_ui_shared::stories::{Key, LIGHT, LIGHT_DARK, Scene, Story, Theme, WINDOW};
use quickie_ui_widgets::CommandPalette;
use url::Url;

use crate::fixtures::{LONG_TITLE, NOW, TABS, World, url};

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
        Self::open_with(scene, world, text, None::<fn(&Url) -> Option<Rc<[u8]>>>)
    }

    fn open_with(
        scene: &mut Scene,
        world: World,
        text: &str,
        favicon: Option<impl Fn(&Url) -> Option<Rc<[u8]>> + 'static>,
    ) -> Rc<Self> {
        let host = Rc::new(Self {
            palette: CommandPalette::new(scene.mtm()),
            world,
            results: RefCell::new(Vec::new()),
            ran: RefCell::new(None),
        });
        if let Some(favicon) = favicon {
            host.palette.set_favicons(favicon);
        }
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
        self.palette.set_items(&items, NOW);
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

/// Open tabs, one pinned to `⌘3` and one with a kept favicon, then history visited before.
fn go_to(scene: &mut Scene) {
    let mut world = World::new(&TABS);
    if let (Some(tab), Ok(slot)) = (world.tab_at(TABS[1].0), PinnedSlot::try_from(3)) {
        let _ = world.session.pin(tab, slot);
    }
    for (address, title, minutes_ago, times) in [
        (TABS[3].0, TABS[3].1, 14, 1),
        (
            "https://google.github.io/eng-practices/review/",
            "Reviewing Pull Requests: Best Practices",
            26 * 60,
            6,
        ),
        (
            "https://webkit.org/blog/%D0%BE%D0%B1%D0%B7%D0%BE%D1%80",
            "Обзор WebKit",
            3 * 24 * 60,
            18,
        ),
    ] {
        for _ in 0..times {
            world.history.record_visit(
                &url(address),
                title,
                Timestamp::from_millis(
                    NOW.millis_since(Timestamp::from_millis(0)) - minutes_ago * 60_000,
                ),
            );
        }
    }
    let host = Host::open_with(scene, world, "", Some(favicon()));
    scene.check(
        host.titles().len() >= TABS.len(),
        "every open tab is offered",
    );
    let pinned = host
        .results
        .borrow()
        .iter()
        .find_map(|item| item.facts.slot.map(PinnedSlot::get));
    scene.check_eq(pinned, Some(3), "the pinned tab carries its slot");
    let accessibility = scene.accessibility();
    scene.check(
        accessibility.contains("Open tabs, 6 tabs") && accessibility.contains("⌘3"),
        "sections and slots are read out",
    );
}

/// A picture for `github.com`: a symbol drawn into a TIFF, as a kept favicon would be bytes.
fn favicon() -> impl Fn(&Url) -> Option<Rc<[u8]>> {
    let bytes: Option<Rc<[u8]>> = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str("swift"),
        None,
    )
    .and_then(|image| image.TIFFRepresentation())
    .map(|data| Rc::from(data.to_vec()));
    move |page: &Url| {
        (page.host_str() == Some("github.com"))
            .then(|| bytes.clone())
            .flatten()
    }
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
    let host = Host::open(scene, World::new(&tabs), "");
    sections(scene, &host);
}

/// The answer comes in sections, tabs first, and `↓` walks the rows across them, past the
/// headers.
fn sections(scene: &mut Scene, host: &Host) {
    let accessibility = scene.accessibility();
    for header in ["Open tabs, 3 tabs", "Bookmarks, 1 page", "History, 1 page"] {
        scene.check(accessibility.contains(header), header);
    }
    for _ in 0..3 {
        scene.press(Key::Down);
    }
    scene.check_eq(
        host.palette.selected().and_then(|row| {
            host.results
                .borrow()
                .get(row)
                .map(|item| item.title.clone())
        }),
        Some("objc2 - Rust".to_owned()),
        "↓ moves to the bookmark, past its header",
    );
    // Rendered with the first row selected again, for the picture.
    scene.press(Key::Up);
    scene.press(Key::Up);
    scene.press(Key::Up);
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
