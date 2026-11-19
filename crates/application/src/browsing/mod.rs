//! Browsing use cases: tabs, windows, slots, the trash, `⌃⇥`.
//!
//! The domain and the store are transactional: a command changes a copy of the session, the
//! copy is saved, and only then does it replace the session, so a failed save changes nothing.
//! The engine follows after the commit. An engine failure is reported, but the session keeps
//! the change and the engine catches up: pages are created lazily whenever a tab is shown.

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use quickie_domain_browsing::{
    BrowsingError, Closed, ClosedWindow, CorruptSession, Direction, Focus, JarChoice, JarId,
    PinnedSlot, Session, TabCycle, TabId, WindowId,
};
use url::Url;

use crate::{AppError, Clock, EngineError, EnginePort, Navigation, SessionRepo};

/// The browsing use cases over one profile's session.
#[derive(Debug)]
pub struct Browsing<E, R, C> {
    session: Session,
    /// Tabs whose page exists in the engine; restored tabs get one when first shown.
    pages: HashSet<TabId>,
    corrupt_on_start: Option<CorruptSession>,
    engine: E,
    repo: R,
    clock: C,
}

impl<E: EnginePort, R: SessionRepo, C: Clock> Browsing<E, R, C> {
    /// Restores the saved session, or starts a fresh one with a window on `home`. A corrupt
    /// save is replaced by a fresh session and reported by [`Self::corrupt_on_start`]. Every
    /// window ends up with a tab, and each window's active tab is shown.
    ///
    /// # Errors
    ///
    /// The store could not be read or written, or the engine could not show a tab.
    pub fn start(engine: E, mut repo: R, clock: C, home: &Url) -> Result<Self, AppError> {
        let (mut session, corrupt_on_start) = match repo.load()? {
            None => (Session::new(), None),
            Some(snapshot) => match Session::restore(snapshot) {
                Ok(session) => (session, None),
                Err(corrupt) => (Session::new(), Some(corrupt)),
            },
        };
        if session.windows().next().is_none() {
            session.open_window();
        }
        let empty: Vec<WindowId> = session
            .windows()
            .filter(|&window| session.active(window).is_none())
            .collect();
        for window in empty {
            session.open(window, home.clone(), JarChoice::Main, Focus::Foreground)?;
        }
        repo.save(&session.snapshot())?;
        let mut browsing = Self {
            session,
            pages: HashSet::new(),
            corrupt_on_start,
            engine,
            repo,
            clock,
        };
        let shown: Vec<(WindowId, TabId)> = browsing
            .session
            .windows()
            .filter_map(|window| browsing.session.active(window).map(|tab| (window, tab)))
            .collect();
        for (window, tab) in shown {
            browsing.show(window, tab)?;
        }
        Ok(browsing)
    }

    /// The session, for reading.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Why the saved session was discarded at start, if it was.
    #[must_use]
    pub fn corrupt_on_start(&self) -> Option<CorruptSession> {
        self.corrupt_on_start
    }

    /// The engine adapter, for the shell to reach its views.
    pub fn engine_mut(&mut self) -> &mut E {
        &mut self.engine
    }

    /// Opens an empty window; the caller opens its first tab.
    ///
    /// # Errors
    ///
    /// The store could not be written.
    pub fn open_window(&mut self) -> Result<WindowId, AppError> {
        self.commit(|session| Ok(session.open_window()))
    }

    /// Opens a tab the user asked for. Its page is created at once, so a background tab
    /// loads while the user stays where they are.
    ///
    /// # Errors
    ///
    /// The window or the opener is gone, the store failed, or the engine failed.
    pub fn open(
        &mut self,
        window: WindowId,
        url: Url,
        jar: JarChoice,
        focus: Focus,
    ) -> Result<TabId, AppError> {
        let tab = self.commit(|session| session.open(window, url, jar, focus))?;
        self.materialize(tab)?;
        self.show_if_active(window, tab)?;
        Ok(tab)
    }

    /// Adopts a tab a page opened (a popup, a ⌘-click). The engine already has its page; the
    /// returned id is how the adapter refers to it from now on. It stays in its opener's window
    /// and jar.
    ///
    /// # Errors
    ///
    /// The opener is gone, the store failed, or the engine failed to show the tab.
    pub fn adopt(&mut self, opener: TabId, url: Url, focus: Focus) -> Result<TabId, AppError> {
        let window = self.window_of(opener)?;
        let tab =
            self.commit(|session| session.open(window, url, JarChoice::SameAs(opener), focus))?;
        self.pages.insert(tab);
        self.show_if_active(window, tab)?;
        Ok(tab)
    }

    /// Activates a tab and shows it; returns the window to bring forward.
    ///
    /// # Errors
    ///
    /// The tab is gone, the store failed, or the engine failed.
    pub fn activate(&mut self, tab: TabId) -> Result<WindowId, AppError> {
        let window = self.commit(|session| session.activate(tab))?;
        self.show(window, tab)?;
        Ok(window)
    }

    /// Closes a tab into the trash, destroys its page and a jar it was the last user of, and
    /// shows the window's next tab. When `next_active` is `None` the window is empty and the
    /// shell decides whether to close it.
    ///
    /// # Errors
    ///
    /// The tab is gone, the store failed, or the engine failed. Every engine step is attempted
    /// even when an earlier one fails, so a throwaway jar is never left behind.
    pub fn close(&mut self, tab: TabId) -> Result<Closed, AppError> {
        let now = self.clock.now();
        let closed = self.commit(|session| session.close(tab, now))?;
        let mut outcome = self.close_page(tab);
        outcome = outcome.and(self.destroy_jars(closed.released_jar));
        if let Some(next) = closed.next_active {
            outcome = outcome.and(self.show(closed.window, next));
        }
        outcome.map(|()| closed)
    }

    /// Closes a window and all its tabs into the trash.
    ///
    /// # Errors
    ///
    /// The window is gone, the store failed, or the engine failed (every step is attempted).
    pub fn close_window(&mut self, window: WindowId) -> Result<ClosedWindow, AppError> {
        let now = self.clock.now();
        let closed = self.commit(|session| session.close_window(window, now))?;
        let mut outcome = Ok(());
        for &tab in &closed.tabs {
            outcome = outcome.and(self.close_page(tab));
        }
        outcome = outcome.and(self.destroy_jars(closed.released_jars.iter().copied()));
        outcome.map(|()| closed)
    }

    /// Reopens the most recently closed tab in a window and shows it.
    ///
    /// # Errors
    ///
    /// The window is gone, the store failed, or the engine failed.
    pub fn reopen_closed(&mut self, window: WindowId) -> Result<Option<TabId>, AppError> {
        let Some(tab) = self.commit(|session| session.reopen_closed(window))? else {
            return Ok(None);
        };
        self.show(window, tab)?;
        Ok(Some(tab))
    }

    /// Navigates a tab: load, back, forward, reload, stop. The session changes only when the
    /// engine reports the committed navigation through [`Self::navigated`].
    ///
    /// # Errors
    ///
    /// The tab is gone or the engine failed.
    pub fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), AppError> {
        self.materialize(tab)?;
        self.engine.navigate(tab, navigation)?;
        Ok(())
    }

    /// Records a navigation the engine committed. A tab closed a moment ago is not an error:
    /// engine events race with user commands.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn navigated(&mut self, tab: TabId, url: Url, title: String) -> Result<(), AppError> {
        match self.commit(|session| session.navigated(tab, url, title)) {
            Err(AppError::Browsing(BrowsingError::UnknownTab(_))) => Ok(()),
            other => other,
        }
    }

    /// Pins a tab to a slot; returns the tab it displaced.
    ///
    /// # Errors
    ///
    /// The tab is gone or the store failed.
    pub fn pin(&mut self, tab: TabId, slot: PinnedSlot) -> Result<Option<TabId>, AppError> {
        self.commit(|session| session.pin(tab, slot))
    }

    /// Empties a slot; returns the tab that held it.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn unpin(&mut self, slot: PinnedSlot) -> Result<Option<TabId>, AppError> {
        self.commit(|session| Ok(session.unpin(slot)))
    }

    /// `⌘1` … `⌘9`: activates the tab in a slot; `None` for an empty slot.
    ///
    /// # Errors
    ///
    /// The store or the engine failed.
    pub fn go_to_slot(&mut self, slot: PinnedSlot) -> Result<Option<WindowId>, AppError> {
        match self.session.in_slot(slot) {
            Some(tab) => self.activate(tab).map(Some),
            None => Ok(None),
        }
    }

    /// `⌃` went down with `⇥`: a cycle over the window's MRU order, fixed until `⌃` is
    /// released. `None` when there is no other tab.
    #[must_use]
    pub fn begin_cycle(&self, window: WindowId, direction: Direction) -> Option<TabCycle> {
        TabCycle::begin(self.session.mru(window)?, direction)
    }

    /// `⌃` was released: activates the selection.
    ///
    /// # Errors
    ///
    /// The selected tab closed during the cycle, or the store or the engine failed.
    pub fn commit_cycle(&mut self, cycle: &TabCycle) -> Result<WindowId, AppError> {
        self.activate(cycle.selected())
    }

    /// Drops trash entries past the 30-day retention; returns how many went.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn purge_trash(&mut self) -> Result<usize, AppError> {
        let now = self.clock.now();
        self.commit(|session| Ok(session.purge_trash(now).len()))
    }

    fn commit<T>(
        &mut self,
        change: impl FnOnce(&mut Session) -> Result<T, BrowsingError>,
    ) -> Result<T, AppError> {
        let mut next = self.session.clone();
        let outcome = change(&mut next)?;
        self.repo.save(&next.snapshot())?;
        self.session = next;
        Ok(outcome)
    }

    fn window_of(&self, tab: TabId) -> Result<WindowId, AppError> {
        self.session
            .windows()
            .find(|&window| {
                self.session
                    .mru(window)
                    .is_some_and(|mru| mru.contains(&tab))
            })
            .ok_or(AppError::Browsing(BrowsingError::UnknownTab(tab)))
    }

    fn materialize(&mut self, tab: TabId) -> Result<(), AppError> {
        if self.pages.contains(&tab) {
            return Ok(());
        }
        let page = self
            .session
            .tab(tab)
            .ok_or(BrowsingError::UnknownTab(tab))?;
        self.engine.create(tab, &page.url, page.jar)?;
        self.pages.insert(tab);
        Ok(())
    }

    fn show(&mut self, window: WindowId, tab: TabId) -> Result<(), AppError> {
        self.materialize(tab)?;
        self.engine.show(window, tab)?;
        Ok(())
    }

    fn show_if_active(&mut self, window: WindowId, tab: TabId) -> Result<(), AppError> {
        if self.session.active(window) == Some(tab) {
            self.show(window, tab)?;
        }
        Ok(())
    }

    fn close_page(&mut self, tab: TabId) -> Result<(), AppError> {
        if self.pages.remove(&tab) {
            self.engine.close(tab)?;
        }
        Ok(())
    }

    fn destroy_jars(&mut self, jars: impl IntoIterator<Item = JarId>) -> Result<(), AppError> {
        let mut outcome: Result<(), EngineError> = Ok(());
        for jar in jars {
            outcome = outcome.and(self.engine.destroy_jar(jar));
        }
        Ok(outcome?)
    }
}
