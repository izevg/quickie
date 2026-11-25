use std::ffi::{CString, OsString};

use quickie_domain_browsing::Destination;

#[derive(Debug, PartialEq, Eq)]
pub enum Options {
    Empty,
    Updates,
    Mcp { attach_only: bool },
    Navigate(CString),
    Smoke(CString),
}

impl Options {
    pub fn parse(mut arguments: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let Some(first) = arguments.next() else {
            return Ok(Self::Empty);
        };
        if first == "--check-for-updates" {
            return if arguments.next().is_none() {
                Ok(Self::Updates)
            } else {
                Err("the update action does not take an address".into())
            };
        }
        if first == "mcp" || first == "--mcp" {
            return parse_mcp_options(arguments);
        }
        if first == "--attach-only" {
            return Err("--attach-only requires `mcp` or `--mcp`".into());
        }
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

fn parse_mcp_options(mut arguments: impl Iterator<Item = OsString>) -> Result<Options, String> {
    let Some(flag) = arguments.next() else {
        return Ok(Options::Mcp { attach_only: false });
    };
    if flag != "--attach-only" {
        return Err("MCP mode accepts only optional --attach-only".into());
    }
    if arguments.next().is_some() {
        return Err("MCP mode accepts at most one --attach-only flag".into());
    }
    Ok(Options::Mcp { attach_only: true })
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::Options;

    #[test]
    fn startup_with_no_address_has_no_destination() {
        assert_eq!(Options::parse(std::iter::empty()), Ok(Options::Empty));
    }

    #[test]
    fn desktop_update_action_does_not_navigate_or_accept_an_address() {
        assert_eq!(
            Options::parse(["--check-for-updates".into()].into_iter()),
            Ok(Options::Updates)
        );
        assert!(
            Options::parse(["--check-for-updates".into(), "example.com".into()].into_iter())
                .is_err()
        );
    }

    #[test]
    fn explicit_addresses_use_the_shared_destination_rules() {
        assert_eq!(
            Options::parse(["example.com".into()].into_iter()),
            Ok(Options::Navigate(CString::from(c"https://example.com/")))
        );
        assert_eq!(
            Options::parse(["rust gtk".into()].into_iter()),
            Ok(Options::Navigate(CString::from(
                c"https://duckduckgo.com/?q=rust+gtk"
            )))
        );
        assert!(Options::parse([" ".into()].into_iter()).is_err());
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

    #[test]
    fn mcp_accepts_subcommand_and_flag_spellings_with_optional_attach_only() {
        // Arrange
        let subcommand = ["mcp".into()];
        let flag = ["--mcp".into()];
        let subcommand_attach = ["mcp".into(), "--attach-only".into()];
        let flag_attach = ["--mcp".into(), "--attach-only".into()];

        // Act
        let parsed = (
            Options::parse(subcommand.into_iter()),
            Options::parse(flag.into_iter()),
            Options::parse(subcommand_attach.into_iter()),
            Options::parse(flag_attach.into_iter()),
        );

        // Assert
        assert_eq!(
            parsed,
            (
                Ok(Options::Mcp { attach_only: false }),
                Ok(Options::Mcp { attach_only: false }),
                Ok(Options::Mcp { attach_only: true }),
                Ok(Options::Mcp { attach_only: true }),
            )
        );
    }

    #[test]
    fn mcp_rejects_unknown_extra_arguments_and_stray_attach_only() {
        // Arrange
        let cases: &[&[&str]] = &[
            &["--attach-only"],
            &["mcp", "--unknown"],
            &["--mcp", "https://example.com"],
            &["mcp", "--attach-only", "--attach-only"],
            &["--mcp", "--attach-only", "extra"],
        ];

        // Act & Assert
        for args in cases {
            let parsed = Options::parse(args.iter().map(Into::into));
            assert!(parsed.is_err(), "expected error for {args:?}");
        }
    }
}
