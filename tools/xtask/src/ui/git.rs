//! Git as `ui diff` and `cargo xtask base` run it: always on the repository its directory
//! belongs to, never on the one a hook's environment points at.
//!
//! git exports `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE` and their kin into every hook. A
//! child git inherits them and acts on the repository they name, whatever its directory: run
//! from the pre-push hook, `git checkout --detach <base>` in the base worktree detached the
//! pushing worktree's `HEAD` instead (issue #75).

use std::path::Path;
use std::process::Command;

/// The variables that locate a repository, as `git rev-parse --local-env-vars` lists them; git
/// itself clears them before it runs a command in another repository, such as a submodule.
pub const LOCATING: [&str; 15] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// `command` without the hook's repository: run in `dir`, it finds the repository from there.
/// Also for cargo in the base worktree, whose build scripts and tests may call git.
pub fn in_dir<'a>(command: &'a mut Command, dir: &Path) -> &'a mut Command {
    for name in LOCATING {
        command.env_remove(name);
    }
    command.current_dir(dir)
}

/// Runs git with `args` in `dir`; its stdout, or its stderr as the error.
pub fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = in_dir(&mut Command::new("git"), dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{LOCATING, in_dir, run};

    #[test]
    fn a_hooks_repository_variables_do_not_reach_the_command() {
        let mut command = Command::new("git");
        command
            .env("GIT_DIR", "/pushing/.git")
            .env("GIT_WORK_TREE", "/pushing")
            .env("GIT_INDEX_FILE", "/pushing/.git/index");

        in_dir(&mut command, Path::new("/base"));

        let envs: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
        for name in LOCATING {
            assert!(
                envs.contains(&(OsStr::new(name), None)),
                "{name} is not removed"
            );
        }
    }

    #[test]
    fn the_command_runs_in_the_given_directory() {
        let mut command = Command::new("git");

        in_dir(&mut command, Path::new("/base"));

        assert_eq!(command.get_current_dir(), Some(Path::new("/base")));
    }

    /// A repository on `main` with two commits and a second worktree detached at the first,
    /// like `ui diff`'s base worktree; returns its two directories and the first commit.
    fn repository_with_a_base_worktree(name: &str) -> (PathBuf, PathBuf, String) {
        let scratch = std::env::temp_dir().join(format!("xtask-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let (main, base) = (scratch.join("main"), scratch.join("base"));
        fs::create_dir_all(&main).unwrap();
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "-c"];
        let commit = [&commit[..], &["commit.gpgsign=false", "commit", "--quiet"]].concat();
        run(&main, &["init", "--quiet", "--initial-branch=main"]).unwrap();
        run(
            &main,
            &[&commit[..], &["--allow-empty", "-m", "first"]].concat(),
        )
        .unwrap();
        let first = run(&main, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_owned();
        run(
            &main,
            &[&commit[..], &["--allow-empty", "-m", "second"]].concat(),
        )
        .unwrap();
        let base_path = base.to_string_lossy();
        run(
            &main,
            &["worktree", "add", "--quiet", "--detach", &base_path, "HEAD"],
        )
        .unwrap();
        (main, base, first)
    }

    #[test]
    fn a_checkout_in_the_base_worktree_leaves_the_hooks_branch_alone() {
        let (main, base, first) = repository_with_a_base_worktree("checkout");
        let mut checkout = Command::new("git");
        checkout.env("GIT_DIR", main.join(".git"));
        checkout.env("GIT_WORK_TREE", &main);

        let status = in_dir(&mut checkout, &base)
            .args(["checkout", "--quiet", "--force", "--detach", &first])
            .status()
            .unwrap();

        assert!(status.success());
        let branch = run(&main, &["symbolic-ref", "--short", "HEAD"]).unwrap();
        assert_eq!(branch.trim(), "main");
        let moved = run(&base, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(moved.trim(), first);
        fs::remove_dir_all(main.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_failing_command_reports_its_arguments() {
        let error = run(Path::new("/"), &["no-such-command"]);

        assert!(error.is_err_and(|e| e.starts_with("git no-such-command: ")));
    }
}
