//! What pages ask the user: JavaScript dialogs (`alert`, `confirm`, `prompt`) and permissions
//! (camera, microphone). Pure policy over what the user answered; the shell shows the sheets and
//! the engine carries the answers back to the page.
//!
//! - A page shows one dialog at a time; another one while it is open is cancelled.
//! - From the second dialog since the page last navigated, the sheet offers to block the page's
//!   dialogs; once blocked, they are cancelled without a sheet until the page navigates.
//! - "Allow on this site" is remembered for the origin, in memory for now; "allow once" and
//!   "deny" (the safe default) are not.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use quickie_domain_browsing::TabId;
use url::{Origin, Url};

/// A device a page wants to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    Camera,
    Microphone,
    CameraAndMicrophone,
}

impl Permission {
    /// The devices it covers, each on its own.
    fn devices(self) -> &'static [Self] {
        match self {
            Self::Camera => &[Self::Camera],
            Self::Microphone => &[Self::Microphone],
            Self::CameraAndMicrophone => &[Self::Camera, Self::Microphone],
        }
    }
}

/// The user's answer on a permission sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionAnswer {
    /// `1`: this request only.
    AllowOnce,
    /// `2`: this request and every later one from the same origin.
    AllowOnSite,
    /// `⎋`: the safe default.
    Deny,
}

/// What to do with a permission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionGate {
    /// The user allowed it on this site before: grant without asking.
    Allow,
    /// Show the sheet.
    Ask,
}

/// What to do with a dialog a page opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogGate {
    /// Show it; `offer_block` adds "Block dialogs from this page".
    Show { offer_block: bool },
    /// Cancel it without a sheet: the page has one open, or the user blocked its dialogs.
    Cancel,
}

/// A tab's dialogs since it last navigated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Run {
    shown: u32,
    open: bool,
    blocked: bool,
}

/// The answers the user gave pages, and the dialogs they have open.
#[derive(Debug, Default)]
pub struct Prompts {
    /// Devices allowed per origin.
    // TODO(shields): persist through the shields' `SitePolicy` instead of memory.
    allowed: HashSet<(Origin, Permission)>,
    runs: HashMap<TabId, Run>,
}

impl Prompts {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the page at `origin` may use `permission` without asking.
    #[must_use]
    pub fn permission(&self, origin: &Url, permission: Permission) -> PermissionGate {
        let origin = origin.origin();
        let allowed = origin.is_tuple()
            && permission
                .devices()
                .iter()
                .all(|&device| self.allowed.contains(&(origin.clone(), device)));
        if allowed {
            PermissionGate::Allow
        } else {
            PermissionGate::Ask
        }
    }

    /// Records the user's answer; true when the page gets the devices. An origin that is not
    /// a website (a `data:` or `file:` page) is never remembered.
    pub fn answer_permission(
        &mut self,
        origin: &Url,
        permission: Permission,
        answer: PermissionAnswer,
    ) -> bool {
        let origin = origin.origin();
        match answer {
            PermissionAnswer::Deny => false,
            PermissionAnswer::AllowOnce => true,
            PermissionAnswer::AllowOnSite => {
                if origin.is_tuple() {
                    for &device in permission.devices() {
                        self.allowed.insert((origin.clone(), device));
                    }
                }
                true
            }
        }
    }

    /// A page in `tab` opens a dialog.
    pub fn dialog(&mut self, tab: TabId) -> DialogGate {
        let run = self.runs.entry(tab).or_default();
        if run.open || run.blocked {
            return DialogGate::Cancel;
        }
        run.shown += 1;
        run.open = true;
        DialogGate::Show {
            offer_block: run.shown > 1,
        }
    }

    /// The tab's dialog was answered; `block` when the user chose to block its next ones.
    pub fn dialog_closed(&mut self, tab: TabId, block: bool) {
        if let Some(run) = self.runs.get_mut(&tab) {
            run.open = false;
            run.blocked |= block;
        }
    }

    /// The tab committed a navigation: a new page, whose dialogs count afresh. A dialog still
    /// open stays open.
    pub fn navigated(&mut self, tab: TabId) {
        if let Some(run) = self.runs.get_mut(&tab) {
            *run = Run {
                open: run.open,
                ..Run::default()
            };
        }
    }

    /// The tab closed.
    pub fn forget(&mut self, tab: TabId) {
        self.runs.remove(&tab);
    }
}
