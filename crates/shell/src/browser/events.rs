//! What the engine reports reaches the browser here, and only here: [`Browser::drain`] takes
//! every waiting [`EngineEvent`] in the order pages did it and routes each, in one `match`, to
//! the use case or the widget that takes it. It runs from the engine's waker on the main run
//! loop and after every command, never from inside an engine call.
//!
//! A page whose web content process crashed keeps its tab: a toast says so and how to reload,
//! at once when it is on screen, else when it next comes forward.

use quickie_application::{AppError, Engine, EngineEvent};
use quickie_domain_browsing::{TabId, WindowId};
use url::Url;

use super::Browser;
use crate::OpenedPage;

/// What the engine reports.
pub type Event = EngineEvent<OpenedPage>;

/// What a crashed page's toast says.
const CRASHED: &str = "This page crashed · ⌘R to reload";

/// What the events of one drain leave to do once, after all of them.
#[derive(Default)]
struct Drained {
    /// Some page committed or retitled itself: the window titles follow.
    navigated: bool,
    /// The tabs that committed a navigation: their zoom and shields are applied again.
    committed: Vec<TabId>,
    /// What the session or history failed to record.
    failures: Vec<AppError>,
    /// A download moved: the popover catches up.
    downloads: bool,
    /// A page's blocked count changed: the shields popover catches up.
    blocked: bool,
}

impl Browser {
    /// Routes every engine event waiting since the last time, then puts what they changed on
    /// screen: the window titles, the zooms of committed pages, the downloads popover, the
    /// sheets of asks, the origin chip. Tells of a save that failed on a writer thread.
    pub fn drain(&self) {
        if self.updater.terminating() {
            return;
        }
        let mut events = std::mem::take(&mut *self.events.borrow_mut());
        if let Ok(mut browsing) = self.browsing.try_borrow_mut() {
            browsing.engine_mut().drain(&mut events);
        }
        let mut drained = Drained::default();
        for event in events.drain(..) {
            self.dispatch_event(event, &mut drained);
        }
        *self.events.borrow_mut() = events;
        self.drain_extensions();
        self.settle_drained(&drained);
        for writes in &self.writes {
            if let Some(error) = writes.take_error() {
                self.toast(&format!("Not saved: {error}"));
            }
        }
    }

    fn dispatch_event(&self, event: Event, drained: &mut Drained) {
        match event {
            EngineEvent::Committed { tab, url, title } => {
                self.agent_source_navigated(tab, &url);
                drained.committed.push(tab);
                self.shields.borrow_mut().page_left(tab);
                drained.blocked = true;
                self.crashed.borrow_mut().remove(&tab);
                self.asks_navigated(tab);
                self.record_navigation(tab, &url, &title, true, drained);
                self.mcp_tab_committed(tab, &url);
            }
            EngineEvent::Titled { tab, url, title } => {
                self.record_navigation(tab, &url, &title, false, drained);
            }
            EngineEvent::Loading { tab, progress } => self.loading(tab, progress),
            EngineEvent::Opened {
                opener,
                page,
                url,
                focus,
            } => self.adopt_opened(opener, page, url, focus),
            EngineEvent::ClosedByPage { tab } => self.closed_by_page(tab),
            EngineEvent::Crashed { tab } => self.crashed(tab),
            EngineEvent::Fullscreen { tab, state } => self.fullscreen_changed(tab, state),
            EngineEvent::Video { tab, video } => self.video_reported(tab, video),
            EngineEvent::PictureInPicture { tab, active } => {
                self.native_player_reported(tab, active);
            }
            EngineEvent::PictureInPictureReturnRequested { tab } => {
                self.native_player_return_requested(tab);
            }
            EngineEvent::Asked(ask) => self.asked(ask),
            EngineEvent::Found(found) => self.found(&found),
            EngineEvent::Targets {
                tab,
                ticket,
                targets,
            } => self.show_hints(tab, ticket, &targets),
            EngineEvent::Followed {
                tab,
                ticket,
                followed,
            } => self.followed(tab, ticket, followed),
            EngineEvent::Text { .. } => {}
            EngineEvent::Pictured { tab } => self.thumbnail_ready(tab),
            EngineEvent::Blocked { tab, requests } => {
                self.shields.borrow_mut().record_blocked(tab, requests);
                drained.blocked = true;
            }
            EngineEvent::Download(event) => {
                drained.downloads = true;
                self.download_report(event);
            }
        }
    }

    /// A page's navigation into the session, and, for a commit, a visit into history; a later
    /// title renames the visit. A tab closed a moment ago records nothing.
    fn record_navigation(
        &self,
        tab: TabId,
        url: &Url,
        title: &str,
        committed: bool,
        drained: &mut Drained,
    ) {
        drained.navigated = true;
        self.palette_sources_changed();
        let jar = {
            let mut browsing = self.browsing.borrow_mut();
            if let Err(error) = browsing.navigated(tab, url.clone(), title.to_owned()) {
                drained.failures.push(error);
                return;
            }
            browsing.session().tab(tab).map(|tab| tab.jar)
        };
        let Some(jar) = jar else {
            return;
        };
        let mut library = self.library.borrow_mut();
        let recorded = if committed {
            library.record_visit(url, title, jar).map(drop)
        } else {
            library.retitle_visit(url, title, jar).map(drop)
        };
        if let Err(error) = recorded {
            drained.failures.push(error);
        }
    }

    /// What a drain changed, on screen once.
    fn settle_drained(&self, drained: &Drained) {
        for error in &drained.failures {
            self.report(error);
        }
        if drained.navigated {
            self.show_zooms(Some(&drained.committed));
            for &tab in &drained.committed {
                self.applied(tab);
            }
            self.refresh_titles();
        }
        if drained.downloads {
            self.refresh_downloads();
        }
        if drained.blocked {
            self.refresh_shields_popover();
        }
        self.present_asks();
        self.page_changes();
        self.arrange_extensions();
    }

    /// A page's process is gone and the page blank. The tab stays; its window says so now if
    /// it shows the tab, else when it comes forward.
    fn crashed(&self, tab: TabId) {
        self.mcp_process_terminated(tab);
        self.agent_document_lost(tab);
        match self.showing(tab) {
            Some(window) => self.toast_in(window, CRASHED),
            None => {
                self.crashed.borrow_mut().insert(tab);
            }
        }
    }

    /// A tab came forward in `window`: if its page crashed in the background, the window says
    /// so.
    pub(super) fn crash_notice(&self, window: WindowId, tab: TabId) {
        if self.crashed.borrow_mut().remove(&tab) {
            self.toast_in(window, CRASHED);
        }
    }

    /// The window whose active tab `tab` is.
    pub(in crate::browser) fn showing(&self, tab: TabId) -> Option<WindowId> {
        let windows: Vec<WindowId> = self.windows.borrow().keys().copied().collect();
        windows
            .into_iter()
            .find(|&window| self.active(window) == Some(tab))
    }

    fn toast_in(&self, window: WindowId, text: &str) {
        if self.page_fullscreen(window) {
            return;
        }
        if let Some(host) = self.window(window).and_then(|window| window.contentView()) {
            self.toasts.show(&host, text);
        }
    }
}
