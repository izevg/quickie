//! Validate the reply from `WebKit` before it reaches the agent.

use std::collections::HashSet;

use quickie_application::{AgentLink, AgentLinks, EngineError};
use serde_json::Value;
use url::Url;

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
    let mut seen = HashSet::with_capacity(candidates.len().min(MAX_LINKS));
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
    // web_url only accepts HTTP(S), whose origins are these normalized URL components.
    if &url == document
        || url.scheme() != document.scheme()
        || url.host() != document.host()
        || url.port_or_known_default() != document.port_or_known_default()
    {
        return None;
    }
    let label = value.get("label")?.as_str()?.trim();
    if label.is_empty() {
        return None;
    }
    let end = if label.len() <= MAX_LABEL_CHARS {
        label.len()
    } else {
        label
            .char_indices()
            .nth(MAX_LABEL_CHARS)
            .map_or(label.len(), |(offset, _)| offset)
    };
    Some(AgentLink {
        url,
        label: label[..end].to_owned(),
    })
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
