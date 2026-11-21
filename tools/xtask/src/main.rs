//! Repository checks that cargo and clippy cannot express: the rule of four, the layer map, and
//! commit messages. Run through `cargo xtask <command>`; the rules are in docs/quality.md.
//! Also `cargo xtask bundle`, which makes the development app a macOS app bundle, and
//! `cargo xtask base`, which prints the commit a branch's changes are measured from (the
//! merge-base `ui diff` and the mutation gate compare with).
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool reports to the terminal"
)]

mod bundle;
mod commit_msg;
mod layers;
mod structure;
mod ui;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

const USAGE: &str = "usage: cargo xtask <structure | layers | commit-msg <file> | \
                     bundle [--release] [--bundle-id <id>] [--version <X.Y.Z>] \
                     [--sign <identity>] [--dmg] | ui <list | render | check | accept> \
                     [<widget>[/<state>[.<theme>]] ...] | ui diff [--base <rev>] [--when-foreign] \
                     | base>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if let ["ui", rest @ ..] = args.as_slice() {
        return match ui::run(&cargo(), rest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask ui: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if let ["base"] = args.as_slice() {
        // The commit alone on stdout, for scripts; where it comes from on stderr, for people.
        return match ui::base_commit(None) {
            Ok((commit, label)) => {
                println!("{commit}");
                eprintln!("base: {label}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("xtask base: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if let ["bundle", rest @ ..] = args.as_slice() {
        return match run_bundle(rest) {
            Ok(made) => {
                for path in made {
                    println!("bundle: {}", path.display());
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("xtask: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let outcome = match args.as_slice() {
        ["structure"] => run_structure().map(|f| ("rule of four", f)),
        ["layers"] => run_layers().map(|f| ("layers", f)),
        ["commit-msg", file] => run_commit_msg(Path::new(file)).map(|f| ("commit message", f)),
        _ => Err(USAGE.to_owned()),
    };
    match outcome {
        Ok((check, findings)) if findings.is_empty() => {
            println!("{check}: ok");
            ExitCode::SUCCESS
        }
        Ok((check, findings)) => {
            for finding in &findings {
                eprintln!("{finding}");
            }
            eprintln!(
                "{check}: {} finding(s), see docs/quality.md",
                findings.len()
            );
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_structure() -> Result<Vec<String>, String> {
    let root = PathBuf::from(output("git", &["rev-parse", "--show-toplevel"])?.trim());
    let listed = output(
        "git",
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let paths: Vec<String> = listed
        .split('\0')
        .filter(|path| !path.is_empty() && root.join(path).exists())
        .map(str::to_owned)
        .collect();
    Ok(structure::check(&paths))
}

fn run_bundle(args: &[&str]) -> Result<Vec<PathBuf>, String> {
    let options = bundle::options(args)?;
    let cargo = cargo();
    let metadata = metadata(&cargo)?;
    let root = metadata["workspace_root"]
        .as_str()
        .ok_or("cargo metadata has no workspace_root")?;
    let target = metadata["target_directory"]
        .as_str()
        .ok_or("cargo metadata has no target_directory")?;
    bundle::run(&options, Path::new(root), Path::new(target), &cargo)
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

fn metadata(cargo: &str) -> Result<Value, String> {
    let metadata = output(cargo, &["metadata", "--format-version", "1", "--no-deps"])?;
    serde_json::from_str(&metadata).map_err(|e| e.to_string())
}

fn run_layers() -> Result<Vec<String>, String> {
    let metadata = metadata(&cargo())?;
    let root = metadata["workspace_root"]
        .as_str()
        .ok_or("cargo metadata has no workspace_root")?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata has no packages")?;
    let crates = packages
        .iter()
        .map(|package| read_crate(Path::new(root), package))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(layers::check(&crates))
}

fn run_commit_msg(file: &Path) -> Result<Vec<String>, String> {
    let message = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(commit_msg::check(&message))
}

fn read_crate(root: &Path, package: &Value) -> Result<layers::Crate, String> {
    let manifest = PathBuf::from(
        package["manifest_path"]
            .as_str()
            .ok_or("package without manifest_path")?,
    );
    let dir = manifest.parent().ok_or("manifest without a directory")?;
    let manifest_text =
        fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut sources = Vec::new();
    collect_sources(&dir.join("src"), &dir.join("src"), &mut sources)?;
    let dependencies = package["dependencies"]
        .as_array()
        .map_or_else(Vec::new, |deps| {
            deps.iter().map(|dep| read_dependency(root, dep)).collect()
        });
    Ok(layers::Crate {
        name: package["name"].as_str().unwrap_or_default().to_owned(),
        dir: relative(root, dir),
        dependencies,
        inherits_lints: layers::inherits_workspace_lints(&manifest_text),
        opts_into_unsafe: sources.iter().any(|s| layers::opts_into_unsafe(&s.text)),
        sources,
    })
}

fn read_dependency(root: &Path, dep: &Value) -> layers::Dependency {
    layers::Dependency {
        name: dep["name"].as_str().unwrap_or_default().to_owned(),
        internal_dir: dep["path"].as_str().map(|p| relative(root, Path::new(p))),
        normal: dep["kind"].is_null(),
    }
}

fn collect_sources(base: &Path, dir: &Path, out: &mut Vec<layers::Source>) -> Result<(), String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect_sources(base, &path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.push(layers::Source {
                path: relative(base, &path),
                text,
            });
        }
    }
    Ok(())
}

fn relative(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}
