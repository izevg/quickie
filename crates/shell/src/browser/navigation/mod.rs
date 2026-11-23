//! The navigation overlays that reach past the keymap: the tab overview, which a pinch opens
//! as well as a command, and link hints, which take letters while they show. Both are driven
//! from here; the widgets only show.
//!
//! - [`overview`]: opening, the grid's keys, switching and closing tabs from it, thumbnails.
//! - [`hints`]: asking the page for its targets, the typed labels, following one.
//! - [`player`]: the mini player that floats the video of a tab the user left.
//!
//! Swipe back and forward is not here: the page view does it natively (`engine-webkit`).

mod hints;
mod overview;
mod player;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSEventPhase};
use quickie_application::{Playback, Ticket};
use quickie_domain_browsing::{TabId, WindowId};
use quickie_domain_commands::KeyStroke;
use quickie_ui_shared::Trigger;
use quickie_ui_widgets::{LinkHintsLayer, MiniPlayer, PinchStep, TabOverview};

use super::Browser;

/// The overlays and what they are showing.
pub struct NavigationOverlays {
    overview: Rc<TabOverview>,
    /// The window the overview shows, and its tabs in the order of the cards.
    overview_tabs: RefCell<(Option<WindowId>, Vec<TabId>)>,
    hints: LinkHintsLayer,
    /// The page the hints label.
    hinted: Cell<Option<(WindowId, TabId)>>,
    /// The link targets the hints wait for: the ticket of the page's answer, its window, and
    /// whether links open in the background.
    targets_asked: Cell<Option<(Ticket, WindowId, bool)>>,
    /// The hint being followed: the ticket of the page's answer and its window.
    following: Cell<Option<(Ticket, WindowId)>>,
    /// The mini player, and whose video it shows.
    player: Rc<MiniPlayer>,
    playback: RefCell<Playback>,
    native_player: player::picture_in_picture::Player,
}

impl NavigationOverlays {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            overview: TabOverview::new(mtm),
            overview_tabs: RefCell::new((None, Vec::new())),
            hints: LinkHintsLayer::new(mtm),
            hinted: Cell::new(None),
            targets_asked: Cell::new(None),
            following: Cell::new(None),
            player: MiniPlayer::new(mtm),
            playback: RefCell::new(Playback::new()),
            native_player: player::picture_in_picture::Player::default(),
        }
    }

    /// True when one of them takes keys.
    #[must_use]
    pub fn overview_open(&self) -> bool {
        self.overview.is_open()
    }

    #[must_use]
    pub fn hints_open(&self) -> bool {
        self.hints.is_open()
    }
}

impl Browser {
    /// Hands the overview's clicks to the browser: on a card, and on its close button; and the
    /// mini player's buttons.
    pub(super) fn wire_navigation(&self) {
        self.wire_player();
        let this = self.this.clone();
        self.navigation.overview.on_activate(move |index| {
            if let Some(browser) = this.upgrade() {
                browser.switch_from_overview(index, Trigger::Pointer);
                browser.drain();
            }
        });
        let this = self.this.clone();
        self.navigation.overview.on_close(move |index| {
            if let Some(browser) = this.upgrade() {
                browser.close_from_overview(index);
                browser.drain();
            }
        });
    }

    /// The keys of an open overview or of the hints, before the keymap.
    pub(super) fn navigation_key(&self, stroke: KeyStroke) -> bool {
        self.hints_key(stroke) || self.overview_key(stroke)
    }

    /// Takes the overview and the hints away: another overlay or command takes over.
    pub(super) fn hide_navigation(&self) {
        self.navigation.hints.hide();
        if self.navigation.overview.is_open() {
            self.navigation.overview.hide(Trigger::Keyboard);
        }
    }

    /// A click or a scroll on the page: the labels no longer match what is under them.
    pub(super) fn pointer(&self) {
        self.navigation.hints.hide();
    }

    /// A pinch on the trackpad. Returns true when the overview took the event; the page keeps
    /// the rest, and always the start and the end of the gesture.
    pub(super) fn magnify(&self, event: &NSEvent) -> bool {
        let overview = &self.navigation.overview;
        let phase = event.phase();
        if phase == NSEventPhase::Began {
            self.navigation.hints.hide();
            if !self.other_overlay_open() {
                overview.pinch_began();
            }
            return false;
        }
        if phase == NSEventPhase::Changed {
            return self.pinch_moved(event.magnification(), event.timestamp());
        }
        if phase == NSEventPhase::Ended || phase == NSEventPhase::Cancelled {
            self.pinch_lifted();
        }
        false
    }

    /// The fingers moved by `magnification` at `time`.
    pub(super) fn pinch_moved(&self, magnification: f64, time: f64) -> bool {
        let overview = &self.navigation.overview;
        match overview.pinch_changed(magnification, time) {
            PinchStep::Page => false,
            PinchStep::Follow(_) => {
                if !overview.is_open() {
                    self.open_overview(Trigger::Pointer);
                }
                true
            }
        }
    }

    /// The fingers lifted: the overview settles, and the page takes the keyboard back if it
    /// closes.
    pub(super) fn pinch_lifted(&self) {
        if self.navigation.overview.pinch_ended() == Some(false)
            && let Some((window, _)) = self.current()
        {
            self.settle(window);
        }
    }

    /// The palette, the address overlay, or the switcher: a pinch leaves them alone.
    fn other_overlay_open(&self) -> bool {
        self.address.is_open() || self.command_palette.is_open() || self.cycle.borrow().is_some()
    }
}
