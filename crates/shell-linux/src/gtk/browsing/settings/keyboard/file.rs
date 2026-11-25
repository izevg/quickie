//! Serialize native shortcut file actions through the shared application use cases.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use quickie_application::{LoadedKeymap, RejectedEntry, load_keymap, rebind, reset_binding};
use quickie_domain_commands::{Catalog, Keymap, Platform};
use quickie_store::KeybindingsFile;

use super::Action;

const PLATFORM: Platform = Platform::Linux;

pub(in crate::gtk) struct State {
    pub keymap: Keymap,
    pub rejected: Vec<RejectedEntry>,
    pub unread: Option<String>,
}

impl State {
    pub(in crate::gtk) fn defaults(unread: Option<String>) -> Self {
        Self {
            keymap: Keymap::defaults(PLATFORM),
            rejected: Vec::new(),
            unread,
        }
    }

    pub(in crate::gtk) fn problems(&self) -> Vec<String> {
        self.unread
            .iter()
            .map(|error| {
                format!("The defaults apply because keybindings.json could not be read. {error}")
            })
            .chain(self.rejected.iter().map(|entry| {
                format!(
                    "Entry {} ({} → {}): {}",
                    entry.position + 1,
                    entry.entry.key,
                    entry.entry.command,
                    entry.reason,
                )
            }))
            .collect()
    }

    fn load(file: &KeybindingsFile, catalog: &Catalog) -> Self {
        match load_keymap(file, catalog, PLATFORM) {
            Ok(loaded) => Self::from(loaded),
            Err(error) => Self::defaults(Some(error.to_string())),
        }
    }
}

impl From<LoadedKeymap> for State {
    fn from(loaded: LoadedKeymap) -> Self {
        Self {
            keymap: loaded.keymap,
            rejected: loaded.rejected,
            unread: None,
        }
    }
}

pub(in crate::gtk) enum Outcome {
    Loaded(State),
    Open(PathBuf),
}

type Reply = Result<Outcome, String>;

pub(in crate::gtk) struct Worker {
    path: PathBuf,
    commands: Option<Sender<Action>>,
    replies: Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(in crate::gtk) fn start(path: PathBuf, catalog: Catalog) -> Result<(Self, State), String> {
        let file = KeybindingsFile::new(path.clone());
        let (requests, actions) = mpsc::channel();
        let (finish, replies) = mpsc::channel();
        let (startup, state) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("quickie-keybindings".into())
            .spawn(move || {
                if startup.send(State::load(&file, &catalog)).is_err() {
                    return;
                }
                for action in actions {
                    if finish.send(execute(&file, &catalog, action)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        let state = state.recv().map_err(|_| stopped())?;
        Ok((
            Self {
                path,
                commands: Some(requests),
                replies,
                thread: Some(thread),
            },
            state,
        ))
    }

    pub(in crate::gtk) fn path(&self) -> &Path {
        &self.path
    }

    pub(in crate::gtk) fn submit(&self, action: Action) -> Result<(), String> {
        self.commands
            .as_ref()
            .ok_or_else(stopped)?
            .send(action)
            .map_err(|_| stopped())
    }

    pub(in crate::gtk) fn outcome(&self) -> Option<Reply> {
        match self.replies.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(stopped())),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        drop(self.commands.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn execute(file: &KeybindingsFile, catalog: &Catalog, action: Action) -> Reply {
    match action {
        Action::Open => {
            ensure_directory(file.path())?;
            file.create_if_missing()
                .map_err(|error| error.to_string())?;
            Ok(Outcome::Open(file.path().to_path_buf()))
        }
        Action::Reload => Ok(Outcome::Loaded(State::load(file, catalog))),
        Action::Bind(command, chord) => {
            ensure_directory(file.path())?;
            let loaded = rebind(file, catalog, PLATFORM, &command, chord)
                .map_err(|error| error.to_string())?;
            Ok(Outcome::Loaded(State::from(loaded)))
        }
        Action::Reset(command) => {
            let loaded = reset_binding(file, catalog, PLATFORM, &command)
                .map_err(|error| error.to_string())?;
            Ok(Outcome::Loaded(State::from(loaded)))
        }
    }
}

fn ensure_directory(file: &Path) -> Result<(), String> {
    let directory = file
        .parent()
        .ok_or("The shortcut file has no parent directory.")?;
    fs::create_dir_all(directory).map_err(|error| error.to_string())
}

fn stopped() -> String {
    "The keybindings worker stopped.".into()
}

pub(in crate::gtk) fn profile_path() -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })?;
    Some(config.join("quickie").join("keybindings.json"))
}
