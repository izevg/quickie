//! Behavioral tests for shared MCP diagnostic state.

mod bounds;
mod events;
mod lifecycle;

use quickie_domain_browsing::{Profile, TabId};
use url::Url;

use super::DocumentBinding;

pub(super) fn main_tab(serial: u64) -> TabId {
    TabId::new(Profile::Main, serial)
}

pub(super) fn agent_tab(serial: u64) -> TabId {
    TabId::new(Profile::Agent, serial)
}

pub(super) fn binding(tab: TabId, raw_url: &str, epoch: u64) -> DocumentBinding {
    let url = Url::parse(raw_url).expect("test URL must parse");
    DocumentBinding {
        tab,
        origin: url.origin(),
        epoch,
    }
}
