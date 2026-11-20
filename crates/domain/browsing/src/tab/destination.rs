//! Where typed text takes a tab: an address when it looks like one, a `DuckDuckGo` search
//! otherwise. The search request happens only because the user chose to go; nothing typed is
//! sent anywhere before that.

use url::{Host, Url, form_urlencoded};

/// The search engine typed text that is no address goes to, as the UI names it.
pub const SEARCH_ENGINE: &str = "DuckDuckGo";

const SEARCH: &str = "https://duckduckgo.com/?q=";
/// Schemes typed out in full that load as they are.
const SCHEMES: [&str; 4] = ["http", "https", "about", "file"];

/// Where typed text goes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Destination {
    /// The text is an address: load it.
    Address(Url),
    /// The text is anything else: search [`SEARCH_ENGINE`] for it.
    Search(Url),
}

impl Destination {
    /// Where `input` goes, or `None` when there is nothing to go to.
    #[must_use]
    pub fn of(input: &str) -> Option<Self> {
        let text = input.trim();
        if text.is_empty() {
            return None;
        }
        if text.contains(char::is_whitespace) {
            return Self::search(text);
        }
        if let Some(url) = Url::parse(text)
            .ok()
            .filter(|url| SCHEMES.contains(&url.scheme()))
        {
            return Some(Self::Address(url));
        }
        address(text)
            .map(Self::Address)
            .or_else(|| Self::search(text))
    }

    /// The [`SEARCH_ENGINE`]'s search for `text` as it is, even when it reads as an address.
    /// `None` only for text no URL can carry, which form encoding rules out.
    #[must_use]
    pub fn search(text: &str) -> Option<Self> {
        let query: String = form_urlencoded::byte_serialize(text.as_bytes()).collect();
        Url::parse(&format!("{SEARCH}{query}"))
            .ok()
            .map(Self::Search)
    }

    /// The URL to load.
    #[must_use]
    pub const fn url(&self) -> &Url {
        match self {
            Self::Address(url) | Self::Search(url) => url,
        }
    }

    /// The URL to load, taken.
    #[must_use]
    pub fn into_url(self) -> Url {
        match self {
            Self::Address(url) | Self::Search(url) => url,
        }
    }
}

/// `text` without a scheme, as a web address when its host looks like one.
fn address(text: &str) -> Option<Url> {
    let local = is_local(host_part(text));
    let scheme = if local { "http" } else { "https" };
    let url = Url::parse(&format!("{scheme}://{text}")).ok()?;
    let web = match url.host()? {
        Host::Domain(domain) => local || has_public_suffix(domain),
        Host::Ipv4(_) | Host::Ipv6(_) => local,
    };
    web.then_some(url)
}

/// The host as typed, with any port: everything before the first `/`, `?`, or `#`.
fn host_part(text: &str) -> &str {
    text.split(['/', '?', '#']).next().unwrap_or(text)
}

/// `localhost` and written-out IP addresses are loaded over plain HTTP, as development servers.
fn is_local(host_and_port: &str) -> bool {
    if host_and_port.starts_with('[') {
        return true;
    }
    let host = host_and_port.split(':').next().unwrap_or(host_and_port);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// A dotted name whose last label is at least two letters: `example.com`, not `v1.2` or `foo`.
fn has_public_suffix(domain: &str) -> bool {
    match domain.rsplit_once('.') {
        Some((name, suffix)) => {
            !name.is_empty() && suffix.len() >= 2 && suffix.bytes().all(|b| b.is_ascii_alphabetic())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::Destination;

    fn to(input: &str) -> Option<String> {
        Destination::of(input).map(|destination| destination.url().as_str().to_owned())
    }

    fn is_address(input: &str) -> bool {
        matches!(Destination::of(input), Some(Destination::Address(_)))
    }

    #[test]
    fn nothing_typed_goes_nowhere() {
        assert_eq!(to(""), None);
        assert_eq!(to("   "), None);
    }

    #[test]
    fn full_urls_load_as_typed() {
        assert_eq!(
            to("https://example.com"),
            Some("https://example.com/".into())
        );
        assert_eq!(
            to("  http://example.com/a?b=c#d "),
            Some("http://example.com/a?b=c#d".into())
        );
        assert_eq!(to("about:blank"), Some("about:blank".into()));
        assert_eq!(to("file:///tmp/a.html"), Some("file:///tmp/a.html".into()));
        assert!(is_address("https://example.com"));
    }

    #[test]
    fn bare_domains_get_https() {
        assert_eq!(to("example.com"), Some("https://example.com/".into()));
        assert_eq!(
            to("docs.rs/objc2/latest"),
            Some("https://docs.rs/objc2/latest".into())
        );
        assert_eq!(
            to("example.com:8443/x"),
            Some("https://example.com:8443/x".into())
        );
        assert_eq!(to("ab.cd"), Some("https://ab.cd/".into()));
        assert!(is_address("example.com"));
    }

    #[test]
    fn local_hosts_get_http() {
        assert_eq!(to("localhost:3000"), Some("http://localhost:3000/".into()));
        assert_eq!(to("LocalHost"), Some("http://localhost/".into()));
        assert_eq!(
            to("127.0.0.1:8080/api"),
            Some("http://127.0.0.1:8080/api".into())
        );
        assert_eq!(to("[::1]:8080"), Some("http://[::1]:8080/".into()));
    }

    #[test]
    fn everything_else_is_searched() {
        let search = |q: &str| Some(format!("https://duckduckgo.com/?q={q}"));
        assert_eq!(to("rust objc2"), search("rust+objc2"));
        assert_eq!(to("quickie"), search("quickie"));
        assert_eq!(to("1.5"), search("1.5"));
        assert_eq!(to("v1.2"), search("v1.2"));
        assert_eq!(to("a.b"), search("a.b"));
        assert_eq!(to("a.c0"), search("a.c0"));
        assert_eq!(to("a&b=c"), search("a%26b%3Dc"));
        assert_eq!(to("погода"), search("%D0%BF%D0%BE%D0%B3%D0%BE%D0%B4%D0%B0"));
        assert_eq!(to("mailto:me"), search("mailto%3Ame"));
        assert_eq!(to(".com"), search(".com"));
        assert_eq!(to("1.2.3"), search("1.2.3"));
        assert_eq!(to("1.2.3.4.5"), search("1.2.3.4.5"));
        assert_eq!(to("1..3.4"), search("1..3.4"));
        assert_eq!(to("1.2.3.x"), search("1.2.3.x"));
        assert_eq!(to("10.0.0.1"), Some("http://10.0.0.1/".into()));
        assert!(!is_address("quickie"));
    }

    #[test]
    fn an_ip_address_only_loads_when_written_out_as_local() {
        assert_eq!(to("8.8.8.8"), Some("http://8.8.8.8/".into()));
        assert_eq!(to("[2001:db8::1]"), Some("http://[2001:db8::1]/".into()));
    }

    #[test]
    fn the_url_can_be_taken() {
        let destination = Destination::of("example.com").unwrap();
        assert_eq!(
            destination.into_url(),
            Url::parse("https://example.com/").unwrap()
        );
        let search = Destination::of("hello").unwrap();
        assert_eq!(
            search.clone().into_url(),
            Url::parse("https://duckduckgo.com/?q=hello").unwrap()
        );
        assert!(matches!(search, Destination::Search(_)));
    }

    #[test]
    fn an_address_can_be_searched_for_too() {
        assert_eq!(
            Destination::search("github.com"),
            Some(Destination::Search(
                Url::parse("https://duckduckgo.com/?q=github.com").unwrap()
            ))
        );
    }
}
