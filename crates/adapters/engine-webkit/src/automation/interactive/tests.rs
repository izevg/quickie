use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use serde_json::json;
use url::Url;

use super::{Operation, Pending, Stage};
use crate::automation::Reply;

#[test]
fn cancellation_revokes_callback_reply_and_scheduled_poll() {
    let called = Rc::new(Cell::new(false));
    let completed = Rc::clone(&called);
    let mut pending = Pending {
        operation: Some(operation(move |_| completed.set(true))),
        generation: 4,
    };
    let mut reply = Reply::default();
    reply.expect_id(7);
    reply.receive(&json!({"id": 7, "result": {}}).to_string());

    pending.cancel(&mut reply);
    reply.receive(&json!({"id": 7, "result": {}}).to_string());

    assert!(pending.operation.is_none());
    assert!(!called.get());
    assert!(reply.take().is_none());
    assert_ne!(pending.generation, 4);
}

#[test]
fn navigation_checks_live_eligibility_once_at_dispatch() {
    let revision = Rc::new(Cell::new(8));
    let current = Rc::clone(&revision);
    let mut navigation = operation(|_| {});
    navigation.eligible = Some(Box::new(move || current.get() == 8));

    // The observation was valid when queued, but changed before native dispatch.
    revision.set(9);
    assert!(!navigation.authorize());

    let mut navigation = operation(|_| {});
    let current = Rc::clone(&revision);
    navigation.eligible = Some(Box::new(move || current.get() == 9));
    assert!(navigation.authorize());
    // Its own navigation commit must not invalidate an already authorized dispatch.
    revision.set(10);
    assert!(navigation.authorize());
}

fn operation(complete: impl FnOnce(Result<(), crate::AutomationError>) + 'static) -> Operation {
    Operation {
        stage: Stage::Starting,
        deadline: Instant::now(),
        url: Url::parse("https://example.test/").expect("fixture URL"),
        complete: Box::new(complete),
        eligible: None,
    }
}
