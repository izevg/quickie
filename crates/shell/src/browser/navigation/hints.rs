//! Link hints: `F` asks the active page where its links and controls are, the labels show
//! over them, and typing one follows it. The page answers asynchronously, off the key.

use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_application::{Followed, LinkTarget, PageContent, PageRect, Ticket};
use quickie_domain_browsing::{Focus, JarChoice, TabId};
use quickie_domain_commands::{Key, KeyStroke, Modifiers};
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
        let ticket = self
            .browsing
            .borrow_mut()
            .engine_mut()
            .link_targets(tab)
            .ok();
        self.navigation
            .targets_asked
            .set(ticket.map(|ticket| (ticket, window, background)));
    }

    /// The page answered ([`EngineEvent::Targets`]). It must be the answer the hints wait
    /// for, and its tab still the one on screen.
    ///
    /// [`EngineEvent::Targets`]: quickie_application::EngineEvent::Targets
    pub(in crate::browser) fn show_hints(
        &self,
        tab: TabId,
        ticket: Ticket,
        targets: &[LinkTarget],
    ) {
        let Some((asked, window, background)) = self.navigation.targets_asked.get() else {
            return;
        };
        if asked != ticket {
            return;
        }
        self.navigation.targets_asked.set(None);
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
        // The page view fills the host, whose origin is at its bottom left.
        let height = host.bounds().size.height;
        let frames: Vec<NSRect> = targets
            .iter()
            .map(|target| in_host(target.frame, height))
            .collect();
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
        let ticket = self
            .browsing
            .borrow_mut()
            .engine_mut()
            .follow_link(tab, index, background)
            .ok();
        self.navigation
            .following
            .set(ticket.map(|ticket| (ticket, window)));
    }

    /// What following left for the browser ([`EngineEvent::Followed`]): a link to open in
    /// another tab, in the opener's jar.
    ///
    /// [`EngineEvent::Followed`]: quickie_application::EngineEvent::Followed
    pub(in crate::browser) fn followed(&self, opener: TabId, ticket: Ticket, followed: Followed) {
        let Some((asked, window)) = self.navigation.following.get() else {
            return;
        };
        if asked != ticket {
            return;
        }
        self.navigation.following.set(None);
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

/// A rectangle of the page view, origin top left, in a host of `height` whose origin is at
/// its bottom left.
fn in_host(rect: PageRect, height: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(rect.x, height - rect.y - rect.height),
        NSSize::new(rect.width, rect.height),
    )
}

#[cfg(test)]
mod tests {
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use quickie_application::PageRect;

    use super::in_host;

    #[test]
    fn a_page_rect_flips_into_the_host_from_its_bottom_edge() {
        let rect = PageRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 30.0,
        };

        assert_eq!(
            in_host(rect, 600.0),
            NSRect::new(NSPoint::new(10.0, 550.0), NSSize::new(100.0, 30.0))
        );
    }
}
