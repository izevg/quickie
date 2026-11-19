//! The origin of a page as the origin chip shows it: the registrable domain (eTLD+1) that says
//! who runs the site, what precedes it, and whether the connection is encrypted.
//!
//! The registrable domain comes from a small built-in suffix table, not the full Public Suffix
//! List: a host's last two labels, or its last three when the last two are a known shared
//! suffix (`co.uk`, `com.au`, `github.io`, ...). A suffix missing from [`SHARED_SUFFIXES`]
//! makes the chip show one label too few in bold (`example.co.zz` shows `co.zz`); it never
//! shows a label that belongs to someone else's site as the owner's. The product takes the
//! list from Chromium's registry instead.

use url::{Host, Url};

/// Suffixes under which different people register sites: second-level country domains and
/// the best-known hosting platforms. Lowercase, two labels each.
const SHARED_SUFFIXES: [&str; 44] = [
    "ac.uk",
    "co.uk",
    "gov.uk",
    "ltd.uk",
    "me.uk",
    "net.uk",
    "org.uk",
    "plc.uk",
    "com.au",
    "edu.au",
    "gov.au",
    "net.au",
    "org.au",
    "co.nz",
    "org.nz",
    "ac.jp",
    "co.jp",
    "ne.jp",
    "or.jp",
    "co.kr",
    "co.in",
    "co.il",
    "co.za",
    "com.ar",
    "com.br",
    "com.cn",
    "com.hk",
    "com.mx",
    "com.sg",
    "com.tr",
    "com.tw",
    "com.ua",
    "net.cn",
    "org.cn",
    "appspot.com",
    "blogspot.com",
    "cloudfront.net",
    "github.io",
    "gitlab.io",
    "herokuapp.com",
    "netlify.app",
    "pages.dev",
    "vercel.app",
    "workers.dev",
];

/// How the page reached the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// `https`: encrypted and authenticated.
    Secure,
    /// `http`: anyone on the way can read and change it.
    Insecure,
}

/// A web page's origin, split for display: `developer.` `apple.com` `:8443`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    /// `https` or `http`.
    pub transport: Transport,
    /// The labels before the registrable domain, with the trailing dot (`developer.`), or
    /// empty.
    pub subdomain: String,
    /// The registrable domain (`apple.com`), or the whole host when it has none: an IP address,
    /// `localhost`, a bare suffix.
    pub registrable: String,
    /// A port other than the scheme's default.
    pub port: Option<u16>,
}

impl Origin {
    /// The origin of `url`, or `None` for a page that is not on the web (`quickie:`, `about:`,
    /// `file:`, `data:`).
    #[must_use]
    pub fn of(url: &Url) -> Option<Self> {
        let transport = match url.scheme() {
            "https" => Transport::Secure,
            "http" => Transport::Insecure,
            _ => return None,
        };
        let host = match url.host()? {
            Host::Domain(domain) => domain.trim_end_matches('.').to_owned(),
            Host::Ipv4(address) => address.to_string(),
            Host::Ipv6(address) => format!("[{address}]"),
        };
        let registrable = registrable_domain(&host);
        let subdomain = host[..host.len() - registrable.len()].to_owned();
        Some(Self {
            transport,
            registrable: registrable.to_owned(),
            subdomain,
            port: url.port(),
        })
    }

    /// The whole host, `developer.apple.com`.
    #[must_use]
    pub fn host(&self) -> String {
        format!("{}{}", self.subdomain, self.registrable)
    }
}

/// The registrable domain of `host` (eTLD+1, see the module docs): `apple.com` for
/// `developer.apple.com`, `bbc.co.uk` for `www.bbc.co.uk`. An IP address, a single label, or
/// a bare shared suffix is its own registrable domain.
#[must_use]
pub fn registrable_domain(host: &str) -> &str {
    // An IPv6 address (`[::1]`) has no dots, so only IPv4 needs telling apart.
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return host;
    }
    let last_two = tail(host, 2);
    let labels = if is_shared_suffix(last_two) { 3 } else { 2 };
    tail(host, labels)
}

/// True when a navigation from `from` to `to` changes the origin: the scheme, the host, or
/// the port.
#[must_use]
pub fn is_cross_origin(from: &Url, to: &Url) -> bool {
    from.scheme() != to.scheme()
        || from.host_str() != to.host_str()
        || from.port_or_known_default() != to.port_or_known_default()
}

fn is_shared_suffix(suffix: &str) -> bool {
    SHARED_SUFFIXES
        .iter()
        .any(|shared| shared.eq_ignore_ascii_case(suffix))
}

/// The last `labels` labels of `host`, or all of it when it has no more.
fn tail(host: &str, labels: usize) -> &str {
    host.rmatch_indices('.')
        .nth(labels - 1)
        .map_or(host, |(dot, _)| &host[dot + 1..])
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{Origin, Transport, is_cross_origin, registrable_domain};

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn the_registrable_domain_is_the_last_two_labels() {
        assert_eq!(registrable_domain("developer.apple.com"), "apple.com");
        assert_eq!(registrable_domain("a.b.c.example.org"), "example.org");
        assert_eq!(registrable_domain("example.com"), "example.com");
    }

    #[test]
    fn a_shared_suffix_takes_a_third_label() {
        assert_eq!(registrable_domain("www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(registrable_domain("bbc.co.uk"), "bbc.co.uk");
        assert_eq!(registrable_domain("someone.github.io"), "someone.github.io");
        assert_eq!(
            registrable_domain("x.Someone.GitHub.IO"),
            "Someone.GitHub.IO"
        );
    }

    #[test]
    fn short_hosts_and_addresses_are_their_own_registrable_domain() {
        assert_eq!(registrable_domain("localhost"), "localhost");
        assert_eq!(registrable_domain("co.uk"), "co.uk");
        assert_eq!(registrable_domain("github.io"), "github.io");
        assert_eq!(registrable_domain("192.168.1.20"), "192.168.1.20");
        assert_eq!(registrable_domain("[::1]"), "[::1]");
        assert_eq!(registrable_domain(""), "");
    }

    #[test]
    fn an_origin_splits_its_host_around_the_registrable_domain() {
        let origin = Origin::of(&url("https://developer.apple.com/documentation")).unwrap();

        assert_eq!(
            origin,
            Origin {
                transport: Transport::Secure,
                subdomain: "developer.".into(),
                registrable: "apple.com".into(),
                port: None,
            }
        );
        assert_eq!(origin.host(), "developer.apple.com");
    }

    #[test]
    fn plain_http_is_insecure_and_a_port_is_kept() {
        let origin = Origin::of(&url("http://127.0.0.1:8080/")).unwrap();

        assert_eq!(origin.transport, Transport::Insecure);
        assert_eq!(origin.subdomain, "");
        assert_eq!(origin.registrable, "127.0.0.1");
        assert_eq!(origin.port, Some(8080));
    }

    #[test]
    fn a_default_port_is_not_shown() {
        let origin = Origin::of(&url("https://example.com:443/")).unwrap();

        assert_eq!(origin.port, None);
    }

    #[test]
    fn an_ipv6_host_keeps_its_brackets_and_a_trailing_dot_goes() {
        let v6 = Origin::of(&url("http://[::1]:3000/")).unwrap();
        let dotted = Origin::of(&url("https://www.example.com./")).unwrap();

        assert_eq!(v6.registrable, "[::1]");
        assert_eq!(dotted.subdomain, "www.");
        assert_eq!(dotted.registrable, "example.com");
    }

    #[test]
    fn pages_off_the_web_have_no_origin() {
        for text in [
            "quickie://start/",
            "about:blank",
            "file:///tmp/a.html",
            "data:text/plain,hi",
        ] {
            assert_eq!(Origin::of(&url(text)), None, "{text}");
        }
    }

    #[test]
    fn a_new_scheme_host_or_port_crosses_the_origin() {
        let from = url("https://a.example/one");

        assert!(!is_cross_origin(&from, &url("https://a.example/two?q#f")));
        assert!(!is_cross_origin(&from, &url("https://a.example:443/")));
        assert!(is_cross_origin(&from, &url("http://a.example/one")));
        assert!(is_cross_origin(&from, &url("https://b.example/one")));
        assert!(is_cross_origin(&from, &url("https://a.example:8443/one")));
    }
}
