//! Routes the browsing context's mini-player decisions to the original live `WebKit` view.

mod view;

use std::cell::{Cell, RefCell};
use std::ptr;

use quickie_domain_browsing::{MiniPlayer, PlayerChange, TabId, Video};

use super::script;
use crate::gtk::browsing::{Pages, activate};
use crate::gtk::ffi::Object;
use crate::gtk::{App, connect};

#[derive(Clone, Copy, Default)]
enum PlayerPin {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

impl PlayerPin {
    fn next(self) -> Self {
        match self {
            Self::BottomRight => Self::TopRight,
            Self::TopRight => Self::TopLeft,
            Self::TopLeft => Self::BottomLeft,
            Self::BottomLeft => Self::BottomRight,
        }
    }
}

#[derive(Default)]
pub(super) struct Player {
    state: RefCell<MiniPlayer>,
    presentation: Cell<Option<view::View>>,
    pin: Cell<PlayerPin>,
    pending: Cell<Option<TabId>>,
    mounted: Cell<Object>,
    revision: Cell<u64>,
}

impl Player {
    pub(super) fn attach(&self, overlay: Object, data: Object) {
        let view = view::View::new(overlay);
        view.set_pin(self.pin.get());
        connect(view.play, c"clicked", toggle as *const _, data);
        connect(view.back, c"clicked", return_to_tab as *const _, data);
        connect(view.position, c"clicked", move_player as *const _, data);
        connect(view.minimize, c"clicked", minimize as *const _, data);
        connect(view.close, c"clicked", close as *const _, data);
        self.presentation.set(Some(view));
    }

    fn move_to_next_pin(&self) {
        if let Some(view) = self.presentation.get() {
            let pin = self.pin.get().next();
            self.pin.set(pin);
            view.set_pin(pin);
        }
    }

    pub(super) fn switched(&self, pages: &Pages, from: Object, to: Object) {
        let Some(to) = pages.tab(to) else { return };
        let change = {
            let mut state = self.state.borrow_mut();
            if let Some(from) = pages.tab(from) {
                state.switched(from, to)
            } else if state.shown() == Some(to) {
                state.dismissed()
            } else {
                PlayerChange::default()
            }
        };
        self.apply(change);
    }

    pub(super) fn mount(&self, app: &App) {
        let Some(tab) = self.pending.take() else {
            return;
        };
        let Some(view) = app.browsing.pages.view(tab) else {
            self.closed(tab);
            return;
        };
        self.mounted.set(view);
        self.revision.set(self.revision.get().wrapping_add(1));
        script::enter_player(view, app, self.revision.get());
    }

    pub(super) fn float_finished(&self, view: Object, revision: u64, ready: bool) {
        if self.revision.get() != revision || self.mounted.get() != view {
            return;
        }
        if !ready {
            self.dismiss(false);
        } else if let Some(player) = self.presentation.get() {
            player.set_playing(self.playing());
            player.show(view);
        }
    }

    pub(super) fn reported(&self, tab: TabId, video: Option<Video>) {
        let change = self.state.borrow_mut().reported(tab, video);
        self.apply(change);
        if self.state.borrow().shown() == Some(tab)
            && let Some(player) = self.presentation.get()
        {
            player.set_playing(self.playing());
        }
    }

    pub(super) fn closed(&self, tab: TabId) {
        let change = self.state.borrow_mut().closed(tab);
        self.apply(change);
    }

    pub(super) fn shutdown(&self) {
        if let Some(player) = self.presentation.take() {
            player.release();
        }
    }

    fn apply(&self, change: PlayerChange) {
        if change.ended.is_some() {
            self.pending.set(None);
            let page = self.mounted.replace(ptr::null_mut());
            if !page.is_null() {
                script::dock(page);
                if let Some(player) = self.presentation.get() {
                    player.hide(page);
                }
            }
        }
        if let Some(tab) = change.shown {
            self.pending.set(Some(tab));
        }
    }

    fn playing(&self) -> bool {
        let state = self.state.borrow();
        state
            .shown()
            .and_then(|tab| state.video(tab))
            .is_some_and(|video| matches!(video, Video::Playing { .. }))
    }

    fn dismiss(&self, pause: bool) {
        let view = self.mounted.get();
        if pause && !view.is_null() {
            script::play(view, false);
        }
        let change = self.state.borrow_mut().dismissed();
        self.apply(change);
    }
}

extern "C" fn toggle(_: Object, data: Object) {
    // SAFETY: The widget signal runs with its live App data on the GTK thread.
    let player = unsafe { &*data.cast::<App>() };
    let player = &player.browsing.media.player;
    let view = player.mounted.get();
    if !view.is_null() {
        script::play(view, !player.playing());
    }
}

extern "C" fn return_to_tab(_: Object, data: Object) {
    // SAFETY: The widget signal runs with its live App data on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    let tab = app.browsing.media.player.state.borrow().shown();
    if let Some(tab) = tab {
        activate(app, tab);
    }
}

extern "C" fn move_player(_: Object, data: Object) {
    // SAFETY: The widget signal runs with its live App data on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    app.browsing.media.player.move_to_next_pin();
}

extern "C" fn minimize(_: Object, data: Object) {
    // SAFETY: The widget signal runs with its live App data on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    app.browsing.media.player.dismiss(false);
}

extern "C" fn close(_: Object, data: Object) {
    // SAFETY: The widget signal runs with its live App data on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    app.browsing.media.player.dismiss(true);
}
