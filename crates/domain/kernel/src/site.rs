//! The site a per-site setting applies to: shields, page zoom.

use std::fmt;

use url::{Host, Url};

/// A site the user changes something for: a page's host, lowercase, without a leading `www.`.
/// A setting for a site also covers its subdomains where the context says so (shields do).
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

/// Why there is no site to set something for.
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

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{Site, SiteError};

    fn site(text: &str) -> Site {
        Site::try_from(text).unwrap()
    }

    fn site_of(url: &str) -> Result<Site, SiteError> {
        Site::try_from(&Url::parse(url).unwrap())
    }

    #[test]
    fn a_web_page_s_site_is_its_host() {
        let found = site_of("https://news.example.com/a?b#c").unwrap();

        assert_eq!(found.as_str(), "news.example.com");
        assert_eq!(found.to_string(), "news.example.com");
    }

    #[test]
    fn a_plain_http_page_has_a_site_too() {
        let found = site_of("http://example.com/").unwrap();

        assert_eq!(found, site("example.com"));
    }

    #[test]
    fn a_leading_www_is_not_part_of_the_site() {
        let found = site_of("https://www.example.com/").unwrap();

        assert_eq!(found.as_str(), "example.com");
    }

    #[test]
    fn www_stays_when_nothing_with_a_dot_is_left() {
        let found = site_of("https://www.com/").unwrap();

        assert_eq!(found.as_str(), "www.com");
    }

    #[test]
    fn www_inside_a_host_stays() {
        let found = site_of("https://shop.www.example.com/").unwrap();

        assert_eq!(found.as_str(), "shop.www.example.com");
    }

    #[test]
    fn an_ip_address_is_its_own_site() {
        assert_eq!(
            site_of("http://127.0.0.1:8080/").unwrap().as_str(),
            "127.0.0.1"
        );
        assert_eq!(site_of("http://[::1]/").unwrap().as_str(), "[::1]");
    }

    #[test]
    fn a_page_off_the_web_has_no_site() {
        for (url, scheme) in [
            ("quickie://start/", "quickie"),
            ("about:blank", "about"),
            ("file:///tmp/a.html", "file"),
        ] {
            assert_eq!(site_of(url), Err(SiteError::NotWeb(scheme.to_owned())));
        }
    }

    #[test]
    fn a_stored_site_is_lowercased_and_loses_its_www() {
        assert_eq!(site("WWW.Example.COM").as_str(), "example.com");
    }

    #[test]
    fn a_stored_site_round_trips() {
        for text in ["example.com", "127.0.0.1", "[::1]", "www.com"] {
            assert_eq!(site(text).as_str(), text);
        }
    }

    #[test]
    fn text_that_is_no_host_is_no_site() {
        let error = Site::try_from("exa mple.com").unwrap_err();

        assert_eq!(error, SiteError::NotAHost("exa mple.com".to_owned()));
        assert_eq!(error.to_string(), "\"exa mple.com\" is not a host");
    }

    #[test]
    fn errors_name_what_was_wrong() {
        let error = SiteError::NotWeb("about".to_owned());

        assert_eq!(error.to_string(), "a about: page is not a website");
    }
}
