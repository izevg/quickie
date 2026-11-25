use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use dispatch2::{DispatchQueue, DispatchTime, MainThreadBound};
use objc2::MainThreadMarker;
use objc2_web_kit::WKWebView;
use quickie_application::{AgentLinks, EngineError};
use serde_json::{Value, json};
use url::Url;

use super::native::Session;
use super::{
    AutomationError, PAGE_TIMEOUT_MS, POLL_INTERVAL, REQUEST_TIMEOUT, Reply, reply, validate_url,
};
use crate::page::scripts::{self, agent_links};

#[cfg(test)]
mod tests;

/// A visible or offscreen automation session with a separate process pool and private data store.
/// It completes requests on the main queue. Dropping it closes the transport and abandons work.
pub struct InteractiveAutomation(Rc<State>);

struct State {
    session: Session,
    reply: Rc<RefCell<Reply>>,
    last_id: Cell<u32>,
    pending: RefCell<Pending>,
    mtm: MainThreadMarker,
}

type Completion = Box<dyn FnOnce(Result<(), AutomationError>)>;

struct Operation {
    stage: Stage,
    deadline: Instant,
    url: Url,
    complete: Completion,
    eligible: Option<Box<dyn FnOnce() -> bool>>,
}

#[derive(Default)]
struct Pending {
    operation: Option<Operation>,
    generation: u64,
}

impl Pending {
    fn cancel(&mut self, reply: &mut Reply) {
        self.operation.take();
        self.generation = self.generation.wrapping_add(1);
        reply.clear();
    }
}

impl Operation {
    fn authorize(&mut self) -> bool {
        self.eligible.take().is_none_or(|eligible| eligible())
    }
}

enum Progress {
    Waiting,
    Finished,
    Revoked,
}

enum Stage {
    Starting,
    FindingContext,
    Navigating,
}

impl InteractiveAutomation {
    /// Starts the private blank page without waiting for it to finish loading.
    ///
    /// # Errors
    /// The loaded `WebKit` lacks local automation or could not open its transport.
    pub fn open(mtm: MainThreadMarker) -> Result<Self, AutomationError> {
        let reply = Rc::new(RefCell::new(Reply::default()));
        let session = Session::new(mtm, Rc::clone(&reply), true)?;
        Ok(Self(Rc::new(State {
            session,
            reply,
            last_id: Cell::new(0),
            pending: RefCell::default(),
            mtm,
        })))
    }

    /// The session's view. Keep the session alive for as long as the view is attached.
    #[must_use]
    pub fn view(&self) -> &WKWebView {
        self.0.session.view()
    }

    /// Changes when the main document commits a navigation or loses its content process.
    #[must_use]
    pub fn document_revision(&self) -> u64 {
        self.0.session.document_revision()
    }

    /// Whether this browsing context can still execute an automation request.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.0.session.is_connected()
    }

    /// Reads rendered text from the isolated page's own main frame.
    pub fn read_text(
        &self,
        max_chars: usize,
        done: impl FnOnce(Result<String, EngineError>) + 'static,
    ) {
        let state = &self.0;
        let world = scripts::world(state.mtm);
        scripts::evaluate_checked(
            state.session.view(),
            &world,
            None,
            &scripts::text_script(max_chars),
            done,
        );
    }

    /// Finds a quoted passage in the isolated page using the engine's native find support.
    pub fn find_text(&self, quote: &str) {
        crate::page::find::find(
            self.view(),
            quote,
            quickie_application::FindStep::Refine,
            |_| {},
        );
    }

    /// Reads link labels and their verified destinations from the isolated page.
    pub fn read_links(&self, done: impl FnOnce(Result<AgentLinks, EngineError>) + 'static) {
        let state = &self.0;
        let world = scripts::world(state.mtm);
        scripts::evaluate(
            state.session.view(),
            &world,
            None,
            agent_links::COLLECT_SCRIPT,
            move |answer| {
                done(agent_links::parse(&answer));
            },
        );
    }

    /// Queues a navigation to a provenance-checked URL and allows same-origin GET redirects.
    /// Calls `done` on the main queue after completion. Drop abandons the callback.
    ///
    /// # Errors
    /// `url` is invalid, another navigation is pending, or the transport is closed.
    pub fn navigate(
        &mut self,
        url: &Url,
        done: impl FnOnce(Result<(), AutomationError>) + 'static,
    ) -> Result<(), AutomationError> {
        self.navigate_when(url, || true, done)
    }

    /// Queues navigation and checks its provenance again immediately before native dispatch.
    /// A revoked request completes with `CommandFailed` and leaves the session connected.
    ///
    /// # Errors
    /// `url` is invalid, another navigation is pending, or the transport is closed.
    pub fn navigate_when(
        &mut self,
        url: &Url,
        eligible: impl FnOnce() -> bool + 'static,
        done: impl FnOnce(Result<(), AutomationError>) + 'static,
    ) -> Result<(), AutomationError> {
        validate_url(url)?;
        if !self.0.session.is_connected() {
            return Err(AutomationError::Transport);
        }
        let mut pending = self.0.pending.borrow_mut();
        if pending.operation.is_some() {
            return Err(AutomationError::Busy);
        }
        self.0.schedule(pending.generation)?;
        pending.operation = Some(Operation {
            stage: Stage::Starting,
            deadline: Instant::now() + REQUEST_TIMEOUT,
            url: url.clone(),
            complete: Box::new(done),
            eligible: Some(Box::new(eligible)),
        });
        Ok(())
    }

    /// Revokes pending work and its callback, retaining this view and its private data store.
    pub fn cancel_navigation(&mut self) {
        self.0
            .pending
            .borrow_mut()
            .cancel(&mut self.0.reply.borrow_mut());
        self.0.session.stop_loading();
    }
}

impl State {
    fn schedule(self: &Rc<Self>, generation: u64) -> Result<(), AutomationError> {
        let weak = MainThreadBound::new(Rc::downgrade(self), self.mtm);
        let when =
            DispatchTime::try_from(POLL_INTERVAL).map_err(|()| AutomationError::Transport)?;
        DispatchQueue::main()
            .after(when, move || {
                if let Some(state) =
                    MainThreadMarker::new().and_then(|mtm| weak.into_inner(mtm).upgrade())
                    && state.pending.borrow().generation == generation
                {
                    state.step();
                }
            })
            .map_err(|_| AutomationError::Transport)
    }

    fn step(self: &Rc<Self>) {
        let progress = self
            .pending
            .borrow_mut()
            .operation
            .as_mut()
            .map(|op| self.advance(op));
        let result = match progress {
            None => return,
            Some(Ok(Progress::Waiting)) => match self.schedule(self.pending.borrow().generation) {
                Ok(()) => return,
                Err(error) => Err(error),
            },
            Some(Ok(Progress::Revoked)) => {
                self.finish(Err(AutomationError::CommandFailed), false);
                return;
            }
            Some(result) => result.map(|_| ()),
        };
        let close = result.is_err();
        self.finish(result, close);
    }

    fn finish(&self, result: Result<(), AutomationError>, close: bool) {
        self.reply.borrow_mut().clear();
        if close {
            self.session.close();
        }
        let pending = self.pending.borrow_mut().operation.take();
        if let Some(pending) = pending {
            (pending.complete)(result);
        }
    }

    fn advance(&self, operation: &mut Operation) -> Result<Progress, AutomationError> {
        if Instant::now() >= operation.deadline {
            return Err(AutomationError::Timeout);
        }
        match operation.stage {
            Stage::Starting if !self.session.is_loading() => {
                self.send("Automation.getBrowsingContexts", &json!({}))?;
                operation.stage = Stage::FindingContext;
            }
            Stage::FindingContext => {
                if let Some(answer) = self.take_reply()? {
                    match reply::contexts(&answer)?.as_slice() {
                        [context] => {
                            if !operation.authorize() {
                                return Ok(Progress::Revoked);
                            }
                            self.session.restrict_to(&operation.url);
                            self.session.reset_navigation_failure();
                            self.send(
                                "Automation.navigateBrowsingContext",
                                &json!({
                                    "handle": context.handle,
                                    "url": operation.url.as_str(),
                                    "pageLoadStrategy": "Normal",
                                    "pageLoadTimeout": PAGE_TIMEOUT_MS,
                                }),
                            )?;
                            operation.stage = Stage::Navigating;
                        }
                        [] => operation.stage = Stage::Starting,
                        _ => return Err(AutomationError::InvalidReply),
                    }
                }
            }
            Stage::Navigating => {
                if self.session.navigation_failed() {
                    return Err(AutomationError::CommandFailed);
                }
                return Ok(if self.take_reply()?.is_some() {
                    Progress::Finished
                } else {
                    Progress::Waiting
                });
            }
            Stage::Starting => {}
        }
        Ok(Progress::Waiting)
    }

    fn send(&self, method: &str, params: &Value) -> Result<(), AutomationError> {
        let id = self
            .last_id
            .get()
            .checked_add(1)
            .ok_or(AutomationError::Transport)?;
        self.last_id.set(id);
        self.reply.borrow_mut().expect_id(id);
        self.session
            .dispatch(&json!({"id": id, "method": method, "params": params}).to_string())
    }

    fn take_reply(&self) -> Result<Option<Value>, AutomationError> {
        self.reply.borrow_mut().take().transpose()
    }
}
