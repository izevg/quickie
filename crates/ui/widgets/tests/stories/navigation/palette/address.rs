//! The address overlay (`⌘L`): the page as it opens, then suggestions as the user types.

use std::cell::RefCell;
use std::rc::Rc;

use quickie_application::{PaletteIndex, Target};
use quickie_ui_shared::stories::{Key, LIGHT, LIGHT_DARK, Modifiers, Scene, Story, WINDOW};
use quickie_ui_widgets::AddressOverlay;

use crate::fixtures::{TABS, World, url};

/// The page the overlay opens on.
const PAGE: &str = "https://developer.apple.com/documentation/appkit/nswindow";

const fn story(
    state: &'static str,
    themes: &'static [quickie_ui_shared::stories::Theme],
    scene: fn(&mut Scene),
) -> Story {
    Story {
        group: "palette",
        widget: "address-overlay",
        state,
        themes,
        window: WINDOW,
        scene,
    }
}

pub const STORIES: &[Story] = &[
    story("current-url", LIGHT_DARK, current_url),
    story("blocked-count", LIGHT, blocked_count),
    story("inline-completion", LIGHT_DARK, inline_completion),
    story("mixed-kinds", LIGHT, mixed_kinds),
    story("url-typed", LIGHT, url_typed),
    story("cyrillic", LIGHT, cyrillic),
    story("long-urls", LIGHT, long_urls),
    story("go-to-tab", LIGHT, go_to_tab),
];

/// How a row was chosen: its target, and the modifiers held with `⏎`.
type Ran = (Target, Modifiers);

/// The overlay as the browser drives it: it opens on the page with the page's actions, every
/// edit asks the palette index, `↑` `↓` move, `⏎` records the chosen target, `⎋` reverts and
/// then closes, and any other key wakes the field before it lands.
struct Host {
    overlay: AddressOverlay,
    index: RefCell<PaletteIndex>,
    world: World,
    ran: RefCell<Option<Ran>>,
}

impl Host {
    fn open(scene: &mut Scene, world: World, page: &str, blocked: Option<u32>) -> Rc<Self> {
        let host = Rc::new(Self {
            overlay: AddressOverlay::new(scene.mtm()),
            index: RefCell::new(world.index()),
            world,
            ran: RefCell::new(None),
        });
        let weak = Rc::downgrade(&host);
        host.overlay.on_change(move || {
            if let Some(host) = weak.upgrade() {
                host.requery();
            }
        });
        host.overlay
            .show(scene.host(), scene.window(), page, blocked);
        host.overlay.set_actions(host.index.borrow().page_actions());
        let weak = Rc::downgrade(&host);
        scene.route(move |stroke| {
            let Some(host) = weak.upgrade() else {
                return false;
            };
            match stroke.key {
                Key::Down => host.overlay.move_selection(1),
                Key::Up => host.overlay.move_selection(-1),
                Key::Return => {
                    *host.ran.borrow_mut() = host.overlay.chosen().map(|t| (t, stroke.modifiers));
                }
                Key::Escape => {
                    if !host.overlay.revert() {
                        host.overlay.hide();
                    }
                }
                _ => {
                    host.overlay.wake();
                    return false;
                }
            }
            true
        });
        scene.keep(Rc::clone(&host));
        host
    }

    fn requery(&self) {
        let answer = self
            .index
            .borrow_mut()
            .suggest(&self.overlay.typed(), self.overlay.completes());
        self.overlay.set_suggestions(answer);
    }

    fn typing(scene: &mut Scene, world: World, text: &str) -> Rc<Self> {
        let host = Self::open(scene, world, PAGE, None);
        scene.type_text(text);
        host
    }
}

/// The page with its actions. Played: typing shows the field and suggestions, `⎋` brings the
/// page back, `⎋` again closes; pictured open on the page again.
fn current_url(scene: &mut Scene) {
    let host = Host::open(scene, World::new(&TABS), PAGE, None);
    scene.check(host.overlay.shows_page(), "the overlay shows the page");
    scene.check_eq(host.overlay.text(), PAGE.to_owned(), "the page's URL");
    let copy_url = |target: Option<Target>| match target {
        Some(Target::Command(id)) => id.as_str() == "quickie.page.copyUrl",
        _ => false,
    };
    scene.check(
        copy_url(host.overlay.chosen()),
        "Copy URL is the first action",
    );
    scene.type_text("news");
    scene.check(!host.overlay.shows_page(), "typing shows the field");
    scene.check(
        !copy_url(host.overlay.chosen()),
        "suggestions replace the actions",
    );
    scene.press(Key::Escape);
    scene.check(host.overlay.shows_page(), "⎋ brings the page back");
    scene.check_eq(host.overlay.text(), PAGE.to_owned(), "the page's URL again");
    scene.check(copy_url(host.overlay.chosen()), "the actions are back");
    scene.press(Key::Escape);
    scene.check(!host.overlay.is_open(), "⎋ again closes");
    host.overlay.show(scene.host(), scene.window(), PAGE, None);
}

/// A count from Shields, when it has one, sits on the right of the URL.
fn blocked_count(scene: &mut Scene) {
    let host = Host::open(scene, World::new(&TABS), PAGE, Some(12));
    scene.check(host.overlay.shows_page(), "the overlay shows the page");
}

fn inline_completion(scene: &mut Scene) {
    let host = Host::typing(scene, World::new(&TABS), "git");
    scene.check(!host.overlay.shows_page(), "typing shows the field");
    scene.check_eq(host.overlay.typed(), "git".to_owned(), "what was typed");
    scene.check_eq(
        host.overlay.text(),
        "github.com".to_owned(),
        "the host completes in the field",
    );
    scene.check_eq(
        host.overlay.chosen(),
        Some(Target::Url(url("https://github.com/"))),
        "⏎ goes to the completed host",
    );
}

fn mixed_kinds(scene: &mut Scene) {
    let host = Host::typing(scene, World::new(&TABS), "rust");
    scene.check_eq(
        host.overlay.text(),
        "rust-lang.org".to_owned(),
        "www. is completed past",
    );
}

fn url_typed(scene: &mut Scene) {
    let host = Host::typing(scene, World::new(&TABS), "example.com/docs");
    scene.check_eq(
        host.overlay.chosen(),
        Some(Target::Url(url("https://example.com/docs"))),
        "a typed address is gone to first",
    );
}

/// Russian text: nothing completes, the search for it comes first, and the Russian title
/// matches.
fn cyrillic(scene: &mut Scene) {
    let host = Host::typing(scene, World::new(&TABS), "лента");
    scene.check_eq(host.overlay.text(), "лента".to_owned(), "nothing completes");
    scene.check_eq(
        host.overlay.chosen(),
        Some(Target::Search(url(
            "https://duckduckgo.com/?q=%D0%BB%D0%B5%D0%BD%D1%82%D0%B0",
        ))),
        "plain text is searched first",
    );
    let world = &host.world;
    scene.check_eq(
        host.overlay.target_at(1),
        world.tabs.get(2).map(|&tab| Target::Tab(tab)),
        "the Russian title is found",
    );
}

fn long_urls(scene: &mut Scene) {
    let tabs = [
        (
            "https://example.com/a-very/long/path/that/goes/on/and/on/and/on/forever/index.html",
            "A page with a long address",
        ),
        (
            "https://subdomain.of.a.very.long.host.name.example.com/search?q=a+long+query+string",
            "Search results on a long host",
        ),
    ];
    let host = Host::open(scene, World::new(&tabs), PAGE, None);
    scene.type_text("exam");
    scene.check_eq(
        host.overlay.text(),
        "example.com".to_owned(),
        "the host completes",
    );
}

/// Type "git": the field completes "hub.com"; `↓` selects the open tab, the field shows its
/// URL; `⏎` switches to that tab instead of opening the page again.
fn go_to_tab(scene: &mut Scene) {
    let host = Host::typing(scene, World::new(&TABS), "git");
    scene.check_eq(
        host.overlay.text(),
        "github.com".to_owned(),
        "inline completion",
    );
    scene.press(Key::Down);
    scene.check_eq(host.overlay.selected(), Some(1), "↓ selects the second row");
    scene.check_eq(
        host.overlay.text(),
        "https://github.com/izevg/quickie/pulls".to_owned(),
        "the field shows the selected row",
    );
    scene.press(Key::Return);
    let tab = host.world.tabs.get(3).copied();
    scene.check_eq(
        host.ran.borrow().clone(),
        tab.map(|tab| (Target::Tab(tab), Modifiers::NONE)),
        "⏎ switches to the open tab",
    );
}
