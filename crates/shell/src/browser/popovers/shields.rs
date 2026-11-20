//! The shields popover: the active page's site policy, its blocked count, and the enabled filter
//! lists, from the shields use cases; its switches and buttons go back to them.

use std::time::SystemTime;

use quickie_application::{Clock, Engine, Navigation};
use quickie_domain_shields::{Site, SitePolicy};
use quickie_engine_webkit::BlockerStatus;
use quickie_ui_shared::Trigger;
use quickie_ui_shared::format::ago;
use quickie_ui_widgets::{ShieldsRequest, ShieldsSummary, SiteToggle};

use super::super::Browser;
use crate::input::keystroke::OverlayKey;
use crate::{MainEngine, SystemClock};

impl Browser {
    /// `⌥⌘S`: opens the popover for the active page, or closes it when it shows.
    pub(in crate::browser) fn toggle_shields_popover(&self, trigger: Trigger) {
        if self.popovers.shields.is_open() {
            self.close_popovers();
            return;
        }
        self.close_overlays();
        self.hide_find();
        let Some(host) = self.current().and_then(|(_, window)| window.contentView()) else {
            return;
        };
        if let Some(summary) = self.shields_summary() {
            self.popovers.shields.show(&host, summary, trigger);
        }
    }

    /// What the popover shows about the current window's active page.
    fn shields_summary(&self) -> Option<ShieldsSummary> {
        let (window, _) = self.current()?;
        let tab = self.active(window)?;
        let url = self.active_tab(window)?.url;
        let shields = self.shields.borrow();
        let lists = shields
            .lists()
            .enabled()
            .map(|list| (list.id, list.fetched.map(|fetch| fetch.at)))
            .collect();
        let blocked = shields.blocked_count(tab);
        let policy = shields.policy(&url);
        drop(shields);
        Some(ShieldsSummary {
            site: Site::try_from(&url).ok().map(|site| site.to_string()),
            policy,
            // Counted by the engine's content blocker, from zero with each page; none on an
            // engine that counts nothing.
            blocked: <MainEngine as Engine>::CAPS
                .blocked_counts
                .then_some(blocked),
            lists,
            now: SystemClock.now(),
            reload_offered: self.popovers.reload_offered.get() == Some(tab),
            rules: self.rules_line(),
        })
    }

    /// What the content blocker made of the lists, measured: rules active, compiling, or why
    /// it failed; empty when it was never given lists.
    pub(in crate::browser) fn rules_line(&self) -> String {
        let mut shields = self.shields.borrow_mut();
        let blocker = shields.blocker_mut();
        let cosmetics = if blocker.cosmetics_ready() {
            ""
        } else {
            " · cosmetic filters loading…"
        };
        match blocker.status() {
            BlockerStatus::Idle => String::new(),
            BlockerStatus::Compiling => "Compiling rules…".to_owned(),
            BlockerStatus::Installed(installed) if installed.lists == 0 => {
                "No rules active: no enabled list has arrived yet".to_owned()
            }
            BlockerStatus::Installed(installed) => {
                let since = SystemTime::now()
                    .duration_since(installed.compiled)
                    .map_or(0, |since| {
                        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
                    });
                format!(
                    "Rules active: {} rules in {} {}, compiled {}{cosmetics}",
                    grouped(installed.rules),
                    installed.lists,
                    if installed.lists == 1 {
                        "list"
                    } else {
                        "lists"
                    },
                    ago(since),
                )
            }
            BlockerStatus::Failed(reason) => format!("Failed: {reason}"),
        }
    }

    /// The content blocker settled the latest rules: Settings and the popover show it. When
    /// the user's own change of lists brought them, the open pages are offered a reload, as
    /// rules apply only to loads made from now on.
    pub(in crate::browser) fn rules_settled(&self) {
        let status = self.shields.borrow_mut().blocker_mut().status();
        if self.rules_wanted.replace(false) {
            match status {
                BlockerStatus::Installed(installed) if installed.lists > 0 => {
                    if let Some(tab) = self.current().and_then(|(window, _)| self.active(window)) {
                        self.popovers.reload_offered.set(Some(tab));
                    }
                    self.toast("Filter rules are active · reload open pages to apply them");
                }
                BlockerStatus::Failed(reason) => {
                    self.toast(&format!("Filter rules failed: {reason}"));
                }
                _ => {}
            }
        }
        self.refresh_shields_popover();
        self.refresh_settings();
    }

    /// The popover shows the shields as they are now.
    pub(in crate::browser) fn refresh_shields_popover(&self) {
        if self.popovers.shields.is_open()
            && let Some(summary) = self.shields_summary()
        {
            self.popovers.shields.update(summary);
        }
    }

    /// What a switch or a button in the popover asked.
    pub(in crate::browser) fn shields_request(&self, request: ShieldsRequest) {
        let Some((window, _)) = self.current() else {
            return;
        };
        match request {
            ShieldsRequest::Set(toggle, on) => self.set_site_toggle(window, toggle, on),
            ShieldsRequest::RefreshLists => {
                let requested = self.shields.borrow().requested_fetches();
                self.fetch_lists(requested);
            }
            ShieldsRequest::EnableLists => self.enable_lists(),
            ShieldsRequest::Reload => return self.reload_to_apply(),
        }
        self.refresh_shields_popover();
    }

    /// Sets one switch of the active page's site. Shields changed: a reload applies them.
    fn set_site_toggle(
        &self,
        window: quickie_domain_browsing::WindowId,
        toggle: SiteToggle,
        on: bool,
    ) {
        let (Some(tab), Some(url)) = (self.active(window), self.active_tab(window).map(|t| t.url))
        else {
            return;
        };
        let Some(policy) = self.shields.borrow().policy(&url) else {
            return;
        };
        let next = match toggle {
            SiteToggle::Shields => SitePolicy {
                shields: on,
                ..policy
            },
            SiteToggle::HttpsOnly => SitePolicy {
                https_only: on,
                ..policy
            },
            SiteToggle::JavaScript => SitePolicy {
                javascript: on,
                ..policy
            },
        };
        let changed = self.shields.borrow_mut().set_policy(&url, next);
        match changed {
            Ok(true) if toggle == SiteToggle::Shields => {
                self.popovers.reload_offered.set(Some(tab));
            }
            Ok(_) => {}
            Err(error) => self.report(&error),
        }
    }

    /// `⏎` or the button: the page loads again under its shields.
    fn reload_to_apply(&self) {
        self.popovers.reload_offered.set(None);
        self.hide_popovers();
        if let Some((window, _)) = self.current() {
            self.navigate(window, &Navigation::Reload);
            self.settle(window);
        }
    }

    /// The page the offer was for loaded again: nothing is left to apply.
    pub(in crate::browser) fn applied(&self, tab: quickie_domain_browsing::TabId) {
        if self.popovers.reload_offered.get() == Some(tab) {
            self.popovers.reload_offered.set(None);
            self.refresh_shields_popover();
        }
    }

    /// Fetches what is due at start, and has the content blocker tell the browser each time the
    /// latest rules are installed or fail.
    pub(in crate::browser) fn wire_shields(&self) {
        // Only lists the user enabled and that never arrived, or stale ones when the user keeps
        // lists fresh: nothing on a fresh profile.
        let due = self.shields.borrow().due_fetches();
        self.fetch_lists(due);
        let settled = self.this.clone();
        self.shields.borrow_mut().blocker_mut().on_settled(move || {
            if let Some(browser) = settled.upgrade() {
                browser.rules_settled();
            }
        });
    }

    /// Marks that the user changed the lists: the rules they bring offer a reload once
    /// compiled.
    pub(in crate::browser) fn lists_changed_by_user(&self) {
        self.rules_wanted.set(true);
    }

    pub(super) fn shields_key(&self, key: OverlayKey) -> bool {
        match key {
            OverlayKey::Accept { .. } if self.popovers.shields.reload_offered() => {
                self.reload_to_apply();
                true
            }
            OverlayKey::Dismiss => {
                self.close_popovers();
                true
            }
            OverlayKey::Accept { .. } | OverlayKey::Move(_) | OverlayKey::Previous => false,
        }
    }
}

/// `81203` → `81,203`.
fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::grouped;

    #[test]
    fn counts_are_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(81_203), "81,203");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }
}
