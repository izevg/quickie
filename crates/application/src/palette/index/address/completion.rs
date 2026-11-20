//! Inline completion: typed text that starts a known page's address is completed to the end of
//! the host or of the next path segment, never into the middle of a word.
//!
//! An address is a web URL without its scheme and without a leading `www.`, unless the user
//! typed those: `git` starts `https://www.github.com/x`, and so do `www.git` and `https://git`.

use std::ops::Range;

use url::Url;

/// What the field shows while a completion is offered: the typed text, then the tail, selected
/// so that typing goes on over it and `⌫` takes it away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineCompletion {
    /// What the user typed, as typed.
    pub typed: String,
    /// What the completion adds after it.
    pub tail: String,
    /// Where the completed text goes.
    pub url: Url,
}

impl InlineCompletion {
    /// The whole text in the field: typed, then the tail.
    #[must_use]
    pub fn text(&self) -> String {
        format!("{}{}", self.typed, self.tail)
    }
}

/// The schemes whose addresses complete; typed out, they must match.
const SCHEMES: [&str; 2] = ["http", "https"];
const WWW: &str = "www.";

/// The byte range of `url` that `typed` covers when it starts the URL's address, ignoring ASCII
/// case; `None` when it does not, or when `typed` names no more than a scheme.
pub(super) fn matched(typed: &str, url: &str) -> Option<Range<usize>> {
    let (scheme, rest) = url.split_once("://")?;
    if !SCHEMES.contains(&scheme) {
        return None;
    }
    let typed = match split_scheme(typed) {
        Some((typed_scheme, typed)) if typed_scheme.eq_ignore_ascii_case(scheme) => typed,
        Some(_) => return None,
        None => typed,
    };
    if typed.is_empty() {
        return None;
    }
    let skip_www = starts_with(rest, WWW) && !starts_with(typed, WWW);
    let start = scheme.len() + "://".len() + if skip_www { WWW.len() } else { 0 };
    starts_with(&url[start..], typed).then(|| start..start + typed.len())
}

/// The completion of `typed` from the page at `url`, when `typed` starts its address and stops
/// short of a boundary: the end of the host, of a path segment, or of the URL. Text that ends
/// on a boundary is a whole name already and is left alone.
pub(super) fn complete(typed: &str, url: &Url) -> Option<InlineCompletion> {
    let url = url.as_str();
    let matched = matched(typed, url)?;
    let address = &url[matched.start..];
    let end = boundary_after(address, matched.len())?;
    let tail = address.get(matched.len()..end)?;
    Some(InlineCompletion {
        typed: typed.to_owned(),
        tail: tail.to_owned(),
        url: Url::parse(url.get(..matched.start + end)?).ok()?,
    })
}

/// The first boundary of `address` after `typed` bytes, or `None` when `typed` ends on one.
/// Boundaries sit before each `/`, `?`, and `#`, and at the end.
fn boundary_after(address: &str, typed: usize) -> Option<usize> {
    let mut boundaries = address
        .match_indices(['/', '?', '#'])
        .map(|(at, _)| at)
        .chain([address.len()]);
    boundaries.find(|&at| at >= typed).filter(|&at| at != typed)
}

/// `typed` split into a typed-out scheme and the rest, when it starts with `scheme://`.
fn split_scheme(typed: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = typed.split_once("://")?;
    SCHEMES
        .iter()
        .any(|known| known.eq_ignore_ascii_case(scheme))
        .then_some((scheme, rest))
}

/// Whether `text` starts with `prefix`, ignoring ASCII case.
fn starts_with(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix.as_bytes()))
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{InlineCompletion, boundary_after, complete, matched};

    fn completes(typed: &str, url: &str) -> Option<(String, String)> {
        complete(typed, &Url::parse(url).unwrap())
            .map(|completion| (completion.text(), completion.url.to_string()))
    }

    #[test]
    fn a_host_completes_to_its_end() {
        assert_eq!(
            completes("git", "https://github.com/izevg/quickie/pulls"),
            Some(("github.com".into(), "https://github.com/".into()))
        );
        assert_eq!(
            completes("local", "http://localhost:3000/"),
            Some(("localhost:3000".into(), "http://localhost:3000/".into()))
        );
    }

    #[test]
    fn a_path_completes_to_the_end_of_its_segment() {
        assert_eq!(
            completes("github.com/iz", "https://github.com/izevg/quickie/pulls"),
            Some(("github.com/izevg".into(), "https://github.com/izevg".into()))
        );
        assert_eq!(
            completes(
                "github.com/izevg/quickie/p",
                "https://github.com/izevg/quickie/pulls"
            ),
            Some((
                "github.com/izevg/quickie/pulls".into(),
                "https://github.com/izevg/quickie/pulls".into()
            ))
        );
        assert_eq!(
            completes("a.org/s", "https://a.org/search?q=1#top"),
            Some(("a.org/search".into(), "https://a.org/search".into()))
        );
    }

    #[test]
    fn text_that_ends_a_name_is_left_alone() {
        assert_eq!(completes("github.com", "https://github.com/izevg"), None);
        assert_eq!(completes("github.com/", "https://github.com/"), None);
        assert_eq!(
            completes("github.com/izevg", "https://github.com/izevg/x"),
            None
        );
    }

    #[test]
    fn www_and_the_scheme_are_skipped_unless_typed() {
        assert_eq!(
            completes("rust", "https://www.rust-lang.org/learn"),
            Some(("rust-lang.org".into(), "https://www.rust-lang.org/".into()))
        );
        assert_eq!(
            completes("www.ru", "https://www.rust-lang.org/learn"),
            Some((
                "www.rust-lang.org".into(),
                "https://www.rust-lang.org/".into()
            ))
        );
        assert_eq!(
            completes("HTTPS://Gi", "https://github.com/"),
            Some(("HTTPS://Github.com".into(), "https://github.com/".into()))
        );
        assert_eq!(completes("http://gi", "https://github.com/"), None);
        assert_eq!(completes("https://", "https://github.com/"), None);
        assert_eq!(completes("www.gi", "https://github.com/"), None);
    }

    #[test]
    fn case_is_kept_as_typed() {
        let completion = complete("GitH", &Url::parse("https://github.com/").unwrap());

        assert_eq!(
            completion,
            Some(InlineCompletion {
                typed: "GitH".into(),
                tail: "ub.com".into(),
                url: Url::parse("https://github.com/").unwrap(),
            })
        );
    }

    #[test]
    fn only_web_pages_complete() {
        assert_eq!(completes("blank", "about:blank"), None);
        assert_eq!(completes("tmp", "file:///tmp/a"), None);
        assert_eq!(completes("ftp", "ftp://ftp.example.com/"), None);
        assert_eq!(matched("ftp://ex", "https://example.com/"), None);
    }

    #[test]
    fn what_does_not_start_the_address_does_not_complete() {
        assert_eq!(completes("hub", "https://github.com/"), None);
        assert_eq!(completes("лента", "https://lenta.ru/"), None);
        assert_eq!(completes("github.company", "https://github.com/"), None);
        assert_eq!(completes("", "https://github.com/"), None);
    }

    #[test]
    fn the_match_covers_the_typed_part_of_the_url() {
        assert_eq!(matched("git", "https://github.com/"), Some(8..11));
        assert_eq!(matched("rust", "https://www.rust-lang.org/"), Some(12..16));
        assert_eq!(matched("www.r", "http://www.rust-lang.org/"), Some(7..12));
        assert_eq!(matched("http://ru", "http://rust-lang.org/"), Some(7..9));
    }

    #[test]
    fn boundaries_are_the_ends_of_names() {
        assert_eq!(boundary_after("a.org/b/c", 1), Some(5));
        assert_eq!(boundary_after("a.org/b/c", 5), None);
        assert_eq!(boundary_after("a.org/b/c", 6), Some(7));
        assert_eq!(boundary_after("a.org/b/c", 8), Some(9));
        assert_eq!(boundary_after("a.org/b/c", 9), None);
        assert_eq!(boundary_after("a.org/b/c", 10), None);
    }
}
