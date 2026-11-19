//! Where each crate sits and what each layer may use. The table is mirrored in docs/quality.md.

use std::fmt;

/// External crates the pure core (domain and application) may use.
/// The fuzzy matcher is `nucleo-matcher`, not `nucleo`: the latter runs its own thread pool.
const PURE_EXTERNALS: &[&str] = &["thiserror", "indexmap", "nucleo-matcher", "serde", "url"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Kernel,
    Context,
    Application,
    Adapter,
    Ui(UiLayer),
    Shell,
    Tool,
}

/// Feature-Sliced Design layers, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UiLayer {
    Shared,
    Entities,
    Features,
    Widgets,
    Screens,
    App,
}

impl Layer {
    /// The layer of a crate directory, or `None` when the directory is outside the map.
    pub fn of(dir: &str) -> Option<Self> {
        let parts: Vec<&str> = dir.split('/').collect();
        match parts.as_slice() {
            ["crates", "domain", "kernel"] => Some(Self::Kernel),
            ["crates", "domain", _] => Some(Self::Context),
            ["crates", "application"] => Some(Self::Application),
            ["crates", "adapters", _] => Some(Self::Adapter),
            ["crates", "ui", name] => UiLayer::named(name).map(Self::Ui),
            ["crates", "shell"] => Some(Self::Shell),
            ["tools", _] => Some(Self::Tool),
            _ => None,
        }
    }

    pub fn may_depend_on(self, target: Self) -> bool {
        match (self, target) {
            (Self::Context | Self::Ui(_), Self::Kernel)
            | (Self::Application | Self::Adapter | Self::Ui(_), Self::Kernel | Self::Context)
            | (Self::Adapter, Self::Application) => true,
            (Self::Ui(own), Self::Ui(other)) => other < own,
            (Self::Ui(own), Self::Application) => own >= UiLayer::Features,
            (Self::Shell, target) => !matches!(target, Self::Shell | Self::Tool),
            _ => false,
        }
    }

    pub fn may_use_unsafe(self) -> bool {
        matches!(self, Self::Adapter | Self::Ui(_) | Self::Shell)
    }

    pub fn external_allowlist(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Kernel | Self::Context | Self::Application => Some(PURE_EXTERNALS),
            _ => None,
        }
    }

    /// How many leading modules name a slice: none outside the sliced UI layers, `group/slice`
    /// in widgets (whose slices are grouped by meaning to keep the rule of four), one elsewhere.
    pub fn slice_depth(self) -> usize {
        match self {
            Self::Ui(UiLayer::Widgets) => 2,
            Self::Ui(UiLayer::Entities | UiLayer::Features | UiLayer::Screens) => 1,
            _ => 0,
        }
    }
}

impl UiLayer {
    fn named(name: &str) -> Option<Self> {
        match name {
            "shared" => Some(Self::Shared),
            "entities" => Some(Self::Entities),
            "features" => Some(Self::Features),
            "widgets" => Some(Self::Widgets),
            "screens" => Some(Self::Screens),
            "app" => Some(Self::App),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Entities => "entities",
            Self::Features => "features",
            Self::Widgets => "widgets",
            Self::Screens => "screens",
            Self::App => "app",
        }
    }
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel => f.write_str("domain kernel"),
            Self::Context => f.write_str("domain context"),
            Self::Application => f.write_str("application"),
            Self::Adapter => f.write_str("adapter"),
            Self::Ui(layer) => write!(f, "ui {}", layer.name()),
            Self::Shell => f.write_str("shell"),
            Self::Tool => f.write_str("tool"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Layer, UiLayer};

    #[test]
    fn maps_directories_to_layers() {
        assert_eq!(Layer::of("crates/domain/kernel"), Some(Layer::Kernel));
        assert_eq!(Layer::of("crates/domain/browsing"), Some(Layer::Context));
        assert_eq!(Layer::of("crates/application"), Some(Layer::Application));
        assert_eq!(Layer::of("crates/adapters/store"), Some(Layer::Adapter));
        assert_eq!(
            Layer::of("crates/ui/widgets"),
            Some(Layer::Ui(UiLayer::Widgets))
        );
        assert_eq!(Layer::of("crates/shell"), Some(Layer::Shell));
        assert_eq!(Layer::of("tools/xtask"), Some(Layer::Tool));
        assert_eq!(Layer::of("crates/ui/pages"), None);
        assert_eq!(Layer::of("crates/domain/browsing/extra"), None);
        assert_eq!(Layer::of("crates"), None);
    }

    #[test]
    fn domain_contexts_are_isolated_from_each_other() {
        assert!(Layer::Context.may_depend_on(Layer::Kernel));
        assert!(!Layer::Context.may_depend_on(Layer::Context));
        assert!(!Layer::Kernel.may_depend_on(Layer::Kernel));
    }

    #[test]
    fn ui_depends_downward_and_reaches_use_cases_from_features_up() {
        let ui = Layer::Ui;
        assert!(ui(UiLayer::App).may_depend_on(ui(UiLayer::Shared)));
        assert!(!ui(UiLayer::Shared).may_depend_on(ui(UiLayer::Shared)));
        assert!(!ui(UiLayer::Entities).may_depend_on(Layer::Application));
        assert!(ui(UiLayer::Features).may_depend_on(Layer::Application));
        assert!(ui(UiLayer::Shared).may_depend_on(Layer::Context));
        assert!(!ui(UiLayer::App).may_depend_on(Layer::Adapter));
    }

    #[test]
    fn only_the_shell_wires_adapters() {
        assert!(Layer::Shell.may_depend_on(Layer::Adapter));
        assert!(Layer::Shell.may_depend_on(Layer::Ui(UiLayer::App)));
        assert!(!Layer::Shell.may_depend_on(Layer::Tool));
        assert!(!Layer::Adapter.may_depend_on(Layer::Adapter));
        assert!(!Layer::Tool.may_depend_on(Layer::Kernel));
    }

    #[test]
    fn unsafe_and_purity_follow_the_layer() {
        assert!(!Layer::Application.may_use_unsafe());
        assert!(Layer::Ui(UiLayer::Shared).may_use_unsafe());
        assert!(Layer::Shell.may_use_unsafe());
        assert!(!Layer::Tool.may_use_unsafe());
        assert!(Layer::Kernel.external_allowlist().is_some());
        assert!(Layer::Adapter.external_allowlist().is_none());
        assert_eq!(Layer::Ui(UiLayer::Features).slice_depth(), 1);
        assert_eq!(Layer::Ui(UiLayer::Entities).slice_depth(), 1);
        assert_eq!(Layer::Ui(UiLayer::Widgets).slice_depth(), 2);
        assert_eq!(Layer::Ui(UiLayer::Shared).slice_depth(), 0);
        assert_eq!(Layer::Ui(UiLayer::App).slice_depth(), 0);
        assert_eq!(Layer::Shell.slice_depth(), 0);
    }

    #[test]
    fn the_pure_core_matches_fuzzily_without_threads() {
        let allowed = Layer::Context.external_allowlist().unwrap();

        assert!(allowed.contains(&"nucleo-matcher"));
        assert!(!allowed.contains(&"nucleo"));
    }

    #[test]
    fn names_layers_for_findings() {
        assert_eq!(Layer::Ui(UiLayer::Entities).to_string(), "ui entities");
        assert_eq!(Layer::Context.to_string(), "domain context");
    }
}
