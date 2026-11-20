//! In-memory fakes of the ports, for the use case tests.

use std::cell::Cell;
use std::rc::Rc;

use quickie_domain_browsing::{Jar, JarId, SessionSnapshot, TabId, Timestamp, WindowId};
use quickie_domain_library::{
    BookmarksSnapshot, DownloadsSnapshot, HistorySnapshot, SearchShortcutsSnapshot,
};
use url::Url;

use crate::{
    Clock, EngineError, EnginePort, KeybindingText, KeymapRepo, LibraryRepo, LibrarySnapshot,
    Navigation, SessionRepo, StoreError,
};

/// Keeps what was saved last, like a store would; counts saves per part, and fails loads or
/// saves on request.
#[derive(Debug, Default)]
pub struct FakeLibraryRepo {
    pub saved: LibrarySnapshot,
    pub saves: usize,
    pub load_fails: bool,
    pub save_fails: bool,
}

impl FakeLibraryRepo {
    fn save(&mut self) -> Result<(), StoreError> {
        if self.save_fails {
            return Err(StoreError("save".to_owned()));
        }
        self.saves += 1;
        Ok(())
    }
}

impl LibraryRepo for FakeLibraryRepo {
    fn load_library(&self) -> Result<LibrarySnapshot, StoreError> {
        if self.load_fails {
            Err(StoreError("load".to_owned()))
        } else {
            Ok(self.saved.clone())
        }
    }

    fn save_history(&mut self, history: &HistorySnapshot) -> Result<(), StoreError> {
        self.save()?;
        self.saved.history = Some(history.clone());
        Ok(())
    }

    fn save_bookmarks(&mut self, bookmarks: &BookmarksSnapshot) -> Result<(), StoreError> {
        self.save()?;
        self.saved.bookmarks = Some(bookmarks.clone());
        Ok(())
    }

    fn save_shortcuts(&mut self, shortcuts: &SearchShortcutsSnapshot) -> Result<(), StoreError> {
        self.save()?;
        self.saved.shortcuts = Some(shortcuts.clone());
        Ok(())
    }

    fn save_downloads(&mut self, downloads: &DownloadsSnapshot) -> Result<(), StoreError> {
        self.save()?;
        self.saved.downloads = Some(downloads.clone());
        Ok(())
    }
}

/// The user's `keybindings.json` entries, or a failing read.
#[derive(Debug, Default)]
pub struct FakeKeymapRepo {
    pub entries: Vec<KeybindingText>,
    pub load_fails: bool,
}

impl KeymapRepo for FakeKeymapRepo {
    fn load_keybindings(&self) -> Result<Vec<KeybindingText>, StoreError> {
        if self.load_fails {
            Err(StoreError("load".to_owned()))
        } else {
            Ok(self.entries.clone())
        }
    }
}

/// One call the engine received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Adopt(u64, TabId),
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
    type Page = u64;

    fn adopt(&mut self, page: u64, tab: TabId) -> Result<(), EngineError> {
        self.record("adopt", Call::Adopt(page, tab))
    }

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

    fn save(&mut self, snapshot: SessionSnapshot) -> Result<(), StoreError> {
        if self.save_fails.get() {
            return Err(StoreError("save".to_owned()));
        }
        self.saved = Some(snapshot);
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
