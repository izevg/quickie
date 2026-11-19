//! What pages ask the user: JavaScript dialogs and camera or microphone requests, as E3 sheets
//! attached to the page's window. The `Prompts` use case decides what is asked at all (a page's
//! second dialog while one is open, a blocked page, a site allowed before); the rest waits in
//! arrival order and shows when its tab is the window's active one. A sheet belongs to its tab:
//! switching away hides it, coming back shows it again, and closing the tab declines it.
//!
//! Keys on a sheet: `⏎` OK, `⎋` cancel or block (the safe default), `1` / `2` the numbered
//! permission answers, `⌘B` blocks a page's dialogs. Tab and window shortcuts still work; the
//! page under the sheet gets nothing.

use std::cell::RefCell;
use std::collections::BTreeMap;

use objc2::MainThreadMarker;
use quickie_application::{DialogGate, PermissionAnswer, PermissionGate, Prompts};
use quickie_domain_browsing::{TabId, WindowId};
use quickie_domain_commands::KeyStroke;
use quickie_engine_webkit::{Answer, Ask, AskId, AskKind, Dialog};
use quickie_ui_widgets::{DialogChoice, DialogKind, DialogRequest, DialogSheet, PermissionSheet};

use super::super::Browser;
use crate::input::keystroke::{SheetKey, sheet_key};

/// An ask waiting for the user.
struct Queued {
    ask: Ask,
    /// A dialog offers "Block dialogs from this page".
    offer_block: bool,
}

/// A window's two sheets, and the ask on screen in it.
struct WindowSheets {
    dialog: DialogSheet,
    permission: PermissionSheet,
    shown: Option<AskId>,
}

/// The asks waiting for the user and the sheets that show them.
pub struct PageAsks {
    prompts: RefCell<Prompts>,
    queue: RefCell<Vec<Queued>>,
    windows: RefCell<BTreeMap<WindowId, WindowSheets>>,
    mtm: MainThreadMarker,
}

impl PageAsks {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            prompts: RefCell::new(Prompts::new()),
            queue: RefCell::new(Vec::new()),
            windows: RefCell::new(BTreeMap::new()),
            mtm,
        }
    }
}

/// What a key on a sheet comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetRoute {
    /// The sheet took it, or nobody may have it.
    Taken,
    /// The sheet's text field types it.
    Field,
    /// A shortcut for the keymap; only tab and window commands may run.
    Keymap,
}

impl Browser {
    /// Takes what pages asked since the last time: answered at once when `Prompts` says so,
    /// else queued for a sheet.
    pub(in crate::browser) fn take_asks(&self) {
        let asks = {
            let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
                return;
            };
            browsing.engine_mut().take_asks()
        };
        for ask in asks {
            let mut prompts = self.page.asks.prompts.borrow_mut();
            let offer_block = match &ask.kind {
                AskKind::Dialog(_) => match prompts.dialog(ask.tab) {
                    DialogGate::Cancel => {
                        drop(prompts);
                        self.reply(ask.id, &Answer::Decline);
                        continue;
                    }
                    DialogGate::Show { offer_block } => offer_block,
                },
                AskKind::Permission(permission) => {
                    if prompts.permission(&ask.origin, *permission) == PermissionGate::Allow {
                        drop(prompts);
                        self.reply(ask.id, &Answer::Accept(None));
                        continue;
                    }
                    false
                }
            };
            drop(prompts);
            self.page
                .asks
                .queue
                .borrow_mut()
                .push(Queued { ask, offer_block });
        }
    }

    /// Puts each window's sheet on screen: the oldest ask of its active tab, or none. Asks of
    /// closed tabs leave the queue.
    pub(in crate::browser) fn present_asks(&self) {
        let active: Vec<(WindowId, Option<TabId>)> = {
            let Ok(browsing) = self.browsing.try_borrow() else {
                return;
            };
            let session = browsing.session();
            let mut queue = self.page.asks.queue.borrow_mut();
            let mut prompts = self.page.asks.prompts.borrow_mut();
            queue.retain(|queued| {
                let live = session.tab(queued.ask.tab).is_some();
                if !live {
                    prompts.forget(queued.ask.tab);
                }
                live
            });
            self.windows
                .borrow()
                .keys()
                .map(|&window| (window, session.active(window)))
                .collect()
        };
        self.page
            .asks
            .windows
            .borrow_mut()
            .retain(|window, _| active.iter().any(|(live, _)| live == window));
        for (window, tab) in active {
            let ask = tab.and_then(|tab| {
                let queue = self.page.asks.queue.borrow();
                queue
                    .iter()
                    .find(|queued| queued.ask.tab == tab)
                    .map(|queued| (queued.ask.clone(), queued.offer_block))
            });
            self.present(window, ask);
        }
    }

    /// True when a sheet shows in `window`.
    pub(in crate::browser) fn sheet_open(&self, window: WindowId) -> bool {
        self.page
            .asks
            .windows
            .borrow()
            .get(&window)
            .is_some_and(|sheets| sheets.shown.is_some())
    }

    /// Routes a key while the current window shows a sheet; `None` when it shows none.
    pub(in crate::browser) fn sheet_route(&self, stroke: KeyStroke) -> Option<SheetRoute> {
        let (window, _) = self.current()?;
        let (id, is_dialog, has_field, composing) = {
            let windows = self.page.asks.windows.borrow();
            let sheets = windows.get(&window)?;
            let id = sheets.shown?;
            (
                id,
                sheets.dialog.is_open(),
                sheets.dialog.field().is_some(),
                sheets.dialog.is_composing(),
            )
        };
        if composing {
            return Some(SheetRoute::Field);
        }
        let Some(key) = sheet_key(stroke) else {
            return Some(SheetRoute::Keymap);
        };
        match (key, is_dialog) {
            (SheetKey::Accept, true) => self.answer_dialog(window, id, DialogChoice::Ok),
            (SheetKey::Dismiss, true) => self.answer_dialog(window, id, DialogChoice::Cancel),
            (SheetKey::Block, true) if self.offers_block(id) => {
                self.answer_dialog(window, id, DialogChoice::Block);
            }
            (SheetKey::Typing | SheetKey::Number(_), true) if has_field => {
                return Some(SheetRoute::Field);
            }
            (SheetKey::Number(1), false) => {
                self.answer_permission(window, id, PermissionAnswer::AllowOnce);
            }
            (SheetKey::Number(2), false) => {
                self.answer_permission(window, id, PermissionAnswer::AllowOnSite);
            }
            (SheetKey::Dismiss, false) => {
                self.answer_permission(window, id, PermissionAnswer::Deny);
            }
            _ => {}
        }
        Some(SheetRoute::Taken)
    }

    /// True when a key the keymap cannot name must not go on: the current window shows a
    /// sheet without a field to type it.
    pub(in crate::browser) fn sheet_takes_unnamed(&self) -> bool {
        let Some((window, _)) = self.current() else {
            return false;
        };
        let windows = self.page.asks.windows.borrow();
        windows.get(&window).is_some_and(|sheets| {
            sheets.shown.is_some() && !(sheets.dialog.is_open() && sheets.dialog.field().is_some())
        })
    }

    fn offers_block(&self, id: AskId) -> bool {
        self.page
            .asks
            .queue
            .borrow()
            .iter()
            .any(|queued| queued.ask.id == id && queued.offer_block)
    }

    fn present(&self, window: WindowId, ask: Option<(Ask, bool)>) {
        let shown = self
            .page
            .asks
            .windows
            .borrow()
            .get(&window)
            .and_then(|sheets| sheets.shown);
        let wanted = ask.as_ref().map(|(ask, _)| ask.id);
        if shown == wanted {
            return;
        }
        {
            let mut windows = self.page.asks.windows.borrow_mut();
            let mtm = self.page.asks.mtm;
            let sheets = windows.entry(window).or_insert_with(|| WindowSheets {
                dialog: DialogSheet::new(mtm),
                permission: PermissionSheet::new(mtm),
                shown: None,
            });
            sheets.dialog.hide();
            sheets.permission.hide();
            sheets.shown = wanted;
        }
        let Some((ask, offer_block)) = ask else {
            return;
        };
        let Some(ns_window) = self.window(window) else {
            return;
        };
        let Some(host) = ns_window.contentView() else {
            return;
        };
        // An E3 collapses every E2 under it.
        self.close_overlays();
        self.hide_find();
        self.hide_navigation();
        let id = ask.id;
        let windows = self.page.asks.windows.borrow();
        let Some(sheets) = windows.get(&window) else {
            return;
        };
        let this = self.this.clone();
        match ask.kind {
            AskKind::Dialog(dialog) => {
                let (kind, message) = match dialog {
                    Dialog::Alert { message } => (DialogKind::Alert, message),
                    Dialog::Confirm { message } => (DialogKind::Confirm, message),
                    Dialog::Prompt { message, default } => {
                        (DialogKind::Prompt { default }, message)
                    }
                };
                let request = DialogRequest {
                    origin: ask.origin,
                    kind,
                    message,
                    offer_block,
                };
                sheets
                    .dialog
                    .show(&host, &ns_window, &request, move |choice| {
                        if let Some(browser) = this.upgrade() {
                            browser.answer_dialog(window, id, choice);
                        }
                    });
            }
            AskKind::Permission(permission) => {
                sheets
                    .permission
                    .show(&host, &ns_window, &ask.origin, permission, move |answer| {
                        if let Some(browser) = this.upgrade() {
                            browser.answer_permission(window, id, answer);
                        }
                    });
            }
        }
    }

    /// Takes the ask `id` off the queue if `window` still shows it.
    fn take_shown(&self, window: WindowId, id: AskId) -> Option<Ask> {
        let shown = self
            .page
            .asks
            .windows
            .borrow()
            .get(&window)
            .and_then(|sheets| sheets.shown);
        if shown != Some(id) {
            return None;
        }
        let mut queue = self.page.asks.queue.borrow_mut();
        let index = queue.iter().position(|queued| queued.ask.id == id)?;
        Some(queue.remove(index).ask)
    }

    fn answer_dialog(&self, window: WindowId, id: AskId, choice: DialogChoice) {
        let Some(ask) = self.take_shown(window, id) else {
            return;
        };
        let text = self
            .page
            .asks
            .windows
            .borrow()
            .get(&window)
            .and_then(|sheets| sheets.dialog.text());
        let answer = match choice {
            DialogChoice::Ok => Answer::Accept(text),
            DialogChoice::Cancel | DialogChoice::Block => Answer::Decline,
        };
        self.page
            .asks
            .prompts
            .borrow_mut()
            .dialog_closed(ask.tab, choice == DialogChoice::Block);
        self.reply(id, &answer);
        self.settle(window);
    }

    fn answer_permission(&self, window: WindowId, id: AskId, answer: PermissionAnswer) {
        let Some(ask) = self.take_shown(window, id) else {
            return;
        };
        let AskKind::Permission(permission) = ask.kind else {
            return;
        };
        let granted =
            self.page
                .asks
                .prompts
                .borrow_mut()
                .answer_permission(&ask.origin, permission, answer);
        let reply = if granted {
            Answer::Accept(None)
        } else {
            Answer::Decline
        };
        self.reply(id, &reply);
        self.settle(window);
    }

    /// Sends an answer to the page that waits on `id`.
    fn reply(&self, id: AskId, answer: &Answer) {
        if let Ok(mut browsing) = self.browsing.try_borrow_mut() {
            browsing.engine_mut().answer(id, answer);
        }
    }

    /// The tab committed a navigation: its dialogs count afresh.
    pub(in crate::browser) fn asks_navigated(&self, tab: TabId) {
        self.page.asks.prompts.borrow_mut().navigated(tab);
    }
}
