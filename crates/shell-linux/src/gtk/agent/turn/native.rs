//! Native completions and bounded source reads for a single workspace turn.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::Builder;

use quickie_application::{
    AgentPage, AgentTool, Conversation, StopSignal, ToolChatMessage, ToolChatReply,
};
use quickie_domain_agents::LlmEndpoint;
use quickie_llm::OpenAiClient;
use url::Origin;

use crate::gtk::App;
use crate::gtk::ffi::Object;
use crate::updates::network::WebKitHttp;

use super::super::SOURCE_CHANGED;
use super::service::ProviderStep;
use super::{AskedPage, MAX_CITATIONS, Turn, View, citation, failed, read, service, stopped, view};

type Client = OpenAiClient<WebKitHttp>;
const SOURCE_READ_LIMIT: usize = 8_000;

pub(in crate::gtk::agent) struct ProviderWorkspace {
    origin: Option<Origin>,
    legacy: Option<LlmEndpoint>,
    conversation: Conversation,
    client: Client,
}

impl Default for ProviderWorkspace {
    fn default() -> Self {
        Self {
            origin: None,
            legacy: None,
            conversation: Conversation::default(),
            client: Client::new(WebKitHttp),
        }
    }
}

impl ProviderWorkspace {
    pub(in crate::gtk::agent) fn bind(&mut self, endpoint: Option<&LlmEndpoint>) {
        let origin = endpoint.map(|endpoint| endpoint.base().as_url().origin());
        if origin != self.origin {
            *self = Self {
                origin,
                ..Self::default()
            };
        }
    }

    pub(super) fn uses_legacy(&self, endpoint: &LlmEndpoint) -> bool {
        self.legacy.as_ref() == Some(endpoint)
    }

    pub(in crate::gtk::agent) fn cancel(&mut self) {
        self.conversation.cancel();
    }
}

pub(in crate::gtk::agent) struct NativeTurn {
    asked: AskedPage,
    client: Arc<Client>,
    stop: StopSignal,
    stage: Stage,
    accepted_read: bool,
    excerpts: Vec<String>,
}

enum Stage {
    Model(Receiver<ProviderStep>),
    Reading {
        task: read::Read,
        call_id: String,
        row: Object,
    },
}

impl Drop for NativeTurn {
    fn drop(&mut self) {
        self.stop.stop();
    }
}

impl NativeTurn {
    pub(super) fn start(app: &App, asked: AskedPage, panel: View) -> Result<Self, String> {
        let stop = StopSignal::new();
        let (client, stage) = {
            let mut model = app.agent.model.borrow_mut();
            if !model.conversation.begin(asked.question.as_str()) {
                return Err("The agent is already answering".into());
            }
            let client = Arc::new(model.client.for_turn());
            let stage = match request_step(&client, &asked.endpoint, &model.conversation, &stop) {
                Ok(stage) => stage,
                Err(error) => {
                    model.cancel();
                    return Err(error);
                }
            };
            (client, stage)
        };
        panel.row(asked.question.as_str(), c"agent-prompt");
        panel.activity("Thinking…", true, true);
        Ok(Self {
            asked,
            client,
            stop,
            stage,
            accepted_read: false,
            excerpts: Vec::new(),
        })
    }

    pub(super) fn show_cancelled_read(&self) {
        if let Stage::Reading { row, .. } = &self.stage {
            view::set_label(*row, "Reading cancelled");
        }
    }

    pub(super) fn poll(self: Box<Self>, app: &App, panel: View) -> Option<Turn> {
        if !self.asked.is_current(app) {
            app.agent.model.borrow_mut().cancel();
            self.show_cancelled_read();
            stopped(panel, SOURCE_CHANGED);
            return None;
        }
        match &self.stage {
            Stage::Model(pending) => match pending.try_recv() {
                Ok(event) => self.model_finished(app, panel, event),
                Err(TryRecvError::Empty) => Some(Turn::Native(self)),
                Err(TryRecvError::Disconnected) => {
                    failed_turn(app, panel, "The provider request ended unexpectedly")
                }
            },
            Stage::Reading { task, call_id, row } => match task.results.try_recv() {
                Ok(Ok(text)) => {
                    let call_id = call_id.clone();
                    let row = *row;
                    self.read_finished(app, panel, &call_id, row, text)
                }
                Ok(Err(error)) => {
                    view::set_label(*row, "Reading failed");
                    failed_turn(app, panel, &error)
                }
                Err(TryRecvError::Empty) => Some(Turn::Native(self)),
                Err(TryRecvError::Disconnected) => {
                    view::set_label(*row, "Reading failed");
                    failed_turn(app, panel, "The page read ended unexpectedly")
                }
            },
        }
    }

    fn model_finished(
        mut self: Box<Self>,
        app: &App,
        panel: View,
        event: ProviderStep,
    ) -> Option<Turn> {
        let reply = match event {
            ProviderStep::Reply(reply) => reply,
            ProviderStep::NeedsStreaming if !self.accepted_read => {
                let mut model = app.agent.model.borrow_mut();
                model.cancel();
                model.legacy = Some(self.asked.endpoint.clone());
                drop(model);
                return match Turn::start_legacy(self.asked.clone(), panel, false) {
                    Ok(turn) => Some(turn),
                    Err(error) => failed(panel, &error),
                };
            }
            ProviderStep::NeedsStreaming => {
                return failed_turn(
                    app,
                    panel,
                    "The provider cannot continue its native tool turn",
                );
            }
            ProviderStep::Failed(message) => return failed_turn(app, panel, &message),
        };
        if matches!(&reply, ToolChatReply::Call(call) if call.tool != AgentTool::ReadPage(AgentPage::Source))
        {
            return failed_turn(
                app,
                panel,
                "The provider requested an unavailable browser tool",
            );
        }
        if !app.agent.model.borrow_mut().conversation.accept(&reply) {
            return failed_turn(app, panel, "The agent could not continue this turn");
        }
        match reply {
            ToolChatReply::Answer(text) => {
                self.client.finish_turn();
                let (answer, citations) = service::verified_answer(&text, &self.excerpts);
                panel.row(&answer, c"agent-answer");
                for source in citations.into_iter().take(MAX_CITATIONS) {
                    citation::show(app, panel, self.asked.revision, source);
                }
                panel.activity("Answer complete", false, true);
                None
            }
            ToolChatReply::Call(call) => {
                if let Err(error) = read::source(self.asked.source) {
                    return failed_turn(app, panel, &error);
                }
                let task = match read::Read::start(self.asked.source, SOURCE_READ_LIMIT) {
                    Ok(task) => task,
                    Err(error) => return failed_turn(app, panel, &error),
                };
                let row = panel.row("Reading source page…", c"agent-source");
                panel.activity("Reading page…", true, true);
                self.accepted_read = true;
                self.stage = Stage::Reading {
                    task,
                    call_id: call.id,
                    row,
                };
                Some(Turn::Native(self))
            }
        }
    }

    fn read_finished(
        mut self: Box<Self>,
        app: &App,
        panel: View,
        call_id: &str,
        row: Object,
        text: String,
    ) -> Option<Turn> {
        let stage = {
            let mut model = app.agent.model.borrow_mut();
            let conversation = &mut model.conversation;
            if !conversation.tool_result(call_id, text) {
                drop(model);
                view::set_label(row, "Reading failed");
                return failed_turn(app, panel, "The source result arrived out of order");
            }
            if let Some(ToolChatMessage::Result { content, .. }) = conversation.messages().last() {
                self.excerpts.push(content.clone());
            }
            request_step(&self.client, &self.asked.endpoint, conversation, &self.stop)
        };
        view::set_label(row, "Read source page");
        match stage {
            Ok(stage) => {
                self.stage = stage;
                panel.activity("Thinking…", true, true);
                Some(Turn::Native(self))
            }
            Err(error) => failed_turn(app, panel, &error),
        }
    }
}

fn request_step(
    client: &Arc<Client>,
    endpoint: &LlmEndpoint,
    conversation: &Conversation,
    stop: &StopSignal,
) -> Result<Stage, String> {
    let client = Arc::clone(client);
    let endpoint = endpoint.clone();
    let messages = conversation.messages().to_vec();
    let stop = stop.clone();
    let (send, receive) = mpsc::channel();
    Builder::new()
        .name("quickie-agent-provider".into())
        .spawn(move || {
            let result = service::provider_step(&client, &endpoint, &messages, &stop);
            let _ = send.send(result);
        })
        .map_err(|_| "Could not start the provider request")?;
    Ok(Stage::Model(receive))
}

fn failed_turn(app: &App, panel: View, message: &str) -> Option<Turn> {
    app.agent.model.borrow_mut().cancel();
    failed(panel, message)
}
