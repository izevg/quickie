//! Native presentation of page fullscreen and video playback.

mod fullscreen;
mod player;
mod script;

use crate::gtk::{App, ffi::Object};
use quickie_domain_browsing::TabId;

#[derive(Default)]
pub(in crate::gtk) struct Playback {
    fullscreen: fullscreen::Fullscreen,
    player: player::Player,
}

impl Playback {
    pub(super) fn attach(&self, _window: Object, overlay: Object, data: Object) {
        self.fullscreen.attach(overlay);
        self.player.attach(overlay, data);
    }

    pub(super) fn active_view_changed(&self, app: &App, previous: Object, next: Object) {
        self.fullscreen.active_view_changed(previous);
        self.player.switched(&app.browsing.pages, previous, next);
    }

    pub(super) fn mount_player(&self, app: &App) {
        self.player.mount(app);
    }

    pub(super) fn tab_closed(&self, tab: TabId) {
        self.player.closed(tab);
    }

    pub(super) fn shutdown(&self, _app: &App) {
        self.player.shutdown();
        self.fullscreen.shutdown();
    }
}

pub(super) fn bind(view: Object, data: Object) {
    fullscreen::bind(view, data);
    script::bind(view, data);
}
