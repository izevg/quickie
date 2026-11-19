//! The dispatcher: one place where a command id becomes the use case that does it, whether a
//! key or the palette asked. A command Quickie lists but cannot do yet says so in a toast
//! rather than doing nothing.

use std::thread;

use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::rc::Retained;
use objc2::{ClassType, MainThreadMarker, msg_send};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;
use quickie_application::{FetchedList, ListFetch, Navigation, fetch_lists};
use quickie_domain_browsing::{Direction, Focus, JarChoice, PinnedSlot, WindowId};
use quickie_domain_commands::{CommandId, KeyStroke, Modifiers};
use quickie_domain_shields::FilterListId;
use quickie_engine_webkit::UrlSessionHttp;
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
    Back,
    Forward,
    CopyUrl,
    CopyMarkdownLink,
    Bookmark,
    Find,
    FindNext,
    FindPrevious,
    ToggleShields,
    EnableLists,
    RefreshLists,
}

impl Action {
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
            "quickie.navigation.back" => Self::Back,
            "quickie.navigation.forward" => Self::Forward,
            "quickie.page.copyUrl" => Self::CopyUrl,
            "quickie.page.copyAsMarkdownLink" => Self::CopyMarkdownLink,
            "quickie.page.bookmark" => Self::Bookmark,
            "quickie.page.find" => Self::Find,
            "quickie.page.findNext" => Self::FindNext,
            "quickie.page.findPrevious" => Self::FindPrevious,
            "quickie.shields.toggleSite" => Self::ToggleShields,
            "quickie.shields.enableLists" => Self::EnableLists,
            "quickie.shields.refreshLists" => Self::RefreshLists,
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
    /// Runs the command `id`. `stroke` is the key that asked, `None` for the palette.
    pub(super) fn dispatch(&self, id: &CommandId, stroke: Option<KeyStroke>) {
        self.pending.set(None);
        let Some(action) = Action::of(id) else {
            self.close_overlays();
            let title = self
                .catalog
                .get(id)
                .map_or(id.as_str(), |command| command.title);
            self.toast(&format!("Not yet: {title}"));
            return;
        };
        match action {
            Action::OpenCommands => return self.open_palette(">"),
            Action::GoTo => return self.open_palette(""),
            Action::OpenAddress => return self.open_address(),
            Action::Find => return self.open_find(),
            Action::FindNext => return self.find_step(false),
            Action::FindPrevious => return self.find_step(true),
            Action::Cycle(direction) => {
                if let Some((window, _)) = self.current() {
                    self.cycle(window, direction, holding(stroke));
                }
                return;
            }
            _ => {}
        }
        self.close_overlays();
        let Some((window, _)) = self.current() else {
            return;
        };
        match action {
            Action::NewTab => self.new_tab(window, JarChoice::Main),
            Action::NewThrowawayTab => self.new_tab(window, JarChoice::NewThrowaway),
            Action::CloseTab => self.close_tab(window),
            Action::ReopenClosed => self.reopen_closed(window),
            Action::GoToSlot(slot) => self.go_to_slot(window, slot),
            Action::PinToSlot(slot) => self.pin(window, slot),
            Action::OpenInThrowawayJar => self.open_in_throwaway_jar(window),
            Action::Reload => self.navigate(window, &Navigation::Reload),
            Action::Back => self.navigate(window, &Navigation::Back),
            Action::Forward => self.navigate(window, &Navigation::Forward),
            Action::CopyUrl => self.copy(window, |tab| tab.url.to_string()),
            Action::CopyMarkdownLink => self.copy(window, |tab| tab.markdown_link()),
            Action::Bookmark => self.bookmark(window),
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
            | Action::Cycle(_) => {}
        }
        self.settle(window);
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

    /// `⌘W`: the tab goes to the trash. A window left empty gets a fresh start page for now,
    /// rather than closing the last window.
    fn close_tab(&self, window: WindowId) {
        if let Some(tab) = self.active(window) {
            self.run(|browsing| browsing.close(tab));
        }
        if self.active(window).is_none() {
            let home = self.home.clone();
            self.run(|browsing| browsing.open(window, home, JarChoice::Main, Focus::Foreground));
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

    /// `⇧⌘C`, `⌥⌘C`: the active page as `text` makes it, on the general pasteboard.
    fn copy(&self, window: WindowId, text: impl FnOnce(quickie_domain_browsing::Tab) -> String) {
        let Some(tab) = self.active_tab(window) else {
            return;
        };
        // SAFETY: an immutable `NSString` constant exported by AppKit.
        let kind = unsafe { NSPasteboardTypeString };
        let copied = general_pasteboard().is_some_and(|pasteboard| {
            pasteboard.clearContents();
            pasteboard.setString_forType(&NSString::from_str(&text(tab)), kind)
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
    fn enable_lists(&self) {
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
        match spawned {
            Ok(_) => self.toast("Fetching filter lists"),
            Err(error) => self.toast(&format!("Couldn't fetch filter lists: {error}")),
        }
    }

    fn lists_fetched(&self, fetched: Vec<FetchedList>) {
        let refreshed = self.shields.borrow_mut().lists_fetched(fetched);
        match refreshed {
            Ok(refreshed) if refreshed.failed.is_empty() => self.toast(&format!(
                "Filter lists: {} updated, {} unchanged",
                refreshed.updated.len(),
                refreshed.unchanged.len()
            )),
            Ok(refreshed) => self.toast(&format!(
                "Filter lists: {} could not be fetched",
                refreshed.failed.len()
            )),
            Err(error) => self.report(&error),
        }
    }

    fn active_tab(&self, window: WindowId) -> Option<quickie_domain_browsing::Tab> {
        let browsing = self.browsing.borrow();
        let tab = browsing.session().active(window)?;
        browsing.session().tab(tab).cloned()
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::{Direction, PinnedSlot};
    use quickie_domain_commands::{Catalog, CommandId, KeyStroke, Modifiers};

    use super::{Action, holding};

    fn of(id: &str) -> Option<Action> {
        Action::of(&CommandId::try_from(id).unwrap())
    }

    fn slot(n: u8) -> PinnedSlot {
        PinnedSlot::try_from(n).unwrap()
    }

    #[test]
    fn the_commands_this_shell_does_have_actions() {
        let cases = [
            ("quickie.palette.openCommands", Action::OpenCommands),
            ("quickie.palette.goto", Action::GoTo),
            ("quickie.address.open", Action::OpenAddress),
            ("quickie.tab.new", Action::NewTab),
            ("quickie.tab.newThrowaway", Action::NewThrowawayTab),
            ("quickie.tab.close", Action::CloseTab),
            ("quickie.tab.reopenClosed", Action::ReopenClosed),
            ("quickie.tab.nextRecent", Action::Cycle(Direction::Forward)),
            (
                "quickie.tab.previousRecent",
                Action::Cycle(Direction::Backward),
            ),
            ("quickie.tab.openInThrowawayJar", Action::OpenInThrowawayJar),
            ("quickie.tab.reload", Action::Reload),
            ("quickie.navigation.back", Action::Back),
            ("quickie.navigation.forward", Action::Forward),
            ("quickie.page.copyUrl", Action::CopyUrl),
            ("quickie.page.copyAsMarkdownLink", Action::CopyMarkdownLink),
            ("quickie.page.bookmark", Action::Bookmark),
            ("quickie.page.find", Action::Find),
            ("quickie.page.findNext", Action::FindNext),
            ("quickie.page.findPrevious", Action::FindPrevious),
            ("quickie.shields.toggleSite", Action::ToggleShields),
            ("quickie.shields.enableLists", Action::EnableLists),
            ("quickie.shields.refreshLists", Action::RefreshLists),
        ];
        for (id, action) in cases {
            assert_eq!(of(id), Some(action), "{id}");
        }
    }

    #[test]
    fn every_slot_goes_and_pins() {
        for n in 1..=9 {
            assert_eq!(
                of(&format!("quickie.tab.goToSlot{n}")),
                Some(Action::GoToSlot(slot(n)))
            );
            assert_eq!(
                of(&format!("quickie.tab.pinToSlot{n}")),
                Some(Action::PinToSlot(slot(n)))
            );
        }
        assert_eq!(of("quickie.tab.goToSlot0"), None);
        assert_eq!(of("quickie.tab.goToSlot10"), None);
    }

    #[test]
    fn commands_not_built_yet_have_no_action() {
        for id in [
            "quickie.reader.toggle",
            "quickie.hints.toggle",
            "quickie.devtools.toggle",
        ] {
            assert_eq!(of(id), None, "{id}");
        }
    }

    #[test]
    fn every_action_names_a_command_of_the_catalog() {
        let catalog = Catalog::builtin();
        let built = catalog
            .iter()
            .filter(|command| Action::of(&command.id).is_some())
            .count();

        // 22 named commands plus nine slots to go to and nine to pin.
        assert_eq!(built, 22 + 9 + 9);
    }

    #[test]
    fn a_cycle_is_held_by_its_modifiers_but_shift() {
        let backward = KeyStroke::try_from("ctrl+shift+tab").unwrap();
        let bare = KeyStroke::try_from("tab").unwrap();

        assert_eq!(holding(Some(backward)), Modifiers::CONTROL);
        assert_eq!(holding(Some(bare)), Modifiers::NONE);
        assert_eq!(holding(None), Modifiers::NONE);
    }
}
