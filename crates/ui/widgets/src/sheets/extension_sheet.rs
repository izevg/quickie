//! An extension asks for what it may do: an E3 sheet when it is installed, updated to ask for
//! more, or asks while it runs. `1` allows, `⎋` refuses, the safe default; `⏎` does nothing
//! here, so a stray Return never grants an extension the user's pages. What it asks for is
//! said in plain words, broadest first.

use std::rc::Rc;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSView, NSWindow};
use quickie_ui_shared::tokens::{TextRole, color};
use quickie_ui_shared::{
    Emphasis, Sheet, SheetAnswer, SheetContent, SheetIcon, SheetLine, named_header,
};

/// What an extension asks for, as the sheet shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionRequest {
    /// The extension's name.
    pub name: String,
    /// What is happening: installing, updating, or running.
    pub moment: Moment,
    /// API permissions, as the manifest names them.
    pub permissions: Vec<String>,
    /// Host match patterns.
    pub patterns: Vec<String>,
}

/// When an extension asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Moment {
    /// It is being added, at this version.
    Install(String),
    /// A new version asks for more than the installed one had.
    Update(String),
    /// It asks while it runs.
    Running,
}

/// The extension sheet; one per window.
pub struct ExtensionSheet {
    sheet: Sheet,
}

impl ExtensionSheet {
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

    /// Shows `request` at the top of `host`; a click on an answer calls `answered` with
    /// whether it allows.
    pub fn show(
        &self,
        host: &NSView,
        window: &NSWindow,
        request: &ExtensionRequest,
        answered: impl Fn(bool) + 'static,
    ) {
        let answered = Rc::new(answered);
        self.sheet
            .show(host, window, &content(request), move |index| {
                answered(index == 0);
            });
    }

    pub fn hide(&self) {
        self.sheet.hide();
    }
}

fn content(request: &ExtensionRequest) -> SheetContent {
    let (before, after, allow, refuse) = match &request.moment {
        Moment::Install(_) => ("Add ", "?", "Add Extension", "Cancel"),
        Moment::Update(_) => ("Update ", "?", "Allow and Update", "Keep Current Version"),
        Moment::Running => ("", " asks for more", "Allow", "Don't Allow"),
    };
    let mut lines = Vec::new();
    match &request.moment {
        Moment::Install(version) | Moment::Update(version) => lines.push(SheetLine {
            text: format!("Version {version}"),
            role: TextRole::RowDetail,
            dimmed: true,
        }),
        Moment::Running => {}
    }
    let said = describe(&request.permissions, &request.patterns);
    if said.is_empty() {
        lines.push(line("It asks for no access to your pages or data."));
    } else {
        lines.push(line("It will be able to:"));
        lines.extend(said.iter().map(|text| line(&format!("·  {text}"))));
    }
    SheetContent {
        header: named_header(before, &request.name, after),
        icon: Some(SheetIcon {
            symbol: "puzzlepiece.extension.fill",
            tint: color::accent,
            fill: accent_fill,
        }),
        lines,
        card: None,
        field: None,
        answers: vec![
            answer(allow, "1", Emphasis::Plain, None),
            answer(refuse, "⎋", Emphasis::Default, Some("nosign")),
        ],
    }
}

fn accent_fill() -> Retained<NSColor> {
    color::accent().colorWithAlphaComponent(color::TINT_ALPHA)
}

fn line(text: &str) -> SheetLine {
    SheetLine {
        text: text.to_owned(),
        role: TextRole::RowTitle,
        dimmed: false,
    }
}

fn answer(
    label: &str,
    keys: &str,
    emphasis: Emphasis,
    symbol: Option<&'static str>,
) -> SheetAnswer {
    SheetAnswer {
        label: label.to_owned(),
        keys: keys.to_owned(),
        emphasis,
        symbol,
    }
}

/// What the permissions and patterns let an extension do, broadest first, each once.
#[must_use]
pub fn describe(permissions: &[String], patterns: &[String]) -> Vec<String> {
    let mut said = Vec::new();
    let everywhere = patterns.iter().any(|pattern| {
        matches!(
            pattern.as_str(),
            "<all_urls>" | "*://*/*" | "http://*/*" | "https://*/*"
        )
    });
    if everywhere {
        said.push("Read and change your data on all websites".to_owned());
    } else {
        let hosts: Vec<&str> = patterns
            .iter()
            .filter_map(|pattern| host(pattern))
            .collect();
        if !hosts.is_empty() {
            said.push(format!("Read and change your data on {}", hosts.join(", ")));
        }
    }
    for permission in permissions {
        let text = match permission.as_str() {
            "tabs" | "history" | "topSites" | "webNavigation" => "Read your browsing history",
            "cookies" => "Read and change cookies",
            "clipboardWrite" => "Write to the clipboard",
            "clipboardRead" => "Read what you copy and paste",
            "nativeMessaging" => "Talk to apps on this Mac",
            "notifications" => "Show notifications",
            "downloads" => "Manage your downloads",
            "webRequest" | "declarativeNetRequest" | "declarativeNetRequestWithHostAccess" => {
                "Block content on any page"
            }
            "privacy" => "Change privacy settings",
            "management" => "Manage your other extensions",
            "storage" | "unlimitedStorage" => "Keep data on this Mac",
            "scripting" | "activeTab" | "contextMenus" | "alarms" | "idle" | "offscreen" => {
                continue;
            }
            other => other,
        };
        if !said.iter().any(|known| known == text) {
            said.push(text.to_owned());
        }
    }
    said
}

/// The host a pattern names, without its scheme and path: `*.example.com`.
fn host(pattern: &str) -> Option<&str> {
    let rest = pattern.split_once("://")?.1;
    let host = rest.split('/').next()?;
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::describe;

    fn owned(names: &[&str]) -> Vec<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    }

    #[test]
    fn all_sites_come_first_and_quiet_permissions_are_not_said() {
        let said = describe(
            &owned(&["storage", "tabs", "scripting", "history"]),
            &owned(&["https://a.com/*", "<all_urls>"]),
        );

        assert_eq!(
            said,
            [
                "Read and change your data on all websites",
                "Keep data on this Mac",
                "Read your browsing history",
            ]
        );
    }

    #[test]
    fn some_sites_are_named() {
        let said = describe(&[], &owned(&["*://*.example.com/*", "https://a.com/x/*"]));

        assert_eq!(said, ["Read and change your data on *.example.com, a.com"]);
    }

    #[test]
    fn an_unknown_permission_is_said_by_its_name() {
        assert_eq!(describe(&owned(&["fontSettings"]), &[]), ["fontSettings"]);
        assert_eq!(describe(&[], &[]), Vec::<String>::new());
    }
}
