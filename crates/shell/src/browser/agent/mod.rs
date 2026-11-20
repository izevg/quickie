//! The built-in agent in Ask mode: `?` in the palette asks it about the page, its answer
//! streams into the answer panel, the badge shows while it works, and its settings set the
//! endpoint. Nothing here contacts the endpoint on its own: asking, listing models, and
//! looking for a local LLM each follow a key press or a click.
//!
//! - [`answer`]: reading the page, asking on a worker thread, the answer's events on the main
//!   thread batched per frame, citations, links, stopping.
//! - [`settings`]: the agent settings form (the Settings window's Agent pane) and its requests.
//!
//! No question and no answer is kept anywhere but on screen: none is written to the store,
//! whichever jar the page is in, throwaway or not.

mod answer;
mod settings;

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::MainThreadMarker;
use quickie_application::StopSignal;
use quickie_domain_agents::Request;
use quickie_domain_browsing::TabId;
use quickie_system::KeychainSecrets;
use quickie_ui_shared::Trigger;
use quickie_ui_widgets::{
    Activity, AgentAnswerPanel, AgentBadge, AgentFrame, AgentSettings, agent_wording,
};

use super::Browser;
use crate::input::keystroke::OverlayKey;

/// The agent's views and the answer in progress.
pub struct AgentUi {
    panel: AgentAnswerPanel,
    badge: AgentBadge,
    /// The border around the window while the agent acts.
    frame: AgentFrame,
    /// The settings form, shown in the Settings window.
    pub(super) settings: AgentSettings,
    /// The Keychain as the worker threads read it.
    secrets: KeychainSecrets,
    /// Counts asks; an event or a page text of an older one is dropped.
    generation: Cell<u64>,
    /// The answer streaming now.
    running: RefCell<Option<Running>>,
    /// What was asked last, to ask it again when the answer failed.
    last: RefCell<Option<Request>>,
    /// The pages being read for the latest ask, until every text arrived.
    reading: RefCell<Option<answer::Reading>>,
    /// The tab the answer is about: its citations are found there.
    asked: Cell<Option<TabId>>,
    /// The tab whose page shows a citation's marks.
    cited: Cell<Option<TabId>>,
}

/// An answer on its way: the page being read, then the request out.
struct Running {
    stop: StopSignal,
    /// Still reading the page; nothing was sent yet.
    reading: bool,
    first_token: Option<Instant>,
    tokens: u64,
    /// The speed the badge shows, and when it was measured: it changes at most twice a
    /// second, so the badge is laid out again that often, not on every token.
    shown_rate: Option<(Instant, Option<f64>)>,
}

impl AgentUi {
    pub fn new(secrets: KeychainSecrets, mtm: MainThreadMarker) -> Self {
        Self {
            panel: AgentAnswerPanel::new(mtm),
            badge: AgentBadge::new(mtm),
            frame: AgentFrame::new(mtm),
            settings: AgentSettings::new(mtm),
            secrets,
            generation: Cell::new(0),
            running: RefCell::new(None),
            last: RefCell::new(None),
            reading: RefCell::new(None),
            asked: Cell::new(None),
            cited: Cell::new(None),
        }
    }

    /// Whether the answer panel shows.
    pub fn is_open(&self) -> bool {
        self.panel.is_open()
    }
}

impl Browser {
    /// Hands the panel's and the settings' requests to the browser.
    pub(super) fn wire_agent(&self) {
        let this = self.this.clone();
        self.agent_ui.panel.on_request(move |request| {
            if let Some(browser) = this.upgrade() {
                browser.panel_request(request);
                browser.drain();
            }
        });
        let this = self.this.clone();
        self.agent_ui.settings.on_request(move |request| {
            if let Some(browser) = this.upgrade() {
                browser.settings_request(request);
                browser.drain();
            }
        });
    }

    /// Takes the panel away; an answer streams on, and the panel shows it again when reopened.
    pub(super) fn hide_agent(&self) {
        self.agent_ui.panel.hide();
        self.clear_citation();
        self.place_badge();
    }

    /// The panel's fixed keys: `⎋` closes it, `⏎` follows up on an answer or retries a failed
    /// one. `false` when it does not show or the key is not its.
    pub(super) fn agent_key(&self, key: OverlayKey) -> bool {
        if !self.agent_ui.panel.is_open() {
            return false;
        }
        match key {
            OverlayKey::Dismiss => {
                self.close_agent();
                true
            }
            OverlayKey::Accept {
                new_tab: false,
                throwaway: false,
            } => {
                match self.agent_ui.panel.return_request() {
                    Some(request) => self.panel_request(request),
                    None => return false,
                }
                true
            }
            _ => false,
        }
    }

    /// `⎋`: the panel goes, and the page takes the keyboard back.
    fn close_agent(&self) {
        self.hide_agent();
        if let Some((window, _)) = self.current() {
            self.settle(window);
        }
    }

    /// "Show Agent Answer": the panel again, with the last answer, or the empty state when no
    /// endpoint is set up.
    pub(super) fn show_answer(&self) {
        if self.agent.borrow().endpoint().is_none() && self.agent_ui.running.borrow().is_none() {
            self.agent_ui.panel.unconfigured();
        } else if self.agent_ui.asked.get().is_none() {
            self.toast("No answer yet: type ? in the palette to ask about this page");
            return;
        }
        self.open_panel(Trigger::Keyboard);
    }

    /// `⌘.`: stops the answer; what came so far stays.
    pub(super) fn stop_agent(&self) {
        match self.agent_ui.running.borrow().as_ref() {
            Some(running) => running.stop.stop(),
            None => self.toast("The agent isn't running"),
        }
    }

    /// Opens the panel over the current window, the only E2.
    fn open_panel(&self, trigger: Trigger) {
        let Some(host) = self.current().and_then(|(_, window)| window.contentView()) else {
            return;
        };
        self.close_overlays();
        self.hide_find();
        self.agent_ui.panel.show(&host, trigger);
        self.place_badge();
    }

    /// The badge and the frame while the agent works, the badge over the page beside the
    /// panel; gone otherwise.
    fn place_badge(&self) {
        let ui = &self.agent_ui;
        let text = ui.running.borrow().as_ref().map(Running::badge);
        let host = self.current().and_then(|(_, window)| window.contentView());
        if let (Some(text), Some(host)) = (text, host) {
            let width = if ui.panel.is_open() {
                ui.panel.view().frame().origin.x
            } else {
                host.bounds().size.width
            };
            ui.frame.show(&host);
            ui.badge.show(&host, &text, width);
        } else {
            ui.badge.hide();
            ui.frame.hide();
        }
    }

    /// Runs `work` on a worker thread named `name`, then `done` with its result on the main
    /// thread. `false` when no thread could start.
    pub(in crate::browser) fn off_main<T: Send + 'static>(
        &self,
        name: &str,
        work: impl FnOnce() -> T + Send + 'static,
        done: fn(&Self, T),
    ) -> bool {
        let browser = MainThreadBound::new(self.this.clone(), self.mtm);
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let result = work();
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    if let Some(browser) = browser.into_inner(mtm).upgrade() {
                        done(&browser, result);
                        browser.drain();
                    }
                });
            })
            .is_ok()
    }

    /// A handle on the browser that worker threads can hold and the main thread can use.
    fn shared(&self) -> Arc<MainThreadBound<std::rc::Weak<Self>>> {
        Arc::new(MainThreadBound::new(self.this.clone(), self.mtm))
    }
}

impl Running {
    fn new() -> Self {
        Self {
            stop: StopSignal::new(),
            reading: true,
            first_token: None,
            tokens: 0,
            shown_rate: None,
        }
    }

    /// Counts the tokens of a batch, and measures the speed again when it is due.
    fn received(&mut self, tokens: u64) {
        if tokens == 0 {
            return;
        }
        let first = *self.first_token.get_or_insert_with(Instant::now);
        self.tokens += tokens;
        let due = self
            .shown_rate
            .is_none_or(|(at, _)| at.elapsed() >= RATE_EVERY);
        if due {
            self.shown_rate = Some((Instant::now(), rate(self.tokens, first.elapsed())));
        }
    }

    /// What the agent does, and how fast as last measured.
    fn activity(&self) -> Activity {
        match self.first_token {
            _ if self.reading => Activity::Reading,
            None => Activity::Asking,
            Some(_) => Activity::Answering(self.shown_rate.and_then(|(_, rate)| rate)),
        }
    }

    fn badge(&self) -> String {
        agent_wording::badge(self.activity())
    }
}

/// How often the badge's speed is measured again.
const RATE_EVERY: std::time::Duration = std::time::Duration::from_millis(500);

/// Tokens per second since the first, once half a second has passed to measure it.
fn rate(tokens: u64, since_first: std::time::Duration) -> Option<f64> {
    let seconds = since_first.as_secs_f64();
    #[expect(
        clippy::cast_precision_loss,
        reason = "a token count stays far below 2^52"
    )]
    let tokens = tokens as f64;
    (seconds >= 0.5).then(|| tokens / seconds)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::rate;

    #[test]
    fn the_rate_waits_half_a_second_to_be_measured() {
        assert_eq!(rate(3, Duration::from_millis(100)), None);
        assert_eq!(rate(12, Duration::from_millis(500)), Some(24.0));
        assert_eq!(rate(48, Duration::from_secs(2)), Some(24.0));
    }
}
