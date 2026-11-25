//! Routes typed observations and navigation through the bound page and isolated session.

mod links;

use quickie_application::{
    AgentLinks, AgentPage, AgentTool, AgentToolCall, EngineError, PageContent,
};
use url::Url;

use super::{Browser, Workspace, turn::PendingTool};

pub(super) use links::Observations;

const READ_LIMIT: usize = 8_000;
pub(super) const STALE: &str = "The page changed. List its links again.";
const UNAVAILABLE: &str = "This page is unavailable.";

impl Browser {
    #[cfg_attr(
        not(feature = "main-webkit"),
        expect(
            clippy::unused_self,
            reason = "the WebKit build reads the engine's source revision through self"
        )
    )]
    pub(super) fn workspace_revision(&self, workspace: &Workspace, page: AgentPage) -> Option<u64> {
        match page {
            AgentPage::Source => {
                #[cfg(feature = "main-webkit")]
                {
                    self.browsing
                        .try_borrow()
                        .ok()?
                        .engine()
                        .agent_source_revision(workspace.source)
                }
                #[cfg(not(feature = "main-webkit"))]
                {
                    workspace
                        .source_loaded
                        .get()
                        .then(|| workspace.source_revision.get())
                }
            }
            AgentPage::Workspace => workspace.isolated_revision(),
        }
    }

    pub(super) fn dispatch_workspace_tool(
        &self,
        workspace: &Workspace,
        id: u64,
        call: &AgentToolCall,
    ) {
        let target = match call.tool {
            AgentTool::ReadPage(page) | AgentTool::ListLinks(page) => page_name(page),
            AgentTool::OpenLink(link) => workspace.observations.borrow().label(link),
        };
        let title = match call.tool {
            AgentTool::ReadPage(_) => "Read page",
            AgentTool::ListLinks(_) => "List links",
            AgentTool::OpenLink(_) => "Open link",
        };
        let row = workspace.panel.start_tool(title, &target);
        if let Some(turn) = workspace.turn.borrow_mut().as_mut() {
            turn.tool = Some(PendingTool {
                id: call.id.clone(),
                row,
                navigating: false,
                observed: None,
            });
        }
        match call.tool {
            AgentTool::ReadPage(page) => self.read_workspace_page(workspace, id, &call.id, page),
            AgentTool::ListLinks(page) => self.list_workspace_links(workspace, id, &call.id, page),
            AgentTool::OpenLink(link) => self.open_observed_link(workspace, id, &call.id, link),
        }
    }

    fn read_workspace_page(&self, workspace: &Workspace, id: u64, call: &str, page: AgentPage) {
        let Some(revision) = self.workspace_revision(workspace, page) else {
            self.tool_finished(id, call, Err(UNAVAILABLE.to_owned()));
            return;
        };
        let this = self.this.clone();
        let completed_call = call.to_owned();
        let done = move |result: Result<String, EngineError>| {
            if let Some(browser) = this.upgrade()
                && let Some(workspace) = browser
                    .workspace()
                    .filter(|workspace| workspace.is_current(id))
            {
                let result = if browser.workspace_revision(&workspace, page) == Some(revision) {
                    result
                        .map(|text| {
                            workspace.record_citation_source(
                                id,
                                &completed_call,
                                page,
                                revision,
                                &text,
                            );
                            rendered_text(text)
                        })
                        .map_err(|error| error.to_string())
                } else {
                    Err(UNAVAILABLE.to_owned())
                };
                browser.tool_finished(id, &completed_call, result);
            }
        };
        match page {
            AgentPage::Source => {
                let result = self.browsing.borrow().engine().read_agent_text(
                    workspace.source,
                    READ_LIMIT,
                    done,
                );
                if let Err(error) = result {
                    self.tool_finished(id, call, Err(error.to_string()));
                }
            }
            AgentPage::Workspace => {
                #[cfg(feature = "main-webkit")]
                if let Some(session) = workspace.automation.borrow().as_ref() {
                    session.read_text(READ_LIMIT, done);
                    return;
                }
                self.tool_finished(id, call, Err(UNAVAILABLE.to_owned()));
            }
        }
    }

    fn list_workspace_links(&self, workspace: &Workspace, id: u64, call: &str, page: AgentPage) {
        let Some(revision) = self.workspace_revision(workspace, page) else {
            self.tool_finished(id, call, Err(UNAVAILABLE.to_owned()));
            return;
        };
        self.read_workspace_links(
            workspace,
            id,
            call,
            page,
            move |browser, workspace, call, snapshot| {
                let result = if browser.workspace_revision(workspace, page) == Some(revision) {
                    Ok(workspace
                        .observations
                        .borrow_mut()
                        .record(snapshot, page, revision))
                } else {
                    Err(STALE.to_owned())
                };
                browser.tool_finished(id, &call, result);
            },
        );
    }

    fn open_observed_link(&self, workspace: &Workspace, id: u64, call: &str, link: u32) {
        let selected = workspace.observations.borrow().get(link).cloned();
        let Some(selected) = selected
            .filter(|link| self.workspace_revision(workspace, link.page) == Some(link.revision))
        else {
            self.tool_finished(id, call, Err(STALE.to_owned()));
            return;
        };
        self.read_workspace_links(
            workspace,
            id,
            call,
            selected.page,
            move |browser, workspace, call, live| {
                if browser.workspace_revision(workspace, selected.page) != Some(selected.revision)
                    || !selected.is_in(&live)
                {
                    browser.tool_finished(id, &call, Err(STALE.to_owned()));
                    return;
                }
                workspace.observations.borrow_mut().clear();
                browser.navigate_workspace(
                    workspace,
                    id,
                    &call,
                    &selected.link.url,
                    (selected.page, selected.revision),
                );
            },
        );
    }

    fn read_workspace_links(
        &self,
        workspace: &Workspace,
        id: u64,
        call: &str,
        page: AgentPage,
        done: impl FnOnce(&Self, &Workspace, String, AgentLinks) + 'static,
    ) {
        let this = self.this.clone();
        let completed_call = call.to_owned();
        let complete = move |result: Result<AgentLinks, quickie_application::EngineError>| {
            if let Some(browser) = this.upgrade()
                && let Some(workspace) = browser
                    .workspace()
                    .filter(|workspace| workspace.is_current(id))
            {
                match result {
                    Ok(snapshot) => done(&browser, &workspace, completed_call, snapshot),
                    Err(error) => {
                        browser.tool_finished(id, &completed_call, Err(error.to_string()));
                    }
                }
            }
        };
        match page {
            AgentPage::Source => {
                let read = self
                    .browsing
                    .borrow()
                    .engine()
                    .agent_links(workspace.source, complete);
                if let Err(error) = read {
                    self.tool_finished(id, call, Err(error.to_string()));
                }
            }
            AgentPage::Workspace => {
                #[cfg(feature = "main-webkit")]
                {
                    let session = workspace.automation.borrow();
                    if let Some(session) = session.as_ref().filter(|session| session.is_connected())
                    {
                        session.read_links(complete);
                        return;
                    }
                }
                self.tool_finished(id, call, Err(UNAVAILABLE.to_owned()));
            }
        }
    }

    #[cfg(feature = "main-webkit")]
    fn navigate_workspace(
        &self,
        workspace: &Workspace,
        id: u64,
        call: &str,
        url: &Url,
        observed: (AgentPage, u64),
    ) {
        if let Some(tool) = workspace
            .turn
            .borrow_mut()
            .as_mut()
            .and_then(|turn| turn.tool.as_mut())
        {
            tool.navigating = true;
            tool.observed = Some(observed);
        }
        let this = self.this.clone();
        let dispatching = self.this.clone();
        let dispatch_call = call.to_owned();
        let completed_call = call.to_owned();
        let result = workspace.automation.borrow_mut().as_mut().map(|session| {
            session.navigate_when(
                url,
                move || {
                    let Some(browser) = dispatching.upgrade() else {
                        return false;
                    };
                    browser.workspace().is_some_and(|workspace| {
                        workspace.is_current(id)
                            && browser.workspace_revision(&workspace, observed.0)
                                == Some(observed.1)
                            && (observed.0 != AgentPage::Source
                                || browser
                                    .browsing
                                    .borrow()
                                    .session()
                                    .tab(workspace.source)
                                    .is_some())
                            && workspace
                                .turn
                                .borrow()
                                .as_ref()
                                .and_then(|turn| turn.tool.as_ref())
                                .is_some_and(|tool| {
                                    tool.id == dispatch_call && tool.observed == Some(observed)
                                })
                    })
                },
                move |result| {
                    if let Some(browser) = this.upgrade() {
                        browser.tool_finished(
                            id,
                            &completed_call,
                            result
                                .map(|()| {
                                    (
                                        "The page opened in the workspace.".to_owned(),
                                        "Page opened".to_owned(),
                                    )
                                })
                                .map_err(|error| error.to_string()),
                        );
                    }
                },
            )
        });
        match result {
            Some(Ok(())) => {}
            Some(Err(error)) => self.tool_finished(id, call, Err(error.to_string())),
            None => self.tool_finished(id, call, Err(UNAVAILABLE.to_owned())),
        }
    }

    #[cfg(not(feature = "main-webkit"))]
    fn navigate_workspace(
        &self,
        _workspace: &Workspace,
        id: u64,
        call: &str,
        _url: &Url,
        _observed: (AgentPage, u64),
    ) {
        self.tool_finished(id, call, Err(UNAVAILABLE.to_owned()));
    }
}

impl Workspace {
    pub(super) fn automation_available(&self) -> bool {
        self.isolated_revision().is_some()
    }

    pub(super) fn isolated_revision(&self) -> Option<u64> {
        #[cfg(feature = "main-webkit")]
        if let Some(session) = self
            .automation
            .borrow()
            .as_ref()
            .filter(|session| session.is_connected())
        {
            return Some(session.document_revision());
        }
        #[cfg(not(feature = "main-webkit"))]
        let _ = self;
        None
    }

    pub(super) fn cancel_navigation(&self) {
        #[cfg(feature = "main-webkit")]
        {
            if let Some(session) = self.automation.borrow_mut().as_mut() {
                session.cancel_navigation();
            }
        }
        #[cfg(not(feature = "main-webkit"))]
        let _ = self;
    }
}

fn page_name(page: AgentPage) -> String {
    match page {
        AgentPage::Source => "Source page",
        AgentPage::Workspace => "Workspace page",
    }
    .to_owned()
}

fn rendered_text(text: String) -> (String, String) {
    let description = if text.trim().is_empty() {
        "No visible text"
    } else {
        "Read visible text"
    };
    (text, description.to_owned())
}
