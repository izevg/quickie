use std::ffi::{CString, OsString};

use quickie_domain_browsing::Destination;

#[derive(Debug, PartialEq, Eq)]
pub enum Options {
    Empty,
    Navigate(CString),
    Smoke(CString),
}

impl Options {
    pub fn parse(mut arguments: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let Some(first) = arguments.next() else {
            return Ok(Self::Empty);
        };
        let (input, smoke) = if first == "--smoke-test" {
            let url = arguments
                .next()
                .ok_or("the smoke test needs a loopback URL")?;
            (url, true)
        } else {
            (first, false)
        };
        if arguments.next().is_some() {
            return Err("provide at most one address".into());
        }
        let input = input.to_str().ok_or("the address must be UTF-8")?;
        let address = Destination::of(input).ok_or("provide a nonempty address")?;
        let url = address.url();
        if smoke && (url.scheme() != "http" || url.host_str() != Some("127.0.0.1")) {
            return Err("the smoke test needs an HTTP URL on 127.0.0.1".into());
        }
        let url = CString::new(url.as_str()).map_err(|error| error.to_string())?;
        Ok(if smoke {
            Self::Smoke(url)
        } else {
            Self::Navigate(url)
        })
    }
}

pub fn address_bar_destination(input: &str) -> Option<CString> {
    CString::new(Destination::of(input)?.url().as_str()).ok()
}

#[cfg(test)]
mod tests {
    use super::{Options, address_bar_destination};

    #[test]
    fn startup_with_no_address_has_no_destination() {
        assert_eq!(Options::parse(std::iter::empty()), Ok(Options::Empty));
    }

    #[test]
    fn explicit_addresses_use_the_shared_destination_rules() {
        assert_eq!(
            address_bar_destination("example.com").as_deref(),
            Some(c"https://example.com/")
        );
        assert_eq!(
            address_bar_destination("rust gtk").as_deref(),
            Some(c"https://duckduckgo.com/?q=rust+gtk")
        );
        assert_eq!(address_bar_destination(" "), None);
    }

    #[test]
    fn smoke_can_only_open_the_local_fixture() {
        assert!(matches!(
            Options::parse(["--smoke-test".into(), "http://127.0.0.1:18415/".into()].into_iter()),
            Ok(Options::Smoke(_))
        ));
        for address in [
            "https://127.0.0.1/",
            "http://127.0.0.2/",
            "http://example.com/",
        ] {
            assert!(Options::parse(["--smoke-test".into(), address.into()].into_iter()).is_err());
        }
    }
}
