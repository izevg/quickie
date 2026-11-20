//! What one host gets from the cosmetic filters, and the two shields scripts made from the
//! hosts seen so far. Pure: the lookups run on a worker thread, the scripts are installed on
//! the main thread.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use adblock::cosmetic_filter_cache::{ProceduralOrActionFilter, UrlSpecificResources};
use quickie_domain_shields::Site;
use serde_json::{Map, Value, json};

use super::library::{LIBRARY, RUN};

/// The cosmetic script, with the place its data goes.
const COSMETIC: &str = include_str!("js/cosmetic.js");

/// Where [`COSMETIC`] takes its data.
const DATA: &str = "/*QUICKIE_SHIELDS_DATA*/ null";

/// What the filters do on one host, beyond the generic rules every host gets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HostBundle {
    /// The host's site has shields off: nothing is hidden and no scriptlet runs.
    pub(crate) off: bool,
    /// A `$generichide` exception: no generic rule applies.
    pub(crate) generichide: bool,
    /// CSS rules of this host only: `selector{display:none!important}` or a `:style()`.
    pub(crate) css: Vec<String>,
    /// Generic selectors this host excepts (`#@#`).
    pub(crate) remove: Vec<String>,
    /// Procedural filters as `adblock` writes them (JSON), for the cosmetic script to run.
    pub(crate) procedural: Vec<String>,
    /// The scriptlet calls for this host, with the functions they call.
    pub(crate) scriptlets: String,
    /// Class and id selectors excepted here, for the generic class and id lookups.
    pub(crate) exceptions: HashSet<String>,
}

impl HostBundle {
    /// The bundle of a host whose site has shields off.
    pub(crate) fn off() -> Self {
        Self {
            off: true,
            ..Self::default()
        }
    }

    /// The bundle `resources` describe, given the `generic` selectors every host hides.
    pub(crate) fn of(resources: UrlSpecificResources, generic: &BTreeSet<String>) -> Self {
        let mut css: Vec<String> = resources
            .hide_selectors
            .iter()
            .filter(|selector| !generic.contains(*selector))
            .map(|selector| hide(selector))
            .collect();
        let remove: Vec<String> = if resources.generichide {
            Vec::new()
        } else {
            generic
                .iter()
                .filter(|selector| !resources.hide_selectors.contains(*selector))
                .cloned()
                .collect()
        };
        let mut procedural = Vec::new();
        for text in resources.procedural_actions {
            match serde_json::from_str::<ProceduralOrActionFilter>(&text)
                .ok()
                .and_then(|filter| filter.as_css())
            {
                Some((selector, style)) => css.push(format!("{selector}{{{style}}}")),
                None => procedural.push(text),
            }
        }
        css.sort();
        procedural.sort();
        Self {
            off: false,
            generichide: resources.generichide,
            css,
            remove,
            procedural,
            scriptlets: resources.injected_script,
            exceptions: resources.exceptions,
        }
    }

    /// Whether the host needs its own entry in the scripts: anything but the generic rules.
    pub(crate) fn differs(&self) -> bool {
        self.off
            || self.generichide
            || !self.css.is_empty()
            || !self.remove.is_empty()
            || !self.procedural.is_empty()
            || !self.scriptlets.trim().is_empty()
    }
}

/// The rule that hides what `selector` matches.
fn hide(selector: &str) -> String {
    format!("{selector}{{display:none!important}}")
}

/// Whether `host` belongs to a site whose shields are off: the site itself or a subdomain.
pub(crate) fn unshielded(host: &str, sites: &[Site]) -> bool {
    let host = host.strip_prefix("www.").unwrap_or(host);
    sites.iter().any(|site| {
        let site = site.as_str();
        host == site
            || host
                .strip_suffix(site)
                .is_some_and(|rest| rest.ends_with('.'))
    })
}

/// The hosts whose bundles are known, the most recently used last; at most `capacity`.
#[derive(Debug)]
pub(crate) struct HostTable {
    capacity: usize,
    order: VecDeque<String>,
    bundles: HashMap<String, Arc<HostBundle>>,
}

impl HostTable {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::new(),
            bundles: HashMap::new(),
        }
    }

    /// A host's bundle, if it is known; it becomes the most recently used.
    pub(crate) fn get(&mut self, host: &str) -> Option<Arc<HostBundle>> {
        let bundle = Arc::clone(self.bundles.get(host)?);
        self.order.retain(|known| known != host);
        self.order.push_back(host.to_owned());
        Some(bundle)
    }

    /// Keeps a host's bundle, forgetting the least recently used one beyond the capacity;
    /// returns whether the scripts change: the host has an entry of its own in them, or one
    /// that had an entry was forgotten.
    pub(crate) fn insert(&mut self, host: &str, bundle: Arc<HostBundle>) -> bool {
        let mut changed = bundle.differs();
        if let Some(old) = self.bundles.insert(host.to_owned(), bundle) {
            changed |= old.differs();
        }
        self.order.retain(|known| known != host);
        self.order.push_back(host.to_owned());
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front()
                && let Some(gone) = self.bundles.remove(&oldest)
            {
                changed |= gone.differs();
            }
        }
        changed
    }

    /// Forgets every host: the filters or the unshielded sites changed.
    pub(crate) fn clear(&mut self) {
        self.order.clear();
        self.bundles.clear();
    }

    /// The hosts with an entry of their own, in a stable order.
    fn entries(&self) -> Vec<(&str, &HostBundle)> {
        let mut entries: Vec<(&str, &HostBundle)> = self
            .bundles
            .iter()
            .filter(|(_, bundle)| bundle.differs())
            .map(|(host, bundle)| (host.as_str(), bundle.as_ref()))
            .collect();
        entries.sort_by_key(|(host, _)| *host);
        entries
    }
}

/// The page-world script: the scriptlets of each host that has any, and the library they call,
/// evaluated only on such a host. `None` when no known host has a scriptlet.
pub(crate) fn page_script(table: &HostTable) -> Option<String> {
    let hosts: Vec<String> = table
        .entries()
        .into_iter()
        .filter(|(_, bundle)| !bundle.off && !bundle.scriptlets.trim().is_empty())
        .map(|(host, bundle)| format!("{}: () => {{\n{}\n}}", quoted(host), bundle.scriptlets))
        .collect();
    if hosts.is_empty() {
        return None;
    }
    Some(format!(
        "(() => {{\n'use strict';\nconst hosts = {{\n{}\n}};\nconst own = hosts[location.hostname];\n\
         if (typeof own !== 'function') return;\nconst {RUN} = {LIBRARY};\nown();\n}})();\n",
        hosts.join(",\n")
    ))
}

/// The isolated-world script: the generic selectors, and each host's own entry.
pub(crate) fn cosmetic_script(generic: &BTreeSet<String>, table: &HostTable) -> String {
    let mut hosts = Map::new();
    for (host, bundle) in table.entries() {
        let entry = if bundle.off {
            json!({ "off": true })
        } else {
            json!({
                "generichide": bundle.generichide,
                "css": bundle.css,
                "remove": bundle.remove,
                "procedural": bundle.procedural,
            })
        };
        hosts.insert(host.to_owned(), entry);
    }
    let data = json!({ "generic": generic, "hosts": Value::Object(hosts) });
    COSMETIC.replacen(DATA, &data.to_string(), 1)
}

/// A host as a JavaScript string literal.
fn quoted(host: &str) -> String {
    Value::String(host.to_owned()).to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};
    use std::sync::Arc;

    use adblock::cosmetic_filter_cache::UrlSpecificResources;
    use quickie_domain_shields::Site;

    use super::{COSMETIC, DATA, HostBundle, HostTable, cosmetic_script, page_script, unshielded};

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn generic(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn resources(hide: &[&str], procedural: &[&str], script: &str) -> UrlSpecificResources {
        UrlSpecificResources {
            hide_selectors: set(hide),
            procedural_actions: set(procedural),
            exceptions: set(&[]),
            injected_script: script.to_owned(),
            generichide: false,
        }
    }

    #[test]
    fn a_host_keeps_only_what_differs_from_the_generic_rules() {
        let bundle = HostBundle::of(
            resources(&[".ad", ".host-only"], &[], ""),
            &generic(&[".ad", ".excepted"]),
        );

        assert_eq!(bundle.css, [".host-only{display:none!important}"]);
        assert_eq!(bundle.remove, [".excepted"]);
        assert!(bundle.differs());
    }

    #[test]
    fn a_host_with_only_the_generic_rules_needs_no_entry() {
        let bundle = HostBundle::of(resources(&[".ad"], &[], "  \n"), &generic(&[".ad"]));

        assert!(!bundle.differs());
    }

    #[test]
    fn generichide_drops_no_generic_selector_one_by_one() {
        let mut found = resources(&[".host-only"], &[], "");
        found.generichide = true;

        let bundle = HostBundle::of(found, &generic(&[".ad"]));

        assert!(bundle.generichide);
        assert_eq!(bundle.remove, Vec::<String>::new());
        assert!(bundle.differs());
    }

    #[test]
    fn a_style_filter_becomes_css_and_a_procedural_one_stays_json() {
        let style = r#"{"selector":[{"type":"css-selector","arg":".x"}],"action":{"type":"style","arg":"color: red"}}"#;
        let has_text =
            r#"{"selector":[{"type":"css-selector","arg":"div"},{"type":"has-text","arg":"Ad"}]}"#;

        let bundle = HostBundle::of(resources(&[], &[style, has_text], ""), &generic(&[]));

        assert_eq!(bundle.css, [".x{color: red}"]);
        assert_eq!(bundle.procedural, [has_text]);
    }

    #[test]
    fn each_kind_of_difference_needs_an_entry() {
        let differs = |bundle: HostBundle| bundle.differs();
        assert!(differs(HostBundle::off()));
        assert!(differs(HostBundle {
            procedural: vec!["{}".to_owned()],
            ..HostBundle::default()
        }));
        assert!(differs(HostBundle {
            scriptlets: "f()".to_owned(),
            ..HostBundle::default()
        }));
        assert!(!differs(HostBundle::default()));
    }

    #[test]
    fn a_site_s_subdomains_are_unshielded_with_it() {
        let sites = [Site::try_from("example.com").unwrap()];

        assert!(unshielded("example.com", &sites));
        assert!(unshielded("www.example.com", &sites));
        assert!(unshielded("news.example.com", &sites));
        assert!(!unshielded("badexample.com", &sites));
        assert!(!unshielded("example.org", &sites));
    }

    fn with_scriptlet(script: &str) -> Arc<HostBundle> {
        Arc::new(HostBundle {
            scriptlets: script.to_owned(),
            ..HostBundle::default()
        })
    }

    #[test]
    fn the_table_forgets_the_least_recently_used_host() {
        let mut table = HostTable::new(2);
        table.insert("a.test", with_scriptlet("a()"));
        table.insert("b.test", with_scriptlet("b()"));
        table.get("a.test");

        let changed = table.insert("c.test", with_scriptlet("c()"));

        assert!(changed);
        assert!(table.get("b.test").is_none());
        assert!(table.get("a.test").is_some());
        assert!(table.get("c.test").is_some());
    }

    #[test]
    fn a_plain_host_leaves_the_scripts_as_they_are() {
        let mut table = HostTable::new(4);

        assert!(!table.insert("plain.test", Arc::new(HostBundle::default())));
        assert!(table.insert("x.test", with_scriptlet("x()")));
        assert!(table.insert("x.test", Arc::new(HostBundle::default())));
    }

    #[test]
    fn forgetting_a_plain_host_changes_nothing() {
        let mut table = HostTable::new(1);
        table.insert("plain.test", Arc::new(HostBundle::default()));

        assert!(!table.insert("other.test", Arc::new(HostBundle::default())));
    }

    #[test]
    fn a_cleared_table_knows_no_host() {
        let mut table = HostTable::new(4);
        table.insert("x.test", with_scriptlet("x()"));

        table.clear();

        assert!(table.get("x.test").is_none());
        assert_eq!(page_script(&table), None);
    }

    #[test]
    fn the_page_script_runs_each_host_s_scriptlets_only_there() {
        let mut table = HostTable::new(4);
        table.insert(
            "www.video.test",
            with_scriptlet("quickie_json_prune(\"ads\")"),
        );
        table.insert("plain.test", Arc::new(HostBundle::default()));
        table.insert("off.test", Arc::new(HostBundle::off()));

        let script = page_script(&table).unwrap();

        assert!(script.contains("\"www.video.test\": () => {\nquickie_json_prune(\"ads\")\n}"));
        assert!(!script.contains("plain.test"));
        assert!(!script.contains("off.test"));
        assert!(script.contains("const quickieScriptlet = "));
        assert!(script.contains("hosts[location.hostname]"));
    }

    #[test]
    fn the_cosmetic_script_carries_the_generic_selectors_and_each_entry() {
        let mut table = HostTable::new(4);
        table.insert("off.test", Arc::new(HostBundle::off()));
        table.insert(
            "x.test",
            Arc::new(HostBundle {
                css: vec![".x{display:none!important}".to_owned()],
                ..HostBundle::default()
            }),
        );

        let script = cosmetic_script(&generic(&[".ad"]), &table);

        assert!(COSMETIC.contains(DATA));
        assert!(!script.contains(DATA));
        assert!(script.contains(r#""generic":[".ad"]"#));
        assert!(script.contains(r#""off.test":{"off":true}"#));
        assert!(script.contains(r#""css":[".x{display:none!important}"]"#));
    }
}
