//! What `ui diff` compares with, decided without I/O: the branch whose merge-base is the base,
//! and whether this machine needs the diff at all.

/// Whether this machine can catch a visual change, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// The references come from another renderer: only a diff against the base sees a change.
    Diff,
    /// The references come from this renderer, so `cargo test` already compares with them.
    Compared,
    /// Not macOS: the stories need `AppKit` and do not run here.
    NoRenderer,
}

/// `host` is this machine's macOS version (`None` off macOS), `references` the
/// renderer each story crate's references record (`None` where none were accepted yet).
#[must_use]
pub fn need(host: Option<&str>, references: &[Option<&str>]) -> Need {
    match host {
        None => Need::NoRenderer,
        Some(host) if references.iter().all(|theirs| *theirs == Some(host)) => Need::Compared,
        Some(_) => Need::Diff,
    }
}

/// The branch whose merge-base with `HEAD` is the default base: the upstream when there is
/// one, else the newest `origin/release/X.Y.Z`, else `origin/main`.
#[must_use]
pub fn default_branch(upstream: Option<&str>, remote_branches: &[&str]) -> Option<String> {
    if let Some(upstream) = upstream {
        return Some(upstream.to_owned());
    }
    let newest_release = remote_branches
        .iter()
        .filter_map(|&branch| release_version(branch).map(|version| (version, branch)))
        .max_by_key(|&(version, _)| version)
        .map(|(_, branch)| branch);
    newest_release
        .or_else(|| {
            remote_branches
                .iter()
                .copied()
                .find(|&b| b == "origin/main")
        })
        .map(str::to_owned)
}

/// `origin/release/0.10.2` is `(0, 10, 2)`; versions order by number, not by text.
fn release_version(branch: &str) -> Option<(u64, u64, u64)> {
    let version = branch.strip_prefix("origin/release/")?;
    let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
    let parsed = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::{Need, default_branch, need};

    #[test]
    fn the_upstream_is_the_base_branch_when_there_is_one() {
        let branches = ["origin/main", "origin/release/0.1.0"];

        let branch = default_branch(Some("origin/feat/x"), &branches);

        assert_eq!(branch.as_deref(), Some("origin/feat/x"));
    }

    #[test]
    fn without_an_upstream_the_newest_release_branch_is_the_base_branch() {
        let branches = [
            "origin/main",
            "origin/release/0.9.0",
            "origin/release/0.10.0",
            "origin/release/0.2.5",
        ];

        let branch = default_branch(None, &branches);

        assert_eq!(branch.as_deref(), Some("origin/release/0.10.0"));
    }

    #[test]
    fn a_release_branch_that_is_not_a_version_is_ignored() {
        let branches = [
            "origin/main",
            "origin/release/next",
            "origin/release/1.0",
            "origin/release/1.0.0.1",
            "release/9.9.9",
        ];

        let branch = default_branch(None, &branches);

        assert_eq!(branch.as_deref(), Some("origin/main"));
    }

    #[test]
    fn without_a_release_branch_main_is_the_base_branch() {
        let branch = default_branch(None, &["origin/feat/x", "origin/main"]);

        assert_eq!(branch.as_deref(), Some("origin/main"));
    }

    #[test]
    fn without_any_known_branch_there_is_no_default_base() {
        assert_eq!(default_branch(None, &["origin/feat/x"]), None);
    }

    #[test]
    fn references_from_another_macos_need_a_diff() {
        assert_eq!(
            need(
                Some("macos-26.5.2"),
                &[Some("macos-26.7.2"), Some("macos-26.7.2")],
            ),
            Need::Diff
        );
    }

    #[test]
    fn one_crate_from_another_macos_is_enough_to_need_a_diff() {
        assert_eq!(
            need(Some("macos-27"), &[Some("macos-27"), Some("macos-26")]),
            Need::Diff
        );
    }

    #[test]
    fn a_crate_without_references_needs_a_diff() {
        assert_eq!(
            need(Some("macos-27"), &[Some("macos-27"), None]),
            Need::Diff
        );
    }

    #[test]
    fn references_from_this_macos_are_compared_by_the_tests() {
        assert_eq!(
            need(Some("macos-26"), &[Some("macos-26"), Some("macos-26")]),
            Need::Compared
        );
    }

    #[test]
    fn off_macos_nothing_renders() {
        assert_eq!(need(None, &[Some("macos-26")]), Need::NoRenderer);
    }
}
