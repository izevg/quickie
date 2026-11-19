//! Link hints: `F` asks the active page where its links and controls are, the labels show
//! over them, and typing one follows it. The page answers asynchronously, off the key.

use objc2_foundation::NSRect;
use quickie_domain_browsing::{Focus, JarChoice, TabId, WindowId};
use quickie_domain_commands::{Key, KeyStroke, Modifiers};
use quickie_engine_webkit::{Followed, LinkTarget};
use quickie_ui_widgets::HintKey;

use super::super::Browser;

impl Browser {
    /// `F` (`⇧F` for `background`): labels what the active page shows.
    pub(in crate::browser) fn open_hints(&self, background: bool) {
        let Some((window, _)) = self.current() else {
            return;
        };
        let Some(tab) = self.active(window) else {
            return;
        };
        self.close_overlays();
        self.hide_navigation();
        let this = self.this.clone();
        self.browsing
            .borrow_mut()
            .engine_mut()
            .link_targets(tab, move |targets| {
                if let Some(browser) = this.upgrade() {
                    browser.show_hints(window, tab, &targets, background);
                }
            });
    }

    /// The page answered. Its tab must still be the one on screen.
    fn show_hints(&self, window: WindowId, tab: TabId, targets: &[LinkTarget], background: bool) {
        if self.active(window) != Some(tab) {
            return;
        }
        let Some(host) = self.window(window).and_then(|window| window.contentView()) else {
            return;
        };
        if targets.is_empty() {
            self.toast("No links on screen");
            return;
        }
        let frames: Vec<NSRect> = targets.iter().map(|target| target.frame).collect();
        self.navigation.hints.show(&host, &frames, background);
        self.navigation.hinted.set(Some((window, tab)));
    }

    /// Keys while the hints show: letters type a label (`⇧` on the last one opens a link in
    /// the background), `⌫` takes one back, `⎋` cancels. A shortcut with `⌘`, `⌃`, or `⌥`
    /// cancels them and runs; any other key does nothing.
    pub(super) fn hints_key(&self, stroke: KeyStroke) -> bool {
        let hints = &self.navigation.hints;
        if !hints.is_open() {
            return false;
        }
        let modifiers = stroke.modifiers;
        if modifiers != Modifiers::NONE && modifiers != Modifiers::SHIFT {
            hints.hide();
            return false;
        }
        match stroke.key {
            Key::Escape => hints.hide(),
            Key::Backspace => hints.backspace(),
            Key::Letter(letter) => {
                if let HintKey::Follow(index) = hints.type_letter(letter.as_char()) {
                    let background = hints.background() || modifiers == Modifiers::SHIFT;
                    self.follow_hint(index, background);
                }
            }
            _ => {}
        }
        true
    }

    fn follow_hint(&self, index: usize, background: bool) {
        let Some((window, tab)) = self.navigation.hinted.take() else {
            return;
        };
        let this = self.this.clone();
        self.browsing.borrow_mut().engine_mut().follow_link(
            tab,
            index,
            background,
            move |followed| {
                if let Some(browser) = this.upgrade() {
                    browser.followed(window, tab, followed);
                    browser.drain();
                }
            },
        );
    }

    /// What following left for the browser: a link to open in another tab, in the opener's
    /// jar.
    fn followed(&self, window: WindowId, opener: TabId, followed: Followed) {
        let (url, focus) = match followed {
            Followed::Done => return,
            Followed::Background(url) => (url, Focus::Background),
            Followed::NewTab(url) => (url, Focus::Foreground),
        };
        let jar = JarChoice::SameAs(opener);
        let tab = self.run(|browsing| browsing.open(window, url, jar, focus));
        if tab.is_some() && focus == Focus::Background {
            self.toast("Opened in a background tab");
        }
        self.settle(window);
    }
}
