//! Browsing context: the tabs of a profile, the windows that hold them, most-recently-used order,
//! pinned slots `⌘1` … `⌘9`, and the trash that keeps closed tabs for 30 days.
//!
//! [`Session`] is the aggregate: one per profile, the unit the store saves in one transaction.
//! Chromium knows which tabs are live; this crate knows how they are organized. Pure: no I/O, no
//! clock (the caller passes [`Timestamp`]s), no async.

mod cycle;
mod session;
mod tab;

pub use cycle::{Direction, TabCycle};
pub use quickie_domain_kernel::{TabId, Timestamp, WindowId};
pub use session::{
    Closed, ClosedTab, ClosedWindow, CorruptSession, NextIds, RETENTION_MS, Session,
    SessionSnapshot,
};
pub use tab::{
    FindMatches, Focus, Jar, JarChoice, JarId, Origin, PinnedSlot, SlotOutOfRange, Tab, Transport,
    is_cross_origin, registrable_domain,
};

/// A command named a tab or a window the session does not have.
///
/// Engine events race with user commands, so a use case reconciling an engine event treats
/// these as benign: the tab was closed a moment earlier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BrowsingError {
    /// No live tab has this id.
    #[error("no live {0}")]
    UnknownTab(TabId),
    /// No open window has this id.
    #[error("no open {0}")]
    UnknownWindow(WindowId),
}
