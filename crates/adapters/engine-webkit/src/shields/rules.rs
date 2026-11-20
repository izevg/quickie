//! Filter lists in Adblock Plus syntax → `WebKit` content rule lists (Safari content-blocker
//! JSON), by `adblock`'s converter. Pure and slow: it runs on a worker thread.
//!
//! Only the network rules are converted. The cosmetic rules are the cosmetic module's: as
//! `css-display-none` rules they would be cancelled on every first-party page by the exception
//! that lets top-level documents load (an `ignore-previous-rules` drops every earlier action of
//! the load it matches), and they would take most of the 150 000 rules a list may hold.

use adblock::content_blocking::{CbAction, CbRule, CbTrigger, CbType};
use adblock::lists::{FilterSet, ParseOptions, RuleTypes};
use quickie_application::BlockerRules;
use quickie_domain_shields::ContentHash;

/// The most rules `WebKit` compiles into one content rule list.
pub(crate) const MAX_RULES: usize = 150_000;

/// The start of every rule list identifier this adapter writes, so it can tell its own lists
/// in the store from anyone else's.
pub(crate) const PREFIX: &str = "quickie-shields-";

/// Names the rules a [`BlockerRules`] compiles to: equal rules, equal name, so compiled lists in
/// the store can be found again on the next launch instead of compiled again. Changes when the
/// lists' content, the unshielded sites, or this conversion change.
pub(crate) fn fingerprint(rules: &BlockerRules) -> String {
    let lists = rules
        .lists
        .iter()
        .map(|list| format!("{}={}", list.list, list.hash));
    let sites = rules.unshielded.iter().map(ToString::to_string);
    let text: Vec<String> = [format!("v2 network {MAX_RULES}")]
        .into_iter()
        .chain(lists)
        .chain([String::from("|")])
        .chain(sites)
        .collect();
    ContentHash::of(text.join(" ").as_bytes()).to_string()
}

/// One compiled rule list of a set: which of how many, how many rules it holds, and when it
/// was compiled (seconds since 1970). The store keeps all of it in the list's identifier, so a
/// later launch that finds the list knows what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Part {
    pub(crate) index: usize,
    pub(crate) count: usize,
    pub(crate) rules: usize,
    pub(crate) compiled: u64,
}

/// The identifier of `part` of the rule lists with this fingerprint.
pub(crate) fn identifier(fingerprint: &str, part: Part) -> String {
    let Part {
        index,
        count,
        rules,
        compiled,
    } = part;
    format!("{PREFIX}{fingerprint}-{index}-{count}-{rules}-{compiled}")
}

/// The part an identifier names, when it is one of this fingerprint's.
fn part_of(identifier: &str, fingerprint: &str) -> Option<Part> {
    let rest = identifier
        .strip_prefix(PREFIX)?
        .strip_prefix(fingerprint)?
        .strip_prefix('-')?;
    let numbers: Vec<&str> = rest.split('-').collect();
    let [index, count, rules, compiled] = numbers.as_slice() else {
        return None;
    };
    let part = Part {
        index: index.parse().ok()?,
        count: count.parse().ok()?,
        rules: rules.parse().ok()?,
        compiled: compiled.parse().ok()?,
    };
    (part.index < part.count).then_some(part)
}

/// The rule lists the store already holds, compiled, for this fingerprint, in order: `Some`
/// only when every one of a set is there.
pub(crate) fn cached(identifiers: &[String], fingerprint: &str) -> Option<Vec<Part>> {
    let parts: Vec<Part> = identifiers
        .iter()
        .filter_map(|id| part_of(id, fingerprint))
        .collect();
    let count = parts.first()?.count;
    (0..count)
        .map(|index| {
            parts
                .iter()
                .find(|part| part.index == index && part.count == count)
                .copied()
        })
        .collect()
}

/// The rule lists `rules` compile to, as JSON documents of at most `max_rules` rules each
/// ([`MAX_RULES`] for `WebKit`), each with its number of rules; none when there is nothing to
/// block.
///
/// `ignore-previous-rules` only reaches back within its own rule list, so every document ends
/// with all of the lists' exceptions, and then one exception per unshielded site: a site with
/// shields off loads everything on its pages and their subframes.
pub(crate) fn encode(
    rules: &BlockerRules,
    max_rules: usize,
) -> Result<Vec<(String, usize)>, String> {
    let mut filters = FilterSet::new(true);
    for list in &rules.lists {
        let text = String::from_utf8_lossy(&list.body).into_owned();
        let options = ParseOptions {
            rule_types: RuleTypes::NetworkOnly,
            ..ParseOptions::default()
        };
        filters.add_filter_list(text, options);
    }
    let (converted, _) = filters
        .into_content_blocking()
        .map_err(|()| "the filter lists could not be converted".to_owned())?;
    let (exceptions, blocking): (Vec<CbRule>, Vec<CbRule>) = converted
        .into_iter()
        .partition(|rule| rule.action.typ == CbType::IgnorePreviousRules);
    let mut tail = exceptions;
    tail.extend(
        rules
            .unshielded
            .iter()
            .map(|site| let_through(site.as_str())),
    );
    let room = max_rules.saturating_sub(tail.len());
    if room == 0 {
        return Err(format!(
            "{} exceptions leave no room for a blocking rule",
            tail.len()
        ));
    }
    blocking
        .chunks(room)
        .map(|chunk| {
            let document: Vec<&CbRule> = chunk.iter().chain(&tail).collect();
            serde_json::to_string(&document)
                .map(|json| (json, document.len()))
                .map_err(|error| error.to_string())
        })
        .collect()
}

/// The rule that lets every load on a site's pages through.
fn let_through(site: &str) -> CbRule {
    CbRule {
        trigger: CbTrigger {
            url_filter: ".*".to_owned(),
            if_domain: Some(vec![format!("*{site}")]),
            ..CbTrigger::default()
        },
        action: CbAction {
            typ: CbType::IgnorePreviousRules,
            selector: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use quickie_application::{BlockerRules, ListRules};
    use quickie_domain_shields::{ContentHash, FilterListId, Site};
    use serde_json::Value;

    use super::{MAX_RULES, Part, cached, encode, fingerprint, identifier};

    fn list(id: FilterListId, text: &str) -> ListRules {
        ListRules {
            list: id,
            hash: ContentHash::of(text.as_bytes()),
            body: Arc::from(text.as_bytes()),
        }
    }

    fn rules(text: &str, unshielded: &[&str]) -> BlockerRules {
        BlockerRules {
            lists: vec![list(FilterListId::EasyList, text)],
            unshielded: unshielded
                .iter()
                .map(|site| Site::try_from(*site).unwrap())
                .collect(),
        }
    }

    fn documents(rules: &BlockerRules) -> Vec<Vec<Value>> {
        encode(rules, MAX_RULES)
            .unwrap()
            .iter()
            .map(|(json, _)| serde_json::from_str(json).unwrap())
            .collect()
    }

    fn action(rule: &Value) -> &str {
        rule["action"]["type"].as_str().unwrap()
    }

    #[test]
    fn a_blocking_filter_becomes_a_block_rule() {
        let encoded = documents(&rules("||tracker.test^", &[]));

        let [document] = encoded.as_slice() else {
            panic!("one document expected, got {encoded:?}");
        };
        let first = document.first().unwrap();
        assert_eq!(action(first), "block");
        assert!(
            first["trigger"]["url-filter"]
                .as_str()
                .unwrap()
                .contains("tracker")
        );
    }

    #[test]
    fn an_unshielded_site_ends_every_document_with_its_exception() {
        let encoded = documents(&rules("||tracker.test^", &["example.com"]));

        let last = encoded[0].last().unwrap();
        assert_eq!(action(last), "ignore-previous-rules");
        assert_eq!(last["trigger"]["url-filter"], ".*");
        assert_eq!(last["trigger"]["if-domain"][0], "*example.com");
    }

    #[test]
    fn nothing_to_block_compiles_to_no_list() {
        let encoded = encode(&rules("! only a comment", &["example.com"]), MAX_RULES);

        assert_eq!(encoded, Ok(Vec::new()));
    }

    #[test]
    fn no_lists_compile_to_no_list() {
        let encoded = encode(&BlockerRules::default(), MAX_RULES);

        assert_eq!(encoded, Ok(Vec::new()));
    }

    #[test]
    fn a_list_too_long_for_one_rule_list_is_split_and_each_part_keeps_the_exceptions() {
        let text = "||a.test^\n||b.test^\n||c.test^\n@@||a.test^$image\n";

        let encoded = encode(&rules(text, &["example.com"]), 4).unwrap();

        let parts: Vec<Vec<Value>> = encoded
            .iter()
            .map(|(json, _)| serde_json::from_str(json).unwrap())
            .collect();
        let shapes: Vec<Vec<&str>> = parts
            .iter()
            .map(|part| part.iter().map(action).collect())
            .collect();
        let exception = "ignore-previous-rules";
        assert_eq!(
            shapes,
            [
                ["block", exception, exception, exception],
                ["block", exception, exception, exception],
                ["block", exception, exception, exception],
            ]
        );
        assert!(
            parts
                .iter()
                .all(|part| part.last().unwrap()["trigger"]["if-domain"][0] == "*example.com")
        );
    }

    // Besides one exception per unshielded site, the converter adds one of its own that lets
    // first-party documents load.

    #[test]
    fn exceptions_that_fill_a_rule_list_leave_no_room() {
        let encoded = encode(&rules("||a.test^", &["a.com", "b.com"]), 3);

        assert_eq!(
            encoded,
            Err("3 exceptions leave no room for a blocking rule".to_owned())
        );
    }

    #[test]
    fn one_free_slot_is_room_enough() {
        let encoded = encode(&rules("||a.test^", &["a.com"]), 3);

        assert_eq!(encoded.map(|documents| documents.len()), Ok(1));
    }

    #[test]
    fn content_that_is_not_text_is_read_as_far_as_it_goes() {
        let mut body = b"||tracker.test^\n".to_vec();
        body.extend([0xff, 0xfe, b'\n']);
        let rules = BlockerRules {
            lists: vec![ListRules {
                list: FilterListId::EasyList,
                hash: ContentHash::of(&body),
                body: Arc::from(body),
            }],
            unshielded: Vec::new(),
        };

        let encoded = encode(&rules, MAX_RULES);

        assert_eq!(encoded.map(|documents| documents.len()), Ok(1));
    }

    #[test]
    fn equal_rules_have_equal_fingerprints_and_any_change_changes_it() {
        let base = rules("||a.test^", &[]);

        let same = fingerprint(&rules("||a.test^", &[]));
        let other_content = fingerprint(&rules("||b.test^", &[]));
        let other_sites = fingerprint(&rules("||a.test^", &["example.com"]));

        assert_eq!(fingerprint(&base), same);
        assert_ne!(fingerprint(&base), other_content);
        assert_ne!(fingerprint(&base), other_sites);
    }

    // Without a domain resolver, `adblock` panicked on the first `$to=` rule while parsing, and
    // the worker thread that converted every list died with it.
    #[test]
    fn a_ublock_to_option_converts() {
        let encoded = encode(&rules("||cdn.test^$script,to=ads.test\n", &[]), MAX_RULES);

        assert!(encoded.is_ok());
    }

    #[test]
    fn cosmetic_rules_are_left_to_the_cosmetic_filters() {
        let encoded = documents(&rules(
            "##.ad\nexample.com##.banner\n||tracker.test^\n",
            &[],
        ));

        let actions: Vec<&str> = encoded[0].iter().map(action).collect();
        assert!(!actions.contains(&"css-display-none"), "{actions:?}");
        assert!(actions.contains(&"block"));
    }

    #[test]
    fn each_document_counts_its_rules() {
        let encoded = encode(&rules("||a.test^\n||b.test^\n", &[]), MAX_RULES).unwrap();

        let [(json, count)] = encoded.as_slice() else {
            panic!("one document expected");
        };
        let parsed: Vec<Value> = serde_json::from_str(json).unwrap();
        assert_eq!(*count, parsed.len());
    }

    fn part(index: usize, count: usize) -> Part {
        Part {
            index,
            count,
            rules: 10 + index,
            compiled: 1_700_000_000,
        }
    }

    #[test]
    fn identifiers_carry_the_fingerprint_and_the_part() {
        assert_eq!(
            identifier("ab", part(1, 3)),
            "quickie-shields-ab-1-3-11-1700000000"
        );
    }

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_complete_set_of_compiled_lists_is_reused_in_order() {
        let stored = ids(&[
            "someone-else",
            &identifier("ab", part(1, 2)),
            &identifier("cd", part(0, 1)),
            &identifier("ab", part(0, 2)),
        ]);

        assert_eq!(cached(&stored, "ab"), Some(vec![part(0, 2), part(1, 2)]));
        assert_eq!(cached(&stored, "cd"), Some(vec![part(0, 1)]));
    }

    #[test]
    fn a_partial_set_or_none_is_compiled_again() {
        let stored = ids(&[
            &identifier("ab", part(1, 2)),
            &identifier("abc", part(0, 1)),
        ]);

        assert_eq!(cached(&stored, "ab"), None);
        assert_eq!(cached(&stored, "zz"), None);
        assert_eq!(cached(&ids(&["quickie-shields-ab-x"]), "ab"), None);
        assert_eq!(cached(&ids(&["quickie-shields-ab-0-1-2"]), "ab"), None);
        assert_eq!(cached(&ids(&["quickie-shields-ab-1-1-2-3"]), "ab"), None);
        assert_eq!(cached(&ids(&["someone-ab-0-1-2-3"]), "ab"), None);
    }
}
