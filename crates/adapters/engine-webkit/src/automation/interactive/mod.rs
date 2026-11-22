use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use dispatch2::{DispatchQueue, DispatchTime, MainThreadBound};
use objc2::MainThreadMarker;
use objc2_web_kit::WKWebView;
use serde_json::{Value, json};
use url::Url;

use super::native::Session;
use super::{
    AutomationError, PAGE_TIMEOUT_MS, POLL_INTERVAL, REQUEST_TIMEOUT, Reply, reply, validate_url,
};

/// A visible or offscreen automation session with a separate process pool and private data store.
/// It completes requests on the main queue. Dropping it closes the transport and abandons work.
pub struct InteractiveAutomation(Rc<State>);

struct State {
    session: Session,
    reply: Rc<RefCell<Reply>>,
    last_id: Cell<u32>,
    pending: RefCell<Option<Operation>>,
    mtm: MainThreadMarker,
}

type Completion = Box<dyn FnOnce(Result<(), AutomationError>)>;

struct Operation {
    stage: Stage,
    deadline: Instant,
    url: Url,
    complete: Completion,
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
            pending: RefCell::new(None),
            mtm,
        })))
    }

    /// The session's view. Keep the session alive for as long as the view is attached.
    #[must_use]
    pub fn view(&self) -> &WKWebView {
        self.0.session.view()
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
        validate_url(url)?;
        if !self.0.session.is_connected() {
            return Err(AutomationError::Transport);
        }
        let mut pending = self.0.pending.borrow_mut();
        if pending.is_some() {
            return Err(AutomationError::Busy);
        }
        self.0.schedule()?;
        *pending = Some(Operation {
            stage: Stage::Starting,
            deadline: Instant::now() + REQUEST_TIMEOUT,
            url: url.clone(),
            complete: Box::new(done),
        });
        Ok(())
    }
}

impl State {
    fn schedule(self: &Rc<Self>) -> Result<(), AutomationError> {
        let weak = MainThreadBound::new(Rc::downgrade(self), self.mtm);
        let when =
            DispatchTime::try_from(POLL_INTERVAL).map_err(|()| AutomationError::Transport)?;
        DispatchQueue::main()
            .after(when, move || {
                if let Some(state) =
                    MainThreadMarker::new().and_then(|mtm| weak.into_inner(mtm).upgrade())
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
            .as_mut()
            .map(|op| self.advance(op));
        let result = match progress {
            None => return,
            Some(Ok(false)) => match self.schedule() {
                Ok(()) => return,
                Err(error) => Err(error),
            },
            Some(result) => result.map(|_| ()),
        };
        self.reply.borrow_mut().clear();
        if result.is_err() {
            self.session.close();
        }
        let pending = self.pending.borrow_mut().take();
        if let Some(pending) = pending {
            (pending.complete)(result);
        }
    }

    fn advance(&self, operation: &mut Operation) -> Result<bool, AutomationError> {
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
                return Ok(self.take_reply()?.is_some());
            }
            Stage::Starting => {}
        }
        Ok(false)
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
