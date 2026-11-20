//! In-memory ports for the extension use cases' tests.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use quickie_domain_browsing::TabId;
use quickie_domain_settings::{ExtensionId, Grants, InstalledExtensionsSnapshot};
use url::Url;

use crate::{
    Access, Ended, EngineError, Expected, ExtensionEvent, ExtensionHost, ExtensionPackages,
    ExtensionsRepo, Flow, HttpError, HttpEvent, HttpPort, HttpRequest, PackageError, PackageSource,
    ReplyToken, StopSignal, StoreError, Unpacked, WindowTabs,
};

/// A store that keeps the last save, and fails when told to.
#[derive(Debug, Default)]
pub struct FakeExtensionsRepo {
    pub saved: InstalledExtensionsSnapshot,
    pub saves: usize,
    pub fail: bool,
    pub fail_load: bool,
}

impl ExtensionsRepo for FakeExtensionsRepo {
    fn load_extensions(&self) -> Result<InstalledExtensionsSnapshot, StoreError> {
        if self.fail_load {
            return Err(StoreError("unreadable".to_owned()));
        }
        Ok(self.saved.clone())
    }

    fn save_extensions(
        &mut self,
        extensions: InstalledExtensionsSnapshot,
    ) -> Result<(), StoreError> {
        if self.fail {
            return Err(StoreError("disk full".to_owned()));
        }
        self.saved = extensions;
        self.saves += 1;
        Ok(())
    }
}

/// What the use case asked of the host, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Read(ExtensionId, String),
    Start(ExtensionId, Access),
    Stop(ExtensionId),
    Discard(ExtensionId),
    Remove(ExtensionId),
    Action(ExtensionId, Option<TabId>),
    Command(ExtensionId, String),
    Answer(ReplyToken, Grants),
}

/// A host that records calls, shared with the test that hands it over.
#[derive(Clone, Debug, Default)]
pub struct FakeHost {
    pub calls: Rc<RefCell<Vec<Call>>>,
    pub refuse: bool,
}

impl FakeHost {
    pub fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    pub fn clear(&self) {
        self.calls.borrow_mut().clear();
    }

    fn record(&self, call: Call) -> Result<(), EngineError> {
        self.calls.borrow_mut().push(call);
        if self.refuse {
            return Err(EngineError("no extensions here".to_owned()));
        }
        Ok(())
    }
}

impl ExtensionHost for FakeHost {
    type Popup = ();

    fn set_waker(&mut self, _wake: Box<dyn Fn()>) {}

    fn popup(&self) -> Option<()> {
        None
    }

    fn close_popup(&self) {}

    fn read(&mut self, id: &ExtensionId, location: &str) -> Result<(), EngineError> {
        self.record(Call::Read(id.clone(), location.to_owned()))
    }

    fn start(&mut self, id: &ExtensionId, access: &Access) -> Result<(), EngineError> {
        self.record(Call::Start(id.clone(), access.clone()))
    }

    fn stop(&mut self, id: &ExtensionId) {
        self.calls.borrow_mut().push(Call::Stop(id.clone()));
    }

    fn discard(&mut self, id: &ExtensionId) {
        self.calls.borrow_mut().push(Call::Discard(id.clone()));
    }

    fn remove(&mut self, id: &ExtensionId) {
        self.calls.borrow_mut().push(Call::Remove(id.clone()));
    }

    fn perform_action(&mut self, id: &ExtensionId, tab: Option<TabId>) -> Result<(), EngineError> {
        self.record(Call::Action(id.clone(), tab))
    }

    fn perform_command(&mut self, id: &ExtensionId, command: &str) -> Result<(), EngineError> {
        self.record(Call::Command(id.clone(), command.to_owned()))
    }

    fn answer(&mut self, token: ReplyToken, granted: &Grants) {
        self.calls
            .borrow_mut()
            .push(Call::Answer(token, granted.clone()));
    }

    fn opened(&mut self, _token: ReplyToken, _tab: Option<TabId>) {}

    fn arrange(&mut self, _windows: &[WindowTabs]) {}

    fn drain(&mut self, _into: &mut Vec<ExtensionEvent>) {}
}

/// The network as a table of answers; anything else fails.
#[derive(Debug, Default)]
pub struct FakeHttp {
    pub answers: HashMap<Url, Vec<u8>>,
    pub asked: RefCell<Vec<Url>>,
}

impl HttpPort for FakeHttp {
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError> {
        self.asked.borrow_mut().push(url.clone());
        self.answers
            .get(url)
            .cloned()
            .ok_or_else(|| HttpError(format!("no answer at {url}")))
    }

    fn stream(
        &self,
        _request: &HttpRequest,
        _stop: &StopSignal,
        _on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    ) -> Result<Ended, HttpError> {
        Err(HttpError("not streamed here".to_owned()))
    }
}

/// Packages that unpack to a set place, recording what they were given.
#[derive(Debug, Default)]
pub struct FakePackages {
    pub given: RefCell<Vec<(PackageSource, Option<Expected>)>>,
    pub refuse: bool,
}

impl ExtensionPackages for FakePackages {
    fn unpack(
        &self,
        source: &PackageSource,
        expected: Option<&Expected>,
    ) -> Result<Unpacked, PackageError> {
        self.given
            .borrow_mut()
            .push((source.clone(), expected.cloned()));
        if self.refuse {
            return Err(PackageError("bad signature".to_owned()));
        }
        let id = expected.map_or_else(
            || ExtensionId::from_digest(&[0; 16]),
            |expected| expected.id.clone(),
        );
        Ok(Unpacked {
            location: format!("/kept/{id}.zip"),
            id,
        })
    }

    fn prune(&self, _keep: &[String]) -> Result<(), PackageError> {
        Ok(())
    }
}
