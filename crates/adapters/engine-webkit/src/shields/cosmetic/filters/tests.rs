use std::collections::HashSet;
use std::sync::Arc;

use quickie_application::ListRules;
use quickie_domain_shields::{ContentHash, FilterListId, Site};

use super::{Filters, LONGEST_NAME, MAX_REPORT_BYTES, Report, lists_name};

fn list(id: FilterListId, text: &str) -> ListRules {
    ListRules {
        list: id,
        hash: ContentHash::of(text.as_bytes()),
        body: Arc::from(text.as_bytes()),
    }
}

const LIST: &str = "##div[data-ad]\n\
                    ##.generic-ad\n\
                    video.test##.host-ad\n\
                    video.test##+js(json-prune, adPlacements)\n\
                    video.test##+js(trusted-replace-fetch-response, ads, x)\n\
                    quiet.test#@#div[data-ad]\n\
                    quiet.test#@#.generic-ad\n\
                    @@||plain.test^$generichide\n\
                    ||tracker.test^\n";

#[test]
fn a_host_gets_its_own_rules_and_the_scriptlets_its_list_may_run() {
    let filters = Filters::build(&[list(FilterListId::EasyList, LIST)]);

    let bundle = filters.lookup("www.video.test", &[]);

    assert_eq!(bundle.css, [".host-ad{display:none!important}"]);
    assert!(
        bundle
            .scriptlets
            .contains("quickie_json_prune(\"adPlacements\")")
    );
    assert!(
        !bundle.scriptlets.contains("trusted"),
        "{}",
        bundle.scriptlets
    );
}

#[test]
fn a_trusted_list_may_run_the_trusted_scriptlets() {
    let filters = Filters::build(&[list(FilterListId::UblockFilters, LIST)]);

    let bundle = filters.lookup("video.test", &[]);

    assert!(
        bundle
            .scriptlets
            .contains("quickie_trusted_replace_fetch_response(")
    );
}

#[test]
fn generic_rules_apply_everywhere_but_where_they_are_excepted() {
    let filters = Filters::build(&[list(FilterListId::EasyList, LIST)]);

    assert!(filters.generic.contains("div[data-ad]"));
    let quiet = filters.lookup("quiet.test", &[]);
    assert_eq!(quiet.remove, ["div[data-ad]"]);
    let classes = [String::from("generic-ad")];
    assert_eq!(
        filters.hidden(&classes, &[], &HashSet::new()),
        [".generic-ad"]
    );
    assert_eq!(
        filters.hidden(&classes, &[], &quiet.exceptions),
        Vec::<String>::new()
    );
    assert!(filters.lookup("plain.test", &[]).generichide);
    assert!(!filters.lookup("other.test", &[]).differs());
}

#[test]
fn a_procedural_filter_reaches_the_cosmetic_script_as_json() {
    let filters = Filters::build(&[list(
        FilterListId::EasyList,
        "localhost##div:has-text(Sponsored)\n",
    )]);

    let bundle = filters.lookup("localhost", &[]);

    assert_eq!(
        bundle.procedural,
        [
            r#"{"selector":[{"type":"css-selector","arg":"div"},{"type":"has-text","arg":"Sponsored"}]}"#
        ]
    );
}

// The Shorts feed's ad rules as uBlock filters and EasyList write them (November 2026).
const SHORTS: &str = "www.youtube.com##+js(json-prune-fetch-response, \
    reelWatchSequenceResponse.entries.[-].command.reelWatchEndpoint.adClientParams.isAd \
    entries.[-].command.reelWatchEndpoint.adClientParams.isAd, , propsToMatch, \
    url:/reel_watch_sequence?)\n\
    www.youtube.com##+js(json-prune, entries.[-].command.reelWatchEndpoint.adClientParams.isAd)\n\
    youtube.com###shorts-inner-container > .ytd-shorts:has(> .ytd-reel-video-renderer > ytd-ad-slot-renderer)\n";

#[test]
fn the_shorts_feed_s_ad_rules_reach_the_host_intact() {
    let filters = Filters::build(&[list(FilterListId::UblockFilters, SHORTS)]);

    let bundle = filters.lookup("www.youtube.com", &[]);

    assert!(
        bundle.scriptlets.contains(
            "quickie_json_prune_fetch_response(\"reelWatchSequenceResponse.entries.[-].command.reelWatchEndpoint.adClientParams.isAd entries.[-].command.reelWatchEndpoint.adClientParams.isAd\", \"\", \"propsToMatch\", \"url:/reel_watch_sequence?\")"
        ),
        "{}",
        bundle.scriptlets
    );
    assert!(
        bundle.scriptlets.contains(
            "quickie_json_prune(\"entries.[-].command.reelWatchEndpoint.adClientParams.isAd\")"
        ),
        "{}",
        bundle.scriptlets
    );
    assert_eq!(
        bundle.css,
        [
            "#shorts-inner-container > .ytd-shorts:has(> .ytd-reel-video-renderer > ytd-ad-slot-renderer){display:none!important}"
        ],
        "{:?}",
        bundle.procedural
    );
}

// The extensions test relies on it: a page on 127.0.0.1 makes the shields replace their
// scripts.
#[test]
fn an_ip_address_host_gets_its_own_rules() {
    let filters = Filters::build(&[list(FilterListId::EasyList, "127.0.0.1##.local-ad\n")]);

    let bundle = filters.lookup("127.0.0.1", &[]);

    assert_eq!(bundle.css, [".local-ad{display:none!important}"]);
}

#[test]
fn an_unshielded_site_gets_nothing() {
    let filters = Filters::build(&[list(FilterListId::EasyList, LIST)]);

    let bundle = filters.lookup("video.test", &[Site::try_from("video.test").unwrap()]);

    assert!(bundle.off);
    assert_eq!(bundle.scriptlets, "");
}

#[test]
fn classes_and_ids_find_their_generic_rules() {
    let filters = Filters::build(&[list(
        FilterListId::EasyList,
        "##.banner-ad\n###top-ad\n##.unused\n",
    )]);

    let mut hidden = filters.hidden(
        &["banner-ad".to_owned(), "article".to_owned()],
        &["top-ad".to_owned()],
        &HashSet::new(),
    );
    hidden.sort();

    assert_eq!(hidden, ["#top-ad", ".banner-ad"]);
}

#[test]
fn lists_are_named_by_their_content() {
    let a = [list(FilterListId::EasyList, "##.a")];
    let b = [list(FilterListId::EasyList, "##.b")];

    assert_eq!(lists_name(&[]), None);
    assert_eq!(lists_name(&a), lists_name(&a));
    assert_ne!(lists_name(&a), lists_name(&b));
}

#[test]
fn a_report_is_read_within_bounds() {
    let long = "x".repeat(LONGEST_NAME + 1);
    let text = format!(r#"{{"host":"a.test","classes":["ad","{long}",3],"ids":["top"]}}"#);

    let report = Report::parse(&text).unwrap();

    assert_eq!(report.host, "a.test");
    assert_eq!(report.classes, ["ad"]);
    assert_eq!(report.ids, ["top"]);
    assert!(Report::parse("[]").is_none());
    assert!(Report::parse("not json").is_none());
}

#[test]
fn an_oversized_report_is_rejected_before_reading_the_json() {
    let text = format!(
        r#"{{"host":"a.test","ids":[]}}{}"#,
        " ".repeat(MAX_REPORT_BYTES)
    );

    let report = Report::parse(&text);

    assert!(report.is_none());
}
