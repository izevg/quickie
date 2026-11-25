//! The persistent Agent panel in the source window, with an isolated preview for navigation.

mod citations;
mod tools;
mod turn;

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
#[cfg(feature = "main-webkit")]
use objc2_app_kit::NSAutoresizingMaskOptions;
use objc2_app_kit::{NSView, NSWindow, NSWindowWillCloseNotification};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use quickie_application::AgentPage;
use quickie_domain_agents::{LlmEndpoint, PageScope, Question, Request, Skill, SkillId};
use quickie_domain_browsing::{Focus, JarChoice, TabId, WindowId};
use quickie_ui_widgets::{AgentWorkspacePanel, WorkspacePanelRequest};
use url::Url;

use super::Browser;

#[derive(Default)]
pub(super) struct WorkspaceUi {
    shown: RefCell<Option<Rc<Workspace>>>,
    generation: Cell<u64>,
}

struct Workspace {
    window: Retained<NSWindow>,
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    panel: Rc<AgentWorkspacePanel>,
    page: Retained<NSView>,
    preview_available: Cell<bool>,
    source: TabId,
    source_window: WindowId,
    #[cfg(not(feature = "main-webkit"))]
    source_revision: Cell<u64>,
    #[cfg(not(feature = "main-webkit"))]
    source_loaded: Cell<bool>,
    model: RefCell<turn::ModelContext<crate::Network>>,
    turn: RefCell<Option<turn::InFlight>>,
    reading: RefCell<Option<turn::ReadingTabs>>,
    observations: RefCell<tools::Observations>,
    citations: RefCell<citations::Citations>,
    #[cfg(feature = "main-webkit")]
    automation: RefCell<Option<quickie_engine_webkit::InteractiveAutomation>>,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if let Some(turn) = self.turn.get_mut().take() {
            turn.stop.stop();
        }
        // SAFETY: this token was registered with the default center on the main thread.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver((*self.observer).as_ref()) };
        self.panel.view().removeFromSuperview();
    }
}

impl Browser {
    pub(in crate::browser) fn open_agent_workspace(&self) {
        self.close_overlays();
        if let Some(workspace) = self.workspace() {
            workspace.window.makeKeyAndOrderFront(None);
            workspace.panel.focus_composer();
            return;
        }
        let Some((window, ns_window)) = self.current() else {
            return;
        };
        let source = {
            let browsing = self.browsing.borrow();
            let session = browsing.session();
            session
                .active(window)
                .and_then(|tab| session.tab(tab).map(|page| (tab, page.url.clone())))
        };
        let Some((source, url)) = source else {
            return;
        };
        let Some(workspace) = self.make_workspace(window, ns_window, source, &url) else {
            return;
        };
        self.agent_ui
            .workspace
            .shown
            .replace(Some(Rc::clone(&workspace)));
        workspace.panel.focus_composer();
    }

    pub(in crate::browser) fn ask_question(&self, question: &Question) {
        if self.open_agent_for_request(PageScope::Current) {
            self.submit_workspace(question.as_str(), question.as_str());
        }
    }

    pub(in crate::browser) fn ask_skill(&self, id: &SkillId) {
        match Skill::builtin_named(id) {
            Some(skill) if self.open_agent_for_request(skill.scope()) => {
                self.submit_workspace_skill(Request::Skill(skill));
            }
            Some(_) => {}
            None => self.toast(&format!("No skill named {id}")),
        }
    }

    fn open_agent_for_request(&self, scope: PageScope) -> bool {
        let Some((window, _)) = self.current() else {
            return false;
        };
        if self.workspace().is_some_and(|workspace| {
            !request_matches_source(
                workspace.source_window,
                workspace.source,
                window,
                self.active(window),
                scope,
            )
        }) {
            self.toast(
                "The Agent panel is attached to another tab or window. Close it to ask here.",
            );
            return false;
        }
        self.open_agent_workspace();
        self.workspace().is_some()
    }

    pub(in crate::browser) fn agent_is_open(&self, window: &NSWindow) -> bool {
        self.workspace()
            .is_some_and(|workspace| *workspace.window == *window)
    }

    pub(in crate::browser) fn agent_has_focus(&self, window: &NSWindow) -> bool {
        self.workspace()
            .is_some_and(|workspace| *workspace.window == *window && workspace.panel.has_focus())
    }

    pub(in crate::browser) fn close_agent_workspace(&self) {
        let workspace = self.agent_ui.workspace.shown.borrow_mut().take();
        let Some(workspace) = workspace else {
            return;
        };
        let source = workspace.source_window;
        drop(workspace);
        self.settle(source);
    }

    pub(in crate::browser) fn agent_source_navigated(&self, tab: TabId, url: &Url) {
        self.agent_document_changed(tab, Some(url));
    }

    pub(in crate::browser) fn agent_document_lost(&self, tab: TabId) {
        self.agent_document_changed(tab, None);
    }

    fn agent_document_changed(&self, tab: TabId, url: Option<&Url>) {
        self.agent_comparison_page_moved(tab);
        if let Some(workspace) = self.workspace() {
            workspace.citations.borrow_mut().forget_tab(tab);
            if workspace.source != tab {
                return;
            }
            #[cfg(not(feature = "main-webkit"))]
            {
                let revision = &workspace.source_revision;
                revision.set(revision.get().wrapping_add(1));
                workspace.source_loaded.set(url.is_some());
            }
            if let Some(url) = url {
                workspace.set_source_label(url);
            } else {
                workspace.panel.set_source("Source page unavailable");
            }
            workspace.observations.borrow_mut().clear();
            let revision = self.workspace_revision(&workspace, AgentPage::Source);
            let revoked = workspace.turn.borrow().as_ref().and_then(|turn| {
                let tool = turn.tool.as_ref()?;
                tool.source_changed(revision)
                    .then(|| (turn.id, tool.id.clone()))
            });
            if let Some((id, call)) = revoked {
                workspace.cancel_navigation();
                self.tool_finished(id, &call, Err(tools::STALE.to_owned()));
            }
        }
    }

    pub(super) fn refresh_workspace_settings(&self) {
        if let Some(workspace) = self.workspace() {
            let _ = self.workspace_endpoint(&workspace);
        }
    }

    fn workspace_endpoint(&self, workspace: &Workspace) -> Option<LlmEndpoint> {
        let endpoint = self.agent.borrow().endpoint().cloned();
        let changed = workspace
            .model
            .borrow_mut()
            .bind(endpoint.as_ref(), &self.http);
        if changed {
            Self::stop_workspace_turn(workspace, "Stopped because the agent endpoint changed.");
            workspace.observations.borrow_mut().clear();
            workspace.citations.borrow_mut().clear_reads();
            workspace.panel.set_unconfigured(endpoint.is_none());
        }
        endpoint
    }

    fn workspace(&self) -> Option<Rc<Workspace>> {
        self.agent_ui.workspace.shown.borrow().clone()
    }

    fn make_workspace(
        &self,
        source_window: WindowId,
        window: Retained<NSWindow>,
        source: TabId,
        url: &Url,
    ) -> Option<Rc<Workspace>> {
        let host = window.contentView()?;
        let panel = AgentWorkspacePanel::new(self.mtm);
        let page = NSView::new(self.mtm);
        page.setHidden(true);
        panel.attach_page(&page);
        panel.show_in(&host);
        let this = self.this.clone();
        panel.on_request(move |request| {
            if let Some(browser) = this.upgrade() {
                browser.workspace_request(request);
                browser.drain();
            }
        });
        let observer = self.on_workspace_close(&window);
        let endpoint = self.agent.borrow().endpoint().cloned();
        let workspace = Rc::new(Workspace {
            window,
            observer,
            panel,
            page,
            preview_available: Cell::new(false),
            source,
            source_window,
            #[cfg(not(feature = "main-webkit"))]
            source_revision: Cell::new(0),
            #[cfg(not(feature = "main-webkit"))]
            source_loaded: Cell::new(true),
            model: RefCell::new(turn::ModelContext::new(
                self.http.clone(),
                endpoint.as_ref(),
            )),
            turn: RefCell::default(),
            reading: RefCell::default(),
            observations: RefCell::default(),
            citations: RefCell::default(),
            #[cfg(feature = "main-webkit")]
            automation: RefCell::default(),
        });
        workspace.set_source_label(url);
        workspace.panel.set_unconfigured(endpoint.is_none());
        Some(workspace)
    }

    fn on_workspace_close(
        &self,
        window: &NSWindow,
    ) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
        let this = self.this.clone();
        let closed = RcBlock::new(move |_: NonNull<NSNotification>| {
            if let Some(browser) = this.upgrade() {
                browser.agent_ui.workspace.shown.borrow_mut().take();
            }
        });
        // SAFETY: this window posts its close notification on the main thread.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSWindowWillCloseNotification),
                Some(window.as_ref()),
                None,
                &closed,
            )
        }
    }

    fn workspace_request(&self, request: WorkspacePanelRequest) {
        match request {
            WorkspacePanelRequest::Submit(text) => match Question::try_from(text.as_str()) {
                Ok(question) => self.submit_workspace(question.as_str(), question.as_str()),
                Err(error) => {
                    if let Some(workspace) = self.workspace() {
                        workspace.panel.append_notice(&error.to_string(), true);
                    }
                }
            },
            WorkspacePanelRequest::Stop => {
                if let Some(workspace) = self.workspace() {
                    Self::stop_workspace_turn(&workspace, "Stopped");
                }
            }
            WorkspacePanelRequest::Close => self.close_agent_workspace(),
            WorkspacePanelRequest::TogglePreview => {
                if let Some(workspace) = self.workspace() {
                    workspace.show_preview(workspace.page.isHidden());
                }
            }
            WorkspacePanelRequest::Settings => self.open_agent_settings(),
            WorkspacePanelRequest::Open(url) => self.open_workspace_link(url),
            WorkspacePanelRequest::Cite(id) => self.focus_citation(id),
        }
    }

    fn open_workspace_link(&self, url: Url) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        if !matches!(url.scheme(), "https" | "http")
            || self
                .browsing
                .borrow()
                .session()
                .tab(workspace.source)
                .is_none()
        {
            return;
        }
        self.run(|browsing| {
            browsing.open(
                workspace.source_window,
                url,
                JarChoice::SameAs(workspace.source),
                Focus::Background,
            )
        });
    }
}

fn request_matches_source(
    source_window: WindowId,
    source: TabId,
    requesting_window: WindowId,
    active: Option<TabId>,
    scope: PageScope,
) -> bool {
    source_window == requesting_window && (scope != PageScope::Current || active == Some(source))
}

impl Workspace {
    fn set_source_label(&self, url: &Url) {
        let label = url.host_str().unwrap_or("Current tab");
        self.panel.set_source(label);
    }

    fn show_preview(&self, show: bool) {
        let available = self.preview_available.get();
        let shown = show && available;
        self.page.setHidden(!shown);
        self.panel.set_preview(available, shown);
    }

    fn preview_navigated(&self) {
        self.preview_available.set(true);
        self.show_preview(true);
    }

    #[cfg(feature = "main-webkit")]
    fn prepare_page(&self, mtm: objc2::MainThreadMarker) {
        if self
            .automation
            .borrow()
            .as_ref()
            .is_some_and(quickie_engine_webkit::InteractiveAutomation::is_connected)
        {
            return;
        }
        if let Some(previous) = self.automation.take() {
            previous.view().removeFromSuperview();
        }
        self.preview_available.set(false);
        self.show_preview(false);
        self.observations.borrow_mut().clear();
        self.citations.borrow_mut().forget_isolated_page();
        let Ok(session) = quickie_engine_webkit::InteractiveAutomation::open(mtm) else {
            return;
        };
        let view: &NSView = session.view();
        view.setFrame(self.page.bounds());
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        self.page.addSubview(view);
        self.automation.replace(Some(session));
    }

    #[cfg(not(feature = "main-webkit"))]
    #[expect(
        clippy::unused_self,
        reason = "stub matching the main-webkit prepare_page method signature"
    )]
    fn prepare_page(&self, _mtm: objc2::MainThreadMarker) {}
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::Profile;

    use super::*;

    #[test]
    fn only_the_bound_active_tab_can_ask_about_the_current_page() {
        let window = WindowId::new(Profile::Main, 1);
        let source = TabId::new(Profile::Main, 2);
        let other = TabId::new(Profile::Main, 3);

        assert!(request_matches_source(
            window,
            source,
            window,
            Some(source),
            PageScope::Current
        ));
        assert!(!request_matches_source(
            window,
            source,
            window,
            Some(other),
            PageScope::Current
        ));
        assert!(!request_matches_source(
            window,
            source,
            WindowId::new(Profile::Main, 4),
            Some(source),
            PageScope::Current,
        ));
    }

    #[test]
    fn compare_targets_the_bound_window_even_when_another_tab_is_active() {
        let window = WindowId::new(Profile::Main, 1);
        let source = TabId::new(Profile::Main, 2);
        let active = TabId::new(Profile::Main, 3);

        assert!(request_matches_source(
            window,
            source,
            window,
            Some(active),
            PageScope::OpenTabs
        ));
        assert!(!request_matches_source(
            window,
            source,
            WindowId::new(Profile::Main, 4),
            Some(active),
            PageScope::OpenTabs,
        ));
    }
}
