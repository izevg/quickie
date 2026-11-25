//! Shared content rule lookup and scripts used by the native `WebKit` embedders.

#[path = "bundle.rs"]
mod bundle;
#[path = "library.rs"]
mod library;

use std::collections::{BTreeSet, HashSet};

use adblock::Engine;
use adblock::lists::{FilterSet, ParseOptions};
use quickie_application::ListRules;
use quickie_domain_shields::{ContentHash, Site};
use serde_json::Value;

use bundle::unshielded;
pub use bundle::{HostBundle, HostTable, page_script};

/// The native hook registers this channel in the isolated Shields world.
pub const CHANNEL: &str = "quickieShields";

/// Bound the DOM reports. Element classes and IDs are controlled by the page.
const MOST_NAMES: usize = 4096;
const LONGEST_NAME: usize = 256;
const MAX_REPORT_BYTES: usize = 2 * MOST_NAMES * (LONGEST_NAME + 7) + 4096;

/// The cosmetic filters of the enabled lists, built off the main thread and read from any.
pub struct Filters {
    engine: Engine,
    /// The selectors every host hides, unless it excepts them.
    pub(crate) generic: BTreeSet<String>,
}

impl Filters {
    /// Builds the filters of `lists`: only their cosmetic rules and the `$generichide`
    /// exceptions, since `WebKit`'s content rule lists do the network blocking. Lists that may
    /// use the trusted scriptlets get their permission. Slow: a worker thread runs it.
    #[must_use]
    pub fn build(lists: &[ListRules]) -> Self {
        let mut set = FilterSet::new(false);
        for list in lists {
            let text = String::from_utf8_lossy(&list.body);
            let cosmetic: String = text
                .lines()
                .filter(|line| {
                    line.contains('#') || line.contains("generichide") || line.contains("ghide")
                })
                .flat_map(|line| [line, "\n"])
                .collect();
            let options = ParseOptions {
                permissions: if list.list.trusted() {
                    library::TRUSTED
                } else {
                    adblock::resources::PermissionMask::default()
                },
                ..ParseOptions::default()
            };
            set.add_filter_list(cosmetic, options);
        }
        let mut engine = Engine::new_with_filter_set(set);
        engine.use_resources(library::resources());
        let generic = engine
            .url_cosmetic_resources("https://quickie.invalid/")
            .hide_selectors
            .into_iter()
            .collect();
        Self { engine, generic }
    }

    /// What `host` gets.
    #[must_use]
    pub fn lookup(&self, host: &str, unshielded_sites: &[Site]) -> HostBundle {
        if unshielded(host, unshielded_sites) {
            return HostBundle::off();
        }
        let resources = self
            .engine
            .url_cosmetic_resources(&format!("https://{host}/"));
        HostBundle::of(resources, &self.generic)
    }

    /// The generic rules that hide elements with these classes or ids.
    pub(crate) fn hidden(
        &self,
        classes: &[String],
        ids: &[String],
        exceptions: &HashSet<String>,
    ) -> Vec<String> {
        self.engine
            .hidden_class_id_selectors(classes, ids, exceptions)
    }
}

impl Filters {
    /// The document-start cosmetic script for the engine's isolated Shields world.
    #[must_use]
    pub fn script(&self, hosts: &HostTable) -> String {
        bundle::cosmetic_script(&self.generic, hosts)
    }

    /// Applies the host's exceptions and returns generic selectors for the reported DOM names.
    #[must_use]
    pub fn answer(&self, report: &Report, bundle: Option<&HostBundle>) -> String {
        if bundle.is_some_and(|bundle| bundle.off || bundle.generichide) {
            return "[]".to_owned();
        }
        let empty = HashSet::new();
        let exceptions = bundle.map_or(&empty, |bundle| &bundle.exceptions);
        Value::from(self.hidden(&report.classes, &report.ids, exceptions)).to_string()
    }
}

/// Names the lists filters are built from; `None` for no lists.
#[must_use]
pub fn lists_name(lists: &[ListRules]) -> Option<String> {
    if lists.is_empty() {
        return None;
    }
    let text: Vec<String> = lists
        .iter()
        .map(|list| format!("{}={}", list.list, list.hash))
        .collect();
    Some(ContentHash::of(text.join(" ").as_bytes()).to_string())
}

/// What a frame reported: the classes and ids it saw since its last report.
pub struct Report {
    /// The document host as observed by the isolated script.
    pub host: String,
    classes: Vec<String>,
    ids: Vec<String>,
}

impl Report {
    /// Reads a report as the shields script posts it: JSON text. Bounded, as the page wrote
    /// the names.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() > MAX_REPORT_BYTES {
            return None;
        }
        let value: Value = serde_json::from_str(text).ok()?;
        let names = |key: &str| -> Vec<String> {
            value
                .get(key)
                .and_then(Value::as_array)
                .map_or_else(Vec::new, |names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|name| name.len() <= LONGEST_NAME)
                        .take(MOST_NAMES)
                        .map(str::to_owned)
                        .collect()
                })
        };
        Some(Self {
            host: value.get("host")?.as_str()?.to_owned(),
            classes: names("classes"),
            ids: names("ids"),
        })
    }
}

#[cfg(test)]
#[path = "filters/tests.rs"]
mod tests;
