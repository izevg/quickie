//! The page group: the find bar and the origin chip.

use std::cell::Cell;
use std::rc::Rc;

use quickie_domain_browsing::{FindMatches, Origin};
use quickie_ui_shared::stories::{Key, LIGHT, LIGHT_DARK, Modifiers, Scene, Story, Theme, WINDOW};
use quickie_ui_widgets::{FindBar, FindRequest, OriginChip};

use crate::fixtures;

pub const STORIES: &[Story] = &[
    Story {
        group: "page",
        widget: "find-bar",
        state: "matches",
        themes: &[Theme::Light, Theme::Dark, Theme::Contrast],
        window: WINDOW,
        scene: find_matches,
    },
    Story {
        group: "page",
        widget: "find-bar",
        state: "no-matches",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: find_no_matches,
    },
    Story {
        group: "page",
        widget: "find-bar",
        state: "cyrillic",
        themes: LIGHT,
        window: WINDOW,
        scene: find_cyrillic,
    },
    Story {
        group: "page",
        widget: "find-bar",
        state: "steps",
        themes: LIGHT,
        window: WINDOW,
        scene: find_steps,
    },
    Story {
        group: "page",
        widget: "origin-chip",
        state: "https",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: chip_https,
    },
    Story {
        group: "page",
        widget: "origin-chip",
        state: "http",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: chip_http,
    },
    Story {
        group: "page",
        widget: "origin-chip",
        state: "blocked",
        themes: LIGHT_DARK,
        window: WINDOW,
        scene: chip_blocked,
    },
    Story {
        group: "page",
        widget: "origin-chip",
        state: "russian-path",
        themes: LIGHT,
        window: WINDOW,
        scene: chip_russian,
    },
];

/// A find bar over a page of fixture text that answers its requests as the browser would:
/// a new query finds from the top, a step moves and wraps.
struct FindPage {
    bar: FindBar,
    current: Cell<usize>,
    total: Cell<usize>,
}

impl FindPage {
    fn open(scene: &mut Scene) -> Rc<Self> {
        let page = Rc::new(Self {
            bar: FindBar::new(scene.mtm()),
            current: Cell::new(0),
            total: Cell::new(0),
        });
        let weak = Rc::downgrade(&page);
        page.bar.on_request(move |request| {
            if let Some(page) = weak.upgrade() {
                page.answer(request);
            }
        });
        page.bar.show(scene.host(), scene.window());
        // The browser's routing: `⏎` and `⇧⏎` step while the field has the keyboard.
        let weak = Rc::downgrade(&page);
        scene.route(move |stroke| {
            let Some(page) = weak.upgrade() else {
                return false;
            };
            match stroke.key {
                Key::Return => {
                    page.answer(FindRequest::Step {
                        backwards: stroke.modifiers.shift,
                    });
                    true
                }
                Key::Escape => {
                    page.answer(FindRequest::Close);
                    true
                }
                _ => false,
            }
        });
        scene.keep(Rc::clone(&page));
        page
    }

    fn answer(&self, request: FindRequest) {
        match request {
            FindRequest::Search => {
                let total = fixtures::count(fixtures::PAGE_TEXT, &self.bar.query());
                self.total.set(total);
                self.current.set(usize::from(total > 0));
            }
            FindRequest::Step { backwards } => {
                let total = self.total.get();
                if total > 0 {
                    let current = self.current.get();
                    self.current.set(if backwards {
                        if current <= 1 { total } else { current - 1 }
                    } else if current >= total {
                        1
                    } else {
                        current + 1
                    });
                }
            }
            FindRequest::Close => {
                self.bar.hide();
                return;
            }
        }
        self.bar
            .set_matches(FindMatches::new(Some(self.current.get()), self.total.get()));
    }
}

fn find_matches(scene: &mut Scene) {
    let page = FindPage::open(scene);
    scene.type_text("sovereign");
    scene.check_eq(page.bar.query(), "sovereign".to_owned(), "the typed query");
    scene.check_eq(page.bar.matches_text(), "1 of 4".to_owned(), "after typing");
    scene.press(Key::Return);
    scene.press(Key::Return);
    scene.check_eq(page.bar.matches_text(), "3 of 4".to_owned(), "after ⏎ ⏎");
}

fn find_no_matches(scene: &mut Scene) {
    let page = FindPage::open(scene);
    scene.type_text("sovereignty");
    scene.check_eq(page.bar.matches_text(), "No matches".to_owned(), "no match");
}

fn find_cyrillic(scene: &mut Scene) {
    let page = FindPage::open(scene);
    scene.type_text("суверен");
    scene.check_eq(page.bar.query(), "суверен".to_owned(), "a Cyrillic query");
    scene.check_eq(
        page.bar.matches_text(),
        "1 of 2".to_owned(),
        "Cyrillic matches",
    );
}

/// `⏎` steps forward, `⇧⏎` back, both wrap; `⎋` closes.
fn find_steps(scene: &mut Scene) {
    let page = FindPage::open(scene);
    scene.type_text("sovereign");
    scene.press_with(Key::Return, Modifiers::SHIFT);
    scene.check_eq(
        page.bar.matches_text(),
        "4 of 4".to_owned(),
        "⇧⏎ wraps back",
    );
    scene.press(Key::Return);
    scene.check_eq(
        page.bar.matches_text(),
        "1 of 4".to_owned(),
        "⏎ wraps forward",
    );
    scene.press(Key::Backspace);
    scene.check_eq(page.bar.query(), "sovereig".to_owned(), "⌫ edits the query");
    scene.check_eq(
        page.bar.matches_text(),
        "1 of 4".to_owned(),
        "a refined query",
    );
    scene.check(
        scene.accessibility().contains("AXTextField"),
        "the query field is in the accessibility tree",
    );
    scene.press(Key::Escape);
    scene.check(!page.bar.is_open(), "⎋ closes the bar");
    page.bar.show(scene.host(), scene.window());
}

fn chip(scene: &mut Scene, url: &str, blocked: Option<u32>) {
    let chip = OriginChip::new(scene.mtm());
    let url = fixtures::url(url);
    if let Some(origin) = Origin::of(&url) {
        chip.show(scene.host(), &origin, url.path(), blocked);
    }
    scene.check(chip.text().is_some(), "the chip shows");
    scene.keep(chip);
}

fn chip_https(scene: &mut Scene) {
    chip(scene, "https://developer.apple.com/documentation/", None);
}

fn chip_http(scene: &mut Scene) {
    chip(scene, "http://127.0.0.1:8080/", None);
}

/// A percent-encoded Cyrillic path reads as words, cut when long.
fn chip_russian(scene: &mut Scene) {
    chip(
        scene,
        "https://ru.wikipedia.org/wiki/%D0%9C%D0%BE%D1%81%D0%BA%D0%BE%D0%B2%D1%81%D0%BA%D0%B8%D0%B9_%D0%BA%D1%80%D0%B5%D0%BC%D0%BB%D1%8C",
        None,
    );
}

fn chip_blocked(scene: &mut Scene) {
    chip(scene, "https://www.bbc.co.uk:8443/news", Some(12));
}
