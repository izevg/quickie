//! Saved appearance on the profile's SQLite store, off the GTK thread.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use quickie_application::AppearanceSettings;
use quickie_domain_settings::PageLayout;
use quickie_store::SqliteStore;

use super::data_home;

type Saved = Result<PageLayout, String>;

pub(super) struct Writer {
    requests: Option<Sender<PageLayout>>,
    replies: Receiver<Saved>,
    worker: Option<JoinHandle<()>>,
}

impl Writer {
    pub(super) fn start(isolated: bool) -> Result<(Self, PageLayout), String> {
        let store = if isolated {
            SqliteStore::in_memory()
        } else {
            SqliteStore::open(&data_home()?.join("shell.sqlite"))
        }
        .map_err(|error| error.to_string())?;
        let mut appearance = AppearanceSettings::start(store).map_err(|error| error.to_string())?;
        let layout = appearance.appearance().page();
        let (requests, changes) = mpsc::channel();
        let (finish, replies) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("quickie-appearance".into())
            .spawn(move || {
                for layout in changes {
                    let result = appearance
                        .set_page(layout)
                        .map(|_| layout)
                        .map_err(|error| error.to_string());
                    if finish.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok((
            Self {
                requests: Some(requests),
                replies,
                worker: Some(worker),
            },
            layout,
        ))
    }

    pub(super) fn save(&self, layout: PageLayout) -> Result<(), String> {
        self.requests
            .as_ref()
            .ok_or_else(stopped)?
            .send(layout)
            .map_err(|_| stopped())
    }

    pub(super) fn outcome(&self) -> Option<Saved> {
        match self.replies.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(stopped())),
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        drop(self.requests.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn stopped() -> String {
    "the appearance writer has stopped".into()
}
