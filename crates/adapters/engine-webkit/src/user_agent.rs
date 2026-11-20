//! The user agent tail that makes `WKWebView` read as Safari.
//!
//! A plain `WKWebView` sends `… AppleWebKit/605.1.15 (KHTML, like Gecko)` and stops there;
//! Safari appends `Version/<its version> Safari/605.1.15`. Sites that sniff for that tail
//! (Outlook on the web, Teams, some banks) serve a degraded page without it. We append exactly
//! what the installed Safari would, so we also look like the crowd rather than a rare agent.

use std::sync::OnceLock;

/// Where macOS keeps Safari's version.
const SAFARI_PLIST: &str = "/Applications/Safari.app/Contents/Info.plist";

/// Used when Safari's version cannot be read (removed, or an unexpected plist).
const FALLBACK_VERSION: &str = "26.0";

/// `Version/<installed Safari> Safari/605.1.15`, read once per process.
pub(crate) fn safari_tail() -> String {
    static TAIL: OnceLock<String> = OnceLock::new();
    TAIL.get_or_init(|| {
        let plist = std::fs::read_to_string(SAFARI_PLIST).unwrap_or_default();
        tail(short_version(&plist).unwrap_or(FALLBACK_VERSION))
    })
    .clone()
}

/// The tail for a Safari version. `605.1.15` is frozen in every Safari since 2017.
fn tail(version: &str) -> String {
    format!("Version/{version} Safari/605.1.15")
}

/// `CFBundleShortVersionString` from an XML plist, if it looks like a version (`27.2`).
fn short_version(plist: &str) -> Option<&str> {
    let after_key = plist
        .split("<key>CFBundleShortVersionString</key>")
        .nth(1)?;
    let value = after_key
        .trim_start()
        .strip_prefix("<string>")?
        .split("</string>")
        .next()?;
    let looks_right = !value.is_empty()
        && value.len() <= 16
        && value.chars().all(|c| c.is_ascii_digit() || c == '.');
    looks_right.then_some(value)
}

#[cfg(test)]
mod tests {
    use super::{short_version, tail};

    #[test]
    fn the_tail_matches_safaris() {
        assert_eq!(tail("27.2"), "Version/27.2 Safari/605.1.15");
    }

    #[test]
    fn reads_the_version_from_safaris_plist() {
        let plist = "<dict>\n\t<key>CFBundleName</key>\n\t<string>Safari</string>\n\
                     \t<key>CFBundleShortVersionString</key>\n\t<string>27.2</string>\n</dict>";

        assert_eq!(short_version(plist), Some("27.2"));
    }

    #[test]
    fn refuses_anything_that_is_not_a_version() {
        let odd = "<key>CFBundleShortVersionString</key><string>27.2 beta</string>";
        let missing = "<key>CFBundleName</key><string>Safari</string>";
        let empty = "<key>CFBundleShortVersionString</key><string></string>";
        let not_a_string = "<key>CFBundleShortVersionString</key><integer>27</integer>";

        assert_eq!(short_version(odd), None);
        assert_eq!(short_version(missing), None);
        assert_eq!(short_version(empty), None);
        assert_eq!(short_version(not_a_string), None);
        assert_eq!(short_version(""), None);
    }
}
