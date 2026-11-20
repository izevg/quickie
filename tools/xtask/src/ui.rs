//! `cargo xtask ui`: the UI crates' stories (docs/quality.md, UI tests), headless.
//!
//! - `list [<filter>]`: the stories and their themes.
//! - `render [<filter>] [--scale <n>]`: images under `target/ui/<widget>/<state>.<theme>.png`,
//!   with each state's accessibility tree beside them; plays run, nothing is compared.
//! - `check [<filter>]`: what `cargo test` runs: renders, plays, and comparisons.
//! - `accept [<filter>] [--from <dir>]`: renders become the references; `--from` takes the
//!   renders of a CI run instead (its `ui-renders` artifact).
//!
//! A filter is `<widget>`, `<widget>/<state>`, or `<widget>/<state>.<theme>`. Every story
//! binary gets the same arguments and keeps the stories the filters name.

use std::process::Command;

/// The crates with a `stories` test target.
const PACKAGES: [&str; 2] = ["quickie-ui-widgets", "quickie-ui-screens"];

const MODES: [&str; 4] = ["list", "render", "check", "accept"];

pub const USAGE: &str = "cargo xtask ui <list | render | check | accept> \
                         [<widget>[/<state>[.<theme>]] ...] [--scale <n>] [--from <dir>]";

/// The `cargo` arguments that run every story binary with `args`.
pub fn arguments(args: &[&str]) -> Result<Vec<String>, String> {
    let Some((mode, rest)) = args.split_first() else {
        return Err(USAGE.to_owned());
    };
    if !MODES.contains(mode) {
        return Err(USAGE.to_owned());
    }
    let mut out: Vec<String> = ["test", "--quiet", "--no-fail-fast"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    for package in PACKAGES {
        out.push("--package".to_owned());
        out.push(package.to_owned());
    }
    out.extend(["--test", "stories", "--", mode].map(str::to_owned));
    out.extend(rest.iter().map(|&arg| arg.to_owned()));
    Ok(out)
}

/// Runs the story binaries; their own output says what they did.
pub fn run(cargo: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(cargo)
        .args(arguments(args)?)
        .status()
        .map_err(|e| format!("{cargo}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("a story failed; see above".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{USAGE, arguments};

    #[test]
    fn a_mode_runs_both_story_binaries_with_the_filters() {
        let args = arguments(&["render", "find-bar/matches", "--scale", "2"]).unwrap();

        assert_eq!(
            args.join(" "),
            "test --quiet --no-fail-fast --package quickie-ui-widgets --package \
             quickie-ui-screens --test stories -- render find-bar/matches --scale 2"
        );
    }

    #[test]
    fn an_unknown_or_missing_mode_is_refused() {
        assert_eq!(arguments(&[]), Err(USAGE.to_owned()));
        assert_eq!(arguments(&["draw"]), Err(USAGE.to_owned()));
    }
}
