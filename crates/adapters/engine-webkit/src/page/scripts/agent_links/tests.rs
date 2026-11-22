use serde_json::json;

use super::parse;

#[test]
fn snapshot_preserves_video_order_and_ignores_ineligible_urls() {
    let answer = json!({
        "document": "http://127.0.0.1:8080/feed",
        "links": [
            {"url": "http://127.0.0.1:8080/watch?v=first", "label": " First video "},
            {"url": "http://outside.test/watch?v=no", "label": "External"},
            {"url": "http://user@127.0.0.1:8080/watch?v=no", "label": "Credentials"},
            {"url": "http://127.0.0.1:8081/watch?v=no", "label": "Another port"},
            {"url": "blob:http://127.0.0.1:8080/no", "label": "Blob"},
            {"url": "http://127.0.0.1:8080/watch?v=first", "label": "Duplicate"},
            {"url": "http://127.0.0.1:8080/watch?v=second", "label": "Second video"},
            {"url": "http://127.0.0.1:8080/watch?v=unlabeled", "label": " "},
        ],
    });

    let snapshot = parse(&answer.to_string()).unwrap();

    assert_eq!(snapshot.document.as_str(), "http://127.0.0.1:8080/feed");
    assert_eq!(snapshot.links.len(), 2);
    assert_eq!(snapshot.links[0].label, "First video");
    assert_eq!(
        snapshot.links[0].url.as_str(),
        "http://127.0.0.1:8080/watch?v=first"
    );
    assert_eq!(snapshot.links[1].label, "Second video");
}

#[test]
fn snapshots_are_bounded_even_when_the_reply_is_oversized() {
    let mut links = Vec::new();
    for index in 0..120 {
        links.push(json!({
            "url": format!("https://video.test/watch/{index}"),
            "label": "é".repeat(400),
        }));
    }
    let answer = json!({"document": "https://video.test/feed", "links": links});

    let snapshot = parse(&answer.to_string()).unwrap();

    assert_eq!(snapshot.links.len(), 100);
    assert_eq!(snapshot.links[0].label.chars().count(), 160);
    assert_eq!(
        snapshot.links.last().unwrap().url.as_str(),
        "https://video.test/watch/99"
    );
}

#[test]
fn unsupported_document_and_missing_snapshot_return_an_error() {
    let unsupported = json!({"document": "quickie://start/", "links": []});
    let missing = json!({"document": "https://video.test/feed"});
    let oversized = json!({
        "document": format!("https://video.test/{}", "abcde".repeat(500)),
        "links": [],
    });

    assert!(parse(&unsupported.to_string()).is_err());
    assert!(parse(&missing.to_string()).is_err());
    assert!(parse(&oversized.to_string()).is_err());
    assert!(parse("").is_err());
}
