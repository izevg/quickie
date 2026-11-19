//! In-memory fakes of the agent's ports.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use quickie_domain_agents::{
    ApiKey, BaseUrl, EndpointSnapshot, KeyRef, LocalServer, ModelId, Prompt,
};

use crate::fakes::FakeClock;
use crate::{
    AgentRepo, ChatEvent, ChatRequest, Ended, LlmError, LlmPort, SecretError, SecretStore,
    StopSignal, StoreError,
};

/// Keeps the endpoint saved last; counts saves, and fails loads or saves on request.
#[derive(Debug, Default)]
pub struct FakeAgentRepo {
    pub saved: Option<EndpointSnapshot>,
    pub saves: usize,
    pub load_fails: bool,
    pub save_fails: bool,
}

impl AgentRepo for FakeAgentRepo {
    fn load_endpoint(&self) -> Result<Option<EndpointSnapshot>, StoreError> {
        if self.load_fails {
            Err(StoreError("load".to_owned()))
        } else {
            Ok(self.saved.clone())
        }
    }

    fn save_endpoint(&mut self, endpoint: Option<&EndpointSnapshot>) -> Result<(), StoreError> {
        if self.save_fails {
            return Err(StoreError("save".to_owned()));
        }
        self.saved = endpoint.cloned();
        self.saves += 1;
        Ok(())
    }
}

/// A Keychain in memory, by key reference; each kind of call fails on request.
#[derive(Debug, Default)]
pub struct FakeSecrets {
    pub keys: RefCell<BTreeMap<String, String>>,
    pub reads: Cell<usize>,
    pub read_fails: bool,
    pub write_fails: bool,
    pub delete_fails: bool,
}

impl FakeSecrets {
    /// A Keychain holding `key` under `reference`.
    pub fn holding(reference: &KeyRef, key: &str) -> Self {
        let secrets = Self::default();
        secrets
            .keys
            .borrow_mut()
            .insert(reference.as_str().to_owned(), key.to_owned());
        secrets
    }

    /// The key kept under `reference`, as text.
    pub fn get(&self, reference: &KeyRef) -> Option<String> {
        self.keys.borrow().get(reference.as_str()).cloned()
    }
}

impl SecretStore for FakeSecrets {
    fn read(&self, key: &KeyRef) -> Result<Option<ApiKey>, SecretError> {
        self.reads.set(self.reads.get() + 1);
        if self.read_fails {
            return Err(SecretError("read".to_owned()));
        }
        Ok(self
            .get(key)
            .map(|text| ApiKey::try_from(text.as_str()).unwrap()))
    }

    fn write(&self, key: &KeyRef, value: &ApiKey) -> Result<(), SecretError> {
        if self.write_fails {
            return Err(SecretError("write".to_owned()));
        }
        self.keys
            .borrow_mut()
            .insert(key.as_str().to_owned(), value.reveal().to_owned());
        Ok(())
    }

    fn delete(&self, key: &KeyRef) -> Result<(), SecretError> {
        if self.delete_fails {
            return Err(SecretError("delete".to_owned()));
        }
        self.keys.borrow_mut().remove(key.as_str());
        Ok(())
    }
}

/// What a chat request carried, as the fake saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asked {
    pub model: String,
    pub key: Option<String>,
    pub prompt: Prompt,
}

/// An endpoint that plays a script: at each `(millis, event)` it sets the clock and delivers
/// the event, then ends with `ended`, or with [`Ended::Stopped`] when the stop signal was
/// given. Records every request.
#[derive(Debug)]
pub struct FakeLlm {
    pub clock: FakeClock,
    pub script: Vec<(u64, ChatEvent)>,
    pub ended: Result<Ended, LlmError>,
    pub asked: RefCell<Vec<Asked>>,
    pub models: Result<Vec<ModelId>, LlmError>,
    pub models_asked: RefCell<Vec<(BaseUrl, Option<String>)>>,
    pub servers: Vec<(LocalServer, Vec<ModelId>)>,
}

impl FakeLlm {
    pub fn new(clock: &FakeClock) -> Self {
        Self {
            clock: clock.clone(),
            script: Vec::new(),
            ended: Ok(Ended::Complete),
            asked: RefCell::default(),
            models: Ok(Vec::new()),
            models_asked: RefCell::default(),
            servers: Vec::new(),
        }
    }
}

impl LlmPort for FakeLlm {
    fn chat(
        &self,
        request: &ChatRequest<'_>,
        stop: &StopSignal,
        on_event: &mut dyn FnMut(ChatEvent),
    ) -> Result<Ended, LlmError> {
        self.asked.borrow_mut().push(Asked {
            model: request.endpoint.model().to_string(),
            key: request.key.map(|key| key.reveal().to_owned()),
            prompt: request.prompt.clone(),
        });
        for (millis, event) in &self.script {
            self.clock.0.set(*millis);
            on_event(event.clone());
        }
        if stop.is_stopped() {
            return Ok(Ended::Stopped);
        }
        self.ended.clone()
    }

    fn models(&self, base: &BaseUrl, key: Option<&ApiKey>) -> Result<Vec<ModelId>, LlmError> {
        self.models_asked
            .borrow_mut()
            .push((base.clone(), key.map(|key| key.reveal().to_owned())));
        self.models.clone()
    }

    fn probe(&self, server: LocalServer) -> Result<Vec<ModelId>, LlmError> {
        self.servers
            .iter()
            .find(|(found, _)| *found == server)
            .map(|(_, models)| models.clone())
            .ok_or_else(|| LlmError::Malformed(format!("nothing at {}", server.probe_url())))
    }
}
