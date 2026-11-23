use std::fs::File;
use std::io::Read;
use std::path::Path;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAlert, NSApplication, NSScrollView, NSWindow, NSWorkspace};
use objc2_foundation::{NSPoint, NSSize, NSString, NSURL, ns_string};
use quickie_ui_widgets::agent::answer_panel::markdown::LinePart;
use quickie_ui_widgets::agent::answer_panel::text::AnswerText;
use quickie_ui_widgets::agent::answer_panel::transcript::Transcript;
use url::Url;

const FIRST_BUTTON: isize = 1000;
const GITHUB_BUTTON: isize = FIRST_BUTTON + 1;
const ALL_CHANGES_BUTTON: isize = FIRST_BUTTON + 2;
const MAX_NOTES_BYTES: u64 = 64 * 1024;

pub(super) struct Progress {
    alert: Retained<NSAlert>,
    parent: Retained<NSWindow>,
}

impl Progress {
    pub(super) fn new(
        title: &str,
        mtm: MainThreadMarker,
        cancel: impl Fn() + 'static,
    ) -> Option<Self> {
        let app = NSApplication::sharedApplication(mtm);
        let parent = app.keyWindow().or_else(|| app.mainWindow())?;
        if parent.attachedSheet().is_some() {
            return None;
        }
        let alert = alert(title, "", mtm);
        alert.addButtonWithTitle(ns_string!("Cancel"));
        let completed = RcBlock::new(move |response| {
            if response == FIRST_BUTTON {
                cancel();
            }
        });
        alert.beginSheetModalForWindow_completionHandler(&parent, Some(&completed));
        Some(Self { alert, parent })
    }

    pub(super) fn close(self) {
        self.parent.endSheet_returnCode(&self.alert.window(), 0);
    }
}

pub(super) fn message(title: &str, detail: &str, mtm: MainThreadMarker) {
    let alert = alert(title, detail, mtm);
    alert.addButtonWithTitle(ns_string!("OK"));
    alert.runModal();
}

#[derive(PartialEq, Eq)]
pub(super) enum Choice {
    Primary,
    Close,
}

pub(super) fn notes(
    version: &str,
    markdown: &str,
    status: &str,
    action: Option<&str>,
    mtm: MainThreadMarker,
) -> Choice {
    let preview = summary(markdown);
    let show_all_toggle = preview.trim() != markdown.trim() && !markdown.trim().is_empty();
    let mut all_changes = false;
    loop {
        let alert = alert(&format!("Quickie {version}"), status, mtm);
        alert.addButtonWithTitle(&NSString::from_str(action.unwrap_or("Done")));
        alert.addButtonWithTitle(ns_string!("View on GitHub"));
        if show_all_toggle {
            alert.addButtonWithTitle(&NSString::from_str(if all_changes {
                "Summary"
            } else {
                "All Changes"
            }));
        }
        if action.is_some() {
            alert.addButtonWithTitle(ns_string!("Later"));
        }
        let text = AnswerText::new(mtm);
        text.on_link(|url| open(url.as_str()));
        let shown = if all_changes { markdown } else { &preview };
        let shown = if shown.trim().is_empty() {
            "Release notes are unavailable."
        } else {
            shown
        };
        let edit = Transcript::default().push([LinePart::Text(shown.into())]);
        text.view().setFrameSize(NSSize::new(440.0, 220.0));
        text.apply(&edit);
        if let Some(scroll) = text.view().downcast_ref::<NSScrollView>()
            && let Some(document) = scroll.documentView()
        {
            document.scrollPoint(NSPoint::ZERO);
        }
        alert.setAccessoryView(Some(text.view()));
        match alert.runModal() {
            FIRST_BUTTON if action.is_some() => return Choice::Primary,
            GITHUB_BUTTON => {
                open(&format!(
                    "https://github.com/izevg/quickie/releases/tag/v{version}"
                ));
                return Choice::Close;
            }
            ALL_CHANGES_BUTTON if show_all_toggle => all_changes = !all_changes,
            _ => return Choice::Close,
        }
    }
}

fn alert(title: &str, detail: &str, mtm: MainThreadMarker) -> Retained<NSAlert> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(detail));
    alert
}

fn open(address: &str) {
    let Ok(url) = Url::parse(address) else { return };
    if matches!(url.scheme(), "https" | "http")
        && let Some(address) = NSURL::URLWithString(&NSString::from_str(address))
    {
        NSWorkspace::sharedWorkspace().openURL(&address);
    }
}

pub(super) fn installed_notes(path: &Path) -> String {
    let mut text = String::new();
    if let Ok(file) = File::open(path)
        && file
            .take(MAX_NOTES_BYTES + 1)
            .read_to_string(&mut text)
            .is_ok()
        && text.len() as u64 <= MAX_NOTES_BYTES
    {
        return text;
    }
    String::new()
}

fn summary(markdown: &str) -> String {
    const COMMIT_LINK: &str = "`](https://github.com/izevg/quickie/commit/";
    const ITEMS_PER_HEADING: usize = 3;
    let mut changes = Vec::new();
    let mut upgrade = Vec::new();
    let mut heading = None;
    let mut remaining = ITEMS_PER_HEADING;
    let mut is_upgrade = false;
    for line in markdown.lines() {
        let title = line.trim_start_matches('#');
        if title != line && title.starts_with(' ') {
            is_upgrade = matches!(title.trim(), "Install" | "Upgrade" | "Installation");
            if is_upgrade {
                upgrade.push(line);
            } else {
                heading = Some(line);
                remaining = ITEMS_PER_HEADING;
            }
            continue;
        }
        if is_upgrade {
            upgrade.push(line);
            continue;
        }
        if !line.trim().is_empty() && remaining > 0 {
            if let Some(heading) = heading.take() {
                changes.push(heading);
            }
            let show = line
                .rsplit_once(" ([`")
                .filter(|(_, link)| {
                    link.strip_suffix("))")
                        .and_then(|link| link.split_once(COMMIT_LINK))
                        .is_some_and(|(short, hash)| {
                            short.len() == 7
                                && hash.len() == 40
                                && hash.starts_with(short)
                                && hash.bytes().all(|digit| digit.is_ascii_hexdigit())
                        })
                })
                .map_or(line, |(change, _)| change);
            changes.push(show);
            remaining -= 1;
        }
    }
    if !upgrade.is_empty() {
        upgrade.push("");
    }
    upgrade.extend(changes);
    upgrade.join("\n")
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn preview_retains_headings_and_prioritizes_installation_instructions() {
        let notes = "# Features\n- First\n- Second\n- Third\n- Fourth\n## Bug fixes\n- Repaired\n## Install\nCheck the OS requirement.";

        assert_eq!(
            summary(notes),
            "## Install\nCheck the OS requirement.\n\n# Features\n- First\n- Second\n- Third\n## Bug fixes\n- Repaired"
        );
    }

    #[test]
    fn preview_hides_generated_hashes_and_leaves_original_links() {
        let notes = "### Features\n- Added [an example](https://example.com) ([`0123456`](https://github.com/izevg/quickie/commit/0123456789abcdef0123456789abcdef01234567))";

        assert_eq!(
            summary(notes),
            "### Features\n- Added [an example](https://example.com)"
        );
    }
}
