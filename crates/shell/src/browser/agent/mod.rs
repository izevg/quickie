//! The agent workspace and endpoint settings. The model is contacted only on an explicit request.

mod settings;
mod workspace;

use std::thread;

use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::MainThreadMarker;
use quickie_system::KeychainSecrets;
use quickie_ui_widgets::AgentSettings;

use super::Browser;

/// The agent's workspace, settings form, and access to saved credentials.
pub struct AgentUi {
    workspace: workspace::WorkspaceUi,
    pub(super) settings: AgentSettings,
    secrets: KeychainSecrets,
}

impl AgentUi {
    pub fn new(secrets: KeychainSecrets, mtm: MainThreadMarker) -> Self {
        Self {
            workspace: workspace::WorkspaceUi::default(),
            settings: AgentSettings::new(mtm),
            secrets,
        }
    }
}

impl Browser {
    pub(super) fn wire_agent(&self) {
        let this = self.this.clone();
        self.agent_ui.settings.on_request(move |request| {
            if let Some(browser) = this.upgrade() {
                browser.settings_request(request);
                browser.drain();
            }
        });
    }

    /// Runs `work` on a worker thread and delivers the result to the main thread.
    pub(in crate::browser) fn off_main<T: Send + 'static>(
        &self,
        name: &str,
        work: impl FnOnce() -> T + Send + 'static,
        done: fn(&Self, T),
    ) -> bool {
        let browser = MainThreadBound::new(self.this.clone(), self.mtm);
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let result = work();
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    if let Some(browser) = browser.into_inner(mtm).upgrade() {
                        done(&browser, result);
                        browser.drain();
                    }
                });
            })
            .is_ok()
    }
}
