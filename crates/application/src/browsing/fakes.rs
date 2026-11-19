//! In-memory fakes of the ports, for the use case tests.

use std::cell::Cell;
use std::rc::Rc;

use quickie_domain_browsing::{Jar, JarId, SessionSnapshot, TabId, Timestamp, WindowId};
use url::Url;

use crate::{Clock, EngineError, EnginePort, Navigation, SessionRepo, StoreError};

/// One call the engine received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Create(TabId, Url, Jar),
    Show(WindowId, TabId),
    Navigate(TabId, Navigation),
    Close(TabId),
    DestroyJar(JarId),
}

/// Records calls; fails every call whose kind is in `failing`.
#[derive(Debug, Default)]
pub struct FakeEngine {
    pub calls: Vec<Call>,
    pub failing: Vec<&'static str>,
}

impl FakeEngine {
    fn record(&mut self, kind: &'static str, call: Call) -> Result<(), EngineError> {
        self.calls.push(call);
        if self.failing.contains(&kind) {
            Err(EngineError(kind.to_owned()))
        } else {
            Ok(())
        }
    }
}

impl EnginePort for FakeEngine {
    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError> {
        self.record("create", Call::Create(tab, url.clone(), jar))
    }

    fn show(&mut self, window: WindowId, tab: TabId) -> Result<(), EngineError> {
        self.record("show", Call::Show(window, tab))
    }

    fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), EngineError> {
        self.record("navigate", Call::Navigate(tab, navigation.clone()))
    }

    fn close(&mut self, tab: TabId) -> Result<(), EngineError> {
        self.record("close", Call::Close(tab))
    }

    fn destroy_jar(&mut self, jar: JarId) -> Result<(), EngineError> {
        self.record("destroy_jar", Call::DestroyJar(jar))
    }
}

/// Keeps the last saved snapshot; the test keeps a handle to switch failures on.
#[derive(Debug, Default)]
pub struct FakeRepo {
    pub saved: Option<SessionSnapshot>,
    pub saves: usize,
    pub load_fails: bool,
    pub save_fails: Rc<Cell<bool>>,
}

impl SessionRepo for FakeRepo {
    fn load(&self) -> Result<Option<SessionSnapshot>, StoreError> {
        if self.load_fails {
            Err(StoreError("load".to_owned()))
        } else {
            Ok(self.saved.clone())
        }
    }

    fn save(&mut self, snapshot: &SessionSnapshot) -> Result<(), StoreError> {
        if self.save_fails.get() {
            return Err(StoreError("save".to_owned()));
        }
        self.saved = Some(snapshot.clone());
        self.saves += 1;
        Ok(())
    }
}

/// A clock the test sets.
#[derive(Debug, Default, Clone)]
pub struct FakeClock(pub Rc<Cell<u64>>);

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(self.0.get())
    }
}
