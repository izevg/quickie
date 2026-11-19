//! FSD: a slice must not use a sibling slice. Shared code moves down a layer instead.
//!
//! A slice is a top-level module of a sliced UI layer, or, where slices are grouped by meaning
//! (`widgets`), a module one level below its group: `palette/command_palette`. A group module
//! only declares its slices and belongs to none of them.

use std::collections::BTreeSet;

use super::Source;

/// Findings for the crate at `crate_dir` whose slices are named by their first `depth` modules.
pub fn check(crate_dir: &str, sources: &[Source], depth: usize) -> Vec<String> {
    let slices: BTreeSet<Vec<&str>> = sources
        .iter()
        .filter_map(|source| slice_of(&module_path(&source.path), depth))
        .collect();
    let mut findings = Vec::new();
    for source in sources {
        let path = module_path(&source.path);
        let Some(own) = slice_of(&path, depth) else {
            continue;
        };
        for (offset, target) in references(&source.text, &path, depth) {
            let Some(target) = slice_of(&target, depth) else {
                continue;
            };
            if target != own && slices.contains(&target) {
                let line = source.text[..offset].matches('\n').count() + 1;
                findings.push(format!(
                    "{crate_dir}/src/{}:{line}: slice `{}` must not use sibling slice `{}`",
                    source.path,
                    own.join("/"),
                    target.join("/"),
                ));
            }
        }
    }
    findings
}

/// The slice a module path lies in: its first `depth` modules, if it is that deep.
fn slice_of<'a>(path: &[&'a str], depth: usize) -> Option<Vec<&'a str>> {
    (path.len() >= depth).then(|| path[..depth].to_vec())
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

/// Paths from the crate root that `text`, the file of module `own`, names through `crate::` or
/// a run of `super::`, each with the offset where it starts and cut to `depth` modules. Grouped
/// imports (`crate::{a, b::c}`) yield one path per item.
fn references<'a>(text: &'a str, own: &[&'a str], depth: usize) -> Vec<(usize, Vec<&'a str>)> {
    let mut found = Vec::new();
    for prefix in ["crate::", "super::"] {
        let mut from = 0;
        while let Some(pos) = text[from..].find(prefix) {
            let at = from + pos;
            from = at + prefix.len();
            let preceded = text[..at].chars().next_back();
            if preceded.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':') {
                continue;
            }
            let base: &[&str] = if prefix == "crate::" {
                &[]
            } else {
                let mut supers = 1;
                while text[from..].starts_with(prefix) {
                    from += prefix.len();
                    supers += 1;
                }
                match own.len().checked_sub(supers) {
                    Some(kept) => &own[..kept],
                    None => continue,
                }
            };
            let need = depth.saturating_sub(base.len());
            for tail in paths_after(text, from, need) {
                found.push((at, base.iter().copied().chain(tail).collect()));
            }
        }
    }
    found
}

/// The paths written from `start`, each cut to `need` modules: `a::b::c` is one, `a::{b, c::d}`
/// is one per item.
fn paths_after(text: &str, start: usize, need: usize) -> Vec<Vec<&str>> {
    let mut prefix = Vec::new();
    let mut rest = &text[start..];
    while prefix.len() < need {
        if let Some(group) = rest.strip_prefix('{') {
            let left = need - prefix.len();
            return group_items(group)
                .into_iter()
                .flat_map(|item| paths_after(group, item, left))
                .map(|tail| prefix.iter().copied().chain(tail).collect())
                .collect();
        }
        let Some(name) = ident(rest) else {
            break;
        };
        prefix.push(name);
        match rest[name.len()..].strip_prefix("::") {
            Some(after) => rest = after,
            None => break,
        }
    }
    vec![prefix]
}

/// Offsets in `group` (the text after a `{`) where each top-level item starts.
fn group_items(group: &str) -> Vec<usize> {
    let mut items = Vec::new();
    let mut depth = 1;
    let mut expect_item = true;
    for (i, c) in group.char_indices() {
        match c {
            '{' if expect_item => {
                expect_item = false;
                items.push(i);
                depth += 1;
            }
            '{' => depth += 1,
            '}' if depth == 1 => break,
            '}' => depth -= 1,
            ',' if depth == 1 => expect_item = true,
            c if expect_item && !c.is_whitespace() => {
                expect_item = false;
                items.push(i);
            }
            _ => {}
        }
    }
    items
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
        check("crates/ui/features", &sources, 1)
    }

    fn run_grouped(files: &[(&str, &str)]) -> Vec<String> {
        let sources: Vec<Source> = files.iter().map(|(p, t)| source(p, t)).collect();
        check("crates/ui/widgets", &sources, 2)
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

    #[test]
    fn a_super_past_the_crate_root_names_nothing() {
        let findings = run(&[("copy_url.rs", "use super::super::find;"), ("find.rs", "")]);
        assert_eq!(findings, Vec::<String>::new(), "{findings:?}");
    }

    #[test]
    fn grouped_slices_are_named_group_and_slice() {
        let findings = run_grouped(&[
            (
                "lib.rs",
                "pub mod palette;\nuse crate::palette::address::X;",
            ),
            ("palette/mod.rs", "pub mod address;\npub mod commands;"),
            ("palette/address.rs", "use crate::palette::address::Own;"),
            (
                "palette/commands.rs",
                "\nuse crate::palette::address::Field;",
            ),
        ]);
        assert_eq!(
            findings,
            [
                "crates/ui/widgets/src/palette/commands.rs:2: slice `palette/commands` must not \
                 use sibling slice `palette/address`"
            ]
        );
    }

    #[test]
    fn grouped_slices_see_siblings_through_super_and_groups() {
        let findings = run_grouped(&[
            ("palette/address.rs", "use super::commands::Row;"),
            (
                "palette/commands/rows.rs",
                "use super::super::address::Field;\nuse super::own_thing;",
            ),
            ("palette/commands/mod.rs", "use super::{address, commands};"),
            (
                "tabs/switcher.rs",
                "use crate::{palette::{address::A, commands}, tabs::switcher::B};",
            ),
        ]);
        assert_eq!(findings.len(), 5, "{findings:?}");
    }

    #[test]
    fn a_group_module_belongs_to_no_slice() {
        let findings = run_grouped(&[
            ("palette/mod.rs", "use crate::tabs::switcher::X;"),
            ("palette/address.rs", "use crate::palette;"),
            ("tabs/switcher.rs", ""),
        ]);
        assert_eq!(findings, Vec::<String>::new(), "{findings:?}");
    }
}
