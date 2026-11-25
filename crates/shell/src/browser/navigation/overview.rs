//! The tab overview: the window's tabs in opening order, with the pictures the engine keeps.
//! Pictures arrive asynchronously; the overview and the switcher never wait for one.

use quickie_application::PageSurface;
use quickie_domain_browsing::{Focus, JarChoice, PinnedSlot, TabId, WindowId};
use quickie_domain_commands::{Key, KeyStroke, Modifiers};
use quickie_ui_shared::Trigger;
use quickie_ui_shared::tab_card::TabCard;
use quickie_ui_widgets::Move;

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
        self.refresh_thumbnails(window, &ids);
        *self.navigation.overview_tabs.borrow_mut() = (Some(window), ids);
        self.navigation
            .overview
            .show(&host, &ns_window, tabs, selected, trigger);
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
                if let Some(index) = overview.selected() {
                    self.close_from_overview(index);
                }
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

    /// `⌘W` in the overview, or a click on a card's close button: the tab of card `index` goes
    /// to the trash, and the grid closes the gap.
    pub(super) fn close_from_overview(&self, index: usize) {
        let (Some(tab), Some(window)) = (self.overview_tab(index), self.overview_window()) else {
            return;
        };
        self.close_in(window, tab);
        let (ids, tabs, _) = self.overview_tabs(window);
        let selected = index.min(ids.len().saturating_sub(1));
        *self.navigation.overview_tabs.borrow_mut() = (Some(window), ids);
        self.navigation.overview.set_tabs(tabs, selected);
    }

    /// Sends `tab` of `window` to the trash from the overview or the switcher. A window left
    /// empty gets a start page, as `⌘W` on the page does.
    pub(in crate::browser) fn close_in(&self, window: WindowId, tab: TabId) {
        if let Some(closed) = self.run(|browsing| browsing.close(tab)) {
            self.agent_document_lost(tab);
            self.forget_zooms(closed.released_jar);
        }
        if self.active(window).is_none() {
            let home = self.home.clone();
            self.run(|browsing| browsing.open(window, home, JarChoice::Main, Focus::Foreground));
        }
    }

    fn overview_tab(&self, index: usize) -> Option<TabId> {
        self.navigation.overview_tabs.borrow().1.get(index).copied()
    }

    fn overview_window(&self) -> Option<WindowId> {
        self.navigation.overview_tabs.borrow().0
    }

    /// The window's tabs in opening order, as cards, and the position of the active one.
    fn overview_tabs(&self, window: WindowId) -> (Vec<TabId>, Vec<TabCard>, usize) {
        let (ids, active) = {
            let browsing = self.browsing.borrow();
            let session = browsing.session();
            let ids: Vec<TabId> = session
                .tabs(window)
                .map_or_else(|_| Vec::new(), |tabs| tabs.map(|tab| tab.id).collect());
            (ids, session.active(window))
        };
        let cards = self.tab_cards(&ids);
        let selected = ids
            .iter()
            .position(|&id| Some(id) == active)
            .unwrap_or_default();
        (ids, cards, selected)
    }

    /// `ids` as cards: title, host, pinned slot, and the picture the engine keeps.
    pub(in crate::browser) fn tab_cards(&self, ids: &[TabId]) -> Vec<TabCard> {
        let browsing = self.browsing.borrow();
        let session = browsing.session();
        let mut cards: Vec<TabCard> = ids
            .iter()
            .map(|&id| {
                let host = session.tab(id).map_or_else(String::new, |tab| {
                    tab.url.host_str().unwrap_or(tab.url.as_str()).to_owned()
                });
                TabCard {
                    title: label(session, id),
                    host,
                    slot: session.slot_of(id).map(PinnedSlot::get),
                    ..TabCard::default()
                }
            })
            .collect();
        let engine = browsing.engine();
        for (&id, card) in ids.iter().zip(&mut cards) {
            card.thumbnail = engine.picture(id);
        }
        cards
    }

    /// Asks the engine for new pictures of the active page of `window`, which is on screen
    /// right now, and of the pages among `ids` never drawn yet; each card updates as its
    /// picture arrives.
    pub(in crate::browser) fn refresh_thumbnails(&self, window: WindowId, ids: &[TabId]) {
        let active = self.active(window);
        let mut browsing = self.browsing.borrow_mut();
        let engine = browsing.engine_mut();
        for &tab in ids {
            if Some(tab) != active && engine.picture(tab).is_some() {
                continue;
            }
            engine.capture(tab);
        }
    }

    /// A new picture of a tab's page is ready ([`EngineEvent::Pictured`]): its card shows it
    /// in the overview or the switcher, whichever is open.
    ///
    /// [`EngineEvent::Pictured`]: quickie_application::EngineEvent::Pictured
    pub(in crate::browser) fn thumbnail_ready(&self, tab: TabId) {
        let in_overview = card_of(
            self.navigation.overview.is_open(),
            &self.navigation.overview_tabs.borrow().1,
            tab,
        );
        let in_switcher = self
            .cycle
            .borrow()
            .as_ref()
            .and_then(|hold| card_of(self.switcher.is_open(), hold.cycle().order(), tab));
        if in_overview.is_none() && in_switcher.is_none() {
            return;
        }
        let Ok(browsing) = self.browsing.try_borrow() else {
            return;
        };
        let image = browsing.engine().picture(tab);
        drop(browsing);
        if let Some(index) = in_overview {
            self.navigation
                .overview
                .set_thumbnail(index, image.as_deref());
        }
        if let Some(index) = in_switcher {
            self.switcher.set_thumbnail(index, image.as_deref());
        }
    }
}

/// The card a new picture of `tab` goes to: its position among the `cards` of the overview or
/// the switcher, while it is open. A closed one reads every picture afresh when it opens.
fn card_of(open: bool, cards: &[TabId], tab: TabId) -> Option<usize> {
    if !open {
        return None;
    }
    cards.iter().position(|&card| card == tab)
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::{Profile, TabId};

    use super::card_of;

    fn tab(serial: u64) -> TabId {
        TabId::new(Profile::Main, serial)
    }

    #[test]
    fn a_picture_reaches_its_tabs_card_while_the_overview_is_open() {
        let cards = [tab(3), tab(1), tab(2)];
        assert_eq!(card_of(true, &cards, tab(1)), Some(1));
        assert_eq!(card_of(true, &cards, tab(2)), Some(2));
    }

    #[test]
    fn a_picture_of_a_tab_without_a_card_or_for_a_closed_overview_goes_nowhere() {
        let cards = [tab(3), tab(1)];
        assert_eq!(card_of(true, &cards, tab(7)), None);
        assert_eq!(card_of(false, &cards, tab(1)), None);
    }
}
