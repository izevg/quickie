//! A cancellable Agent turn using native provider tools or the streaming compatibility path.

mod citation;
mod native;

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::Builder;

use quickie_application::{AnswerEvent, PageReader, StopSignal};
use quickie_domain_agents::{LlmEndpoint, PageText, Question, Request};
use url::Url;

use super::{View, read, service, view};
use crate::gtk::App;
use crate::gtk::ffi::Object;
pub(super) use native::ProviderWorkspace;

const MAX_CITATIONS: usize = 16;

#[derive(Clone)]
struct AskedPage {
    source: Object,
    revision: u64,
    question: Question,
    endpoint: LlmEndpoint,
}

impl AskedPage {
    fn is_current(&self, app: &App) -> bool {
        app.agent.source() == Some(self.source) && app.agent.revision.get() == self.revision
    }
}

pub(super) struct ReadPage {
    task: read::Read,
    url: Url,
    title: String,
    question: Question,
    endpoint: LlmEndpoint,
    revision: u64,
    row: Object,
}

pub(super) struct Answer {
    stop: StopSignal,
    receiver: Receiver<Result<AnswerEvent, String>>,
    text: String,
    row: Object,
    revision: u64,
    citations: usize,
}

impl Drop for Answer {
    fn drop(&mut self) {
        self.stop.stop();
    }
}

pub(super) enum Turn {
    Reading(Box<ReadPage>),
    Answering(Answer),
    Native(Box<native::NativeTurn>),
}

impl Turn {
    pub(super) fn start(
        app: &App,
        source: Object,
        question: Question,
        endpoint: LlmEndpoint,
        revision: u64,
        panel: View,
    ) -> Result<Self, String> {
        let asked = AskedPage {
            source,
            revision,
            question,
            endpoint,
        };
        if app.agent.model.borrow().uses_legacy(&asked.endpoint) {
            return Self::start_legacy(asked, panel, true);
        }
        read::source(source)?;
        native::NativeTurn::start(app, asked, panel).map(|turn| Self::Native(Box::new(turn)))
    }

    fn start_legacy(asked: AskedPage, panel: View, append_prompt: bool) -> Result<Self, String> {
        let (url, title) = read::source(asked.source)?;
        let task = read::Read::start(asked.source, PageReader::MAX_CHARS)?;
        if append_prompt {
            panel.row(asked.question.as_str(), c"agent-prompt");
        }
        let row = panel.row("Reading source page…", c"agent-source");
        panel.activity("Reading page…", true, true);
        Ok(Self::Reading(Box::new(ReadPage {
            task,
            url,
            title,
            question: asked.question,
            endpoint: asked.endpoint,
            revision: asked.revision,
            row,
        })))
    }

    pub(super) fn needs_current_document(&self) -> bool {
        matches!(self, Self::Reading(_) | Self::Native(_))
    }

    pub(super) fn stop(self, panel: View, message: &str) {
        match self {
            Self::Reading(read) => view::set_label(read.row, "Reading cancelled"),
            Self::Native(turn) => turn.show_cancelled_read(),
            Self::Answering(_) => {}
        }
        stopped(panel, message);
    }

    pub(super) fn poll(self, app: &App, panel: View) -> Option<Self> {
        match self {
            Self::Reading(read) => match read.task.results.try_recv() {
                Ok(Ok(text)) => match read.answer(text, panel) {
                    Ok(answer) => Some(Self::Answering(answer)),
                    Err(error) => failed(panel, &error),
                },
                Ok(Err(error)) => {
                    view::set_label(read.row, "Reading failed");
                    failed(panel, &error)
                }
                Err(TryRecvError::Disconnected) => {
                    view::set_label(read.row, "Reading failed");
                    failed(panel, "The page read ended unexpectedly")
                }
                Err(TryRecvError::Empty) => Some(Self::Reading(read)),
            },
            Self::Answering(mut answer) => {
                answer.poll(app, panel).then_some(Self::Answering(answer))
            }
            Self::Native(turn) => turn.poll(app, panel),
        }
    }
}

impl ReadPage {
    fn answer(self, text: String, panel: View) -> Result<Answer, String> {
        let page = PageText::new(self.url, self.title, text);
        let stop = StopSignal::new();
        let worker_stop = stop.clone();
        let (sender, receiver) = mpsc::channel();
        Builder::new()
            .name("quickie-agent-answer".into())
            .spawn(move || {
                let request = Request::Question(self.question);
                if let Err(error) =
                    service::answer(&request, page, &self.endpoint, &worker_stop, &sender)
                {
                    let _ = sender.send(Err(error));
                }
            })
            .map_err(|_| "Could not start the answer request")?;
        view::set_label(self.row, "Read source page");
        panel.activity("Answering…", true, true);
        let row = panel.row("", c"agent-answer");
        Ok(Answer {
            stop,
            receiver,
            text: String::new(),
            row,
            revision: self.revision,
            citations: 0,
        })
    }
}

impl Answer {
    fn poll(&mut self, app: &App, panel: View) -> bool {
        let before = self.text.len();
        let mut outcome = None;
        for _ in 0..64 {
            match self.receiver.try_recv() {
                Ok(Ok(AnswerEvent::Token(text))) => self.text.push_str(&text),
                Ok(Ok(AnswerEvent::Citation(citation))) => {
                    self.text.push('“');
                    self.text.push_str(&citation.quote);
                    self.text.push('”');
                    if self.citations < MAX_CITATIONS {
                        citation::show(app, panel, self.revision, citation);
                        self.citations += 1;
                    }
                }
                Ok(Ok(AnswerEvent::Done(_))) => {
                    outcome = Some(Ok("Answer complete"));
                    break;
                }
                Ok(Ok(AnswerEvent::Stopped)) => {
                    outcome = Some(Ok("Stopped"));
                    break;
                }
                Ok(Ok(AnswerEvent::Error(error))) => {
                    outcome = Some(Err(error.to_string()));
                    break;
                }
                Ok(Err(error)) => {
                    outcome = Some(Err(error));
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    outcome = Some(Err("The answer ended unexpectedly".to_owned()));
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if self.text.len() != before {
            view::set_label(self.row, &self.text);
        }
        match outcome {
            Some(Ok(message)) => panel.activity(message, false, true),
            Some(Err(error)) => {
                failed(panel, &error);
            }
            None => return true,
        }
        false
    }
}

fn failed(panel: View, error: &str) -> Option<Turn> {
    panel.row(error, c"agent-problem");
    panel.activity("Could not answer", false, true);
    None
}

fn stopped(panel: View, message: &str) {
    panel.activity("Stopped", false, true);
    panel.row(message, c"agent-source");
}
