//! Linux update state and the workers that check, download, and stage a release.

pub mod dialog;
pub(crate) mod install;
pub(crate) mod network;

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use quickie_updater::{Preferences, UpdateAsset, Version, download, fetch_latest};
use tempfile::TempDir;

use install::{InstallError, Installation};
use network::{Cancellation, SoupTransport};

#[derive(Clone)]
pub struct Candidate {
    installation: Installation,
    pub asset: UpdateAsset,
}

pub struct Staged {
    pub candidate: Candidate,
    artifact: TempDir,
    automatic: bool,
}

pub enum Status {
    Idle,
    Current,
    Available(Candidate),
    Staged(Staged),
    Cancelled,
    Failed(String),
}

#[derive(Clone, Copy)]
pub enum Phase {
    Checking,
    Downloading,
    Cancelling,
}

enum Event {
    Downloading,
    Complete(Box<Result<Option<Status>, String>>),
}

struct Job {
    events: Receiver<Event>,
    cancellation: Arc<Cancellation>,
    worker: JoinHandle<()>,
    phase: Phase,
    automatic: bool,
}

struct PreferenceChange {
    result: Receiver<Result<(), String>>,
    worker: JoinHandle<()>,
    previous: bool,
}

pub struct Updates {
    version: Option<Version>,
    preferences: Option<Preferences>,
    automatic: bool,
    preference_change: Option<PreferenceChange>,
    job: Option<Job>,
    status: Status,
    restart: bool,
}

impl Updates {
    pub fn new() -> Self {
        let version = option_env!("QUICKIE_RELEASE_VERSION")
            .and_then(|version| Version::parse(version).ok())
            .filter(|version| version.pre.is_empty() && version.build.is_empty());
        let preferences = preferences_path().map(Preferences::new);
        let automatic = preferences.as_ref().is_some_and(Preferences::automatic);
        Self {
            version,
            preferences,
            automatic,
            preference_change: None,
            job: None,
            status: Status::Idle,
            restart: false,
        }
    }

    pub fn version(&self) -> Option<&Version> {
        self.version.as_ref()
    }

    pub fn automatic(&self) -> bool {
        self.automatic
    }

    pub fn can_set_automatic(&self) -> bool {
        self.version.is_some() && self.preferences.is_some() && self.preference_change.is_none()
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn phase(&self) -> Option<Phase> {
        self.job.as_ref().map(|job| job.phase)
    }

    pub fn is_busy(&self) -> bool {
        self.job.is_some() || self.preference_change.is_some()
    }

    pub fn check(&mut self, automatic: bool) {
        let Some(version) = self.version.clone() else {
            return;
        };
        if self.job.is_some()
            || matches!(self.status, Status::Staged(_))
            || (automatic && (!self.automatic || self.preference_change.is_some()))
        {
            return;
        }
        let preferences = self.preferences.clone();
        self.start(automatic, Phase::Checking, move |cancellation, events| {
            if automatic {
                let Some(preferences) = &preferences else {
                    return Ok(None);
                };
                if !preferences
                    .claim_scheduled_check()
                    .map_err(|_| "could not record the scheduled update check")?
                {
                    return Ok(None);
                }
            } else if let Some(preferences) = &preferences {
                let _ = preferences.record_check();
            }

            let installation = Installation::detect(&version.to_string())
                .ok_or("this installation does not support automatic replacement")?;
            let transport = SoupTransport(cancellation);
            let Some(asset) = fetch_latest(&transport, &version, installation.kind())? else {
                return Ok(Some(Status::Current));
            };
            let candidate = Candidate {
                installation,
                asset,
            };
            if automatic {
                let _ = events.send(Event::Downloading);
                stage(&transport, candidate, true).map(Some)
            } else {
                Ok(Some(Status::Available(candidate)))
            }
        });
    }

    pub fn prepare(&mut self) {
        let Status::Available(candidate) = &self.status else {
            return;
        };
        if self.job.is_some() {
            return;
        }
        let candidate = candidate.clone();
        self.start(false, Phase::Downloading, move |cancellation, _| {
            stage(&SoupTransport(cancellation), candidate, false).map(Some)
        });
    }

    pub fn request_restart(&mut self) -> bool {
        self.restart = self.job.is_none() && matches!(self.status, Status::Staged(_));
        self.restart
    }

    pub fn cancel(&mut self) {
        if let Some(job) = &mut self.job {
            job.cancellation.cancel();
            job.phase = Phase::Cancelling;
        }
    }

    pub fn set_automatic(&mut self, enabled: bool) {
        if !self.can_set_automatic() || self.automatic == enabled {
            return;
        }
        let Some(preferences) = self.preferences.clone() else {
            return;
        };
        let (reply, result) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("quickie-update-preferences".into())
            .spawn(move || {
                let outcome = preferences
                    .set_automatic(enabled)
                    .map_err(|_| "could not save the automatic update setting".into());
                let _ = reply.send(outcome);
            });
        let Ok(worker) = worker else {
            self.status = Status::Failed("could not save the automatic update setting".into());
            return;
        };
        self.preference_change = Some(PreferenceChange {
            result,
            worker,
            previous: self.automatic,
        });
        self.automatic = enabled;
        if !enabled {
            if self.job.as_ref().is_some_and(|job| job.automatic) {
                self.cancel();
            }
            if matches!(&self.status, Status::Staged(staged) if staged.automatic) {
                self.status = Status::Cancelled;
            }
        }
    }

    pub fn poll(&mut self) {
        self.poll_job();
        self.poll_preference_change();
    }

    /// Apply a prepared release only after the browser's GTK event loop has closed.
    pub fn finish(mut self) -> Result<(), String> {
        self.poll_job();
        if let Some(job) = self.job.take() {
            job.cancellation.cancel();
            let _ = job.worker.join();
        }
        if let Some(change) = self.preference_change.take() {
            let _ = change.worker.join();
        }
        let Status::Staged(staged) = self.status else {
            return Ok(());
        };
        let automatic_allowed = self.automatic
            && self
                .preferences
                .is_some_and(|preferences| preferences.automatic());
        if staged.automatic && !self.restart && !automatic_allowed {
            return Ok(());
        }
        let Staged {
            candidate,
            artifact,
            ..
        } = staged;
        let path = artifact.path().join(candidate.asset.name());
        match candidate.installation.install(&path, &candidate.asset) {
            Ok(()) | Err(InstallError::Cancelled) => {}
            Err(error) => return Err(error.to_string()),
        }
        if self.restart {
            candidate
                .installation
                .relaunch()
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn start(
        &mut self,
        automatic: bool,
        phase: Phase,
        operation: impl FnOnce(&Cancellation, &Sender<Event>) -> Result<Option<Status>, String>
        + Send
        + 'static,
    ) {
        let cancellation = match Cancellation::new() {
            Ok(cancellation) => Arc::new(cancellation),
            Err(error) => {
                self.status = Status::Failed(error);
                return;
            }
        };
        let cancellable = Arc::clone(&cancellation);
        let (sender, events) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("quickie-updater".into())
            .spawn(move || {
                let result = operation(&cancellable, &sender);
                let _ = sender.send(Event::Complete(Box::new(result)));
            });
        match worker {
            Ok(worker) => {
                self.job = Some(Job {
                    events,
                    cancellation,
                    worker,
                    phase,
                    automatic,
                });
            }
            Err(_) => self.status = Status::Failed("could not start the update worker".into()),
        }
    }

    fn poll_job(&mut self) {
        while let Some(job) = &mut self.job {
            match job.events.try_recv() {
                Ok(Event::Downloading) => {
                    if !job.cancellation.is_cancelled() {
                        job.phase = Phase::Downloading;
                    }
                }
                Ok(Event::Complete(outcome)) => {
                    let cancelled = job.cancellation.is_cancelled();
                    self.job = None;
                    if cancelled {
                        self.status = Status::Cancelled;
                    } else {
                        match *outcome {
                            Ok(Some(status)) => self.status = status,
                            Ok(None) => {}
                            Err(error) => self.status = Status::Failed(error),
                        }
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    self.status = Status::Failed("the update worker stopped unexpectedly".into());
                }
            }
        }
    }

    fn poll_preference_change(&mut self) {
        let Some(change) = &self.preference_change else {
            return;
        };
        let outcome = match change.result.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("could not save the automatic update setting".into())
            }
        };
        let previous = change.previous;
        self.preference_change = None;
        if let Err(error) = outcome {
            self.automatic = previous;
            self.status = Status::Failed(error);
        } else if self.automatic {
            self.check(true);
        }
    }
}

fn stage(
    transport: &SoupTransport<'_>,
    candidate: Candidate,
    automatic: bool,
) -> Result<Status, String> {
    let artifact = download(transport, &candidate.asset)?;
    Ok(Status::Staged(Staged {
        candidate,
        artifact,
        automatic,
    }))
}

fn preferences_path() -> Option<PathBuf> {
    let root = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })?;
    Some(root.join("quickie").join("updates"))
}

pub fn installed_notes_path(prefix: &Path) -> PathBuf {
    prefix.join("share/doc/quickie/ReleaseNotes.md")
}
