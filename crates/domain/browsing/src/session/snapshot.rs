//! The session as plain data: the only door between the store and the aggregate.
//!
//! Only the main profile has this door. The agent leaves an audit log, not tabs: its session
//! starts empty on every launch and is never saved, so [`Session::snapshot`] gives it no
//! snapshot and [`Session::restore`] rebuilds only a main session. No code path, by mistake or
//! otherwise, can hand an agent tab to the store.

use std::collections::HashSet;

use indexmap::IndexMap;
use quickie_domain_kernel::{Profile, TabId, WindowId};

use super::{ClosedTab, MruStack, Session, Trash};
use crate::{Jar, JarId, PinnedSlot, Tab};

/// The next free id of each kind. Ids are never reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NextIds {
    /// Next [`TabId`].
    pub tab: u64,
    /// Next [`WindowId`].
    pub window: u64,
    /// Next [`JarId`].
    pub jar: u64,
}

/// The main profile's whole session as plain data, for the store to write and read back. Every
/// id in it is of [`Profile::Main`]; there is no agent snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// Live tabs in opening order.
    pub tabs: Vec<Tab>,
    /// Open windows in opening order, each with its tabs most recent first.
    pub windows: Vec<(WindowId, Vec<TabId>)>,
    /// Slot `n` is at index `n - 1`.
    pub slots: [Option<TabId>; PinnedSlot::COUNT],
    /// Closed tabs, oldest first.
    pub trash: Vec<ClosedTab>,
    /// The id counters.
    pub next: NextIds,
}

/// A snapshot that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptSession {
    /// A tab, live or in the trash, of a profile other than main: only the main profile is
    /// ever saved.
    #[error("{0} is not of the main profile")]
    ForeignTab(TabId),
    /// A window of a profile other than main.
    #[error("{0} is not of the main profile")]
    ForeignWindow(WindowId),
    /// Two tabs, live or in the trash, share an id.
    #[error("{0} appears twice")]
    DuplicateTab(TabId),
    /// Two windows share an id.
    #[error("{0} appears twice")]
    DuplicateWindow(WindowId),
    /// A live tab is in no window or in more than one place.
    #[error("{0} is not in exactly one window")]
    TabNotInOneWindow(TabId),
    /// A window lists a tab that is not live.
    #[error("a window lists {0}, which is not live")]
    UnknownTabInWindow(TabId),
    /// A slot holds a tab that is not live.
    #[error("a slot holds {0}, which is not live")]
    UnknownTabInSlot(TabId),
    /// One tab holds two slots.
    #[error("{0} holds two slots")]
    TabInTwoSlots(TabId),
    /// A tab id at or above the tab counter: it could be handed out again.
    #[error("{0} is not below the tab counter")]
    TabIdNotAllocated(TabId),
    /// A window id at or above the window counter.
    #[error("{0} is not below the window counter")]
    WindowIdNotAllocated(WindowId),
    /// A jar id at or above the jar counter.
    #[error("jar {} is not below the jar counter", .0.get())]
    JarIdNotAllocated(JarId),
}

impl Session {
    /// Rebuilds the main profile's session from a snapshot, checking every invariant once,
    /// here.
    ///
    /// # Errors
    ///
    /// The first [`CorruptSession`] found.
    pub fn restore(snapshot: SessionSnapshot) -> Result<Self, CorruptSession> {
        let SessionSnapshot {
            tabs,
            windows,
            slots,
            trash,
            next,
        } = snapshot;
        check_ids(&tabs, &windows, &trash, next)?;
        let tabs: IndexMap<TabId, Tab> = tabs.into_iter().map(|tab| (tab.id, tab)).collect();
        check_windows(&tabs, &windows)?;
        check_slots(&tabs, &slots)?;
        Ok(Self {
            profile: Profile::Main,
            tabs,
            windows: windows
                .into_iter()
                .map(|(id, order)| (id, MruStack::from_order(order)))
                .collect(),
            slots,
            trash: Trash::from_oldest_first(trash),
            next,
        })
    }

    /// The main profile's session as plain data; `restore(snapshot())` gives back an equal
    /// session. `None` for the agent's session, which is never saved.
    #[must_use]
    pub fn snapshot(&self) -> Option<SessionSnapshot> {
        match self.profile {
            Profile::Main => Some(SessionSnapshot {
                tabs: self.tabs.values().cloned().collect(),
                windows: self
                    .windows
                    .iter()
                    .map(|(&id, mru)| (id, mru.as_slice().to_vec()))
                    .collect(),
                slots: self.slots,
                trash: self.trash.iter().cloned().collect(),
                next: self.next,
            }),
            // The agent leaves an audit log, not tabs: its session ends with the process.
            Profile::Agent => None,
        }
    }
}

fn check_ids(
    tabs: &[Tab],
    windows: &[(WindowId, Vec<TabId>)],
    trash: &[ClosedTab],
    next: NextIds,
) -> Result<(), CorruptSession> {
    let mut seen = HashSet::new();
    for tab in tabs.iter().chain(trash.iter().map(|closed| &closed.tab)) {
        if tab.id.profile() != Profile::Main {
            return Err(CorruptSession::ForeignTab(tab.id));
        }
        if !seen.insert(tab.id) {
            return Err(CorruptSession::DuplicateTab(tab.id));
        }
        if tab.id.get() >= next.tab {
            return Err(CorruptSession::TabIdNotAllocated(tab.id));
        }
        if let Jar::Throwaway(jar) = tab.jar
            && jar.get() >= next.jar
        {
            return Err(CorruptSession::JarIdNotAllocated(jar));
        }
    }
    let mut seen = HashSet::new();
    for &(window, _) in windows {
        if window.profile() != Profile::Main {
            return Err(CorruptSession::ForeignWindow(window));
        }
        if !seen.insert(window) {
            return Err(CorruptSession::DuplicateWindow(window));
        }
        if window.get() >= next.window {
            return Err(CorruptSession::WindowIdNotAllocated(window));
        }
    }
    Ok(())
}

fn check_windows(
    tabs: &IndexMap<TabId, Tab>,
    windows: &[(WindowId, Vec<TabId>)],
) -> Result<(), CorruptSession> {
    let mut placed = HashSet::new();
    for &id in windows.iter().flat_map(|(_, order)| order) {
        if !tabs.contains_key(&id) {
            return Err(CorruptSession::UnknownTabInWindow(id));
        }
        if !placed.insert(id) {
            return Err(CorruptSession::TabNotInOneWindow(id));
        }
    }
    match tabs.keys().find(|id| !placed.contains(*id)) {
        Some(&id) => Err(CorruptSession::TabNotInOneWindow(id)),
        None => Ok(()),
    }
}

fn check_slots(
    tabs: &IndexMap<TabId, Tab>,
    slots: &[Option<TabId>; PinnedSlot::COUNT],
) -> Result<(), CorruptSession> {
    let mut seen = HashSet::new();
    for &id in slots.iter().flatten() {
        if !tabs.contains_key(&id) {
            return Err(CorruptSession::UnknownTabInSlot(id));
        }
        if !seen.insert(id) {
            return Err(CorruptSession::TabInTwoSlots(id));
        }
    }
    Ok(())
}
