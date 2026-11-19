//! The rule of four: at most 4 files and 8 subdirectories per directory, hidden paths not counted,
//! and only the files the ecosystem requires in the repository root.

use std::collections::{BTreeMap, BTreeSet};

const MAX_FILES: usize = 4;
const MAX_DIRS: usize = 8;
const ROOT_FILES: &[&str] = &[
    "LICENSE",
    "README.md",
    "AGENTS.md",
    "CLAUDE.md",
    "Cargo.toml",
    "Cargo.lock",
];

/// Checks repository-relative file paths (`/`-separated) and returns one finding per violation.
pub fn check(paths: &[String]) -> Vec<String> {
    let mut findings = Vec::new();
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    let mut dirs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for path in paths {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.iter().any(|part| part.starts_with('.')) {
            continue;
        }
        let Some((name, dir_parts)) = parts.split_last() else {
            continue;
        };
        if dir_parts.is_empty() {
            if !ROOT_FILES.contains(name) {
                findings.push(format!("{path}: not an allowed root file"));
            }
            continue;
        }
        *files.entry(dir_parts.join("/")).or_default() += 1;
        for depth in 0..dir_parts.len() {
            let parent = if depth == 0 {
                ".".to_owned()
            } else {
                dir_parts[..depth].join("/")
            };
            dirs.entry(parent)
                .or_default()
                .insert(dir_parts[..=depth].join("/"));
        }
    }

    for (dir, count) in files {
        if count > MAX_FILES {
            findings.push(format!("{dir}: {count} files (max {MAX_FILES})"));
        }
    }
    for (dir, children) in dirs {
        if children.len() > MAX_DIRS {
            findings.push(format!(
                "{dir}: {} subdirectories (max {MAX_DIRS})",
                children.len()
            ));
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::check;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| (*p).to_owned()).collect()
    }

    #[test]
    fn accepts_allowed_root_files_and_four_files_per_directory() {
        let tree = paths(&[
            "README.md",
            "Cargo.toml",
            "docs/a.md",
            "docs/b.md",
            "docs/c.md",
            "docs/d.md",
        ]);
        assert_eq!(check(&tree), Vec::<String>::new());
    }

    #[test]
    fn rejects_other_root_files() {
        assert_eq!(
            check(&paths(&["rustfmt.toml"])),
            ["rustfmt.toml: not an allowed root file"]
        );
    }

    #[test]
    fn rejects_a_fifth_file() {
        let tree = paths(&["docs/a", "docs/b", "docs/c", "docs/d", "docs/e"]);
        assert_eq!(check(&tree), ["docs: 5 files (max 4)"]);
    }

    #[test]
    fn rejects_a_ninth_subdirectory_counting_each_once() {
        let mut tree: Vec<String> = (0..9).map(|i| format!("crates/c{i}/src/lib.rs")).collect();
        tree.push("crates/c0/Cargo.toml".to_owned());
        assert_eq!(check(&tree), ["crates: 9 subdirectories (max 8)"]);
    }

    #[test]
    fn accepts_eight_subdirectories() {
        let tree: Vec<String> = (0..8).map(|i| format!("crates/c{i}/lib.rs")).collect();
        assert_eq!(check(&tree), Vec::<String>::new());
    }

    #[test]
    fn ignores_hidden_files_and_directories() {
        let tree = paths(&[
            ".mise.toml",
            ".githooks/a",
            ".githooks/b",
            ".githooks/c",
            ".githooks/d",
            ".githooks/e",
            "docs/.x/a",
        ]);
        assert_eq!(check(&tree), Vec::<String>::new());
    }

    #[test]
    fn root_directories_count_towards_the_root() {
        let tree: Vec<String> = (0..9).map(|i| format!("d{i}/f")).collect();
        assert_eq!(check(&tree), [".: 9 subdirectories (max 8)"]);
    }
}
