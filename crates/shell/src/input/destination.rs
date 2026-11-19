//! Turns the address overlay's text into the URL to load: an address when it looks like one, a
//! `DuckDuckGo` search otherwise. The search request happens only because the user pressed ⏎.

use url::{Host, Url, form_urlencoded};

const SEARCH: &str = "https://duckduckgo.com/?q=";
/// Schemes typed out in full that load as they are.
const SCHEMES: [&str; 4] = ["http", "https", "about", "file"];

/// The URL for `input`, or `None` when there is nothing to go to.
#[must_use]
pub fn destination(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }
    if text.contains(char::is_whitespace) {
        return Some(search(text));
    }
    if let Some(url) = Url::parse(text)
        .ok()
        .filter(|url| SCHEMES.contains(&url.scheme()))
    {
        return Some(url.into());
    }
    Some(address(text).unwrap_or_else(|| search(text)))
}

/// `text` without a scheme, as a web address when its host looks like one.
fn address(text: &str) -> Option<String> {
    let local = is_local(host_part(text));
    let scheme = if local { "http" } else { "https" };
    let url = Url::parse(&format!("{scheme}://{text}")).ok()?;
    let web = match url.host()? {
        Host::Domain(domain) => local || has_public_suffix(domain),
        Host::Ipv4(_) | Host::Ipv6(_) => local,
    };
    web.then(|| url.into())
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

fn search(text: &str) -> String {
    let query: String = form_urlencoded::byte_serialize(text.as_bytes()).collect();
    format!("{SEARCH}{query}")
}

#[cfg(test)]
mod tests {
    use super::destination;

    fn to(input: &str) -> Option<String> {
        destination(input)
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
    }

    #[test]
    fn local_hosts_get_http() {
        assert_eq!(to("localhost:3000"), Some("http://localhost:3000/".into()));
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
        assert_eq!(to("a&b=c"), search("a%26b%3Dc"));
        assert_eq!(to("погода"), search("%D0%BF%D0%BE%D0%B3%D0%BE%D0%B4%D0%B0"));
        assert_eq!(to("mailto:me"), search("mailto%3Ame"));
        assert_eq!(to(".com"), search(".com"));
    }
}
