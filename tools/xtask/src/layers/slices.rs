//! FSD: a slice (a top-level module of a sliced UI layer) must not use a sibling slice. Shared code
//! moves down a layer instead.

use std::collections::BTreeSet;

use super::Source;

pub fn check(crate_dir: &str, sources: &[Source]) -> Vec<String> {
    let slices: BTreeSet<&str> = sources
        .iter()
        .filter_map(|source| module_path(&source.path).first().copied())
        .collect();
    let mut findings = Vec::new();
    for source in sources {
        let path = module_path(&source.path);
        let Some(own) = path.first() else {
            continue;
        };
        for (offset, target) in crate_root_references(&source.text, path.len()) {
            if target != *own && slices.contains(target) {
                let line = source.text[..offset].matches('\n').count() + 1;
                findings.push(format!(
                    "{crate_dir}/src/{}:{line}: slice `{own}` must not use sibling slice `{target}`",
                    source.path
                ));
            }
        }
    }
    findings
}

/// Module path of a file below `src/`: `lib.rs` → [], `a.rs` → [a], `a/mod.rs` → [a],
/// `a/b.rs` → [a, b].
fn module_path(path: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = path.split('/').collect();
    if let Some(last) = parts.pop() {
        let stem = last.strip_suffix(".rs").unwrap_or(last);
        if !matches!(stem, "mod" | "lib" | "main") {
            parts.push(stem);
        }
    }
    parts
}

/// Names that resolve against the crate root: after `crate::`, or after as many `super::` as the
/// file is deep. Grouped imports (`crate::{a, b::c}`) yield each top-level name.
fn crate_root_references(text: &str, depth: usize) -> Vec<(usize, &str)> {
    let mut found = Vec::new();
    let supers = "super::".repeat(depth);
    for prefix in ["crate::", supers.as_str()] {
        if prefix.is_empty() {
            continue;
        }
        let mut from = 0;
        while let Some(pos) = text[from..].find(prefix) {
            let at = from + pos;
            from = at + prefix.len();
            let preceded = text[..at].chars().next_back();
            if preceded.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':') {
                continue;
            }
            found.extend(names_after(text, from).into_iter().map(|name| (at, name)));
        }
    }
    found
}

fn names_after(text: &str, start: usize) -> Vec<&str> {
    let rest = &text[start..];
    let Some(group) = rest.strip_prefix('{') else {
        return ident(rest).into_iter().collect();
    };
    let mut names = Vec::new();
    let mut depth = 1;
    let mut expect_name = true;
    for (i, c) in group.char_indices() {
        match c {
            '{' => depth += 1,
            '}' if depth == 1 => break,
            '}' => depth -= 1,
            ',' if depth == 1 => expect_name = true,
            c if expect_name && !c.is_whitespace() => {
                expect_name = false;
                names.extend(ident(&group[i..]));
            }
            _ => {}
        }
    }
    names
}

fn ident(text: &str) -> Option<&str> {
    let end = text
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

#[cfg(test)]
mod tests {
    use super::{Source, check};

    fn source(path: &str, text: &str) -> Source {
        Source {
            path: path.to_owned(),
            text: text.to_owned(),
        }
    }

    fn run(files: &[(&str, &str)]) -> Vec<String> {
        let sources: Vec<Source> = files.iter().map(|(p, t)| source(p, t)).collect();
        check("crates/ui/features", &sources)
    }

    #[test]
    fn accepts_own_slice_and_lower_layers() {
        let findings = run(&[
            (
                "lib.rs",
                "pub mod copy_url;\npub mod find;\nuse crate::find::Bar;",
            ),
            (
                "copy_url.rs",
                "use crate::copy_url::x;\nuse quickie_ui_entities::TabRow;",
            ),
            (
                "copy_url/inner.rs",
                "use super::helper;\nuse crate::copy_url::y;",
            ),
            ("find.rs", "use super::super_thing;"),
        ]);
        assert_eq!(findings, Vec::<String>::new(), "{findings:?}");
    }

    #[test]
    fn rejects_sibling_use_through_crate() {
        let findings = run(&[("copy_url.rs", "\nuse crate::find::Bar;"), ("find.rs", "")]);
        assert_eq!(
            findings,
            [
                "crates/ui/features/src/copy_url.rs:2: slice `copy_url` must not use sibling slice `find`"
            ]
        );
    }

    #[test]
    fn rejects_sibling_use_through_super_at_slice_root() {
        let findings = run(&[
            ("copy_url.rs", "use super::find::Bar;"),
            (
                "copy_url/deep.rs",
                "use super::super::find::Bar;\nuse super::find_local;",
            ),
            ("find/mod.rs", ""),
        ]);
        assert_eq!(findings.len(), 2, "{findings:?}");
    }

    #[test]
    fn rejects_siblings_inside_grouped_imports() {
        let findings = run(&[
            (
                "copy_url.rs",
                "use crate::{\n    copy_url::x,\n    find::{Bar, Baz},\n};",
            ),
            ("find.rs", ""),
        ]);
        assert_eq!(findings.len(), 1, "{findings:?}");
    }

    #[test]
    fn ignores_lookalike_paths() {
        let findings = run(&[
            (
                "copy_url.rs",
                "use other_crate::find::Bar;\nuse my_crate::find;\nuse ::crate_x;",
            ),
            ("find.rs", ""),
        ]);
        assert_eq!(findings, Vec::<String>::new(), "{findings:?}");
    }
}
