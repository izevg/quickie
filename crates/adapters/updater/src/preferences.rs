//! The app update opt-in and cadence, outside the browser's session database.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DAY: u64 = 86_400;

#[derive(Clone, Debug)]
pub struct Preferences {
    root: PathBuf,
}

#[derive(Default)]
struct State {
    automatic: bool,
    checked: Option<u64>,
}

impl State {
    fn due(&self, now: u64) -> bool {
        self.automatic
            && self
                .checked
                .is_none_or(|last| now.saturating_sub(last) >= DAY)
    }
}

impl Preferences {
    /// Creates a handle without writing to disk.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Whether automatic updating was explicitly enabled.
    #[must_use]
    pub fn automatic(&self) -> bool {
        self.read().automatic
    }

    /// Whether an automatic check may run now.
    #[must_use]
    pub fn is_due(&self) -> bool {
        self.read().due(now())
    }

    /// Saves the setting atomically.
    ///
    /// # Errors
    /// Returns an error if the update settings could not be written.
    pub fn set_automatic(&self, enabled: bool) -> io::Result<()> {
        self.edit(|state| state.automatic = enabled)
    }

    /// Records the time a manual release check was scheduled.
    ///
    /// # Errors
    /// Returns an error if the timestamp could not be written.
    pub fn record_check(&self) -> io::Result<()> {
        self.edit(|state| state.checked = Some(now()))
    }

    /// Claims the next due automatic check across browser processes.
    ///
    /// # Errors
    /// Returns an error if the claim could not be written durably.
    pub fn claim_scheduled_check(&self) -> io::Result<bool> {
        if !self.is_due() {
            return Ok(false);
        }
        self.edit(|state| {
            let now = now();
            if state.due(now) {
                state.checked = Some(now);
                true
            } else {
                false
            }
        })
    }

    fn read(&self) -> State {
        let Ok(file) = File::open(self.root.join("preferences")) else {
            return State::default();
        };
        let mut bytes = String::new();
        if file.take(128).read_to_string(&mut bytes).is_err() {
            return State::default();
        }
        let mut lines = bytes.lines();
        let Some(setting @ ("on" | "off")) = lines.next() else {
            return State::default();
        };
        State {
            automatic: setting == "on",
            checked: lines.next().and_then(|value| value.parse().ok()),
        }
    }

    fn edit<T>(&self, change: impl FnOnce(&mut State) -> T) -> io::Result<T> {
        fs::create_dir_all(&self.root)?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("preferences.lock"))?;
        lock.lock()?;
        let mut state = self.read();
        let changed = change(&mut state);
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        writeln!(file, "{}", if state.automatic { "on" } else { "off" })?;
        if let Some(checked) = state.checked {
            writeln!(file, "{checked}")?;
        }
        file.as_file().sync_all()?;
        file.persist(self.root.join("preferences"))?;
        File::open(&self.root)?.sync_all()?;
        Ok(changed)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests;
