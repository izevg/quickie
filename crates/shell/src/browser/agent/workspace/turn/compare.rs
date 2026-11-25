use quickie_application::{EngineError, Entropy, PageContent, PageReader, PageSurface};
use quickie_domain_agents::{PageScope, PageText, Prompt, Request, TokenBudget};
use quickie_domain_browsing::{Tab, TabId};
use quickie_system::SystemEntropy;
use quickie_ui_widgets::WorkspaceToolStatus;

use super::{Browser, ReadingTabs, Workspace};

const READ_LIMIT: usize = 16_000;
const CHANGED: &str = "A tab changed. Run Compare Tabs again.";

pub(super) struct ReadingPage {
    tab: Tab,
    revision: u64,
    text: Option<String>,
}

fn tab_revision(engine: &(impl PageContent + PageSurface), tab: TabId) -> Option<u64> {
    #[cfg(feature = "main-webkit")]
    {
        engine.agent_source_revision(tab)
    }
    #[cfg(not(feature = "main-webkit"))]
    {
        engine.view(tab).map(|_| 0)
    }
}

impl Browser {
    pub(in crate::browser::agent::workspace) fn submit_workspace_skill(&self, request: Request) {
        if request.scope() == PageScope::Current {
            let prompt = format!(
                "{}\n\n{}",
                request.instructions(),
                request.output().instruction()
            );
            self.submit_workspace(&prompt, request.instructions());
            return;
        }
        let Some(workspace) = self.workspace() else {
            return;
        };
        if self.workspace_endpoint(&workspace).is_none()
            || workspace.turn.borrow().is_some()
            || workspace.reading.borrow().is_some()
        {
            return;
        }
        let pages: Vec<_> = {
            let browsing = self.browsing.borrow();
            let engine = browsing.engine();
            PageReader::tabs(
                browsing.session(),
                workspace.source_window,
                request.scope(),
                |tab| tab_revision(engine, tab).is_some(),
            )
            .into_iter()
            .filter_map(|tab| {
                Some(ReadingPage {
                    revision: tab_revision(engine, tab.id)?,
                    tab,
                    text: None,
                })
            })
            .collect()
        };
        if pages.len() < request.scope().min_pages() {
            workspace
                .panel
                .append_notice("Open at least two pages to compare tabs.", true);
            return;
        }
        let targets: Vec<_> = pages.iter().map(|page| page.tab.id).collect();
        let id = self.next_workspace_generation();
        workspace.panel.append_prompt(request.instructions());
        let row = workspace
            .panel
            .start_tool("Read open tabs", &format!("{} pages", pages.len()));
        workspace.panel.set_activity("Reading open tabs…", true);
        workspace.reading.replace(Some(ReadingTabs {
            id,
            request,
            pages,
            row,
        }));
        for (index, tab) in targets.into_iter().enumerate() {
            let weak = self.this.clone();
            let result =
                self.browsing
                    .borrow()
                    .engine()
                    .read_agent_text(tab, READ_LIMIT, move |result| {
                        if let Some(browser) = weak.upgrade() {
                            browser.comparison_read(id, index, result);
                        }
                    });
            if let Err(error) = result {
                self.comparison_read(id, index, Err(error));
                return;
            }
        }
    }

    pub(in crate::browser::agent::workspace) fn agent_comparison_page_moved(&self, tab: TabId) {
        if let Some(workspace) = self.workspace() {
            let changed = workspace
                .reading
                .borrow_mut()
                .take_if(|reading| reading.pages.iter().any(|page| page.tab.id == tab));
            if let Some(reading) = changed {
                fail(&workspace, reading.row, CHANGED);
            }
        }
    }

    fn comparison_revision(&self, tab: TabId) -> Option<u64> {
        tab_revision(self.browsing.try_borrow().ok()?.engine(), tab)
    }

    fn comparison_read(&self, id: u64, index: usize, result: Result<String, EngineError>) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        let _ = self.workspace_endpoint(&workspace);
        let mut pending = workspace.reading.borrow_mut();
        let Some(reading) = pending.as_mut().filter(|reading| reading.id == id) else {
            return;
        };
        if let Err(error) = reading.record(index, result, |tab| self.comparison_revision(tab)) {
            let row = reading.row;
            pending.take();
            drop(pending);
            fail(&workspace, row, &error);
            return;
        }
        let finished =
            pending.take_if(|reading| reading.pages.iter().all(|page| page.text.is_some()));
        drop(pending);
        if let Some(reading) = finished {
            self.finish_comparison(&workspace, reading);
        }
    }

    fn finish_comparison(&self, workspace: &Workspace, mut reading: ReadingTabs) {
        if !reading.is_current(|tab| self.comparison_revision(tab)) {
            fail(workspace, reading.row, CHANGED);
            return;
        }
        let (ids, pages): (Vec<_>, Vec<_>) = reading
            .pages
            .iter_mut()
            .filter_map(|page| {
                let tab = &page.tab;
                let text = page.text.take()?;
                Some((
                    tab.id,
                    PageText::new(tab.url.clone(), tab.title.clone(), text),
                ))
            })
            .unzip();
        let marker = SystemEntropy.fence_marker();
        match Prompt::assemble(&reading.request, &pages, TokenBudget::DEFAULT, &marker) {
            Ok(prompt) => {
                workspace.panel.finish_tool(
                    reading.row,
                    WorkspaceToolStatus::Completed,
                    "Read open pages",
                );
                let message = format!(
                    "{}\n\n{}",
                    prompt.messages()[1].content,
                    reading.request.output().instruction()
                );
                let excerpts = ids
                    .into_iter()
                    .zip(prompt.sent_texts().iter().cloned())
                    .collect();
                if !reading.is_current(|tab| self.comparison_revision(tab)) {
                    fail(workspace, reading.row, CHANGED);
                    return;
                }
                self.start_workspace_turn(&message, None, excerpts);
            }
            Err(error) => fail(workspace, reading.row, &error.to_string()),
        }
    }
}

impl ReadingTabs {
    fn is_current(&self, mut revision: impl FnMut(TabId) -> Option<u64>) -> bool {
        self.pages
            .iter()
            .all(|page| revision(page.tab.id) == Some(page.revision))
    }

    fn record(
        &mut self,
        index: usize,
        result: Result<String, EngineError>,
        revision: impl FnMut(TabId) -> Option<u64>,
    ) -> Result<(), String> {
        if !self.is_current(revision) {
            return Err(CHANGED.to_owned());
        }
        let text = result.map_err(|error| error.to_string())?;
        if let Some(page) = self.pages.get_mut(index) {
            page.text = Some(text);
        }
        Ok(())
    }
}

fn fail(workspace: &Workspace, row: usize, message: &str) {
    workspace
        .panel
        .finish_tool(row, WorkspaceToolStatus::Failed, "");
    workspace.panel.set_activity("", false);
    workspace.panel.append_notice(message, true);
}

#[cfg(test)]
mod tests {
    use quickie_domain_agents::Skill;
    use quickie_domain_browsing::{Jar, Profile};
    use url::Url;

    use super::*;

    fn pending_comparison() -> ReadingTabs {
        let skill = Skill::builtin()
            .into_iter()
            .find(|skill| skill.scope() == PageScope::OpenTabs)
            .unwrap();
        let pages = (1..=2)
            .map(|id| ReadingPage {
                tab: Tab {
                    id: TabId::new(Profile::Main, id),
                    url: Url::parse("https://example.com/").unwrap(),
                    title: String::new(),
                    jar: Jar::Main,
                },
                revision: 7,
                text: None,
            })
            .collect();
        ReadingTabs {
            id: 1,
            request: Request::Skill(skill),
            pages,
            row: 0,
        }
    }

    #[test]
    fn a_failed_page_read_is_not_a_completed_comparison() {
        let mut reading = pending_comparison();
        reading
            .record(0, Ok("First page".to_owned()), |_| Some(7))
            .unwrap();
        let error = EngineError("Page process stopped".to_owned());

        assert_eq!(
            reading.record(1, Err(error.clone()), |_| Some(7)),
            Err(error.to_string())
        );
        assert_eq!(reading.pages[0].text.as_deref(), Some("First page"));
        assert_eq!(reading.pages[1].text, None);
    }

    #[test]
    fn a_prior_non_source_tab_commit_rejects_the_final_reply_without_an_inbox_drain() {
        let mut reading = pending_comparison();
        let non_source = reading.pages[1].tab.id;
        reading
            .record(1, Ok("Old text from another tab".to_owned()), |_| Some(7))
            .unwrap();

        let final_read = reading.record(0, Ok("Source text".to_owned()), |tab| {
            Some(if tab == non_source { 8 } else { 7 })
        });

        assert_eq!(final_read, Err(CHANGED.to_owned()));
        assert_eq!(reading.pages[0].text, None);
    }
}
