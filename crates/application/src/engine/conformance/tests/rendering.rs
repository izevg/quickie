//! What engines that run HTML are served, and how a report reads.

use crate::conformance::site::resource;
use crate::conformance::{Group, Outcome, Report, Resource, Verdict};

fn served(path: &str) -> Resource {
    resource(path).unwrap()
}

#[test]
fn each_resource_is_served_with_its_type() {
    let types = ["opener", "tracker.js", "report.bin", "stalled.bin"]
        .map(|path| served(path).content_type());

    assert_eq!(
        types,
        [
            "text/html; charset=utf-8",
            "text/javascript",
            "application/octet-stream",
            "application/octet-stream"
        ]
    );
}

#[test]
fn a_file_is_served_as_it_is_and_a_stalled_one_only_starts() {
    let bodies = ["tracker.js", "report.bin", "stalled.bin"].map(|path| served(path).body());

    assert_eq!(
        bodies,
        [
            "window.ran = (window.ran || '') + 'tracker.js;';",
            "conformance report, whole\n",
            "conformance report, the start"
        ]
    );
}

#[test]
fn a_page_is_its_title_text_controls_and_what_it_does_once_loaded() {
    let html = served("opener").body();

    assert_eq!(
        html,
        "<!doctype html><meta charset=\"utf-8\"><title>Opener</title><p></p>\
         <p><a id=\"c0\" href=\"popup\" target=\"_blank\">link 0</a></p>\
         <p><a id=\"c1\" href=\"other\">link 1</a></p>\
         <p><button id=\"c2\" onclick=\"window.open('popup');\">button 2</button></p>\
         <p><button id=\"c3\" onclick=\"document.getElementById('c0').click();\">button 3</button></p>\
         <p><button id=\"c4\" onclick=\"document.title='Clicked';\">button 4</button></p>\
         <script>addEventListener('load',()=>setTimeout(()=>{},50));</script>"
    );
}

#[test]
fn a_page_loads_its_scripts_first_and_acts_once_loaded() {
    let html = served("tracked").body();

    assert_eq!(
        html,
        "<!doctype html><meta charset=\"utf-8\"><title>Tracked</title>\
         <script src=\"app.js\"></script><script src=\"tracker.js\"></script><p></p>\
         <script>addEventListener('load',()=>setTimeout(()=>\
         {document.title='scripts:'+(window.ran||'');},50));</script>"
    );
}

#[test]
fn every_act_is_a_script_of_its_own() {
    let scripts = [
        "popup",
        "set-cookie",
        "alert",
        "confirm",
        "prompt",
        "camera",
    ]
    .map(|path| served(path).body());

    for (script, expected) in scripts.iter().zip([
        "onclick=\"window.close();\"",
        "{document.cookie='jar=a; path=/';document.title='cookies:'+document.cookie;}",
        "{alert('Hello');document.title='Alerted';}",
        "document.title='confirm:'+confirm('Sure');",
        "document.title='prompt:'+prompt('Name','Ada');",
        "if(!navigator.mediaDevices){document.title='camera:unavailable';}else{navigator.\
         mediaDevices.getUserMedia({video:true}).then(()=>{document.title='camera:allowed'},\
         ()=>{document.title='camera:denied'});}",
    ]) {
        assert!(script.contains(expected), "{script} lacks {expected}");
    }
}

#[test]
fn a_report_reads_as_a_test_run() {
    let report = Report::new(vec![
        Verdict {
            group: Group::Core,
            case: "passes",
            outcome: Outcome::Passed,
        },
        Verdict {
            group: Group::Find,
            case: "fails",
            outcome: Outcome::Failed("why".to_owned()),
        },
        Verdict {
            group: Group::Crash,
            case: "skips",
            outcome: Outcome::Skipped("cannot"),
        },
    ]);

    let shown = report.to_string();

    assert_eq!(
        shown,
        "test Core::passes ... ok\n\
         test Find::fails ... FAILED: why\n\
         test Crash::skips ... skipped: cannot\n"
    );
    assert!(!report.passed());
}
