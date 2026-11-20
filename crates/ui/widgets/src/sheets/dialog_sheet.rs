//! A page's JavaScript dialog (`alert`, `confirm`, `prompt`) as an E3 sheet. The origin that
//! asks heads it, large, so a page cannot pass its words off as the browser's or another site's.
//! `⏎` is OK, `⎋` cancels, and from the second dialog in a row `⌘B` blocks the page's dialogs.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSTextField, NSTextInputClient, NSTextView, NSView, NSWindow};
use quickie_domain_browsing::Origin;
use quickie_ui_shared::tokens::TextRole;
use quickie_ui_shared::{Emphasis, Sheet, SheetAnswer, SheetContent, SheetLine, header};
use url::Url;

/// Which dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    /// With the text the field starts with.
    Prompt {
        default: String,
    },
}

/// What the sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogRequest {
    /// Who asks.
    pub origin: Url,
    pub kind: DialogKind,
    pub message: String,
    /// Adds "Block dialogs from this page".
    pub offer_block: bool,
}

/// The user's choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogChoice {
    Ok,
    Cancel,
    /// Cancel, and no more dialogs from this page.
    Block,
}

/// The dialog sheet; one per window.
pub struct DialogSheet {
    sheet: Sheet,
    choices: Rc<RefCell<Vec<DialogChoice>>>,
}

impl DialogSheet {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            sheet: Sheet::new(mtm),
            choices: Rc::default(),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.sheet.is_open()
    }

    /// Shows `request` at the top of `host`; a click on an answer calls `chosen`.
    pub fn show(
        &self,
        host: &NSView,
        window: &NSWindow,
        request: &DialogRequest,
        chosen: impl Fn(DialogChoice) + 'static,
    ) {
        let choices = choices(&request.kind, request.offer_block);
        let content = content(request, &choices);
        *self.choices.borrow_mut() = choices;
        let shown = Rc::clone(&self.choices);
        self.sheet.show(host, window, &content, move |index| {
            let choice = shown.borrow().get(index).copied();
            if let Some(choice) = choice {
                chosen(choice);
            }
        });
    }

    pub fn hide(&self) {
        self.sheet.hide();
    }

    /// The prompt's text.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.sheet.text()
    }

    /// The prompt's field, to tell whether it has the keyboard.
    #[must_use]
    pub fn field(&self) -> Option<Retained<NSTextField>> {
        self.sheet.field()
    }

    /// True while an input method composes in the prompt's field: `⏎` then commits the
    /// composition, not the dialog.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.field()
            .and_then(|field| field.currentEditor())
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
            .is_some_and(|editor| editor.hasMarkedText())
    }
}

/// The answers a dialog offers, in order.
fn choices(kind: &DialogKind, offer_block: bool) -> Vec<DialogChoice> {
    let mut choices = vec![DialogChoice::Ok];
    if *kind != DialogKind::Alert {
        choices.push(DialogChoice::Cancel);
    }
    if offer_block {
        choices.push(DialogChoice::Block);
    }
    choices
}

fn content(request: &DialogRequest, choices: &[DialogChoice]) -> SheetContent {
    let (before, owner, after) = who(&request.origin);
    let answers = choices
        .iter()
        .map(|choice| match choice {
            DialogChoice::Ok => answer("OK", "⏎", Emphasis::Primary),
            DialogChoice::Cancel => answer("Cancel", "⎋", Emphasis::Plain),
            DialogChoice::Block => answer("Block dialogs", "⌘B", Emphasis::Plain),
        })
        .collect();
    SheetContent {
        header: header(&before, &owner, &after, " says"),
        icon: None,
        lines: vec![SheetLine {
            text: request.message.clone(),
            role: TextRole::RowTitle,
            dimmed: false,
        }],
        card: None,
        field: match &request.kind {
            DialogKind::Prompt { default } => Some(default.clone()),
            DialogKind::Alert | DialogKind::Confirm => None,
        },
        answers,
    }
}

fn answer(label: &str, keys: &str, emphasis: Emphasis) -> SheetAnswer {
    SheetAnswer {
        label: label.to_owned(),
        keys: keys.to_owned(),
        emphasis,
        symbol: None,
    }
}

/// The origin split for the header: `developer.` `apple.com` `:8443`; a page that is not on
/// the web is named whole (`about:blank`).
fn who(url: &Url) -> (String, String, String) {
    match Origin::of(url) {
        Some(origin) => (
            origin.subdomain,
            origin.registrable,
            origin
                .port
                .map_or_else(String::new, |port| format!(":{port}")),
        ),
        None => (
            String::new(),
            url.as_str().trim_end_matches('/').to_owned(),
            String::new(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{DialogChoice, DialogKind, choices, who};

    #[test]
    fn an_alert_has_only_ok_and_confirm_and_prompt_can_cancel() {
        assert_eq!(choices(&DialogKind::Alert, false), [DialogChoice::Ok]);
        assert_eq!(
            choices(&DialogKind::Confirm, false),
            [DialogChoice::Ok, DialogChoice::Cancel]
        );
        assert_eq!(
            choices(
                &DialogKind::Prompt {
                    default: String::new()
                },
                false
            ),
            [DialogChoice::Ok, DialogChoice::Cancel]
        );
    }

    #[test]
    fn blocking_comes_last_when_offered() {
        assert_eq!(
            choices(&DialogKind::Alert, true),
            [DialogChoice::Ok, DialogChoice::Block]
        );
        assert_eq!(
            choices(&DialogKind::Confirm, true),
            [DialogChoice::Ok, DialogChoice::Cancel, DialogChoice::Block]
        );
    }

    #[test]
    fn the_header_names_the_origin() {
        let url = |text: &str| Url::parse(text).unwrap();
        let parts = |a: &str, b: &str, c: &str| (a.to_owned(), b.to_owned(), c.to_owned());

        assert_eq!(
            who(&url("https://developer.apple.com:8443/x")),
            parts("developer.", "apple.com", ":8443")
        );
        assert_eq!(who(&url("http://127.0.0.1/")), parts("", "127.0.0.1", ""));
        assert_eq!(who(&url("about:blank")), parts("", "about:blank", ""));
    }
}
