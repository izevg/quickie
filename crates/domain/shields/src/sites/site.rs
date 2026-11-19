//! The site a policy applies to.

use std::fmt;

use url::{Host, Url};

/// A site the user sets shields for: a page's host, lowercase, without a leading `www.`. A
/// policy for a site also covers its subdomains.
///
/// This stands in for the registrable domain (eTLD+1) until the public suffix list is part of
/// the domain: for now `news.example.com` and `example.com` are two sites.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Site(String);

impl Site {
    /// The site as stored and as the engine matches it: `example.com`, `127.0.0.1`, `[::1]`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn from_host<S: AsRef<str>>(host: &Host<S>) -> Self {
        let text = host.to_string();
        if let Host::Domain(_) = host
            && let Some(rest) = text.strip_prefix("www.")
            && rest.contains('.')
        {
            return Self(rest.to_owned());
        }
        Self(text)
    }
}

impl fmt::Display for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why there is no site to set a policy for.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SiteError {
    /// The page is not on the web: the start page, `about:blank`, a `data:` URL. Holds the
    /// scheme.
    #[error("a {0}: page is not a website")]
    NotWeb(String),
    /// The text is not a host name or an IP address.
    #[error("{0:?} is not a host")]
    NotAHost(String),
}

/// The site of a web page (http or https).
impl TryFrom<&Url> for Site {
    type Error = SiteError;

    fn try_from(url: &Url) -> Result<Self, Self::Error> {
        match (url.scheme(), url.host()) {
            ("http" | "https", Some(host)) => Ok(Self::from_host(&host)),
            (scheme, _) => Err(SiteError::NotWeb(scheme.to_owned())),
        }
    }
}

/// A site as stored, or as typed: a host name or an IP address.
impl TryFrom<&str> for Site {
    type Error = SiteError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        Host::parse(text)
            .map(|host| Self::from_host(&host))
            .map_err(|_| SiteError::NotAHost(text.to_owned()))
    }
}
