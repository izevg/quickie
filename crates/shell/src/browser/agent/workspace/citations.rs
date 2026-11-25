use std::collections::BTreeMap;

use quickie_application::{AgentPage, Clock, FindStep, PageContent};
use quickie_domain_agents::AgentSession;
use quickie_domain_browsing::TabId;
use quickie_ui_widgets::WorkspaceCitation;

use super::{Browser, Workspace};
use crate::SystemClock;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Document {
    Tab(TabId),
    Isolated(u64),
}

#[derive(Clone)]
struct Passage {
    document: Document,
    anchor: String,
}

#[derive(Default)]
pub(super) struct Citations {
    reads: Vec<(Document, String)>,
    passages: BTreeMap<u32, Passage>,
    next: u32,
}

impl Citations {
    pub(super) fn begin_reads(&mut self, excerpts: Vec<(TabId, String)>) {
        self.reads = excerpts
            .into_iter()
            .map(|(tab, text)| (Document::Tab(tab), text))
            .collect();
    }

    pub(super) fn clear_reads(&mut self) {
        self.reads.clear();
    }

    #[cfg(feature = "main-webkit")]
    pub(super) fn forget_isolated_page(&mut self) {
        self.discard_where(|document| matches!(document, Document::Isolated(_)));
    }

    pub(super) fn forget_tab(&mut self, tab: TabId) {
        self.discard_where(|document| document == Document::Tab(tab));
    }

    fn discard_where(&mut self, changed: impl Fn(Document) -> bool) {
        self.reads.retain(|(document, _)| !changed(*document));
        self.passages
            .retain(|_, passage| !changed(passage.document));
    }

    pub(super) fn verify_answer(&mut self, text: &str) -> (String, Vec<WorkspaceCitation>) {
        let at = SystemClock.now();
        let (documents, excerpts): (Vec<_>, Vec<_>) = self.reads.drain(..).unzip();
        let mut session = AgentSession::for_texts(&excerpts, at);
        session.received(text, at);
        let finished = session.finish(at, None);
        let sources = finished
            .citations
            .into_iter()
            .filter_map(|citation| {
                let index = citation.page_in(&excerpts)?;
                self.next = self.next.checked_add(1)?;
                self.passages.insert(
                    self.next,
                    Passage {
                        document: documents[index],
                        anchor: citation.anchor,
                    },
                );
                Some(WorkspaceCitation {
                    id: self.next,
                    quote: citation.quote,
                })
            })
            .collect();
        (finished.answer, sources)
    }
}

impl Workspace {
    pub(super) fn record_citation_source(
        &self,
        turn_id: u64,
        call_id: &str,
        page: AgentPage,
        revision: u64,
        text: &str,
    ) {
        if self.turn.borrow().as_ref().is_some_and(|turn| {
            turn.id == turn_id && turn.tool.as_ref().is_some_and(|tool| tool.id == call_id)
        }) {
            let document = match page {
                AgentPage::Source => Document::Tab(self.source),
                AgentPage::Workspace => Document::Isolated(revision),
            };
            self.citations
                .borrow_mut()
                .reads
                .push((document, text.to_owned()));
        }
    }
}

impl Browser {
    pub(super) fn focus_citation(&self, id: u32) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        let Some(passage) = workspace.citations.borrow().passages.get(&id).cloned() else {
            workspace.panel.expire_citation(id);
            return;
        };
        match passage.document {
            Document::Tab(tab) => {
                if let Some(window) = self.run(|browsing| browsing.activate(tab)) {
                    workspace.show_preview(false);
                    if let Some(window) = self.window(window) {
                        window.makeKeyAndOrderFront(None);
                    }
                    let _ = self.browsing.borrow_mut().engine_mut().find(
                        tab,
                        &passage.anchor,
                        FindStep::Refine,
                    );
                } else {
                    workspace.panel.expire_citation(id);
                }
            }
            Document::Isolated(revision) if workspace.isolated_revision() == Some(revision) =>
            {
                #[cfg(feature = "main-webkit")]
                if let Some(session) = workspace.automation.borrow().as_ref() {
                    workspace.show_preview(true);
                    session.find_text(&passage.anchor);
                }
            }
            Document::Isolated(_) => workspace.panel.expire_citation(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_citations_only_target_text_read_for_that_turn() {
        let source = Document::Isolated(3);
        let mut citations = Citations::default();
        citations.reads.push((
            source,
            "The current page\ncontains this passage.".to_owned(),
        ));

        let (_, checked) = citations.verify_answer("[[cite: current page contains this passage]] [[cite: invented text belongs elsewhere]]");

        assert_eq!(checked.len(), 1);
        assert_eq!(checked[0].quote, "current page contains this passage");
        assert!(citations.passages[&checked[0].id].document == source);
        assert_eq!(
            citations
                .verify_answer("[[cite: current page contains this passage]]")
                .1,
            [],
        );
    }

    #[test]
    fn cites_the_compared_tab_that_contains_the_quote_and_forgets_it_on_navigation() {
        use quickie_domain_browsing::Profile;

        let first = TabId::new(Profile::Main, 1);
        let second = TabId::new(Profile::Main, 2);
        let mut citations = Citations::default();
        citations.begin_reads(vec![
            (first, "This excerpt is from the first tab.".to_owned()),
            (
                second,
                "Only the second document contains this line.".to_owned(),
            ),
        ]);

        let (_, sources) =
            citations.verify_answer("[[cite: the second document contains this line]]");

        assert_eq!(sources.len(), 1);
        assert!(citations.passages[&sources[0].id].document == Document::Tab(second));
        citations.forget_tab(second);
        assert!(!citations.passages.contains_key(&sources[0].id));
    }
}
