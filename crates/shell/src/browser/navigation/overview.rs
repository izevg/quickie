//! The tab overview: the window's tabs in opening order, with the pictures the engine keeps.
//! Pictures arrive asynchronously; the overview never waits for one.

use quickie_domain_browsing::{Focus, JarChoice, PinnedSlot, TabId, WindowId};
use quickie_domain_commands::{Key, KeyStroke, Modifiers};
use quickie_ui_shared::Trigger;
use quickie_ui_widgets::{Move, OverviewTab};

use super::super::{Browser, label};

impl Browser {
    /// `quickie.tab.overview`: opens the overview, or closes it when it is open.
    pub(in crate::browser) fn toggle_overview(&self, trigger: Trigger) {
        if self.navigation.overview.is_open() {
            self.close_overview(trigger);
        } else {
            self.open_overview(trigger);
        }
    }

    pub(super) fn open_overview(&self, trigger: Trigger) {
        let Some((window, ns_window)) = self.current() else {
            return;
        };
        let Some(host) = ns_window.contentView() else {
            return;
        };
        self.close_overlays();
        self.hide_find();
        self.navigation.hints.hide();
        let (ids, tabs, selected) = self.overview_tabs(window);
        *self.navigation.overview_tabs.borrow_mut() = (Some(window), ids);
        self.navigation
            .overview
            .show(&host, &ns_window, &tabs, selected, trigger);
        self.refresh_thumbnails(window);
    }

    fn close_overview(&self, trigger: Trigger) {
        self.navigation.overview.hide(trigger);
        if let Some((window, _)) = self.current() {
            self.settle(window);
        }
    }

    /// The overview's fixed keys: the arrows, `⏎`, `⌘W`, `⎋`. Everything else types into its
    /// filter or runs as a shortcut; an input method composing keeps `⏎` and `⎋`.
    pub(super) fn overview_key(&self, stroke: KeyStroke) -> bool {
        let overview = &self.navigation.overview;
        if !overview.is_open() {
            return false;
        }
        let plain = stroke.modifiers == Modifiers::NONE;
        let arrow = match stroke.key {
            Key::Left => Some(Move::Left),
            Key::Right => Some(Move::Right),
            Key::Up => Some(Move::Up),
            Key::Down => Some(Move::Down),
            _ => None,
        };
        match (arrow, stroke.key) {
            (Some(by), _) if plain => overview.move_selection(by),
            (None, Key::Enter) if plain && !overview.is_composing() => {
                if let Some(index) = overview.selected() {
                    self.switch_from_overview(index, Trigger::Keyboard);
                }
            }
            (None, Key::Escape) if plain && !overview.is_composing() => {
                self.close_overview(Trigger::Keyboard);
            }
            (None, Key::Letter(letter))
                if stroke.modifiers == Modifiers::COMMAND && letter.as_char() == 'w' =>
            {
                self.close_from_overview();
            }
            _ => return false,
        }
        true
    }

    /// Switches to the tab of card `index`, and the overview goes.
    pub(super) fn switch_from_overview(&self, index: usize, trigger: Trigger) {
        let tab = self.overview_tab(index);
        self.navigation.overview.hide(trigger);
        let Some(tab) = tab else {
            return;
        };
        if let Some(window) = self.run(|browsing| browsing.activate(tab)) {
            self.settle(window);
        }
    }

    /// `⌘W` in the overview: the selected tab goes to the trash, and the grid closes the gap.
    /// A window left empty gets a start page, as `⌘W` on the page does.
    fn close_from_overview(&self) {
        let overview = &self.navigation.overview;
        let Some(index) = overview.selected() else {
            return;
        };
        let (Some(tab), Some(window)) = (self.overview_tab(index), self.overview_window()) else {
            return;
        };
        if let Some(closed) = self.run(|browsing| browsing.close(tab)) {
            self.forget_zooms(closed.released_jar);
        }
        if self.active(window).is_none() {
            let home = self.home.clone();
            self.run(|browsing| browsing.open(window, home, JarChoice::Main, Focus::Foreground));
        }
        let (ids, tabs, _) = self.overview_tabs(window);
        let selected = index.min(ids.len().saturating_sub(1));
        *self.navigation.overview_tabs.borrow_mut() = (Some(window), ids);
        overview.set_tabs(&tabs, selected);
    }

    fn overview_tab(&self, index: usize) -> Option<TabId> {
        self.navigation.overview_tabs.borrow().1.get(index).copied()
    }

    fn overview_window(&self) -> Option<WindowId> {
        self.navigation.overview_tabs.borrow().0
    }

    /// The window's tabs in opening order, as cards, and the position of the active one.
    fn overview_tabs(&self, window: WindowId) -> (Vec<TabId>, Vec<OverviewTab>, usize) {
        let mut browsing = self.browsing.borrow_mut();
        let session = browsing.session();
        let active = session.active(window);
        let mut cards: Vec<(TabId, OverviewTab)> = session.tabs(window).map_or_else(
            |_| Vec::new(),
            |tabs| {
                tabs.map(|tab| {
                    let card = OverviewTab {
                        title: label(session, tab.id),
                        host: tab.url.host_str().unwrap_or(tab.url.as_str()).to_owned(),
                        slot: session.slot_of(tab.id).map(PinnedSlot::get),
                        thumbnail: None,
                    };
                    (tab.id, card)
                })
                .collect()
            },
        );
        let engine = browsing.engine_mut();
        for (id, card) in &mut cards {
            card.thumbnail = engine.thumbnail(*id);
        }
        let selected = cards
            .iter()
            .position(|(id, _)| Some(*id) == active)
            .unwrap_or_default();
        let (ids, cards) = cards.into_iter().unzip();
        (ids, cards, selected)
    }

    /// Asks the engine for new pictures of the active page, which is on screen right now, and
    /// of loaded pages never drawn yet; each card updates as its picture arrives.
    fn refresh_thumbnails(&self, window: WindowId) {
        let active = self.active(window);
        let ids = self.navigation.overview_tabs.borrow().1.clone();
        let mut browsing = self.browsing.borrow_mut();
        let engine = browsing.engine_mut();
        for tab in ids {
            if Some(tab) != active && engine.thumbnail(tab).is_some() {
                continue;
            }
            let this = self.this.clone();
            engine.capture(tab, move || {
                if let Some(browser) = this.upgrade() {
                    browser.thumbnail_ready(tab);
                }
            });
        }
    }

    fn thumbnail_ready(&self, tab: TabId) {
        let Some(index) = self
            .navigation
            .overview_tabs
            .borrow()
            .1
            .iter()
            .position(|&id| id == tab)
        else {
            return;
        };
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let image = browsing.engine_mut().thumbnail(tab);
        drop(browsing);
        self.navigation
            .overview
            .set_thumbnail(index, image.as_deref());
    }
}
