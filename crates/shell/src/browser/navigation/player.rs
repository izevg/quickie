//! The mini player: leaving a tab whose video plays aloud floats that video over the window
//! the user stays in, until they come back to the tab, close the player, close the tab, or
//! leave another playing tab, whose video takes its place. The `Playback` use case decides and
//! moves the page through the engine; this routes tab switches, the pages' video reports, and
//! the player's buttons to it, and shows or hides the widget as each step says.

pub(super) mod picture_in_picture;

use objc2::Message;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use quickie_domain_browsing::{PlayerChange, TabId, Video, WindowId};
use quickie_ui_shared::Trigger;
use quickie_ui_widgets::PlayerRequest;

use super::super::{Browser, label};

impl Browser {
    /// Hands the player's buttons to the browser.
    pub(in crate::browser) fn wire_player(&self) {
        let this = self.this.clone();
        self.navigation.player.on_request(move |request| {
            if let Some(browser) = this.upgrade() {
                browser.player_request(request);
                browser.drain();
            }
        });
    }

    /// The user left tab `from` for tab `to` in `window`: a tab switch, the switcher, a new
    /// tab, or a tab closed under them. Coming back to the floating tab ends the player; a
    /// tab left with its video playing aloud floats it over `window`.
    pub(in crate::browser) fn left_tab(&self, window: WindowId, from: TabId, to: TabId) {
        if self.native_player_left_tab(from, to)
            && self.navigation.playback.borrow().shown().is_none()
        {
            self.schedule_tab_player_fallback(window, from, to);
            return;
        }
        self.float_tab_player(window, from, to);
    }

    fn float_tab_player(&self, window: WindowId, from: TabId, to: TabId) {
        let slot: Retained<NSView> = self.navigation.player.slot().retain();
        let switched = {
            let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
                return;
            };
            let mut playback = self.navigation.playback.borrow_mut();
            if browsing.session().tab(from).is_none() {
                playback.closed(from);
            }
            playback.switched(browsing.engine_mut(), &slot, from, to)
        };
        match switched {
            Ok(change) => self.show_player(change, window, Trigger::Keyboard),
            Err(error) => {
                self.report(&error);
                self.navigation.player.hide(Trigger::Keyboard);
            }
        }
    }

    /// A page said what its video does: the player follows its play and pause, and ends when
    /// the floating page's video is gone.
    pub(in crate::browser) fn video_reported(&self, tab: TabId, video: Option<Video>) {
        let change = {
            let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
                return;
            };
            self.navigation
                .playback
                .borrow_mut()
                .reported(browsing.engine_mut(), tab, video)
        };
        let playback = self.navigation.playback.borrow();
        if change.ended.is_some() {
            self.navigation.player.hide(Trigger::App);
        } else if playback.shown() == Some(tab) {
            self.navigation.player.set_playing(playback.playing());
        }
    }

    /// The floating tab closed (from the overview, the switcher, a command): the player ends.
    pub(in crate::browser) fn player_tab_closed(&self) {
        self.forget_closed_native_player();
        let gone = {
            let Ok(browsing) = self.browsing.try_borrow() else {
                return;
            };
            let shown = self.navigation.playback.borrow().shown();
            shown.filter(|&tab| browsing.session().tab(tab).is_none())
        };
        if let Some(tab) = gone {
            self.navigation.playback.borrow_mut().closed(tab);
            self.navigation.player.hide(Trigger::Keyboard);
        }
    }

    /// A button of the player, or a click on its video: play or pause, back to the tab (its
    /// window comes forward, and switching to the tab ends the player as coming back always
    /// does), or close.
    fn player_request(&self, request: PlayerRequest) {
        let outcome = match request {
            PlayerRequest::PlayPause => {
                let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
                    return;
                };
                self.navigation
                    .playback
                    .borrow_mut()
                    .toggle(browsing.engine_mut())
            }
            PlayerRequest::BackToTab => {
                let shown = self.navigation.playback.borrow().shown();
                self.navigation.player.hide(Trigger::Pointer);
                if let Some(window) = shown.and_then(|tab| self.run(|b| b.activate(tab))) {
                    // The tab's window comes forward with it, wherever the player was.
                    if let Some(ns_window) = self.window(window) {
                        ns_window.makeKeyAndOrderFront(None);
                    }
                    self.settle(window);
                }
                Ok(())
            }
            PlayerRequest::Close => {
                self.navigation.player.hide(Trigger::Pointer);
                let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
                    return;
                };
                self.navigation
                    .playback
                    .borrow_mut()
                    .dismiss(browsing.engine_mut())
                    .map(drop)
            }
        };
        if let Err(error) = outcome {
            self.report(&error);
        }
    }

    /// Puts what a step changed on screen: the player over `window` for the tab whose video it
    /// shows now, or gone when it ended.
    fn show_player(&self, change: PlayerChange, window: WindowId, trigger: Trigger) {
        let Some(tab) = change.shown else {
            if change.ended.is_some() {
                self.navigation.player.hide(trigger);
            }
            return;
        };
        let title = label(self.browsing.borrow().session(), tab);
        let playing = self.navigation.playback.borrow().playing();
        if let Some(host) = self.window(window).and_then(|window| window.contentView()) {
            self.navigation.player.show(&host, &title, playing, trigger);
        }
    }
}
