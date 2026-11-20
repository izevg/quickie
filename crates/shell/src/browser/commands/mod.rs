//! The dispatcher: one place where a command id becomes the use case that does it, whether a
//! key or the palette asked. A command Quickie lists but cannot do yet says so in a toast
//! rather than doing nothing.
//!
//! - [`zoom`]: page zoom, remembered per site, applied to every page as it commits.
//! - [`output`]: printing a page, saving it as a PDF, and where Web Inspector is.

mod output;
#[cfg(test)]
mod tests;
mod zoom;

use std::thread;

use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::rc::Retained;
use objc2::{ClassType, MainThreadMarker, msg_send};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;
use quickie_application::{FetchedList, ListFetch, Navigation, fetch_lists};
use quickie_domain_browsing::{Direction, Focus, JarChoice, PinnedSlot, WindowId};
use quickie_domain_commands::{CommandId, KeyStroke, Modifiers};
use quickie_domain_settings::ZoomStep;
use quickie_domain_shields::FilterListId;
use quickie_engine_webkit::UrlSessionHttp;
use quickie_ui_shared::Trigger;
use url::Url;

use super::Browser;

/// What a command asks the browser to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    OpenCommands,
    GoTo,
    OpenAddress,
    NewTab,
    NewThrowawayTab,
    CloseTab,
    ReopenClosed,
    Cycle(Direction),
    GoToSlot(PinnedSlot),
    PinToSlot(PinnedSlot),
    OpenInThrowawayJar,
    Reload,
    HardReload,
    Back,
    Forward,
    CopyUrl,
    CopyMarkdownLink,
    CopyTitle,
    Bookmark,
    Find,
    FindNext,
    FindPrevious,
    Zoom(ZoomStep),
    Print,
    SavePdf,
    ToggleShields,
    EnableLists,
    RefreshLists,
    Overview,
    /// Link hints; `background` opens a followed link in a background tab.
    Hints {
        background: bool,
    },
    ShowDownloads,
    ShowShields,
    NewWindow,
    CloseWindow,
    SetDefaultBrowser,
    RevealProfile,
    AskAgent,
    StopAgent,
    ShowAgentAnswer,
    ShowAgentSettings,
    /// The Settings window.
    ShowSettings,
    /// The key debug overlay, on or off.
    DebugKeys,
    /// Any of the developer tools' commands: Web Inspector opens from the page.
    DevTools,
    /// Installing, updating, and placing web extensions.
    Extensions(ExtensionsAction),
}

/// What an extensions command asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionsAction {
    /// An extension from the active Chrome Web Store page, or one pasted into the palette.
    InstallFromUrl,
    /// An extension from a file or folder on disk.
    InstallFromFile,
    CheckUpdates,
    /// Extensions off (`true`) or back on on the active page's site.
    OnSite {
        off: bool,
    },
    /// The Settings window's Extensions pane.
    Show,
}

impl Action {
    /// True for what may run while a sheet waits for an answer: leaving the page that asked
    /// (closing it, switching away, another tab or window), never an overlay over the sheet.
    #[must_use]
    pub fn passes_sheet(self) -> bool {
        matches!(
            self,
            Self::NewTab
                | Self::NewThrowawayTab
                | Self::CloseTab
                | Self::ReopenClosed
                | Self::Cycle(_)
                | Self::GoToSlot(_)
                | Self::NewWindow
                | Self::CloseWindow
        )
    }

    /// The action of a command, or `None` for a command not built yet.
    #[must_use]
    pub fn of(id: &CommandId) -> Option<Self> {
        let id = id.as_str();
        let slot = |prefix: &str| {
            id.strip_prefix(prefix)
                .and_then(|n| n.parse::<u8>().ok())
                .and_then(|n| PinnedSlot::try_from(n).ok())
        };
        if let Some(slot) = slot("quickie.tab.goToSlot") {
            return Some(Self::GoToSlot(slot));
        }
        if let Some(slot) = slot("quickie.tab.pinToSlot") {
            return Some(Self::PinToSlot(slot));
        }
        if id.starts_with("quickie.devtools.") {
            return Some(Self::DevTools);
        }
        let action = match id {
            "quickie.palette.openCommands" => Self::OpenCommands,
            "quickie.palette.goto" => Self::GoTo,
            "quickie.address.open" => Self::OpenAddress,
            "quickie.tab.new" => Self::NewTab,
            "quickie.tab.newThrowaway" => Self::NewThrowawayTab,
            "quickie.tab.close" => Self::CloseTab,
            "quickie.tab.reopenClosed" => Self::ReopenClosed,
            "quickie.tab.nextRecent" => Self::Cycle(Direction::Forward),
            "quickie.tab.previousRecent" => Self::Cycle(Direction::Backward),
            "quickie.tab.openInThrowawayJar" => Self::OpenInThrowawayJar,
            "quickie.tab.reload" => Self::Reload,
            "quickie.tab.hardReload" => Self::HardReload,
            "quickie.navigation.back" => Self::Back,
            "quickie.navigation.forward" => Self::Forward,
            "quickie.page.copyUrl" => Self::CopyUrl,
            "quickie.page.copyAsMarkdownLink" => Self::CopyMarkdownLink,
            "quickie.page.copyTitle" => Self::CopyTitle,
            "quickie.page.bookmark" => Self::Bookmark,
            "quickie.page.find" => Self::Find,
            "quickie.page.findNext" => Self::FindNext,
            "quickie.page.findPrevious" => Self::FindPrevious,
            "quickie.page.zoomIn" => Self::Zoom(ZoomStep::In),
            "quickie.page.zoomOut" => Self::Zoom(ZoomStep::Out),
            "quickie.page.zoomReset" => Self::Zoom(ZoomStep::Reset),
            "quickie.page.print" => Self::Print,
            "quickie.page.savePdf" => Self::SavePdf,
            "quickie.shields.toggleSite" => Self::ToggleShields,
            "quickie.shields.enableLists" => Self::EnableLists,
            "quickie.shields.refreshLists" => Self::RefreshLists,
            "quickie.tab.overview" => Self::Overview,
            "quickie.hints.toggle" => Self::Hints { background: false },
            "quickie.hints.openInBackground" => Self::Hints { background: true },
            "quickie.downloads.show" => Self::ShowDownloads,
            "quickie.shields.open" => Self::ShowShields,
            "quickie.window.new" => Self::NewWindow,
            "quickie.window.close" => Self::CloseWindow,
            "quickie.app.setDefaultBrowser" => Self::SetDefaultBrowser,
            "quickie.app.revealProfile" => Self::RevealProfile,
            "quickie.agent.ask" => Self::AskAgent,
            "quickie.agent.stop" => Self::StopAgent,
            "quickie.agent.showAnswer" => Self::ShowAgentAnswer,
            "quickie.agent.settings" => Self::ShowAgentSettings,
            "quickie.app.settings" => Self::ShowSettings,
            "quickie.debug.keys" => Self::DebugKeys,
            "quickie.extensions.installFromUrl" => {
                Self::Extensions(ExtensionsAction::InstallFromUrl)
            }
            "quickie.extensions.installFromFile" => {
                Self::Extensions(ExtensionsAction::InstallFromFile)
            }
            "quickie.extensions.checkUpdates" => Self::Extensions(ExtensionsAction::CheckUpdates),
            "quickie.extensions.offOnSite" => {
                Self::Extensions(ExtensionsAction::OnSite { off: true })
            }
            "quickie.extensions.onOnSite" => {
                Self::Extensions(ExtensionsAction::OnSite { off: false })
            }
            "quickie.extensions.settings" => Self::Extensions(ExtensionsAction::Show),
            _ => return None,
        };
        Some(action)
    }
}

/// The general pasteboard, `None` when the pasteboard server cannot be reached (a sandboxed or
/// headless session). The binding declares it never nil and would panic.
fn general_pasteboard() -> Option<Retained<NSPasteboard>> {
    // SAFETY: `generalPasteboard` takes no arguments and returns an autoreleased pasteboard or
    // nil, which `Option<Retained<_>>` receives.
    unsafe { msg_send![NSPasteboard::class(), generalPasteboard] }
}

/// The modifiers that hold a cycle open: those of its stroke but `⇧`, which picks the direction
/// and may be let go while cycling.
fn holding(stroke: Option<KeyStroke>) -> Modifiers {
    let held = stroke.map_or(Modifiers::NONE, |stroke| stroke.modifiers);
    [Modifiers::CONTROL, Modifiers::OPTION, Modifiers::COMMAND]
        .into_iter()
        .filter(|&modifier| held.contains(modifier))
        .fold(Modifiers::NONE, |all, modifier| all | modifier)
}

impl Browser {
    /// A command Quickie lists but cannot do yet says so.
    fn not_yet(&self, id: &CommandId) {
        self.close_overlays();
        let title = self
            .catalog
            .get(id)
            .map_or(id.as_str(), |command| command.title);
        self.toast(&format!("Not yet: {title}"));
    }

    /// Runs the command `id`. `stroke` is the key that asked, `None` for the palette.
    pub(super) fn dispatch(&self, id: &CommandId, stroke: Option<KeyStroke>) {
        self.pending.take();
        let Some(action) = Action::of(id) else {
            return self.not_yet(id);
        };
        match action {
            Action::Overview => return self.toggle_overview(Trigger::Keyboard),
            Action::Hints { background } => return self.open_hints(background),
            _ => self.hide_navigation(),
        }
        match action {
            Action::OpenCommands => return self.open_palette(">"),
            Action::GoTo => return self.open_palette(""),
            Action::OpenAddress => return self.open_address(),
            Action::Find => return self.open_find(),
            Action::FindNext => return self.find_step(false),
            Action::FindPrevious => return self.find_step(true),
            Action::ShowDownloads => return self.toggle_downloads(Trigger::Keyboard),
            Action::ShowShields => return self.toggle_shields_popover(Trigger::Keyboard),
            Action::AskAgent => return self.open_palette("?"),
            Action::StopAgent => return self.stop_agent(),
            Action::ShowAgentAnswer => return self.show_answer(),
            Action::ShowAgentSettings => return self.open_agent_settings(),
            Action::ShowSettings => return self.open_settings(None),
            Action::DebugKeys => return self.toggle_key_log(),
            Action::Cycle(direction) => {
                if let Some((window, _)) = self.current() {
                    self.cycle(window, direction, holding(stroke));
                }
                return;
            }
            _ => {}
        }
        self.close_overlays();
        match action {
            Action::SetDefaultBrowser => return self.set_default_browser(),
            Action::RevealProfile => return self.reveal_profile(),
            Action::Extensions(action) => return self.extension_command(action),
            _ => {}
        }
        let Some((window, _)) = self.current() else {
            return;
        };
        match action {
            Action::NewWindow => return self.new_window(),
            Action::CloseWindow => return self.close_window(window),
            Action::CloseTab => return self.close_tab(window),
            _ => {}
        }
        match action {
            Action::NewTab => self.new_tab(window, JarChoice::Main),
            Action::NewThrowawayTab => self.new_tab(window, JarChoice::NewThrowaway),
            Action::ReopenClosed => self.reopen_closed(window),
            Action::GoToSlot(slot) => self.go_to_slot(window, slot),
            Action::PinToSlot(slot) => self.pin(window, slot),
            Action::OpenInThrowawayJar => self.open_in_throwaway_jar(window),
            Action::Reload => self.navigate(window, &Navigation::Reload),
            Action::HardReload => self.navigate(window, &Navigation::HardReload),
            Action::Back => self.navigate(window, &Navigation::Back),
            Action::Forward => self.navigate(window, &Navigation::Forward),
            Action::CopyUrl => self.copy(window, |tab| Some(tab.url.to_string())),
            Action::CopyMarkdownLink => self.copy(window, |tab| Some(tab.markdown_link())),
            Action::CopyTitle => self.copy(window, |tab| Some(tab.title).filter(|t| !t.is_empty())),
            Action::Bookmark => self.bookmark(window),
            Action::Zoom(step) => self.zoom(window, step),
            Action::Print => self.print(window),
            Action::SavePdf => self.save_pdf(window),
            Action::DevTools => self.devtools_hint(),
            Action::ToggleShields => self.toggle_shields(window),
            Action::EnableLists => self.enable_lists(),
            Action::RefreshLists => {
                let requested = self.shields.borrow().requested_fetches();
                self.fetch_lists(requested);
            }
            Action::OpenCommands
            | Action::GoTo
            | Action::OpenAddress
            | Action::Find
            | Action::FindNext
            | Action::FindPrevious
            | Action::Cycle(_)
            | Action::Overview
            | Action::Hints { .. }
            | Action::ShowDownloads
            | Action::ShowShields
            | Action::NewWindow
            | Action::CloseWindow
            | Action::CloseTab
            | Action::SetDefaultBrowser
            | Action::RevealProfile
            | Action::AskAgent
            | Action::StopAgent
            | Action::ShowAgentAnswer
            | Action::ShowAgentSettings
            | Action::ShowSettings
            | Action::DebugKeys
            | Action::Extensions(_) => {}
        }
        self.settle(window);
    }

    fn extension_command(&self, action: ExtensionsAction) {
        match action {
            ExtensionsAction::InstallFromUrl => self.install_extension_from_url(),
            ExtensionsAction::InstallFromFile => self.install_extension_file(),
            ExtensionsAction::CheckUpdates => self.check_extension_updates(),
            ExtensionsAction::OnSite { off } => self.extensions_on_site(off),
            ExtensionsAction::Show => self.show_extensions(),
        }
    }

    pub(super) fn navigate(&self, window: WindowId, navigation: &Navigation) {
        if let Some(tab) = self.active(window) {
            self.run(|browsing| browsing.navigate(tab, navigation));
        }
    }

    /// `⌘T`, `⇧⌘N`: a tab on the start page, with the address overlay open to say where to go.
    fn new_tab(&self, window: WindowId, jar: JarChoice) {
        let home = self.home.clone();
        let opened = self.run(|browsing| browsing.open(window, home, jar, Focus::Foreground));
        self.settle(window);
        if opened.is_some() {
            self.open_address();
        }
    }

    /// Opens `url` in a new tab of the current window, in the main jar.
    pub(super) fn open_in_new_tab(&self, url: Url) {
        let Some((window, _)) = self.current() else {
            return;
        };
        self.run(|browsing| browsing.open(window, url, JarChoice::Main, Focus::Foreground));
        self.settle(window);
    }

    /// The current page again, in a new throwaway jar: no cookies or storage of the main one.
    fn open_in_throwaway_jar(&self, window: WindowId) {
        let Some(url) = self.active_tab(window).map(|tab| tab.url) else {
            return;
        };
        self.run(|browsing| browsing.open(window, url, JarChoice::NewThrowaway, Focus::Foreground));
    }

    /// `⌘W`: the tab goes to the trash. The last tab of a window takes the window with it,
    /// unless it is the last window, which gets a fresh start page.
    fn close_tab(&self, window: WindowId) {
        if let Some(tab) = self.active(window) {
            self.close_tab_in(window, tab);
        }
    }

    fn reopen_closed(&self, window: WindowId) {
        self.run(|browsing| browsing.reopen_closed(window));
    }

    /// `⌘1` … `⌘9`. The pinned tab may live in another window, which comes forward.
    fn go_to_slot(&self, window: WindowId, slot: PinnedSlot) {
        let Some(Some(target)) = self.run(|browsing| browsing.go_to_slot(slot)) else {
            return;
        };
        if target != window
            && let Some(ns_window) = self.window(target)
        {
            ns_window.makeKeyAndOrderFront(None);
            self.settle(target);
        }
    }

    fn pin(&self, window: WindowId, slot: PinnedSlot) {
        let Some(tab) = self.active(window) else {
            return;
        };
        if self.run(|browsing| browsing.pin(tab, slot)).is_some() {
            self.toast(&format!("Pinned to ⌘{}", slot.get()));
        }
    }

    /// `⇧⌘C`, `⌥⌘C`, "Copy Page Title": what `text` makes of the active page, on the general
    /// pasteboard; `None` when it has nothing to copy.
    fn copy(
        &self,
        window: WindowId,
        text: impl FnOnce(quickie_domain_browsing::Tab) -> Option<String>,
    ) {
        let Some(tab) = self.active_tab(window) else {
            return;
        };
        let Some(text) = text(tab) else {
            return self.toast("Nothing to copy");
        };
        // SAFETY: an immutable `NSString` constant exported by AppKit.
        let kind = unsafe { NSPasteboardTypeString };
        let copied = general_pasteboard().is_some_and(|pasteboard| {
            pasteboard.clearContents();
            pasteboard.setString_forType(&NSString::from_str(&text), kind)
        });
        self.toast(if copied { "Copied" } else { "Couldn't copy" });
    }

    /// `⌘D`: the active page into the bookmarks, once.
    fn bookmark(&self, window: WindowId) {
        let Some(tab) = self.active_tab(window) else {
            return;
        };
        let saved = self.library.borrow_mut().save_bookmark(&tab.url, tab.title);
        match saved {
            Ok(saved) if saved.created => self.toast("Saved"),
            Ok(_) => self.toast("Already saved"),
            Err(error) => self.report(&error),
        }
    }

    /// Shields off on the active page's site, or back on; its pages load by the new rules once
    /// they are compiled.
    fn toggle_shields(&self, window: WindowId) {
        let Some(tab) = self.active_tab(window) else {
            return;
        };
        let toggled = self.shields.borrow_mut().toggle_site(&tab.url);
        let host = tab.url.host_str().unwrap_or_default();
        match toggled {
            Ok(policy) if policy.shields => self.toast(&format!("Shields up on {host}")),
            Ok(_) => self.toast(&format!("Shields down on {host}")),
            Err(error) => self.report(&error),
        }
    }

    /// Turns every allowed filter list on and fetches the ones never fetched: the user's
    /// request is what opens the connections.
    pub(super) fn enable_lists(&self) {
        let enabled = self.shields.borrow_mut().enable_lists(&FilterListId::ALL);
        if let Err(error) = enabled {
            return self.report(&error);
        }
        let due = self.shields.borrow().due_fetches();
        if due.is_empty() {
            self.toast("Filter lists on");
        }
        self.fetch_lists(due);
    }

    /// Fetches lists on a worker thread and records them back on the main thread.
    pub(super) fn fetch_lists(&self, fetches: Vec<ListFetch>) {
        if fetches.is_empty() {
            return;
        }
        let browser = MainThreadBound::new(self.this.clone(), self.mtm);
        let spawned = thread::Builder::new()
            .name("quickie-filter-lists".to_owned())
            .spawn(move || {
                let answers = fetch_lists(&UrlSessionHttp::new(), &fetches);
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    if let Some(browser) = browser.into_inner(mtm).upgrade() {
                        browser.lists_fetched(answers);
                    }
                });
            });
        let said = match spawned {
            Ok(_) => "Fetching filter lists".to_owned(),
            Err(error) => format!("Couldn't fetch filter lists: {error}"),
        };
        self.toast(&said);
        *self.lists_status.borrow_mut() = format!("{said}…");
    }

    fn lists_fetched(&self, fetched: Vec<FetchedList>) {
        let refreshed = self.shields.borrow_mut().lists_fetched(fetched);
        let said = match refreshed {
            Ok(refreshed) if refreshed.failed.is_empty() => format!(
                "Filter lists: {} updated, {} unchanged",
                refreshed.updated.len(),
                refreshed.unchanged.len()
            ),
            Ok(refreshed) => format!(
                "Filter lists: {} could not be fetched",
                refreshed.failed.len()
            ),
            Err(error) => format!("That didn't work: {error}"),
        };
        self.toast(&said);
        *self.lists_status.borrow_mut() = said;
        self.refresh_shields_popover();
        self.refresh_settings();
    }

    pub(super) fn active_tab(&self, window: WindowId) -> Option<quickie_domain_browsing::Tab> {
        let browsing = self.browsing.borrow();
        let tab = browsing.session().active(window)?;
        browsing.session().tab(tab).cloned()
    }
}
