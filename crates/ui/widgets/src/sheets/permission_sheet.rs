//! A page asks for the camera or the microphone: an E3 sheet with numbered answers
//! (`docs/design/ideas.md`, "Numbered answers in sheets"). `1` allows once, `2` allows on this
//! site, `⎋` blocks, the safe default. `⏎` does nothing here, so a stray Return never grants a
//! device. The origin that asks heads the sheet.

use std::rc::Rc;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSView, NSWindow};
use quickie_application::{Permission, PermissionAnswer};
use quickie_domain_browsing::Origin;
use quickie_ui_shared::tokens::TextRole;
use quickie_ui_shared::{Emphasis, Sheet, SheetAnswer, SheetContent, SheetLine, header};
use url::Url;

/// The answers, in the order the sheet lists them.
const ANSWERS: [PermissionAnswer; 3] = [
    PermissionAnswer::AllowOnce,
    PermissionAnswer::AllowOnSite,
    PermissionAnswer::Deny,
];

/// The permission sheet; one per window.
pub struct PermissionSheet {
    sheet: Sheet,
}

impl PermissionSheet {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            sheet: Sheet::new(mtm),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.sheet.is_open()
    }

    /// Shows the request of `origin` for `permission` at the top of `host`; a click on an
    /// answer calls `answered`.
    pub fn show(
        &self,
        host: &NSView,
        window: &NSWindow,
        origin: &Url,
        permission: Permission,
        answered: impl Fn(PermissionAnswer) + 'static,
    ) {
        let answered = Rc::new(answered);
        self.sheet
            .show(host, window, &content(origin, permission), move |index| {
                if let Some(&answer) = ANSWERS.get(index) {
                    answered(answer);
                }
            });
    }

    pub fn hide(&self) {
        self.sheet.hide();
    }
}

/// What the page wants to use, as the sheet says it.
fn wants(permission: Permission) -> &'static str {
    match permission {
        Permission::Camera => "Wants to use your camera",
        Permission::Microphone => "Wants to use your microphone",
        Permission::CameraAndMicrophone => "Wants to use your camera and microphone",
    }
}

fn glyph(permission: Permission) -> &'static str {
    match permission {
        Permission::Microphone => "mic.fill",
        Permission::Camera | Permission::CameraAndMicrophone => "video.fill",
    }
}

fn content(origin: &Url, permission: Permission) -> SheetContent {
    let (before, owner, after) = who(origin);
    let answers = ANSWERS
        .iter()
        .map(|answer| match answer {
            PermissionAnswer::AllowOnce => row("Allow once", "1", Emphasis::Plain),
            PermissionAnswer::AllowOnSite => row("Allow on this site", "2", Emphasis::Plain),
            PermissionAnswer::Deny => row("Block", "⎋", Emphasis::Default),
        })
        .collect();
    SheetContent {
        header: header(&before, &owner, &after, ""),
        glyph: Some(glyph(permission).to_owned()),
        lines: vec![SheetLine {
            text: wants(permission).to_owned(),
            role: TextRole::RowTitle,
            dimmed: true,
        }],
        field: None,
        answers,
        hints: vec![
            ("1".to_owned(), "allow once".to_owned()),
            ("2".to_owned(), "allow on this site".to_owned()),
            ("⎋".to_owned(), "block".to_owned()),
        ],
    }
}

fn row(label: &str, keys: &str, emphasis: Emphasis) -> SheetAnswer {
    SheetAnswer {
        label: label.to_owned(),
        keys: keys.to_owned(),
        emphasis,
    }
}

/// The origin split for the header: `meet.` `example.com` `:8443`; a page that is not on the
/// web is named whole.
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
    use quickie_application::{Permission, PermissionAnswer};

    use super::{ANSWERS, wants};

    #[test]
    fn the_numbers_allow_and_escape_blocks() {
        assert_eq!(
            ANSWERS,
            [
                PermissionAnswer::AllowOnce,
                PermissionAnswer::AllowOnSite,
                PermissionAnswer::Deny
            ]
        );
    }

    #[test]
    fn the_sheet_names_every_device_asked_for() {
        assert!(wants(Permission::Camera).ends_with("camera"));
        assert!(wants(Permission::Microphone).ends_with("microphone"));
        assert!(wants(Permission::CameraAndMicrophone).ends_with("camera and microphone"));
    }
}
