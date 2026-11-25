//! Live GTK4 / `WebKitGTK` backend implementing [`HostBackend`] on the GTK main thread.

#![expect(
    unsafe_code,
    reason = "Accesses live GTK App reference on the main thread"
)]

use quickie_application::mcp::{
    AttachmentId, DiagnosticCapabilities, DocumentBinding, SnapshotRequest,
};
use quickie_mcp::{ForkCapabilities, ToolError, ToolErrorCode};

use crate::gtk::App;
use crate::gtk::inspector::observation::{
    DirectSubscription, ObservationError, ObservationLease, ObserverCommand, SelectedTabGrant,
    TargetSubscription, is_available,
};
use crate::mcp::product_version;
use crate::mcp::runtime::controller::{ConsentPrompt, HostBackend, LeaseAttachError};
use crate::mcp::runtime::protocol::{DecodedSubscription, DirectMethod, TargetMethod};
use crate::mcp::snapshot::SnapshotEval;

use super::consent::ConsentView;

/// Main-thread GTK backend bridging [`crate::mcp::runtime::controller::McpController`] to live
/// `WebKitGTK` tabs, [`ObservationLease`], isolated-world snapshots, and the GTK consent modal.
pub(super) struct GtkBackend {
    app: *const App,
    consent: ConsentView,
}

impl GtkBackend {
    pub(super) const fn new(app: *const App, consent: ConsentView) -> Self {
        Self { app, consent }
    }

    fn app(&self) -> &App {
        // SAFETY: `GtkBackend` lives only inside the main-thread `ACTIVE_HOST` slot which is
        // cleared during `GtkMcpHost::shutdown` before `App` is dropped.
        unsafe { &*self.app }
    }
}

impl HostBackend for GtkBackend {
    type Lease = ObservationLease;
    type SnapshotHandle = SnapshotEval;

    fn product_version(&self) -> String {
        product_version().to_owned()
    }

    fn capabilities(&self) -> ForkCapabilities {
        let spi = is_available();
        ForkCapabilities {
            diagnostic: DiagnosticCapabilities {
                dom: true,
                accessibility: false,
                console: spi,
                runtime_errors: spi,
                pending_network: spi,
                completed_network: spi,
                lifecycle: spi,
            },
            native_accessibility: false,
            visual_inspector_priority: spi,
            agent_automation: false,
        }
    }

    fn active_document(&self) -> Option<DocumentBinding> {
        self.app().browsing.active_document_binding()
    }

    fn show_consent(&mut self, prompt: ConsentPrompt) {
        self.consent.show_prompt(&prompt);
    }

    fn hide_consent(&mut self) {
        self.consent.hide_prompt();
    }

    fn set_active_indicator(&mut self, active_origins: &[String]) {
        self.consent.set_active_origins(active_origins);
    }

    fn attach_lease(
        &mut self,
        attachment: AttachmentId,
        binding: &DocumentBinding,
    ) -> Result<Self::Lease, LeaseAttachError> {
        let Some(view) = self.app().browsing.view_for_tab(binding.tab) else {
            return Err(LeaseAttachError::ClosedStream);
        };
        let Some(grant) =
            SelectedTabGrant::new(binding.tab, format!("mcp-attachment-{}", attachment.get()))
        else {
            return Err(LeaseAttachError::ClosedStream);
        };
        ObservationLease::attach(
            grant,
            view,
            move |frame| super::on_native_frame(attachment, frame),
            move || super::on_native_closed(attachment),
        )
        .map_err(|error| match error {
            ObservationError::UnsupportedSpi | ObservationError::WrongThread => {
                LeaseAttachError::UnsupportedSpi
            }
            ObservationError::OccupiedInspector => LeaseAttachError::OccupiedInspector,
            ObservationError::ClosedStream | ObservationError::DispatchFailed => {
                LeaseAttachError::ClosedStream
            }
        })
    }

    fn dispatch_lease_command(
        &mut self,
        lease: &mut Self::Lease,
        command: &DecodedSubscription,
    ) -> bool {
        let observer_command = match command {
            DecodedSubscription::Direct { id, method } => {
                let sub = match method {
                    DirectMethod::NetworkEnable => DirectSubscription::NetworkEnable,
                    DirectMethod::NetworkDisable => DirectSubscription::NetworkDisable,
                    DirectMethod::PageEnable => DirectSubscription::PageEnable,
                    DirectMethod::PageDisable => DirectSubscription::PageDisable,
                };
                ObserverCommand::direct(*id, sub)
            }
            DecodedSubscription::Target {
                id,
                target_id,
                inner_id,
                method,
            } => {
                let sub = match method {
                    TargetMethod::ConsoleEnable => TargetSubscription::ConsoleEnable,
                    TargetMethod::ConsoleDisable => TargetSubscription::ConsoleDisable,
                    TargetMethod::RuntimeEnable => TargetSubscription::RuntimeEnable,
                    TargetMethod::RuntimeDisable => TargetSubscription::RuntimeDisable,
                    TargetMethod::NetworkEnable => TargetSubscription::NetworkEnable,
                    TargetMethod::NetworkDisable => TargetSubscription::NetworkDisable,
                    TargetMethod::PageEnable => TargetSubscription::PageEnable,
                    TargetMethod::PageDisable => TargetSubscription::PageDisable,
                };
                let Some(cmd) = ObserverCommand::for_target(*id, target_id, *inner_id, sub) else {
                    return false;
                };
                cmd
            }
        };
        lease.dispatch_subscription(&observer_command).is_ok()
    }

    fn stop_lease(&mut self, mut lease: Self::Lease) {
        lease.stop();
    }

    fn start_snapshot(
        &mut self,
        snapshot_id: u64,
        binding: &DocumentBinding,
        request: &SnapshotRequest,
    ) -> Result<Self::SnapshotHandle, ToolError> {
        let Some(view) = self.app().browsing.view_for_tab(binding.tab) else {
            return Err(ToolError::new(
                ToolErrorCode::AttachmentUnavailable,
                "diagnostic attachment is not available",
            ));
        };
        SnapshotEval::start(
            view,
            request,
            Box::new(move |result| super::on_snapshot_completed(snapshot_id, result)),
        )
    }
}
