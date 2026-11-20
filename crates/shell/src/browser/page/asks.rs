//! What pages ask the user: JavaScript dialogs and camera or microphone requests, as E3 sheets
//! attached to the page's window. The `Prompts` use case decides what is asked at all (a page's
//! second dialog while one is open, a blocked page, a site allowed before), keeps the rest
//! waiting in arrival order, and hands every answer to the engine; a sheet shows when its tab
//! is the window's active one. A sheet belongs to its tab: switching away hides it, coming back
//! shows it again, and closing the tab declines it.
//!
//! Keys on a sheet: `⏎` OK, `⎋` cancel or block (the safe default), `1` / `2` the numbered
//! permission answers, `⌘B` blocks a page's dialogs. Tab and window shortcuts still work; the
//! page under the sheet gets nothing.

use std::cell::RefCell;
use std::collections::BTreeMap;

use objc2::MainThreadMarker;
use quickie_application::{
    Ask, AskKind, Dialog, DialogAnswer, PermissionAnswer, Prompts, ReplyToken,
};
use quickie_domain_browsing::{TabId, WindowId};
use quickie_domain_commands::KeyStroke;
use quickie_ui_widgets::{DialogChoice, DialogKind, DialogRequest, DialogSheet, PermissionSheet};

use super::super::Browser;
use crate::input::keystroke::{SheetKey, sheet_key};

/// A window's two sheets, and the ask on screen in it.
struct WindowSheets {
    dialog: DialogSheet,
    permission: PermissionSheet,
    shown: Option<ReplyToken>,
}

/// The asks waiting for the user and the sheets that show them.
pub struct PageAsks {
    prompts: RefCell<Prompts>,
    windows: RefCell<BTreeMap<WindowId, WindowSheets>>,
    mtm: MainThreadMarker,
}

impl PageAsks {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            prompts: RefCell::new(Prompts::new()),
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
    /// A page asked: `Prompts` answers it at once when the user already chose, else keeps it
    /// for a sheet, which [`Self::present_asks`] puts on screen.
    pub(in crate::browser) fn asked(&self, ask: Ask) {
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        self.page
            .asks
            .prompts
            .borrow_mut()
            .asked(ask, browsing.engine_mut());
    }

    /// Puts each window's sheet on screen: the oldest ask of its active tab, or none. Asks of
    /// closed tabs are forgotten.
    pub(in crate::browser) fn present_asks(&self) {
        let active: Vec<(WindowId, Option<TabId>)> = {
            let Ok(browsing) = self.browsing.try_borrow() else {
                return;
            };
            let session = browsing.session();
            self.page
                .asks
                .prompts
                .borrow_mut()
                .keep_tabs(|tab| session.tab(tab).is_some());
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
                let prompts = self.page.asks.prompts.borrow();
                prompts
                    .waiting(tab)
                    .map(|waiting| (waiting.ask.clone(), waiting.offer_block))
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

    fn offers_block(&self, id: ReplyToken) -> bool {
        self.page.asks.prompts.borrow().offers_block(id)
    }

    fn present(&self, window: WindowId, ask: Option<(Ask, bool)>) {
        let shown = self
            .page
            .asks
            .windows
            .borrow()
            .get(&window)
            .and_then(|sheets| sheets.shown);
        let wanted = ask.as_ref().map(|(ask, _)| ask.token);
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
        let id = ask.token;
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

    /// True when `window` still shows the ask `id`.
    fn shows(&self, window: WindowId, id: ReplyToken) -> bool {
        self.page
            .asks
            .windows
            .borrow()
            .get(&window)
            .is_some_and(|sheets| sheets.shown == Some(id))
    }

    fn answer_dialog(&self, window: WindowId, id: ReplyToken, choice: DialogChoice) {
        if !self.shows(window, id) {
            return;
        }
        let text = self
            .page
            .asks
            .windows
            .borrow()
            .get(&window)
            .and_then(|sheets| sheets.dialog.text());
        let answer = match choice {
            DialogChoice::Ok => DialogAnswer::Ok(text),
            DialogChoice::Cancel => DialogAnswer::Cancel,
            DialogChoice::Block => DialogAnswer::Block,
        };
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let answered =
            self.page
                .asks
                .prompts
                .borrow_mut()
                .dialog_answered(id, answer, browsing.engine_mut());
        drop(browsing);
        if answered {
            self.settle(window);
        }
    }

    fn answer_permission(&self, window: WindowId, id: ReplyToken, answer: PermissionAnswer) {
        if !self.shows(window, id) {
            return;
        }
        let Ok(mut browsing) = self.browsing.try_borrow_mut() else {
            return;
        };
        let answered = self.page.asks.prompts.borrow_mut().permission_answered(
            id,
            answer,
            browsing.engine_mut(),
        );
        drop(browsing);
        if answered {
            self.settle(window);
        }
    }

    /// The tab committed a navigation: its dialogs count afresh.
    pub(in crate::browser) fn asks_navigated(&self, tab: TabId) {
        self.page.asks.prompts.borrow_mut().navigated(tab);
    }
}
