//! What `ui diff` found, decided without I/O. A render is named `widget/state.theme`, as a
//! story filter names it; its reference is `<crate>/tests/snapshots/<group>/<widget>/<state>/
//! <theme>.png`.

use std::collections::BTreeSet;
use std::fmt::{self, Display, Formatter};

/// The renders at the base and at `HEAD`, both made on this machine, and what changed.
pub struct Renders<'a> {
    /// Every render of the base.
    pub base: &'a BTreeSet<String>,
    /// Every render of `HEAD`.
    pub head: &'a BTreeSet<String>,
    /// The renders of `HEAD` that differ from the base's beyond the tolerance.
    pub changed: &'a BTreeSet<String>,
    /// The repository paths that differ between the base and the working tree.
    pub changed_paths: &'a [String],
}

/// The renders `ui diff` reports, each list sorted.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Changed, but the reference is the base's: CI will fail on it.
    pub stale: Vec<String>,
    /// Changed, and the reference changed too (re-accepted, or removed for CI to render).
    pub accepted: Vec<String>,
    /// Only `HEAD` has them.
    pub new: Vec<String>,
    /// Only the base has them.
    pub removed: Vec<String>,
}

/// Sorts the renders into the report.
#[must_use]
pub fn classify(renders: &Renders) -> Report {
    let (accepted, stale) = renders
        .changed
        .iter()
        .filter(|render| renders.base.contains(*render))
        .cloned()
        .partition(|render| reference_changed(render, renders.changed_paths));
    Report {
        stale,
        accepted,
        new: renders.head.difference(renders.base).cloned().collect(),
        removed: renders.base.difference(renders.head).cloned().collect(),
    }
}

/// True when one of `paths` is the reference of `render`.
fn reference_changed(render: &str, paths: &[String]) -> bool {
    let Some((widget, state_theme)) = render.split_once('/') else {
        return false;
    };
    let Some((state, theme)) = state_theme.rsplit_once('.') else {
        return false;
    };
    let ending = format!("/{widget}/{state}/{theme}.png");
    paths
        .iter()
        .any(|path| path.contains("/tests/snapshots/") && path.ends_with(&ending))
}

impl Report {
    /// False when a changed render would reach CI with a stale reference.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.stale.is_empty()
    }
}

impl Display for Report {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for (label, renders) in [
            ("changed, reference re-accepted", &self.accepted),
            ("new", &self.new),
            ("removed", &self.removed),
            ("CHANGED, reference stale", &self.stale),
        ] {
            for render in renders {
                writeln!(f, "  {label}: {render}")?;
            }
        }
        if !self.passed() {
            let list = self.stale.join(" ");
            writeln!(
                f,
                "{} stories changed but their references were not re-accepted: {}; push, then \
                 accept the CI renders (cargo xtask ui accept --from <ui-renders> {list}).\n\
                 To push before CI has rendered them, remove those references (git rm): the \
                 change is then on record, and the CI run fails on the missing references and \
                 uploads their renders.",
                self.stale.len(),
                self.stale.join(", "),
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Renders, Report, classify};

    fn set(renders: &[&str]) -> BTreeSet<String> {
        renders.iter().map(|&render| render.to_owned()).collect()
    }

    const REFERENCE: &str = "crates/ui/widgets/tests/snapshots/find/find-bar/matches/light.png";

    #[test]
    fn a_changed_render_with_its_reference_unchanged_is_stale() {
        let all = set(&["find-bar/matches.light", "find-bar/matches.dark"]);
        let changed = set(&["find-bar/matches.light"]);
        let paths = ["crates/ui/widgets/src/find_bar.rs".to_owned()];

        let report = classify(&Renders {
            base: &all,
            head: &all,
            changed: &changed,
            changed_paths: &paths,
        });

        assert_eq!(report.stale, ["find-bar/matches.light"]);
        assert_eq!(report.accepted, Vec::<String>::new());
        assert!(!report.passed());
    }

    #[test]
    fn a_changed_render_with_its_reference_changed_is_accepted() {
        let all = set(&["find-bar/matches.light"]);
        let paths = [REFERENCE.to_owned()];

        let report = classify(&Renders {
            base: &all,
            head: &all,
            changed: &all,
            changed_paths: &paths,
        });

        assert_eq!(report.accepted, ["find-bar/matches.light"]);
        assert!(report.passed());
    }

    #[test]
    fn another_theme_of_the_same_state_does_not_accept_a_render() {
        let all = set(&["find-bar/matches.dark"]);
        let paths = [REFERENCE.to_owned()];

        let report = classify(&Renders {
            base: &all,
            head: &all,
            changed: &all,
            changed_paths: &paths,
        });

        assert_eq!(report.stale, ["find-bar/matches.dark"]);
    }

    #[test]
    fn a_path_outside_the_snapshots_does_not_accept_a_render() {
        let all = set(&["find-bar/matches.light"]);
        let paths = ["docs/find-bar/matches/light.png".to_owned()];

        let report = classify(&Renders {
            base: &all,
            head: &all,
            changed: &all,
            changed_paths: &paths,
        });

        assert_eq!(report.stale, ["find-bar/matches.light"]);
    }

    #[test]
    fn a_render_without_a_state_or_theme_matches_no_reference() {
        let all = set(&["find-bar", "find-bar/matches"]);
        let paths = [REFERENCE.to_owned()];

        let report = classify(&Renders {
            base: &all,
            head: &all,
            changed: &all,
            changed_paths: &paths,
        });

        assert_eq!(report.stale, ["find-bar", "find-bar/matches"]);
    }

    #[test]
    fn renders_only_at_head_are_new_and_only_at_the_base_removed() {
        let base = set(&["tabs/one.light", "tabs/gone.light"]);
        let head = set(&["tabs/one.light", "tabs/fresh.light"]);

        let report = classify(&Renders {
            base: &base,
            head: &head,
            changed: &BTreeSet::new(),
            changed_paths: &[],
        });

        assert_eq!(
            report,
            Report {
                new: vec!["tabs/fresh.light".to_owned()],
                removed: vec!["tabs/gone.light".to_owned()],
                ..Report::default()
            }
        );
        assert!(report.passed());
    }

    #[test]
    fn a_changed_render_the_base_lacks_counts_as_new_only() {
        let base = BTreeSet::new();
        let head = set(&["tabs/fresh.light"]);

        let report = classify(&Renders {
            base: &base,
            head: &head,
            changed: &head,
            changed_paths: &[],
        });

        assert_eq!(report.new, ["tabs/fresh.light"]);
        assert_eq!(report.stale, Vec::<String>::new());
    }

    #[test]
    fn the_report_names_each_render_and_how_to_accept_the_stale_ones() {
        let report = Report {
            stale: vec!["a/x.light".to_owned(), "b/y.dark".to_owned()],
            accepted: vec!["c/z.light".to_owned()],
            new: vec!["d/n.light".to_owned()],
            removed: vec!["e/r.light".to_owned()],
        };

        let text = report.to_string();

        assert_eq!(
            text.lines().take(5).collect::<Vec<_>>(),
            [
                "  changed, reference re-accepted: c/z.light",
                "  new: d/n.light",
                "  removed: e/r.light",
                "  CHANGED, reference stale: a/x.light",
                "  CHANGED, reference stale: b/y.dark",
            ]
        );
        assert!(text.contains(
            "2 stories changed but their references were not re-accepted: a/x.light, b/y.dark; \
             push, then accept the CI renders (cargo xtask ui accept --from <ui-renders> \
             a/x.light b/y.dark)."
        ));
    }

    #[test]
    fn a_passing_report_gives_no_advice() {
        let report = Report {
            accepted: vec!["c/z.light".to_owned()],
            ..Report::default()
        };

        assert_eq!(
            report.to_string(),
            "  changed, reference re-accepted: c/z.light\n"
        );
    }
}
