use serde_json::json;
use url::Url;

use super::{AutomationError, Reply, reply, validate_url};

#[test]
fn matches_only_the_pending_request() {
    let mut reply = Reply::default();
    reply.expect_id(2);

    reply.receive(r#"{"id":1,"result":{}}"#);
    reply.receive(r#"{"method":"Automation.browsingContextCleared"}"#);
    assert!(reply.take().is_none());
    reply.receive(r#"{"id":2,"result":{"contexts":[]}}"#);
    reply.receive(r#"{"id":2,"error":{"message":"Timeout"}}"#);

    let result = reply
        .take()
        .expect("reply arrived")
        .expect("first reply wins");
    assert_eq!(result, json!({"contexts": []}));
}

#[test]
fn requires_a_valid_context_handle() {
    let missing_handle = json!({"contexts": [{"url":"https://example.test/"}]});

    assert!(matches!(
        reply::contexts(&missing_handle),
        Err(AutomationError::InvalidReply)
    ));
}

#[test]
fn accepts_web_urls_without_credentials() {
    let accepted = ["https://example.test/", "http://127.0.0.1:12345/"];
    let rejected = [
        "https://name:secret@example.test/",
        "file:///tmp/fixture",
        "data:text/html,hello",
        "javascript:void(0)",
    ];

    for address in accepted {
        let url = Url::parse(address).expect("parsed fixture URL");
        assert!(validate_url(&url).is_ok(), "{address}");
    }
    for address in rejected {
        let url = Url::parse(address).expect("parsed fixture URL");
        assert!(
            matches!(validate_url(&url), Err(AutomationError::InvalidUrl)),
            "{address}"
        );
    }
}
