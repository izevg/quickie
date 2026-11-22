//! Candidate links from a single evaluation of the current main document.

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use quickie_application::{AgentLink, AgentLinks, EngineError};
use serde_json::Value;
use url::Url;

pub(crate) const COLLECT_SCRIPT: &str = include_str!("links.js");

const MAX_REPLY_BYTES: usize = 1024 * 1024;
const MAX_URL_BYTES: usize = 2048;
const MAX_LINKS: usize = 100;
const MAX_LABEL_CHARS: usize = 160;

pub(crate) fn parse(answer: &str) -> Result<AgentLinks, EngineError> {
    let invalid = || EngineError("Could not read links from this document".to_owned());
    if answer.len() > MAX_REPLY_BYTES {
        return Err(invalid());
    }
    let reply: Value = serde_json::from_str(answer).map_err(|_| invalid())?;
    let document = reply
        .get("document")
        .and_then(web_url)
        .ok_or_else(invalid)?;
    let candidates = reply
        .get("links")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let mut seen = HashSet::new();
    let links = candidates
        .iter()
        .take(MAX_LINKS)
        .filter_map(|value| candidate(value, &document))
        .filter(|link| seen.insert(link.url.clone()))
        .collect();
    Ok(AgentLinks { document, links })
}

fn candidate(value: &Value, document: &Url) -> Option<AgentLink> {
    let url = value.get("url").and_then(web_url)?;
    if &url == document || url.origin() != document.origin() {
        return None;
    }
    let label: String = value
        .get("label")?
        .as_str()?
        .trim()
        .chars()
        .take(MAX_LABEL_CHARS)
        .collect();
    (!label.is_empty()).then_some(AgentLink { url, label })
}

fn web_url(value: &Value) -> Option<Url> {
    let value = value.as_str()?;
    if value.len() > MAX_URL_BYTES {
        return None;
    }
    let url = Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.as_str().len() <= MAX_URL_BYTES
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}
