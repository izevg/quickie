//! The layer map: which crate may depend on which, which layers stay pure, where `unsafe` may be
//! opted into, and the FSD rule that slices of one layer do not reach into each other.

mod policy;
mod slices;

pub use policy::Layer;

/// A workspace crate as the check sees it.
pub struct Crate {
    pub name: String,
    /// Directory relative to the workspace root, `/`-separated.
    pub dir: String,
    pub dependencies: Vec<Dependency>,
    pub inherits_lints: bool,
    pub opts_into_unsafe: bool,
    pub sources: Vec<Source>,
}

pub struct Dependency {
    pub name: String,
    /// Directory relative to the workspace root for path dependencies.
    pub internal_dir: Option<String>,
    /// A normal (not dev, not build) dependency.
    pub normal: bool,
}

pub struct Source {
    /// Path relative to the crate's `src/`, `/`-separated.
    pub path: String,
    pub text: String,
}

pub fn check(crates: &[Crate]) -> Vec<String> {
    let mut findings = Vec::new();
    for krate in crates {
        let Some(layer) = Layer::of(&krate.dir) else {
            findings.push(format!(
                "{}: crate `{}` is outside the layer map",
                krate.dir, krate.name
            ));
            continue;
        };
        if !krate.inherits_lints {
            findings.push(format!(
                "{}/Cargo.toml: add `[lints] workspace = true`",
                krate.dir
            ));
        }
        if krate.opts_into_unsafe && !layer.may_use_unsafe() {
            findings.push(format!(
                "{}: {layer} must not opt into unsafe_code",
                krate.dir
            ));
        }
        for dep in krate.dependencies.iter().filter(|dep| dep.normal) {
            if let Some(finding) = check_dependency(krate, layer, dep) {
                findings.push(finding);
            }
        }
        let depth = layer.slice_depth();
        if depth > 0 {
            findings.extend(slices::check(&krate.dir, &krate.sources, depth));
        }
    }
    findings
}

fn check_dependency(krate: &Crate, layer: Layer, dep: &Dependency) -> Option<String> {
    match &dep.internal_dir {
        Some(dir) => match Layer::of(dir) {
            Some(target) if layer.may_depend_on(target) => None,
            Some(target) => Some(format!(
                "{}: {layer} must not depend on {target} (`{}`)",
                krate.dir, dep.name
            )),
            None => Some(format!(
                "{}: `{}` at {dir} is outside the layer map",
                krate.dir, dep.name
            )),
        },
        None => layer
            .external_allowlist()
            .filter(|allowed| !allowed.contains(&dep.name.as_str()))
            .map(|_| {
                format!(
                    "{}: external crate `{}` is not allowed in {layer}",
                    krate.dir, dep.name
                )
            }),
    }
}

/// True when the manifest has `workspace = true` in its `[lints]` table.
pub fn inherits_workspace_lints(manifest: &str) -> bool {
    let mut in_lints = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_lints = line == "[lints]";
        } else if in_lints && line.replace(' ', "") == "workspace=true" {
            return true;
        }
    }
    manifest
        .lines()
        .any(|line| line.replace(' ', "") == "lints.workspace=true")
}

/// True when a source file opts into `unsafe_code` through an attribute.
pub fn opts_into_unsafe(text: &str) -> bool {
    text.lines()
        .map(str::trim_start)
        .any(|line| line.starts_with('#') && line.contains("unsafe_code"))
}

#[cfg(test)]
mod tests {
    use super::{Crate, Dependency, Source, check, inherits_workspace_lints, opts_into_unsafe};

    fn krate(dir: &str, deps: &[(&str, Option<&str>)]) -> Crate {
        Crate {
            name: dir.rsplit('/').next().unwrap_or(dir).to_owned(),
            dir: dir.to_owned(),
            dependencies: deps
                .iter()
                .map(|(name, path)| Dependency {
                    name: (*name).to_owned(),
                    internal_dir: path.map(str::to_owned),
                    normal: true,
                })
                .collect(),
            inherits_lints: true,
            opts_into_unsafe: false,
            sources: Vec::new(),
        }
    }

    #[test]
    fn accepts_inward_dependencies() {
        let crates = [
            krate("crates/domain/kernel", &[("thiserror", None)]),
            krate(
                "crates/domain/browsing",
                &[("k", Some("crates/domain/kernel")), ("indexmap", None)],
            ),
            krate(
                "crates/application",
                &[("b", Some("crates/domain/browsing"))],
            ),
            krate(
                "crates/adapters/store",
                &[("a", Some("crates/application")), ("rusqlite", None)],
            ),
            krate("crates/ui/widgets", &[("f", Some("crates/ui/features"))]),
            krate("crates/shell", &[("s", Some("crates/adapters/store"))]),
        ];
        assert_eq!(check(&crates), Vec::<String>::new());
    }

    #[test]
    fn rejects_outward_dependencies() {
        let crates = [
            krate(
                "crates/domain/browsing",
                &[("a", Some("crates/application"))],
            ),
            krate(
                "crates/application",
                &[("s", Some("crates/adapters/store"))],
            ),
            krate("crates/ui/entities", &[("w", Some("crates/ui/widgets"))]),
        ];
        assert_eq!(check(&crates).len(), 3);
    }

    #[test]
    fn rejects_impure_externals_in_the_core_only() {
        let crates = [
            krate("crates/domain/kernel", &[("tokio", None)]),
            krate("crates/application", &[("objc2", None)]),
            krate("crates/adapters/store", &[("tokio", None)]),
        ];
        assert_eq!(
            check(&crates),
            [
                "crates/domain/kernel: external crate `tokio` is not allowed in domain kernel",
                "crates/application: external crate `objc2` is not allowed in application",
            ]
        );
    }

    #[test]
    fn ignores_dev_dependencies() {
        let mut core = krate("crates/domain/kernel", &[("proptest", None)]);
        core.dependencies[0].normal = false;
        assert_eq!(check(&[core]), Vec::<String>::new());
    }

    #[test]
    fn rejects_crates_and_dependencies_outside_the_map() {
        let crates = [
            krate("misc", &[]),
            krate("crates/shell", &[("x", Some("vendor/x"))]),
        ];
        assert_eq!(check(&crates).len(), 2);
    }

    #[test]
    fn requires_workspace_lints() {
        let mut core = krate("crates/application", &[]);
        core.inherits_lints = false;
        assert_eq!(
            check(&[core]),
            ["crates/application/Cargo.toml: add `[lints] workspace = true`"]
        );
    }

    #[test]
    fn allows_unsafe_only_at_the_platform_edge() {
        let mut core = krate("crates/domain/kernel", &[]);
        core.opts_into_unsafe = true;
        let mut edge = krate("crates/adapters/system", &[]);
        edge.opts_into_unsafe = true;
        assert_eq!(
            check(&[core, edge]),
            ["crates/domain/kernel: domain kernel must not opt into unsafe_code"]
        );
    }

    #[test]
    fn checks_slices_in_sliced_ui_layers() {
        let mut features = krate("crates/ui/features", &[]);
        features.sources = vec![
            Source {
                path: "copy_url.rs".to_owned(),
                text: "use crate::find::Bar;".to_owned(),
            },
            Source {
                path: "find.rs".to_owned(),
                text: String::new(),
            },
        ];
        assert_eq!(check(&[features]).len(), 1);
    }

    #[test]
    fn reads_workspace_lints_from_manifests() {
        assert!(inherits_workspace_lints(
            "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n"
        ));
        assert!(inherits_workspace_lints("lints.workspace = true\n"));
        assert!(!inherits_workspace_lints(
            "[lints.clippy]\nworkspace = true\n"
        ));
        assert!(!inherits_workspace_lints(
            "[lints]\n[package]\nworkspace = true\n"
        ));
    }

    #[test]
    fn detects_unsafe_opt_in() {
        assert!(opts_into_unsafe(
            "#![expect(unsafe_code, reason = \"ffi\")]\n"
        ));
        assert!(opts_into_unsafe("  #[allow(unsafe_code)]\nfn f() {}"));
        assert!(!opts_into_unsafe("// unsafe_code is denied\n"));
    }
}
