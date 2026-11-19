//! The shields popover: the active page's site policy, its blocked count, and the enabled filter
//! lists, from the shields use cases; its switches and buttons go back to them.

use quickie_application::{Clock, Navigation};
use quickie_domain_shields::{Site, SitePolicy};
use quickie_ui_shared::Trigger;
use quickie_ui_widgets::{ShieldsRequest, ShieldsSummary, SiteToggle};

use super::super::Browser;
use crate::SystemClock;
use crate::input::keystroke::OverlayKey;

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
        Some(ShieldsSummary {
            site: Site::try_from(&url).ok().map(|site| site.to_string()),
            policy: shields.policy(&url),
            // The WebKit engine blocks with content rule lists, which count nothing.
            blocked: None,
            lists,
            now: SystemClock.now(),
            reload_offered: self.popovers.reload_offered.get() == Some(tab),
        })
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
