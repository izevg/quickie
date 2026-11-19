//! Where the agent asks: an OpenAI-compatible endpoint, its model, and a reference to its key.

mod key;
mod local;
#[cfg(test)]
mod tests;

use std::fmt;

use url::{Host, Url};

pub use key::{ApiKey, ApiKeyError, KeyRef};
pub use local::LocalServer;

/// The base URL of an OpenAI-compatible API, the part before `/chat/completions`:
/// `https://api.openai.com/v1`, `http://127.0.0.1:11434/v1`.
///
/// It is https, or plain http only on a loopback host, where nothing crosses the network: the
/// question and the page go out in the clear otherwise, and so would the key. It carries no
/// credentials, query, or fragment, so the paths of the API can be joined to it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BaseUrl(Url);

impl BaseUrl {
    /// The URL, its path ending in `/`.
    #[must_use]
    pub fn as_url(&self) -> &Url {
        &self.0
    }

    /// The URL as stored and as shown in settings.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Where chat completions stream from: `<base>/chat/completions`.
    #[must_use]
    pub fn chat_completions(&self) -> Url {
        self.joined("chat/completions")
    }

    /// Where the endpoint lists its models: `<base>/models`.
    #[must_use]
    pub fn models(&self) -> Url {
        self.joined("models")
    }

    /// True when the endpoint is on this machine: `127.0.0.0/8`, `::1`, or `localhost`.
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        is_loopback(&self.0)
    }

    fn joined(&self, path: &str) -> Url {
        let mut url = self.0.clone();
        url.set_path(&format!("{}{path}", self.0.path()));
        url
    }
}

impl fmt::Display for BaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a text is not an endpoint's base URL.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BaseUrlError {
    /// The text does not parse as a URL.
    #[error("{0:?} is not a URL")]
    NotAUrl(String),
    /// The scheme is neither https nor http. Holds the scheme.
    #[error("an endpoint is https, not {0}:")]
    Scheme(String),
    /// Plain http to a host that is not this machine. Holds the host.
    #[error("plain http is only for this machine, not {0}; use https")]
    PlainHttp(String),
    /// The URL carries a user name or a password; the key goes to the Keychain instead.
    #[error("an endpoint URL carries no credentials; the key goes to the Keychain")]
    Credentials,
    /// The URL has a query or a fragment, which the API's paths cannot follow.
    #[error("an endpoint URL has no query or fragment")]
    QueryOrFragment,
}

impl TryFrom<&str> for BaseUrl {
    type Error = BaseUrlError;

    /// Parses a base URL as typed in settings; surrounding whitespace is ignored and a missing
    /// trailing `/` is added.
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let mut url =
            Url::parse(text.trim()).map_err(|_| BaseUrlError::NotAUrl(text.to_owned()))?;
        match url.scheme() {
            "https" => {}
            "http" if is_loopback(&url) => {}
            "http" => {
                let host = url.host_str().unwrap_or_default().to_owned();
                return Err(BaseUrlError::PlainHttp(host));
            }
            other => return Err(BaseUrlError::Scheme(other.to_owned())),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(BaseUrlError::Credentials);
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(BaseUrlError::QueryOrFragment);
        }
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(Self(url))
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(name)) => name == "localhost",
        None => false,
    }
}

/// The model an endpoint runs, as the endpoint names it: `gpt-4o-mini`, `llama3.1:8b`,
/// `lmstudio-community/Meta-Llama-3-8B-Instruct-GGUF`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModelId(String);

impl ModelId {
    /// The longest id accepted, in characters.
    pub const MAX_CHARS: usize = 256;

    /// The id as the API takes it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a text is not a model id.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelIdError {
    /// Nothing but whitespace.
    #[error("a model id is not empty")]
    Empty,
    /// Longer than [`ModelId::MAX_CHARS`].
    #[error("a model id has at most {} characters", ModelId::MAX_CHARS)]
    TooLong,
    /// A control character, such as a line break.
    #[error("a model id has no control characters")]
    Control,
}

impl TryFrom<&str> for ModelId {
    type Error = ModelIdError;

    /// Parses a model id; surrounding whitespace is ignored.
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ModelIdError::Empty);
        }
        if text.chars().count() > Self::MAX_CHARS {
            return Err(ModelIdError::TooLong);
        }
        if text.chars().any(char::is_control) {
            return Err(ModelIdError::Control);
        }
        Ok(Self(text.to_owned()))
    }
}

/// The endpoint the built-in agent asks: an OpenAI-compatible API, the model to ask there,
/// and, when the API needs a key, where that key is kept.
///
/// The key is bound to the origin of the base URL: moving the endpoint to another origin drops
/// it, so a key saved for one host is never sent to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmEndpoint {
    base: BaseUrl,
    model: ModelId,
    key: Option<KeyRef>,
}

impl LlmEndpoint {
    /// An endpoint that needs no key, as local servers do.
    #[must_use]
    pub fn new(base: BaseUrl, model: ModelId) -> Self {
        Self {
            base,
            model,
            key: None,
        }
    }

    /// The API's base URL.
    #[must_use]
    pub fn base(&self) -> &BaseUrl {
        &self.base
    }

    /// The model asked.
    #[must_use]
    pub fn model(&self) -> &ModelId {
        &self.model
    }

    /// Where the key is kept, if the endpoint has one.
    #[must_use]
    pub fn key(&self) -> Option<&KeyRef> {
        self.key.as_ref()
    }

    /// The same endpoint with a key, kept under [`KeyRef::for_origin`] of its base URL.
    #[must_use]
    pub fn with_key(self) -> Self {
        let key = Some(KeyRef::for_origin(&self.base));
        Self { key, ..self }
    }

    /// The same endpoint without a key.
    #[must_use]
    pub fn without_key(self) -> Self {
        Self { key: None, ..self }
    }

    /// The endpoint at another base URL or model. It keeps its key only on the same origin.
    #[must_use]
    pub fn moved(&self, base: BaseUrl, model: ModelId) -> Self {
        let key = self
            .key
            .clone()
            .filter(|key| *key == KeyRef::for_origin(&base));
        Self { base, model, key }
    }

    /// The endpoint as plain data, for the store.
    #[must_use]
    pub fn snapshot(&self) -> EndpointSnapshot {
        EndpointSnapshot {
            base_url: self.base.as_str().to_owned(),
            model: self.model.as_str().to_owned(),
            key_ref: self.key.as_ref().map(|key| key.as_str().to_owned()),
        }
    }

    /// Rebuilds an endpoint from what the store kept, checking every invariant once, here.
    ///
    /// # Errors
    ///
    /// The first [`CorruptEndpoint`] found.
    pub fn restore(snapshot: &EndpointSnapshot) -> Result<Self, CorruptEndpoint> {
        let base = BaseUrl::try_from(snapshot.base_url.as_str())?;
        let model = ModelId::try_from(snapshot.model.as_str())?;
        let endpoint = Self::new(base, model);
        match &snapshot.key_ref {
            None => Ok(endpoint),
            Some(key) if key == KeyRef::for_origin(&endpoint.base).as_str() => {
                Ok(endpoint.with_key())
            }
            Some(key) => Err(CorruptEndpoint::ForeignKey(key.clone())),
        }
    }
}

/// An endpoint as plain text, for the store to write and read back. The store does not judge
/// it; [`LlmEndpoint::restore`] does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointSnapshot {
    /// The base URL.
    pub base_url: String,
    /// The model id.
    pub model: String,
    /// The key's reference in the Keychain, if the endpoint has a key.
    pub key_ref: Option<String>,
}

/// A saved endpoint that breaks an invariant: a corrupt file or a broken migration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CorruptEndpoint {
    /// The base URL is not one an endpoint may have.
    #[error(transparent)]
    Base(#[from] BaseUrlError),
    /// The model id is not one.
    #[error(transparent)]
    Model(#[from] ModelIdError),
    /// The key reference names a key of another origin.
    #[error("the key {0:?} belongs to another origin")]
    ForeignKey(String),
}
