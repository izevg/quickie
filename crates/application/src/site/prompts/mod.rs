//! What pages ask the user: JavaScript dialogs (`alert`, `confirm`, `prompt`) and permissions
//! (camera, microphone). The policy over what the user answered, and the asks that wait for
//! the user; the shell shows the sheets, and the engine's [`PageAsks`] carries every answer back
//! to the page.
//!
//! - What the user already chose is answered at once, without a sheet; everything else waits
//!   in the order pages asked, and each tab's oldest ask is the one its sheet shows.
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

use crate::{Answer, Ask, AskKind, PageAsks, ReplyToken};

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

/// The user's answer on a dialog sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogAnswer {
    /// `⏎`: OK, with a prompt's text.
    Ok(Option<String>),
    /// `⎋`: the safe default.
    Cancel,
    /// `⌘B`: cancel, and cancel the page's next dialogs without a sheet.
    Block,
}

/// An ask that waits for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub ask: Ask,
    /// A dialog offers "Block dialogs from this page".
    pub offer_block: bool,
}

/// A tab's dialogs since it last navigated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Run {
    shown: u32,
    open: bool,
    blocked: bool,
}

/// The answers the user gave pages, the dialogs they have open, and the asks that wait.
#[derive(Debug, Default)]
pub struct Prompts {
    /// Devices allowed per origin.
    // TODO(shields): persist through the shields' `SitePolicy` instead of memory.
    allowed: HashSet<(Origin, Permission)>,
    runs: HashMap<TabId, Run>,
    /// What waits for the user, oldest first.
    waiting: Vec<Waiting>,
}

impl Prompts {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A page asked. What the user already chose is answered at once through `pages`: a
    /// dialog of a page that has one open or whose dialogs are blocked is cancelled, a device
    /// the origin was allowed on the site is granted. Anything else waits for the user.
    pub fn asked(&mut self, ask: Ask, pages: &mut impl PageAsks) {
        let offer_block = match &ask.kind {
            AskKind::Dialog(_) => match self.dialog(ask.tab) {
                DialogGate::Cancel => {
                    pages.answer(ask.token, &Answer::Decline);
                    return;
                }
                DialogGate::Show { offer_block } => offer_block,
            },
            AskKind::Permission(permission) => {
                if self.permission(&ask.origin, *permission) == PermissionGate::Allow {
                    pages.answer(ask.token, &Answer::Accept(None));
                    return;
                }
                false
            }
        };
        self.waiting.push(Waiting { ask, offer_block });
    }

    /// The oldest ask of `tab` that waits for the user: the one its sheet shows.
    #[must_use]
    pub fn waiting(&self, tab: TabId) -> Option<&Waiting> {
        self.waiting.iter().find(|waiting| waiting.ask.tab == tab)
    }

    /// True when the dialog `token` names waits and offers to block the page's dialogs.
    #[must_use]
    pub fn offers_block(&self, token: ReplyToken) -> bool {
        self.waiting
            .iter()
            .any(|waiting| waiting.ask.token == token && waiting.offer_block)
    }

    /// Forgets the tabs that are no longer `open`: their asks, which their pages took with
    /// them, and their dialogs.
    pub fn keep_tabs(&mut self, open: impl Fn(TabId) -> bool) {
        self.waiting.retain(|waiting| open(waiting.ask.tab));
        self.runs.retain(|&tab, _| open(tab));
    }

    /// The user answered the dialog `token` names: its page hears OK with the text, or Cancel,
    /// through `pages`, and blocking cancels the page's next dialogs. False when no such
    /// dialog waits.
    pub fn dialog_answered(
        &mut self,
        token: ReplyToken,
        answer: DialogAnswer,
        pages: &mut impl PageAsks,
    ) -> bool {
        let Some(at) = self.waiting.iter().position(|waiting| {
            waiting.ask.token == token && matches!(waiting.ask.kind, AskKind::Dialog(_))
        }) else {
            return false;
        };
        let tab = self.waiting.remove(at).ask.tab;
        self.dialog_closed(tab, answer == DialogAnswer::Block);
        let reply = match answer {
            DialogAnswer::Ok(text) => Answer::Accept(text),
            DialogAnswer::Cancel | DialogAnswer::Block => Answer::Decline,
        };
        pages.answer(token, &reply);
        true
    }

    /// The user answered the device request `token` names: its page gets the devices or not,
    /// through `pages`, and "allow on this site" is remembered. False when no such request
    /// waits.
    pub fn permission_answered(
        &mut self,
        token: ReplyToken,
        answer: PermissionAnswer,
        pages: &mut impl PageAsks,
    ) -> bool {
        let Some((at, permission)) =
            self.waiting
                .iter()
                .enumerate()
                .find_map(|(at, waiting)| match waiting.ask.kind {
                    AskKind::Permission(permission) if waiting.ask.token == token => {
                        Some((at, permission))
                    }
                    _ => None,
                })
        else {
            return false;
        };
        let origin = self.waiting.remove(at).ask.origin;
        let granted = self.answer_permission(&origin, permission, answer);
        let reply = if granted {
            Answer::Accept(None)
        } else {
            Answer::Decline
        };
        pages.answer(token, &reply);
        true
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
}
