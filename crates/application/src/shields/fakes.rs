//! In-memory fakes of the shields' ports.

use std::collections::BTreeMap;

use quickie_domain_shields::{FilterListId, FilterListSetSnapshot, SitePoliciesSnapshot};
use url::Url;

use crate::{
    BlockerError, BlockerRules, ContentBlockerPort, Ended, HttpError, HttpEvent, HttpPort,
    HttpRequest, ShieldsRepo, ShieldsSnapshot, StopSignal, StoreError,
};

/// Keeps what was saved last, like a store would; counts saves, and fails loads, or saves of
/// one kind, on request.
#[derive(Debug, Default)]
pub struct FakeShieldsRepo {
    pub saved: ShieldsSnapshot,
    pub saves: usize,
    pub load_fails: bool,
    /// Saves of these kinds fail: `"sites"`, `"lists"`, `"body"`.
    pub failing: Vec<&'static str>,
}

impl FakeShieldsRepo {
    fn save(&mut self, kind: &'static str) -> Result<(), StoreError> {
        if self.failing.contains(&kind) {
            return Err(StoreError(kind.to_owned()));
        }
        self.saves += 1;
        Ok(())
    }
}

impl ShieldsRepo for FakeShieldsRepo {
    fn load_shields(&self) -> Result<ShieldsSnapshot, StoreError> {
        if self.load_fails {
            Err(StoreError("load".to_owned()))
        } else {
            Ok(self.saved.clone())
        }
    }

    fn save_sites(&mut self, sites: &SitePoliciesSnapshot) -> Result<(), StoreError> {
        self.save("sites")?;
        self.saved.sites = sites.clone();
        Ok(())
    }

    fn save_lists(&mut self, lists: &FilterListSetSnapshot) -> Result<(), StoreError> {
        self.save("lists")?;
        self.saved.lists = lists.clone();
        Ok(())
    }

    fn save_list_body(&mut self, list: FilterListId, body: &[u8]) -> Result<(), StoreError> {
        self.save("body")?;
        self.saved.bodies.retain(|(saved, _)| *saved != list);
        self.saved.bodies.push((list, body.to_vec()));
        Ok(())
    }
}

/// Records every set of rules it was asked to install; refuses them on request.
#[derive(Debug, Default)]
pub struct FakeBlocker {
    pub installed: Vec<BlockerRules>,
    pub fails: bool,
}

impl ContentBlockerPort for FakeBlocker {
    fn install(&mut self, rules: &BlockerRules) -> Result<(), BlockerError> {
        self.installed.push(rules.clone());
        if self.fails {
            Err(BlockerError("refused".to_owned()))
        } else {
            Ok(())
        }
    }
}

/// Answers each URL from a table; a URL not in it fails.
#[derive(Debug, Default)]
pub struct FakeHttp {
    pub bodies: BTreeMap<String, Vec<u8>>,
}

impl HttpPort for FakeHttp {
    fn get(&self, url: &Url) -> Result<Vec<u8>, HttpError> {
        self.bodies
            .get(url.as_str())
            .cloned()
            .ok_or_else(|| HttpError(format!("404 {url}")))
    }

    /// Filter lists are fetched whole; nothing streams.
    fn stream(
        &self,
        request: &HttpRequest,
        _stop: &StopSignal,
        _on_event: &mut dyn FnMut(HttpEvent<'_>),
    ) -> Result<Ended, HttpError> {
        Err(HttpError(format!("unexpected stream of {}", request.url)))
    }
}
