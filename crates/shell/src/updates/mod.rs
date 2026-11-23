//! The macOS release updater and its application lifecycle.

mod dialog;
mod install;
mod network;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::{Rc, Weak};

use block2::RcBlock;
use dispatch2::{DispatchQoS, DispatchQueue, GlobalQueueIdentifier, MainThreadBound};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};
use objc2_foundation::NSTimer;
use quickie_application::StopSignal;
use quickie_store::PendingWrites;
use quickie_updater::{Distribution, Preferences, UpdateAsset, Version, download, fetch_latest};

use dialog::{Choice, Progress};
use install::{InstallError, StagedUpdate};
use network::UpdateHttp;

pub(crate) const INSTALLED_VERSION: &str = match option_env!("QUICKIE_RELEASE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

enum Phase {
    Idle,
    Checking(StopSignal),
    Current,
    Available(UpdateAsset),
    Downloading(StopSignal),
    Staged(UpdateAsset, Box<StagedUpdate>),
    Cancelled,
    Failed(String),
}

enum Check {
    Skipped,
    Completed(Option<Box<UpdateAsset>>),
}

pub(crate) struct Updater {
    installed: Option<(Version, PathBuf)>,
    preferences: Preferences,
    automatic: Cell<bool>,
    scheduled_download: Cell<bool>,
    phase: RefCell<Phase>,
    progress: RefCell<Option<Progress>>,
    show_result: Cell<bool>,
    terminating: Cell<bool>,
    restart: Cell<bool>,
    this: Weak<Self>,
    mtm: MainThreadMarker,
}

impl Updater {
    pub(crate) fn new(profile: &Path, mtm: MainThreadMarker) -> Rc<Self> {
        let preferences = Preferences::new(profile.join("Updates"));
        let installed = (cfg!(all(target_arch = "aarch64", feature = "main-webkit")))
            .then(|| {
                Some((
                    Version::parse(INSTALLED_VERSION).ok()?,
                    install::installed_bundle(INSTALLED_VERSION).ok()?,
                ))
            })
            .flatten();
        Rc::new_cyclic(|this| Self {
            installed,
            automatic: Cell::new(preferences.automatic()),
            scheduled_download: Cell::new(false),
            preferences,
            phase: RefCell::new(Phase::Idle),
            progress: RefCell::new(None),
            show_result: Cell::new(false),
            terminating: Cell::new(false),
            restart: Cell::new(false),
            this: this.clone(),
            mtm,
        })
    }

    pub(crate) fn supported(&self) -> bool {
        self.installed.is_some()
    }

    pub(crate) fn automatic(&self) -> bool {
        self.automatic.get()
    }

    pub(crate) fn set_automatic(&self, enabled: bool) -> Result<(), String> {
        self.preferences
            .set_automatic(enabled)
            .map_err(|error| error.to_string())?;
        self.automatic.set(enabled);
        if enabled {
            self.automatic_check();
        } else if self.scheduled_download.get() && matches!(*self.phase.borrow(), Phase::Staged(..))
        {
            self.set_phase(Phase::Cancelled);
        } else if !self.show_result.get() || self.scheduled_download.get() {
            self.cancel();
        }
        Ok(())
    }

    pub(crate) fn schedule(&self) {
        if !self.supported() {
            return;
        }
        let this = self.this.clone();
        let tick = RcBlock::new(move |_| {
            if let Some(updater) = this.upgrade() {
                updater.automatic_check();
            }
        });
        // SAFETY: the timer is scheduled on the main thread and retains this main-thread block.
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(3600.0, true, &tick) };
        self.automatic_check();
    }

    fn automatic_check(&self) {
        if self.automatic() {
            self.check(false);
        }
    }

    pub(crate) fn check(&self, manual: bool) {
        if self.terminating() {
            return;
        }
        let Some((installed, _)) = &self.installed else {
            if manual {
                dialog::message(
                    "Software Updates",
                    "This Quickie build does not have an installed release to update.",
                    self.mtm,
                );
            }
            return;
        };
        let action = match &*self.phase.borrow() {
            Phase::Checking(_) => Some("Checking for Updates…"),
            Phase::Downloading(_) => Some("Preparing the Update…"),
            Phase::Staged(..) => Some(""),
            _ => None,
        };
        if let Some(progress) = action {
            if manual {
                if progress.is_empty() {
                    self.present_phase();
                } else {
                    self.show_result.set(true);
                    self.present_progress(progress);
                }
            }
            return;
        }

        let stop = StopSignal::new();
        self.scheduled_download.set(false);
        *self.phase.borrow_mut() = Phase::Checking(stop.clone());
        self.show_result.set(manual);
        if manual {
            self.present_progress("Checking for Updates…");
        }
        let version = installed.clone();
        let preferences = self.preferences.clone();
        self.worker(
            move || {
                if manual {
                    preferences
                        .record_check()
                        .map_err(|error| error.to_string())?;
                } else if !preferences
                    .claim_scheduled_check()
                    .map_err(|error| error.to_string())?
                {
                    return Ok(Check::Skipped);
                }
                if stop.is_stopped() {
                    return Err("The update check was cancelled.".into());
                }
                fetch_latest(&UpdateHttp(stop), &version, Distribution::MacosArm64)
                    .map(|asset| Check::Completed(asset.map(Box::new)))
            },
            Self::checked,
        );
    }

    fn checked(&self, result: Result<Check, String>) {
        if self.cancelled() {
            return self.set_phase(Phase::Cancelled);
        }
        match result {
            Ok(Check::Completed(Some(asset))) if !self.show_result.get() && self.automatic() => {
                self.begin_download(*asset, false);
            }
            Ok(Check::Completed(Some(asset))) => self.set_phase(Phase::Available(*asset)),
            Ok(Check::Completed(None)) => self.set_phase(Phase::Current),
            Ok(Check::Skipped) if self.show_result.get() => self.check_after_skipped_schedule(),
            Ok(Check::Skipped) => self.set_phase(Phase::Idle),
            Err(error) => self.set_phase(Phase::Failed(error)),
        }
    }

    fn check_after_skipped_schedule(&self) {
        *self.phase.borrow_mut() = Phase::Idle;
        self.check(true);
    }

    fn begin_download(&self, asset: UpdateAsset, manual: bool) {
        let stop = StopSignal::new();
        self.scheduled_download.set(!manual);
        *self.phase.borrow_mut() = Phase::Downloading(stop.clone());
        self.show_result.set(manual);
        if manual {
            self.present_progress("Preparing the Update…");
        }
        self.worker(
            move || {
                let transport = UpdateHttp(stop.clone());
                let staged = download(&transport, &asset).and_then(|artifact| {
                    if stop.is_stopped() {
                        return Err("The update download was cancelled.".into());
                    }
                    install::stage(
                        &artifact.path().join(asset.name()),
                        INSTALLED_VERSION,
                        &asset.version().to_string(),
                    )
                });
                (asset, staged)
            },
            |updater, (asset, result)| {
                if updater.cancelled() {
                    return updater.set_phase(Phase::Cancelled);
                }
                updater.set_phase(match result {
                    Ok(staged) => Phase::Staged(asset, Box::new(staged)),
                    Err(error) => Phase::Failed(error),
                });
            },
        );
    }

    fn worker<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        done: fn(&Self, T),
    ) {
        let this = MainThreadBound::new(self.this.clone(), self.mtm);
        let queue = DispatchQueue::global_queue(GlobalQueueIdentifier::QualityOfService(
            DispatchQoS::Utility,
        ));
        queue.exec_async(move || {
            let result = work();
            DispatchQueue::main().exec_async(move || {
                if let Some(mtm) = MainThreadMarker::new()
                    && let Some(updater) = this.into_inner(mtm).upgrade()
                {
                    done(&updater, result);
                }
            });
        });
    }

    fn signal(&self) -> Option<StopSignal> {
        match &*self.phase.borrow() {
            Phase::Checking(stop) | Phase::Downloading(stop) => Some(stop.clone()),
            _ => None,
        }
    }

    fn cancel(&self) {
        if let Some(stop) = self.signal() {
            stop.stop();
        }
    }

    fn cancelled(&self) -> bool {
        self.signal().is_some_and(|stop| stop.is_stopped())
    }

    fn present_progress(&self, title: &str) {
        if self.progress.borrow().is_some() {
            return;
        }
        if self.signal().is_some() {
            let this = self.this.clone();
            let progress = Progress::new(title, self.mtm, move || {
                if let Some(updater) = this.upgrade() {
                    updater.cancel();
                    updater.progress.take();
                }
            });
            *self.progress.borrow_mut() = progress;
        }
    }

    fn set_phase(&self, phase: Phase) {
        *self.phase.borrow_mut() = phase;
        if let Some(progress) = self.progress.take() {
            progress.close();
        }
        if self.show_result.replace(false) && !self.terminating() {
            let this = MainThreadBound::new(self.this.clone(), self.mtm);
            DispatchQueue::main().exec_async(move || {
                if let Some(mtm) = MainThreadMarker::new()
                    && let Some(updater) = this.into_inner(mtm).upgrade()
                {
                    updater.present_phase();
                }
            });
        }
    }

    fn present_phase(&self) {
        enum Prompt {
            Release(UpdateAsset, bool),
            Message(&'static str, String),
        }
        let prompt = match &*self.phase.borrow() {
            Phase::Available(asset) => Prompt::Release(asset.clone(), false),
            Phase::Staged(asset, _) => Prompt::Release(asset.clone(), true),
            Phase::Current => Prompt::Message(
                "Quickie Is Up to Date",
                format!("Quickie {INSTALLED_VERSION} is installed."),
            ),
            Phase::Cancelled => Prompt::Message(
                "Update Cancelled",
                "The current Quickie installation is unchanged.".into(),
            ),
            Phase::Failed(reason) => Prompt::Message("Quickie Could Not Update", reason.clone()),
            _ => return,
        };
        match prompt {
            Prompt::Message(title, detail) => dialog::message(title, &detail, self.mtm),
            Prompt::Release(asset, staged) => {
                let action = if staged {
                    "Update and Restart"
                } else {
                    "Download Update"
                };
                let state = if staged {
                    "The update is ready and will install when Quickie quits."
                } else {
                    "A new version of Quickie is available."
                };
                if dialog::notes(
                    &asset.version().to_string(),
                    asset.notes(),
                    state,
                    Some(action),
                    self.mtm,
                ) == Choice::Primary
                {
                    if staged {
                        self.restart.set(true);
                        NSApplication::sharedApplication(self.mtm).terminate(None);
                    } else {
                        self.begin_download(asset, true);
                    }
                }
            }
        }
    }

    pub(crate) fn whats_new(&self) {
        let Some((_, installed)) = &self.installed else {
            return;
        };
        let notes = dialog::installed_notes(&installed.join("Contents/Resources/ReleaseNotes.md"));
        dialog::notes(
            INSTALLED_VERSION,
            &notes,
            "What’s New in Quickie",
            None,
            self.mtm,
        );
    }

    pub(crate) fn terminating(&self) -> bool {
        self.terminating.get()
    }

    pub(crate) fn terminate(&self, writes: &[PendingWrites]) -> NSApplicationTerminateReply {
        use NSApplicationTerminateReply as Reply;

        if self.terminating() {
            return Reply::TerminateLater;
        }
        let phase = self.phase.replace(Phase::Idle);
        let Phase::Staged(_, staged) = phase else {
            *self.phase.borrow_mut() = phase;
            self.cancel();
            return Reply::TerminateNow;
        };
        self.terminating.set(true);
        let writes = writes.to_vec();
        self.worker(
            move || {
                let flushed = writes.iter().try_for_each(PendingWrites::flush);
                (staged, flushed)
            },
            |updater, (staged, flushed)| {
                if let Err(error) = flushed {
                    return updater.install_finished(Err(InstallError::Failed(format!(
                        "Could not save the browser session: {error}"
                    ))));
                }
                let this = MainThreadBound::new(updater.this.clone(), updater.mtm);
                staged.apply(updater.mtm, move |result| {
                    DispatchQueue::main().exec_async(move || {
                        if let Some(mtm) = MainThreadMarker::new()
                            && let Some(updater) = this.into_inner(mtm).upgrade()
                        {
                            updater.install_finished(result);
                        }
                    });
                });
            },
        );
        Reply::TerminateLater
    }

    fn install_finished(&self, result: Result<PathBuf, InstallError>) {
        let app = NSApplication::sharedApplication(self.mtm);
        let restart = self.restart.replace(false);
        self.terminating.set(false);
        match result {
            Ok(installed) => {
                if restart && let Err(error) = restart_after_exit(&installed) {
                    dialog::message(
                        "Quickie Updated",
                        &format!("Open Quickie to start the new version. {error}"),
                        self.mtm,
                    );
                }
                app.replyToApplicationShouldTerminate(true);
            }
            Err(error) => {
                self.set_phase(match error {
                    InstallError::Cancelled => Phase::Cancelled,
                    InstallError::Failed(reason) => Phase::Failed(reason),
                });
                app.replyToApplicationShouldTerminate(false);
                self.present_phase();
            }
        }
    }
}

fn restart_after_exit(installed: &Path) -> std::io::Result<()> {
    // The old process must release its session before Launch Services opens the new app.
    Command::new("/bin/sh")
        .args([
            "-c",
            r#"while /bin/kill -0 "$1" 2>/dev/null; do /bin/sleep 1; done; exec /usr/bin/open "$2""#,
            "quickie-restart",
            &std::process::id().to_string(),
        ])
        .arg(installed)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}
