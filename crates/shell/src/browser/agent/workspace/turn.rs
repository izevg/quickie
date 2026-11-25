//! Executes one completion at a time and delivers results to the matching workspace turn.

mod compare;
mod context;

use std::sync::Arc;

use quickie_application::{
    AgentPage, AskError, StopSignal, ToolChatReply, complete_workspace_step,
};
use quickie_domain_agents::{LlmEndpoint, Request};
use quickie_domain_browsing::TabId;
use quickie_llm::OpenAiClient;
use quickie_ui_widgets::{WorkspaceToolStatus, agent_wording};

use super::{Browser, Workspace};
pub(super) use context::ModelContext;

pub(super) struct InFlight {
    pub(super) id: u64,
    pub(super) stop: StopSignal,
    endpoint: LlmEndpoint,
    llm: Arc<OpenAiClient<crate::Network>>,
    pub(super) tool: Option<PendingTool>,
}

pub(super) struct ReadingTabs {
    id: u64,
    request: Request,
    pages: Vec<compare::ReadingPage>,
    row: usize,
}

pub(super) struct PendingTool {
    pub(super) id: String,
    pub(super) row: usize,
    pub(super) navigating: bool,
    pub(super) observed: Option<(AgentPage, u64)>,
}

impl PendingTool {
    pub(super) fn source_changed(&self, revision: Option<u64>) -> bool {
        self.navigating
            && self.observed.is_some_and(|(page, observed)| {
                page == AgentPage::Source && Some(observed) != revision
            })
    }
}

impl Browser {
    pub(super) fn submit_workspace(&self, prompt: &str, display: &str) {
        self.start_workspace_turn(prompt, Some(display), Vec::new());
    }

    fn start_workspace_turn(
        &self,
        prompt: &str,
        display: Option<&str>,
        excerpts: Vec<(TabId, String)>,
    ) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        let Some(endpoint) = self.workspace_endpoint(&workspace) else {
            return;
        };
        if workspace.reading.borrow().is_some() {
            return;
        }
        let llm = {
            let mut model = workspace.model.borrow_mut();
            if !model.conversation.begin(prompt) {
                return;
            }
            Arc::new(model.llm.for_turn())
        };
        let id = self.next_workspace_generation();
        workspace.turn.replace(Some(InFlight {
            id,
            stop: StopSignal::new(),
            endpoint,
            llm,
            tool: None,
        }));
        workspace.observations.borrow_mut().clear();
        workspace.citations.borrow_mut().begin_reads(excerpts);
        if let Some(display) = display {
            workspace.panel.append_prompt(display);
        }
        self.next_agent_step(&workspace, id);
    }

    fn next_workspace_generation(&self) -> u64 {
        let counter = &self.agent_ui.workspace.generation;
        let id = counter.get().wrapping_add(1);
        counter.set(id);
        id
    }

    pub(in crate::browser) fn stop_agent(&self) {
        let _ = self.stop_mcp_diagnostics();
        if let Some(workspace) = self.workspace() {
            Self::stop_workspace_turn(&workspace, "Stopped");
        }
    }

    pub(super) fn stop_workspace_turn(workspace: &Workspace, message: &str) {
        if let Some(reading) = workspace.reading.take() {
            workspace
                .panel
                .finish_tool(reading.row, WorkspaceToolStatus::Cancelled, "");
            workspace.panel.set_activity("", false);
            workspace.panel.append_notice(message, false);
        }
        Self::end_agent_turn(workspace, message, WorkspaceToolStatus::Cancelled);
    }

    fn next_agent_step(&self, workspace: &Workspace, id: u64) {
        let _ = self.workspace_endpoint(workspace);
        let Some((endpoint, stop, llm)) = workspace
            .turn
            .borrow()
            .as_ref()
            .filter(|turn| turn.id == id)
            .map(|turn| {
                (
                    turn.endpoint.clone(),
                    turn.stop.clone(),
                    Arc::clone(&turn.llm),
                )
            })
        else {
            return;
        };
        workspace.prepare_page(self.mtm);
        let messages = workspace.model.borrow().conversation.messages().to_vec();
        let automation = workspace.automation_available();
        let secrets = self.agent_ui.secrets.clone();
        workspace.panel.set_activity("Thinking…", true);
        let started = self.off_main(
            "quickie-agent-step",
            move || {
                (
                    id,
                    complete_workspace_step(
                        &endpoint,
                        &secrets,
                        llm.as_ref(),
                        &messages,
                        automation,
                        &stop,
                    ),
                )
            },
            Self::agent_step_ready,
        );
        if !started {
            Self::end_agent_turn(
                workspace,
                "Couldn't start the agent.",
                WorkspaceToolStatus::Failed,
            );
        }
    }

    fn agent_step_ready(&self, (id, reply): (u64, Result<ToolChatReply, AskError>)) {
        let Some(workspace) = self
            .workspace()
            .filter(|workspace| workspace.is_current(id))
        else {
            return;
        };
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                let problem = agent_wording::ask_error(&error);
                Self::end_agent_turn(&workspace, &problem, WorkspaceToolStatus::Failed);
                return;
            }
        };
        if !workspace.model.borrow_mut().conversation.accept(&reply) {
            Self::end_agent_turn(
                &workspace,
                "The agent could not continue this turn.",
                WorkspaceToolStatus::Failed,
            );
            return;
        }
        match reply {
            ToolChatReply::Answer(text) => {
                if let Some(turn) = workspace.turn.take() {
                    turn.llm.finish_turn();
                }
                let (answer, sources) = workspace.citations.borrow_mut().verify_answer(&text);
                workspace
                    .panel
                    .append_answer_with_sources(&answer, &sources);
                workspace.panel.set_activity("", false);
            }
            ToolChatReply::Call(call) => self.dispatch_workspace_tool(&workspace, id, &call),
        }
    }

    pub(super) fn tool_finished(
        &self,
        id: u64,
        call_id: &str,
        result: Result<(String, String), String>,
    ) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        let tool = {
            let mut turn = workspace.turn.borrow_mut();
            turn.as_mut()
                .filter(|turn| turn.id == id)
                .and_then(|turn| turn.tool.take_if(|tool| tool.id == call_id))
        };
        let Some(tool) = tool else {
            return;
        };
        let (status, content, detail) = match result {
            Ok((content, detail)) => {
                if tool.navigating {
                    workspace.preview_navigated();
                }
                (WorkspaceToolStatus::Completed, content, detail)
            }
            Err(issue) => (
                WorkspaceToolStatus::Failed,
                format!("Tool failed: {issue}"),
                issue,
            ),
        };
        workspace.panel.finish_tool(tool.row, status, &detail);
        let accepted = workspace
            .model
            .borrow_mut()
            .conversation
            .tool_result(call_id, content);
        if accepted {
            self.next_agent_step(&workspace, id);
        } else {
            Self::end_agent_turn(
                &workspace,
                "The tool response arrived out of order.",
                WorkspaceToolStatus::Failed,
            );
        }
    }

    fn end_agent_turn(workspace: &Workspace, message: &str, status: WorkspaceToolStatus) {
        let Some(turn) = workspace.turn.take() else {
            return;
        };
        turn.stop.stop();
        workspace.model.borrow_mut().conversation.cancel();
        workspace.observations.borrow_mut().clear();
        workspace.citations.borrow_mut().clear_reads();
        if let Some(tool) = turn.tool {
            workspace.panel.finish_tool(tool.row, status, "");
            if tool.navigating {
                workspace.cancel_navigation();
            }
        }
        workspace.panel.set_activity("", false);
        workspace
            .panel
            .append_notice(message, status == WorkspaceToolStatus::Failed);
    }
}

impl Workspace {
    pub(super) fn is_current(&self, id: u64) -> bool {
        self.turn
            .borrow()
            .as_ref()
            .is_some_and(|turn| turn.id == id)
    }
}

#[cfg(test)]
mod tests {
    use quickie_application::AgentPage;

    use super::PendingTool;

    #[test]
    fn source_commit_invalidates_only_navigation_observed_on_that_source_revision() {
        let mut tool = PendingTool {
            id: "open".to_owned(),
            row: 0,
            navigating: true,
            observed: Some((AgentPage::Source, 4)),
        };
        assert!(!tool.source_changed(Some(4)));
        assert!(tool.source_changed(Some(5)));
        assert!(tool.source_changed(None));

        // A workspace link can legitimately commit its own next document.
        tool.observed = Some((AgentPage::Workspace, 4));
        assert!(!tool.source_changed(Some(5)));
        tool.observed = Some((AgentPage::Source, 4));
        tool.navigating = false;
        assert!(!tool.source_changed(Some(5)));
    }
}
