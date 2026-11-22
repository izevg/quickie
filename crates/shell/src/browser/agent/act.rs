//! One user-requested link navigation through the fork's isolated automation session.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSWindow, NSWindowStyleMask, NSWindowWillCloseNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSPoint, NSRect, NSSize, NSString};
use quickie_application::{AgentLink, AgentLinks, PageContent, SelectionError, StopSignal};
use quickie_domain_agents::Question;
use quickie_domain_browsing::{TabId, WindowId};
use quickie_engine_webkit::{AutomationError, InteractiveAutomation};
use quickie_llm::OpenAiClient;
use url::Url;

use super::Browser;

#[derive(Default)]
pub(super) struct ActUi {
    generation: Cell<u64>,
    pending: RefCell<Option<Pending>>,
    window: RefCell<Option<AgentWindow>>,
}

#[derive(Clone, Copy)]
struct Source {
    window: WindowId,
    tab: TabId,
    generation: u64,
}

struct Pending {
    source: Source,
    stop: StopSignal,
    session: InteractiveAutomation,
}

type Selection = Result<Option<AgentLink>, SelectionError>;

struct AgentWindow {
    window: Retained<NSWindow>,
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    _session: InteractiveAutomation,
}

impl Drop for AgentWindow {
    fn drop(&mut self) {
        // SAFETY: this token was registered with the default center on the main thread.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver((*self.observer).as_ref()) };
        self.window.setContentView(None);
    }
}

impl Browser {
    pub(in crate::browser) fn act_on_page(&self, question: Question) {
        let Some((window, _)) = self.current() else {
            return;
        };
        let Some(tab) = self.active(window) else {
            return;
        };
        if self.agent.borrow().endpoint().is_none() {
            self.agent_ui.panel.unconfigured();
            self.open_panel(quickie_ui_shared::Trigger::Keyboard);
            return;
        }
        self.cancel_act("");
        self.cancel_answer();
        self.hide_agent();
        let session = match InteractiveAutomation::open(self.mtm) {
            Ok(session) => session,
            Err(error) => {
                self.toast(&format!("Couldn't start Agent automation: {error}"));
                return;
            }
        };

        let act = &self.agent_ui.act;
        let generation = act.generation.get().wrapping_add(1);
        act.generation.set(generation);
        let source = Source {
            window,
            tab,
            generation,
        };
        *act.pending.borrow_mut() = Some(Pending {
            source,
            stop: StopSignal::new(),
            session,
        });
        self.toast("Agent is choosing a link · ⌘. to stop");
        self.read_act_links(source, move |browser, snapshot| {
            browser.select_act_link(source, &question, snapshot);
        });
    }

    fn read_act_links(&self, source: Source, done: impl FnOnce(&Self, AgentLinks) + 'static) {
        let this = self.this.clone();
        let read = self
            .browsing
            .borrow()
            .engine()
            .agent_links(source.tab, move |result| {
                if let Some(browser) = this.upgrade()
                    && browser.act_is_current(source)
                {
                    match result {
                        Ok(snapshot) => done(&browser, snapshot),
                        Err(_) => {
                            browser.cancel_act("Couldn't read links on this page");
                        }
                    }
                }
            });
        if read.is_err() {
            self.cancel_act("Couldn't read links on this page");
        }
    }

    fn select_act_link(&self, source: Source, question: &Question, snapshot: AgentLinks) {
        let document = snapshot.document.clone();
        let plan = self.agent.borrow().plan_navigation(question, snapshot);
        let select = match plan {
            Ok(select) => select,
            Err(error) => {
                self.cancel_act(&format!("Couldn't choose an Agent link: {error}"));
                return;
            }
        };
        let Some(stop) = self
            .agent_ui
            .act
            .pending
            .borrow()
            .as_ref()
            .map(|act| act.stop.clone())
        else {
            return;
        };
        let llm = OpenAiClient::new(self.http.clone());
        let secrets = self.agent_ui.secrets.clone();
        if !self.off_main(
            "quickie-agent-link",
            move || (source, document, select.run(&llm, &secrets, &stop)),
            Self::act_link_selected,
        ) {
            self.cancel_act("Couldn't start Agent link selection");
        }
    }

    fn act_link_selected(&self, (source, document, selection): (Source, Url, Selection)) {
        if !self.act_is_current(source) {
            return;
        }
        let link = match selection {
            Ok(Some(link)) => link,
            Ok(None) => {
                self.cancel_act("Agent found no matching link");
                return;
            }
            Err(error) => {
                self.cancel_act(&format!("Couldn't choose an Agent link: {error}"));
                return;
            }
        };
        self.read_act_links(source, move |browser, live| {
            if still_on_page(&document, &link, &live) {
                browser.open_act_link(source, &link.url);
            } else {
                browser.cancel_act("The page changed. Try the Agent action again");
            }
        });
    }

    fn open_act_link(&self, source: Source, url: &Url) {
        self.toast("Agent is opening the link · ⌘. to stop");
        let this = self.this.clone();
        let started = {
            let mut pending = self.agent_ui.act.pending.borrow_mut();
            let Some(pending) = pending.as_mut() else {
                return;
            };
            pending.session.navigate(url, move |result| {
                if let Some(browser) = this.upgrade() {
                    browser.act_loaded(source, result);
                }
            })
        };
        if let Err(error) = started {
            self.cancel_act(&format!("Couldn't open the Agent link: {error}"));
        }
    }

    fn act_loaded(&self, source: Source, result: Result<(), AutomationError>) {
        if !self.act_is_current(source) {
            return;
        }
        if let Err(error) = result {
            self.cancel_act(&format!("Couldn't open the Agent link: {error}"));
            return;
        }
        let Some(pending) = self.agent_ui.act.pending.borrow_mut().take() else {
            return;
        };
        let next = self.make_agent_window(pending.session);
        let window = next.window.clone();
        let previous = self.agent_ui.act.window.replace(Some(next));
        if let Some(previous) = previous {
            previous.window.close();
        }
        window.makeKeyAndOrderFront(None);
    }

    fn act_is_current(&self, source: Source) -> bool {
        let pending = self.agent_ui.act.pending.borrow();
        if pending
            .as_ref()
            .is_none_or(|pending| pending.source.generation != source.generation)
        {
            return false;
        }
        drop(pending);
        let app = NSApplication::sharedApplication(self.mtm);
        if !app.isActive()
            || self
                .window(source.window)
                .zip(app.keyWindow())
                .is_none_or(|(source_window, key)| *source_window != *key)
            || self.active(source.window) != Some(source.tab)
        {
            self.cancel_act("");
            return false;
        }
        true
    }

    pub(in crate::browser) fn act_source_navigated(&self, tab: TabId) {
        let from_source = self
            .agent_ui
            .act
            .pending
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.source.tab == tab);
        if from_source {
            self.cancel_act("The page changed. Try the Agent action again");
        }
    }

    pub(super) fn cancel_act(&self, message: &str) -> bool {
        let Some(pending) = self.agent_ui.act.pending.borrow_mut().take() else {
            return false;
        };
        pending.stop.stop();
        drop(pending);
        if !message.is_empty() {
            self.toast(message);
        }
        true
    }

    pub(in crate::browser) fn is_agent_window(&self, window: &NSWindow) -> bool {
        self.agent_ui
            .act
            .window
            .borrow()
            .as_ref()
            .is_some_and(|agent| *agent.window == *window)
    }

    fn make_agent_window(&self, session: InteractiveAutomation) -> AgentWindow {
        let style =
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(1024.0, 700.0));
        // SAFETY: a fresh allocation, with standard window styles and buffered backing.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(self.mtm),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: Retained owns the window and must be its only close-time owner.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("Quickie Agent · Isolated Session"));
        window.setContentView(Some(session.view()));
        window.center();

        let this = self.this.clone();
        let block = RcBlock::new(move |note: NonNull<NSNotification>| {
            // SAFETY: the notification center passes a live notification to the block.
            let window = unsafe { note.as_ref() }
                .object()
                .and_then(|object| object.downcast::<NSWindow>().ok());
            if let (Some(browser), Some(window)) = (this.upgrade(), window)
                && browser.is_agent_window(&window)
            {
                let closed = browser.agent_ui.act.window.borrow_mut().take();
                drop(closed);
            }
        });
        // SAFETY: AppKit posts this window's close notification on the main thread. The
        // center copies the block and holds it until we unregister its token.
        let observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSWindowWillCloseNotification),
                Some(&window),
                None,
                &block,
            )
        };
        AgentWindow {
            window,
            observer,
            _session: session,
        }
    }
}

fn still_on_page(document: &Url, selected: &AgentLink, live: &AgentLinks) -> bool {
    &live.document == document && live.links.contains(selected)
}

#[cfg(test)]
mod tests {
    use super::{AgentLink, AgentLinks, Url, still_on_page};

    #[test]
    fn the_same_document_must_still_show_the_selected_link() {
        let document = Url::parse("https://example.com/").unwrap();
        let link = AgentLink {
            url: document.join("video/1").unwrap(),
            label: "A video".into(),
        };
        let mut live = AgentLinks {
            document: document.clone(),
            links: vec![link.clone()],
        };
        assert!(still_on_page(&document, &link, &live));
        live.links[0].url = document.join("video/2").unwrap();
        assert!(!still_on_page(&document, &link, &live));
        live.links[0] = link.clone();
        live.document = document.join("?changed").unwrap();
        assert!(!still_on_page(&document, &link, &live));
    }
}
