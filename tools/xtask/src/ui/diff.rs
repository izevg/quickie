//! `cargo xtask ui diff [--base <rev>] [--when-foreign]`: renders every story at the base and
//! at `HEAD` on this machine and compares the two, so a visual change shows on any macOS, not
//! only on the one the references come from (docs/quality.md, UI tests).
//!
//! Everything lives in `target/ui-diff/`:
//! - `worktree/`: one detached worktree of the base, reused and moved to each new base.
//! - `target/`: the base's own target directory, so its build stays incremental. It cannot be
//!   the workspace's: cargo gives a workspace crate the same unit hash whatever its path, so
//!   the two builds would overwrite each other and pass each other's binaries as fresh.
//! - `base/<commit>-<macOS build>/<package>/`: the base's renders, made once per base and
//!   machine; only the newest is kept.
//! - `head/<package>/`: this run's renders, the diffs, and `changed.txt`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use super::STORY_CRATES;
use super::base::{self, Need};
use super::git;
use super::report::{self, Renders};

pub const USAGE: &str = "cargo xtask ui diff [--base <rev>] [--when-foreign]";

/// What `ui diff` was asked for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Options<'a> {
    base: Option<&'a str>,
    /// Do nothing where `cargo test` already compares with the references.
    when_foreign: bool,
}

fn options<'a>(args: &[&'a str]) -> Result<Options<'a>, String> {
    let mut options = Options::default();
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        match arg {
            "--base" => options.base = Some(*args.next().ok_or(USAGE)?),
            "--when-foreign" => options.when_foreign = true,
            _ => return Err(USAGE.to_owned()),
        }
    }
    Ok(options)
}

/// Runs the diff; an error when a changed render's reference is stale or a story failed.
pub fn run(cargo: &str, args: &[&str]) -> Result<(), String> {
    let options = options(args)?;
    let started = Instant::now();
    let root = PathBuf::from(git(None, &["rev-parse", "--show-toplevel"])?.trim());
    match need_here(&root) {
        Need::NoRenderer if options.when_foreign => {
            println!("ui diff: skipped, the stories render on macOS only");
            return Ok(());
        }
        Need::NoRenderer => return Err("the stories render on macOS only".to_owned()),
        Need::Compared if options.when_foreign => {
            println!(
                "ui diff: skipped, the references come from this macOS and `cargo test` \
                 compares with them"
            );
            return Ok(());
        }
        Need::Compared | Need::Diff => {}
    }

    let (commit, label) = base_commit(options.base)?;
    let short = commit.get(..10).unwrap_or(&commit);
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let work = target.join("ui-diff");
    let macos = sw_vers("-buildVersion")?;
    let base_renders = work.join("base").join(format!("{commit}-{}", macos.trim()));
    let cached = base_renders.join("complete").exists();
    if cached {
        println!("ui diff: base {short} ({label}), renders cached");
    } else {
        println!(
            "ui diff: base {short} ({label}), rendering it in {} (once per base)",
            work.join("worktree").display()
        );
        render_base(cargo, &root, &work, &commit, &base_renders)?;
    }

    let head = work.join("head");
    let stories_passed = render_head(cargo, &root, &base_renders, &head)?;

    let mut base_set = BTreeSet::new();
    let mut head_set = BTreeSet::new();
    let mut changed = BTreeSet::new();
    for (package, _) in STORY_CRATES {
        collect_renders(&base_renders.join(package), &mut base_set)?;
        collect_renders(&head.join(package), &mut head_set)?;
        if let Ok(text) = fs::read_to_string(head.join(package).join("changed.txt")) {
            changed.extend(text.lines().map(str::to_owned));
        }
    }
    let changed_paths = changed_paths(&root, &commit)?;
    let report = report::classify(&Renders {
        base: &base_set,
        head: &head_set,
        changed: &changed,
        changed_paths: &changed_paths,
    });
    print!("{report}");
    println!(
        "ui diff: {} renders against {short}, {} changed, in {:.1} s",
        head_set.len(),
        changed.len(),
        started.elapsed().as_secs_f64()
    );
    if !stories_passed {
        return Err("a story failed; see above".to_owned());
    }
    if report.passed() {
        Ok(())
    } else {
        Err("references are stale".to_owned())
    }
}

/// Whether this machine needs the diff: its renderer against the one each crate's references
/// record.
fn need_here(root: &Path) -> Need {
    let host = sw_vers("-productVersion").ok().and_then(|version| {
        version
            .split('.')
            .next()
            .map(|major| format!("macos-{major}"))
    });
    let references: Vec<Option<String>> = STORY_CRATES
        .iter()
        .map(|(_, dir)| {
            fs::read_to_string(root.join(dir).join("tests/snapshots/.renderer"))
                .ok()
                .map(|text| text.trim().to_owned())
        })
        .collect();
    let references: Vec<Option<&str>> = references.iter().map(Option::as_deref).collect();
    base::need(host.as_deref(), &references)
}

/// Renders the working tree's stories into `head` and compares each with its base render.
/// True when every story's plays passed.
fn render_head(cargo: &str, root: &Path, base_renders: &Path, head: &Path) -> Result<bool, String> {
    remove(head)?;
    // Built once, then both story binaries render side by side, each on its own main thread.
    let compiled = Command::new(cargo)
        .current_dir(root)
        .args(["test", "--quiet", "--no-run", "--test", "stories"])
        .args(
            STORY_CRATES
                .iter()
                .flat_map(|(package, _)| ["--package", package]),
        )
        .status()
        .map_err(|e| format!("{cargo}: {e}"))?;
    if !compiled.success() {
        return Err("the stories do not build; see above".to_owned());
    }
    let runs = STORY_CRATES
        .iter()
        .map(|(package, _)| {
            Command::new(cargo)
                .current_dir(root)
                .args(story_args(package))
                .args(["diff", "--base"])
                .arg(base_renders.join(package))
                .arg("--out")
                .arg(head.join(package))
                .spawn()
                .map_err(|e| format!("{cargo}: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut passed = true;
    for mut run in runs {
        passed &= run.wait().map_err(|e| e.to_string())?.success();
    }
    Ok(passed)
}

/// The commit to compare with and where it comes from. Also what `cargo xtask base` prints, so
/// the mutation gate diffs against the same base.
pub fn base_commit(requested: Option<&str>) -> Result<(String, String), String> {
    let (rev, label) = if let Some(rev) = requested {
        (rev.to_owned(), rev.to_owned())
    } else {
        let upstream = git(
            None,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )
        .ok()
        .map(|name| name.trim().to_owned());
        let remotes = git(
            None,
            &[
                "for-each-ref",
                "--format=%(refname:short)",
                "refs/remotes/origin",
            ],
        )?;
        let remotes: Vec<&str> = remotes.lines().collect();
        let branch = base::default_branch(upstream.as_deref(), &remotes)
            .ok_or("no upstream, origin/release/*, or origin/main to compare with; pass --base")?;
        let merge_base = git(None, &["merge-base", "HEAD", &branch])?;
        (
            merge_base.trim().to_owned(),
            format!("merge-base with {branch}"),
        )
    };
    let commit = git(
        None,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )?;
    Ok((commit.trim().to_owned(), label))
}

/// Moves the base worktree to `commit` and renders its stories into `renders`; on success
/// marks them complete and drops the renders of earlier bases.
fn render_base(
    cargo: &str,
    root: &Path,
    work: &Path,
    commit: &str,
    renders: &Path,
) -> Result<(), String> {
    let worktree = work.join("worktree");
    checkout(root, &worktree, commit)?;
    remove(renders)?;
    for (package, _) in STORY_CRATES {
        // The base's build scripts and tests may run git too.
        let out = git::in_dir(&mut Command::new(cargo), &worktree)
            .env("CARGO_TARGET_DIR", work.join("target"))
            .args(story_args(package))
            .arg("render")
            .arg("--out")
            .arg(renders.join(package))
            .output()
            .map_err(|e| format!("{cargo}: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "rendering the base failed:\n{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    fs::write(renders.join("complete"), commit)
        .map_err(|e| format!("{}: {e}", renders.display()))?;
    let Some(all) = renders.parent() else {
        return Ok(());
    };
    for entry in fs::read_dir(all).map_err(|e| format!("{}: {e}", all.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path != renders {
            remove(&path)?;
        }
    }
    Ok(())
}

/// One worktree, reused: moved to `commit` when it exists, made again when it is broken.
fn checkout(root: &Path, worktree: &Path, commit: &str) -> Result<(), String> {
    // A worktree deleted with `target/` stays registered until pruned.
    git(Some(root), &["worktree", "prune"])?;
    if worktree.join(".git").exists()
        && git(
            Some(worktree),
            &["checkout", "--quiet", "--force", "--detach", commit],
        )
        .is_ok()
    {
        return Ok(());
    }
    remove(worktree)?;
    git(Some(root), &["worktree", "prune"])?;
    let path = worktree.to_string_lossy();
    git(
        Some(root),
        &["worktree", "add", "--quiet", "--detach", &path, commit],
    )
    .map(drop)
}

/// The `cargo` arguments that run one crate's story binary; the mode follows.
fn story_args(package: &str) -> [&str; 7] {
    [
        "test",
        "--quiet",
        "--package",
        package,
        "--test",
        "stories",
        "--",
    ]
}

/// Adds the renders under `dir` (`<widget>/<state>.<theme>.png`) as `widget/state.theme`.
fn collect_renders(dir: &Path, out: &mut BTreeSet<String>) -> Result<(), String> {
    let Ok(widgets) = fs::read_dir(dir) else {
        return Ok(());
    };
    for widget in widgets {
        let widget = widget.map_err(|e| e.to_string())?.path();
        if !widget.is_dir() {
            continue;
        }
        let name = widget
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for file in fs::read_dir(&widget).map_err(|e| format!("{}: {e}", widget.display()))? {
            let file = file.map_err(|e| e.to_string())?.file_name();
            let file = file.to_string_lossy();
            if let Some(stem) = file.strip_suffix(".png")
                // `<state>.<theme>.diff.png` is a diff, not a render.
                && Path::new(stem).extension().is_none_or(|ext| ext != "diff")
            {
                out.insert(format!("{name}/{stem}"));
            }
        }
    }
    Ok(())
}

/// The paths that differ between `commit` and the working tree, untracked ones included, so
/// a reference accepted but not yet committed counts.
fn changed_paths(root: &Path, commit: &str) -> Result<Vec<String>, String> {
    let tracked = git(
        Some(root),
        &["diff", "--name-only", commit, "--", "crates/ui"],
    )?;
    let untracked = git(
        Some(root),
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "--",
            "crates/ui",
        ],
    )?;
    Ok(tracked
        .lines()
        .chain(untracked.lines())
        .map(str::to_owned)
        .collect())
}

fn remove(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

fn sw_vers(flag: &str) -> Result<String, String> {
    let out = Command::new("sw_vers")
        .arg(flag)
        .output()
        .map_err(|e| format!("sw_vers: {e}"))?;
    if out.status.success() {
        String::from_utf8(out.stdout).map_err(|e| e.to_string())
    } else {
        Err("sw_vers failed".to_owned())
    }
}

/// Runs git in `dir`, or where xtask was started; never on a hook's repository ([`git`]).
fn git(dir: Option<&Path>, args: &[&str]) -> Result<String, String> {
    git::run(dir.unwrap_or_else(|| Path::new(".")), args)
}

#[cfg(test)]
mod tests {
    use super::{Options, USAGE, options};

    #[test]
    fn without_arguments_the_default_base_is_used_every_time() {
        assert_eq!(options(&[]), Ok(Options::default()));
    }

    #[test]
    fn a_base_and_the_foreign_switch_are_read() {
        let parsed = options(&["--when-foreign", "--base", "origin/main"]);

        assert_eq!(
            parsed,
            Ok(Options {
                base: Some("origin/main"),
                when_foreign: true
            })
        );
    }

    #[test]
    fn a_base_without_a_rev_or_an_unknown_argument_is_refused() {
        assert_eq!(options(&["--base"]), Err(USAGE.to_owned()));
        assert_eq!(options(&["find-bar"]), Err(USAGE.to_owned()));
    }
}
