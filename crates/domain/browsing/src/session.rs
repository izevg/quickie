//! The session aggregate: every live tab of a profile, its windows, slots, and trash.

mod mru;
mod snapshot;
#[cfg(test)]
mod tests;
mod trash;

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use quickie_domain_kernel::{Profile, TabId, Timestamp, WindowId};
use url::Url;

use crate::{BrowsingError, Focus, Jar, JarChoice, JarId, PinnedSlot, Tab};
use mru::MruStack;
pub use snapshot::{CorruptSession, NextIds, SessionSnapshot};
use trash::Trash;
pub use trash::{ClosedTab, RETENTION_MS};

/// What closing a tab changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Closed {
    /// The window the tab was in.
    pub window: WindowId,
    /// The window's new active tab; `None` when the window is now empty, and the use case
    /// decides whether to close it.
    pub next_active: Option<TabId>,
    /// A throwaway jar no live tab uses any more: the engine destroys its partition.
    pub released_jar: Option<JarId>,
}

/// What closing a window changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedWindow {
    /// Its tabs, least recent first: the order they entered the trash.
    pub tabs: Vec<TabId>,
    /// Throwaway jars no live tab uses any more.
    pub released_jars: Vec<JarId>,
}

/// One profile's browsing state. Every mutation is one method, so the store saves one
/// consistent aggregate per command.
///
/// Every id it hands out names its profile, and it knows only ids of its own profile: a tab or
/// a window of another profile is as unknown to it as a closed one.
///
/// Tabs are shared, not owned: a use case commits a copy of the session and the store keeps a
/// snapshot of it, and each costs a reference count per tab instead of a copy of every URL and
/// title. A mutation copies only the tab it changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    profile: Profile,
    tabs: IndexMap<TabId, Arc<Tab>>,
    windows: IndexMap<WindowId, MruStack>,
    /// The window of each live tab. The MRU orders say the same, but only through a scan of
    /// every window.
    placement: HashMap<TabId, WindowId>,
    slots: [Option<TabId>; PinnedSlot::COUNT],
    trash: Trash,
    next: NextIds,
}

impl Session {
    /// An empty session of a profile: no windows, no tabs.
    #[must_use]
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            tabs: IndexMap::new(),
            windows: IndexMap::new(),
            placement: HashMap::new(),
            slots: [None; PinnedSlot::COUNT],
            trash: Trash::default(),
            next: NextIds::default(),
        }
    }

    /// The profile whose tabs these are.
    #[must_use]
    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Opens an empty window.
    pub fn open_window(&mut self) -> WindowId {
        let id = WindowId::new(self.profile, self.next.window);
        self.next.window += 1;
        self.windows.insert(id, MruStack::default());
        id
    }

    /// Opens a tab in a window. A background tab joins the back of the MRU order; in an empty
    /// window it is active anyway.
    ///
    /// # Errors
    ///
    /// [`BrowsingError::UnknownWindow`] for a window that is not open,
    /// [`BrowsingError::UnknownTab`] for a [`JarChoice::SameAs`] opener that is not live.
    pub fn open(
        &mut self,
        window: WindowId,
        url: Url,
        jar: JarChoice,
        focus: Focus,
    ) -> Result<TabId, BrowsingError> {
        if !self.windows.contains_key(&window) {
            return Err(BrowsingError::UnknownWindow(window));
        }
        let jar = match jar {
            JarChoice::Main => Jar::Main,
            JarChoice::NewThrowaway => self.new_jar(),
            JarChoice::SameAs(opener) => self.live(opener)?.jar,
        };
        let id = TabId::new(self.profile, self.next.tab);
        self.next.tab += 1;
        self.tabs.insert(
            id,
            Arc::new(Tab {
                id,
                url,
                title: String::new(),
                jar,
            }),
        );
        self.enter(window, id, focus);
        Ok(id)
    }

    /// Makes a tab the active one of its window and returns that window, for the shell to
    /// focus.
    ///
    /// # Errors
    ///
    /// [`BrowsingError::UnknownTab`] for a tab that is not live.
    pub fn activate(&mut self, id: TabId) -> Result<WindowId, BrowsingError> {
        let window = self.placed(id)?;
        if let Some(mru) = self.windows.get_mut(&window) {
            mru.touch(id);
        }
        Ok(window)
    }

    /// Closes a tab: it leaves its window and slot and goes to the trash.
    ///
    /// # Errors
    ///
    /// [`BrowsingError::UnknownTab`] for a tab that is not live.
    pub fn close(&mut self, id: TabId, now: Timestamp) -> Result<Closed, BrowsingError> {
        let window = self.placed(id)?;
        let released_jar = self.discard(id, now);
        let next_active = self.windows.get(&window).and_then(MruStack::front);
        Ok(Closed {
            window,
            next_active,
            released_jar,
        })
    }

    /// Closes a window with all its tabs. They enter the trash least recent first, so `⌘⇧T`
    /// brings back the window's active tab first.
    ///
    /// # Errors
    ///
    /// [`BrowsingError::UnknownWindow`] for a window that is not open.
    pub fn close_window(
        &mut self,
        window: WindowId,
        now: Timestamp,
    ) -> Result<ClosedWindow, BrowsingError> {
        let mru = self
            .windows
            .shift_remove(&window)
            .ok_or(BrowsingError::UnknownWindow(window))?;
        let tabs: Vec<TabId> = mru.as_slice().iter().rev().copied().collect();
        let released_jars = tabs
            .iter()
            .filter_map(|&id| self.discard(id, now))
            .collect();
        Ok(ClosedWindow {
            tabs,
            released_jars,
        })
    }

    /// Reopens the most recently closed tab, active, in the given window. It keeps its id, so
    /// its navigation history comes back with it. A throwaway tab gets a fresh jar: the old one
    /// is gone, and two tabs that shared a jar come back in separate ones.
    ///
    /// # Errors
    ///
    /// [`BrowsingError::UnknownWindow`] for a window that is not open.
    pub fn reopen_closed(&mut self, into: WindowId) -> Result<Option<TabId>, BrowsingError> {
        if !self.windows.contains_key(&into) {
            return Err(BrowsingError::UnknownWindow(into));
        }
        let Some(closed) = self.trash.pop_latest() else {
            return Ok(None);
        };
        let ClosedTab { mut tab, .. } = Arc::unwrap_or_clone(closed);
        if let Jar::Throwaway(_) = tab.jar {
            tab.jar = self.new_jar();
        }
        let id = tab.id;
        self.tabs.insert(id, Arc::new(tab));
        self.enter(into, id, Focus::Foreground);
        Ok(Some(id))
    }

    /// Records a committed navigation reported by the engine.
    ///
    /// # Errors
    ///
    /// [`BrowsingError::UnknownTab`] for a tab that is not live (often closed a moment ago).
    pub fn navigated(&mut self, id: TabId, url: Url, title: String) -> Result<(), BrowsingError> {
        let shared = self
            .tabs
            .get_mut(&id)
            .ok_or(BrowsingError::UnknownTab(id))?;
        let tab = Arc::make_mut(shared);
        tab.url = url;
        tab.title = title;
        Ok(())
    }

    /// Removes and returns the trash entries older than [`RETENTION_MS`]; the store drops their
    /// navigation history.
    pub fn purge_trash(&mut self, now: Timestamp) -> Vec<ClosedTab> {
        self.trash
            .purge(now)
            .into_iter()
            .map(Arc::unwrap_or_clone)
            .collect()
    }

    /// Pins a tab to a slot and returns the tab it displaced, the way a vim mark is overwritten.
    /// A tab holds at most one slot; pinning it again moves it.
    ///
    /// # Errors
    ///
    /// [`BrowsingError::UnknownTab`] for a tab that is not live.
    pub fn pin(&mut self, id: TabId, slot: PinnedSlot) -> Result<Option<TabId>, BrowsingError> {
        self.live(id)?;
        self.clear_slot_of(id);
        Ok(self.slots[slot.index()].replace(id))
    }

    /// Empties a slot and returns the tab that held it.
    pub fn unpin(&mut self, slot: PinnedSlot) -> Option<TabId> {
        self.slots[slot.index()].take()
    }

    /// The tab in a slot.
    #[must_use]
    pub fn in_slot(&self, slot: PinnedSlot) -> Option<TabId> {
        self.slots[slot.index()]
    }

    /// The slot a tab holds.
    #[must_use]
    pub fn slot_of(&self, id: TabId) -> Option<PinnedSlot> {
        self.slots
            .iter()
            .position(|&held| held == Some(id))
            .and_then(PinnedSlot::from_index)
    }

    /// A live tab.
    #[must_use]
    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.get(&id).map(Arc::as_ref)
    }

    /// The window a live tab is in; `None` for a tab that is not live.
    #[must_use]
    pub fn window_of(&self, id: TabId) -> Option<WindowId> {
        self.placement.get(&id).copied()
    }

    /// A window's tabs in opening order, as the tab overview lays them out.
    ///
    /// # Errors
    ///
    /// [`BrowsingError::UnknownWindow`] for a window that is not open.
    pub fn tabs(&self, window: WindowId) -> Result<impl Iterator<Item = &Tab>, BrowsingError> {
        if !self.windows.contains_key(&window) {
            return Err(BrowsingError::UnknownWindow(window));
        }
        Ok(self
            .tabs
            .values()
            .filter(move |tab| self.window_of(tab.id) == Some(window))
            .map(Arc::as_ref))
    }

    /// A window's tabs, most recent first: the input of a [`crate::TabCycle`].
    #[must_use]
    pub fn mru(&self, window: WindowId) -> Option<&[TabId]> {
        self.windows.get(&window).map(MruStack::as_slice)
    }

    /// A window's active tab: the front of its MRU order.
    #[must_use]
    pub fn active(&self, window: WindowId) -> Option<TabId> {
        self.windows.get(&window).and_then(MruStack::front)
    }

    /// Open windows, in opening order.
    pub fn windows(&self) -> impl Iterator<Item = WindowId> {
        self.windows.keys().copied()
    }

    /// The trash, oldest first.
    pub fn trash(&self) -> impl Iterator<Item = &ClosedTab> {
        self.trash.iter().map(Arc::as_ref)
    }

    fn live(&self, id: TabId) -> Result<&Tab, BrowsingError> {
        self.tab(id).ok_or(BrowsingError::UnknownTab(id))
    }

    fn placed(&self, id: TabId) -> Result<WindowId, BrowsingError> {
        self.window_of(id).ok_or(BrowsingError::UnknownTab(id))
    }

    fn new_jar(&mut self) -> Jar {
        let jar = JarId::new(self.next.jar);
        self.next.jar += 1;
        Jar::Throwaway(jar)
    }

    /// Puts a tab into an open window.
    fn enter(&mut self, window: WindowId, id: TabId, focus: Focus) {
        if let Some(mru) = self.windows.get_mut(&window) {
            match focus {
                Focus::Foreground => mru.push_front(id),
                Focus::Background => mru.push_back(id),
            }
            self.placement.insert(id, window);
        }
    }

    fn clear_slot_of(&mut self, id: TabId) {
        for held in &mut self.slots {
            if *held == Some(id) {
                *held = None;
            }
        }
    }

    /// Moves a live tab to the trash and returns its jar if no live tab uses it any more.
    fn discard(&mut self, id: TabId, now: Timestamp) -> Option<JarId> {
        // `close_window` takes the whole window out first; then only the placement goes.
        if let Some(window) = self.placement.remove(&id)
            && let Some(mru) = self.windows.get_mut(&window)
        {
            mru.remove(id);
        }
        self.clear_slot_of(id);
        let tab = Arc::unwrap_or_clone(self.tabs.shift_remove(&id)?);
        let released = match tab.jar {
            Jar::Throwaway(jar) if !self.tabs.values().any(|t| t.jar == tab.jar) => Some(jar),
            Jar::Main | Jar::Throwaway(_) => None,
        };
        self.trash.push(Arc::new(ClosedTab {
            tab,
            closed_at: now,
        }));
        released
    }
}
