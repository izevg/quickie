//! The native player follows an audible video when attention leaves its tab, window or Space.

use std::cell::Cell;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSTimer;
use quickie_application::{NativePictureInPicture, PageVideo};
use quickie_domain_browsing::{TabId, WindowId};

use super::Browser;

const ENTRY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
struct Owned {
    tab: TabId,
    requested: Instant,
    closing: bool,
}

#[derive(Default)]
pub(in crate::browser::navigation) struct Player {
    attended: Cell<Option<TabId>>,
    visible: Cell<Option<TabId>>,
    owned: Cell<Option<Owned>>,
    focus_scheduled: Cell<bool>,
    focus_revision: Cell<u64>,
}

impl Browser {
    /// A tab changed in the front window. True when a native player owns the departure.
    pub(super) fn native_player_left_tab(&self, from: TabId, to: TabId) -> bool {
        if self.attended_video_tab() != Some(to) {
            return false;
        }
        let player = &self.navigation.native_player;
        if player.attended.get().is_none() {
            player.attended.set(Some(from));
        }
        self.native_player_focus(Some(to));
        player.owned.get().is_some() || player.visible.get().is_some()
    }

    pub(super) fn schedule_tab_player_fallback(&self, window: WindowId, from: TabId, to: TabId) {
        let player = &self.navigation.native_player;
        let Some(owned) = player.owned.get().filter(|owned| owned.tab == from) else {
            return;
        };
        let revision = player.focus_revision.get();
        let this = self.this.clone();
        let tick = RcBlock::new(move |_: NonNull<NSTimer>| {
            if let Some(browser) = this.upgrade() {
                let player = &browser.navigation.native_player;
                if player.focus_revision.get() == revision
                    && player.attended.get() == Some(to)
                    && player.visible.get().is_none()
                    && player
                        .owned
                        .get()
                        .is_some_and(|latest| latest.requested == owned.requested)
                {
                    player.owned.set(None);
                    browser.float_tab_player(window, from, to);
                }
            }
        });
        // SAFETY: a one-shot main run-loop timer, which retains its block.
        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                ENTRY_TIMEOUT.as_secs_f64(),
                false,
                &tick,
            )
        };
    }

    /// `AppKit` emits several focus notifications for one action. Reconcile after they settle.
    pub(in crate::browser) fn schedule_native_player_focus(&self) {
        if self.navigation.native_player.focus_scheduled.replace(true) {
            return;
        }
        let this = self.this.clone();
        let tick = RcBlock::new(move |_: NonNull<NSTimer>| {
            if let Some(browser) = this.upgrade() {
                browser.navigation.native_player.focus_scheduled.set(false);
                browser.native_player_focus(browser.attended_video_tab());
            }
        });
        // SAFETY: a one-shot main run-loop timer, which retains its block.
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.01, false, &tick) };
    }

    pub(in crate::browser) fn native_player_reported(&self, tab: TabId, active: bool) {
        let player = &self.navigation.native_player;
        if active {
            player.visible.set(Some(tab));
            self.return_to_native_player(player.attended.get());
        } else {
            if player.visible.get() == Some(tab) {
                player.visible.set(None);
            }
            if player.owned.get().is_some_and(|owned| owned.tab == tab) {
                player.owned.set(None);
            }
        }
    }

    pub(super) fn forget_closed_native_player(&self) {
        let player = &self.navigation.native_player;
        let Ok(browsing) = self.browsing.try_borrow() else {
            return;
        };
        let closed = |tab| browsing.session().tab(tab).is_none();
        if player.visible.get().is_some_and(closed) {
            player.visible.set(None);
        }
        if player.owned.get().is_some_and(|owned| closed(owned.tab)) {
            player.owned.set(None);
        }
    }

    fn attended_video_tab(&self) -> Option<TabId> {
        let app = NSApplication::sharedApplication(self.mtm);
        if !app.isActive() {
            return None;
        }
        let key = app.keyWindow()?;
        let window = key.sheetParent().unwrap_or(key);
        if !window.isOnActiveSpace() {
            return None;
        }
        let id = self
            .windows
            .borrow()
            .iter()
            .find_map(|(&id, own)| (**own == *window).then_some(id))?;
        self.browsing.try_borrow().ok()?.session().active(id)
    }

    fn native_player_focus(&self, next: Option<TabId>) {
        let player = &self.navigation.native_player;
        let before = player.attended.replace(next);
        if before == next {
            return;
        }
        let revision = player.focus_revision.get().wrapping_add(1);
        player.focus_revision.set(revision);
        if let Some(owned) = player.owned.get()
            && player.visible.get() != Some(owned.tab)
            && owned.requested.elapsed() >= ENTRY_TIMEOUT
        {
            player.owned.set(None);
        }
        self.return_to_native_player(next);
        let Some(tab) = before else { return };
        if self.start_native_player(tab) {
            return;
        }
        // The media candidate or audio report can arrive just after the view leaves focus.
        let this = self.this.clone();
        let retry = RcBlock::new(move |_: NonNull<NSTimer>| {
            if let Some(browser) = this.upgrade()
                && browser.navigation.native_player.focus_revision.get() == revision
                && browser.navigation.playback.borrow().shown().is_none()
            {
                browser.start_native_player(tab);
            }
        });
        // SAFETY: a one-shot main run-loop timer, which retains its block.
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.2, false, &retry) };
    }

    fn start_native_player(&self, tab: TabId) -> bool {
        let player = &self.navigation.native_player;
        if player.owned.get().is_some() || player.visible.get().is_some() {
            return false;
        }
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return false;
        };
        let engine = browsing.engine_mut();
        if !engine.is_playing_audio(tab)
            || engine.picture_in_picture(tab) != NativePictureInPicture::Inactive
        {
            return false;
        }
        player.owned.set(Some(Owned {
            tab,
            requested: Instant::now(),
            closing: false,
        }));
        let started = engine.toggle_picture_in_picture(tab);
        if !started {
            player.owned.set(None);
        }
        started
    }

    fn return_to_native_player(&self, next: Option<TabId>) {
        let player = &self.navigation.native_player;
        let Some(mut owned) = player
            .owned
            .get()
            .filter(|value| Some(value.tab) == next && !value.closing)
        else {
            return;
        };
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let engine = browsing.engine_mut();
        if engine.picture_in_picture(owned.tab) == NativePictureInPicture::Active {
            owned.closing = true;
            player.owned.set(Some(owned));
            if !engine.toggle_picture_in_picture(owned.tab) {
                player.owned.set(None);
            }
        }
    }
}
