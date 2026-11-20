//! Asking and answering. `⏎` on a `?` row reads the page (its rendered text, from a script in
//! Quickie's isolated content world), the agent use case assembles the prompt on the main
//! thread, and a worker thread streams the answer. Its events collect in an inbox, and the main
//! thread takes them at most once a frame: however fast tokens come, the panel does one edit
//! per frame, of the line still coming in.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchTime, MainThreadBound};
use objc2::MainThreadMarker;
use quickie_application::{AnswerEvent, Asking, StopSignal};
use quickie_domain_agents::{PageScope, PageText, Question, Request, Skill, SkillId};
use quickie_domain_browsing::{Focus, JarChoice, TabId, WindowId};
use quickie_engine_webkit::{FindStep, UrlSessionHttp};
use quickie_llm::OpenAiClient;
use quickie_ui_shared::Trigger;
use quickie_ui_widgets::{Activity, AnswerHeader, PanelRequest, SettingsRequest, agent_wording};
use url::Url;

use super::super::Browser;
use super::Running;
use crate::SystemClock;

/// How long events wait to be shown together: about a frame.
const FRAME: Duration = Duration::from_millis(16);

/// The answer's events, from the worker thread to the main thread.
#[derive(Default)]
struct Inbox {
    events: Vec<AnswerEvent>,
    /// A delivery to the main thread is on its way.
    scheduled: bool,
}

type Shared = Arc<MainThreadBound<std::rc::Weak<Browser>>>;

impl Browser {
    /// `⏎` on a skill's row: runs it on the current page, or on the open tabs it compares.
    pub(in crate::browser) fn ask_skill(&self, id: &SkillId) {
        match Skill::builtin_named(id) {
            Some(skill) => self.ask(Request::Skill(skill)),
            None => self.toast(&format!("No skill named {id}")),
        }
    }

    /// `⏎` on "Ask about this page": asks the question about the current page.
    pub(in crate::browser) fn ask_question(&self, question: Question) {
        self.ask(Request::Question(question));
    }

    /// Opens the panel on the request and reads the pages; the answer follows.
    fn ask(&self, request: Request) {
        let Some((window, _)) = self.current() else {
            return;
        };
        let endpoint = self.agent.borrow().endpoint().cloned();
        let Some(endpoint) = endpoint else {
            self.agent_ui.panel.unconfigured();
            self.open_panel(Trigger::Keyboard);
            return;
        };
        self.cancel_answer();
        let pages = self.pages_to_read(window, request.scope());
        let Some(&(first, _, _)) = pages.first() else {
            self.toast("There is no page to read here yet");
            return;
        };
        let title = match &request {
            Request::Question(question) => question.as_str().to_owned(),
            Request::Skill(skill) => skill.name().to_owned(),
        };
        let ui = &self.agent_ui;
        let generation = ui.generation.get() + 1;
        ui.generation.set(generation);
        ui.asked.set(Some(first));
        *ui.running.borrow_mut() = Some(Running::new());
        ui.panel.start(&AnswerHeader {
            title,
            model: agent_wording::model_chip(&endpoint),
        });
        self.open_panel(Trigger::Keyboard);
        self.read_pages(generation, request, pages);
    }

    /// The tabs a request reads, the active one first, with their address and title: the
    /// active tab alone, or the window's open tabs that have a page, up to the scope's most.
    fn pages_to_read(&self, window: WindowId, scope: PageScope) -> Vec<(TabId, Url, String)> {
        let candidates: Vec<(TabId, Url, String)> = {
            let browsing = self.browsing.borrow();
            let session = browsing.session();
            let active = session.active(window);
            let mut tabs: Vec<_> = session
                .tabs(window)
                .map_or_else(|_| Vec::new(), |tabs| tabs.cloned().collect());
            tabs.sort_by_key(|tab| Some(tab.id) != active);
            tabs.into_iter()
                .filter(|tab| scope == PageScope::OpenTabs || Some(tab.id) == active)
                .map(|tab| (tab.id, tab.url, tab.title))
                .collect()
        };
        let mut browsing = self.browsing.borrow_mut();
        let engine = browsing.engine_mut();
        candidates
            .into_iter()
            .filter(|(tab, _, _)| engine.view(*tab).is_some())
            .take(scope.max_pages())
            .collect()
    }

    /// Reads each page's text; the last to arrive hands them all on.
    fn read_pages(&self, generation: u64, request: Request, pages: Vec<(TabId, Url, String)>) {
        let texts: Rc<RefCell<Vec<Option<PageText>>>> =
            Rc::new(RefCell::new(vec![None; pages.len()]));
        let left = Rc::new(Cell::new(pages.len()));
        let request = Rc::new(request);
        for (index, (tab, url, title)) in pages.into_iter().enumerate() {
            let (this, texts, left, request) = (
                self.this.clone(),
                Rc::clone(&texts),
                Rc::clone(&left),
                Rc::clone(&request),
            );
            let done = move |text: Option<String>| {
                if let Some(slot) = texts.borrow_mut().get_mut(index) {
                    *slot = Some(PageText::new(url, title, text.unwrap_or_default()));
                }
                left.set(left.get().saturating_sub(1));
                if left.get() > 0 {
                    return;
                }
                let pages: Vec<PageText> = texts.take().into_iter().flatten().collect();
                if let Some(browser) = this.upgrade() {
                    browser.pages_read(generation, &request, &pages);
                }
            };
            self.browsing.borrow_mut().engine_mut().page_text(tab, done);
        }
    }

    /// The pages are read: the prompt is assembled here, and the answer streams on a worker.
    fn pages_read(&self, generation: u64, request: &Request, pages: &[PageText]) {
        let ui = &self.agent_ui;
        if generation != ui.generation.get() {
            return;
        }
        let stop = match ui.running.borrow_mut().as_mut() {
            Some(running) => {
                running.reading = false;
                running.stop.clone()
            }
            None => return,
        };
        if stop.is_stopped() {
            self.answer_ended(&[AnswerEvent::Stopped]);
            return;
        }
        let asking = self.agent.borrow().ask(request, pages);
        match asking {
            Ok(asking) => {
                ui.panel.set_activity(Activity::Asking);
                if !self.stream(asking, generation, stop) {
                    self.answer_failed("The answer couldn't start: no thread for it.");
                }
            }
            Err(error) => self.answer_failed(&format!("The agent couldn't ask: {error}.")),
        }
        self.place_badge();
    }

    /// Streams `asking` on a worker thread; its events reach [`Self::answer_events`] in
    /// batches, at most one per frame. `false` when the thread could not start.
    fn stream(&self, asking: Asking, generation: u64, stop: StopSignal) -> bool {
        let shared = self.shared();
        let secrets = self.agent_ui.secrets.clone();
        thread::Builder::new()
            .name("quickie-agent-answer".to_owned())
            .spawn(move || {
                let inbox = Arc::new(Mutex::new(Inbox::default()));
                let llm = OpenAiClient::new(UrlSessionHttp::new());
                asking.run(&llm, &secrets, &SystemClock, &stop, &mut |event| {
                    let schedule = inbox.lock().is_ok_and(|mut inbox| {
                        inbox.events.push(event);
                        !std::mem::replace(&mut inbox.scheduled, true)
                    });
                    if schedule {
                        deliver(&inbox, &shared, generation);
                    }
                });
            })
            .is_ok()
    }

    /// A batch of the answer's events, on the main thread.
    fn answer_events(&self, generation: u64, events: &[AnswerEvent]) {
        let ui = &self.agent_ui;
        if generation != ui.generation.get() || events.is_empty() {
            return;
        }
        if events.iter().any(is_terminal) {
            self.answer_ended(events);
            return;
        }
        let activity = {
            let mut running = ui.running.borrow_mut();
            let Some(running) = running.as_mut() else {
                return;
            };
            let tokens = events
                .iter()
                .filter(|event| matches!(event, AnswerEvent::Token(token) if !token.is_empty()))
                .count();
            running.received(u64::try_from(tokens).unwrap_or(u64::MAX));
            running.activity()
        };
        ui.panel.set_activity(activity);
        ui.panel.receive(events);
        self.place_badge();
    }

    /// The last events of an answer: it is done, stopped, or failed.
    fn answer_ended(&self, events: &[AnswerEvent]) {
        *self.agent_ui.running.borrow_mut() = None;
        self.agent_ui.panel.receive(events);
        self.place_badge();
    }

    /// The answer could not go on; `message` says why.
    fn answer_failed(&self, message: &str) {
        *self.agent_ui.running.borrow_mut() = None;
        self.agent_ui.panel.fail(message);
        self.place_badge();
    }

    /// A new ask replaces the answer running: it stops, and its last events are dropped.
    fn cancel_answer(&self) {
        if let Some(running) = self.agent_ui.running.borrow_mut().take() {
            running.stop.stop();
        }
        self.clear_citation();
    }

    /// What the panel asked.
    pub(super) fn panel_request(&self, request: PanelRequest) {
        match request {
            PanelRequest::Cite(anchor) => self.cite(&anchor),
            PanelRequest::Open(url) => self.open_link(url),
            PanelRequest::Stop => self.stop_agent(),
            PanelRequest::Settings => self.open_agent_settings(Trigger::Pointer),
            PanelRequest::DetectLocal => {
                self.open_agent_settings(Trigger::Pointer);
                self.settings_request(SettingsRequest::DetectLocal);
            }
        }
    }

    /// A citation chip: the page the answer is about comes forward, and the find machinery
    /// selects the quote, scrolls to it, and marks it. The panel stays.
    fn cite(&self, anchor: &str) {
        let Some(tab) = self.agent_ui.asked.get() else {
            return;
        };
        let shown = self
            .current()
            .and_then(|(window, _)| self.active(window))
            .is_some_and(|active| active == tab);
        if !shown && let Some(window) = self.run(|browsing| browsing.activate(tab)) {
            self.settle(window);
        }
        self.browsing
            .borrow_mut()
            .engine_mut()
            .find(tab, anchor, FindStep::Refine);
        self.agent_ui.cited.set(Some(tab));
    }

    /// The marks a citation left in its page go.
    pub(super) fn clear_citation(&self) {
        if let Some(tab) = self.agent_ui.cited.take() {
            self.browsing
                .borrow_mut()
                .engine_mut()
                .find(tab, "", FindStep::Refine);
        }
    }

    /// A link the user clicked in the answer: a background tab, in the jar of the page the
    /// answer is about, so a throwaway page's answer opens no door into the main jar.
    fn open_link(&self, url: Url) {
        let Some((window, _)) = self.current() else {
            return;
        };
        let live = self
            .agent_ui
            .asked
            .get()
            .filter(|&tab| self.browsing.borrow().session().tab(tab).is_some());
        let jar = live.map_or(JarChoice::Main, JarChoice::SameAs);
        let opened = self.run(|browsing| browsing.open(window, url, jar, Focus::Background));
        if opened.is_some() {
            self.toast("Opened in a background tab");
        }
    }
}

/// Hands the inbox to the main thread a frame from now.
fn deliver(inbox: &Arc<Mutex<Inbox>>, shared: &Shared, generation: u64) {
    let (inbox, shared) = (Arc::clone(inbox), Arc::clone(shared));
    let when = DispatchTime::try_from(FRAME).unwrap_or(DispatchTime::NOW);
    let _queued = DispatchQueue::main().after(when, move || {
        let events = inbox.lock().map_or_else(
            |_| Vec::new(),
            |mut inbox| {
                inbox.scheduled = false;
                std::mem::take(&mut inbox.events)
            },
        );
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        if let Some(browser) = shared.get(mtm).upgrade() {
            browser.answer_events(generation, &events);
        }
    });
}

fn is_terminal(event: &AnswerEvent) -> bool {
    matches!(
        event,
        AnswerEvent::Done(_) | AnswerEvent::Stopped | AnswerEvent::Error(_)
    )
}
