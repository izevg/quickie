use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use url::Url;

use super::dispatcher::{AutomationRequestId, Dispatcher, InboundDispatch};
use super::protocol::{
    ApprovedOrigin, AutomationEvent, CancelInteractionSequenceCommand, CommandResponse,
    EncodedCommand, InputSource, InputSourceState, InputSourceType, InteractionSequenceCommand,
    InteractionStep, KeyModifier, KeyboardCommand, KeyboardStep, MAX_HANDLE_BYTES,
    MAX_INBOUND_MESSAGE_BYTES, MAX_KEYBOARD_INTERACTIONS, MAX_KEYBOARD_TEXT_BYTES,
    MAX_PAGE_LOAD_TIMEOUT_MS, MAX_SCREENSHOT_BASE64_BYTES, MAX_SEQUENCE_DURATION_MS,
    MAX_VIEWPORT_COORD, MouseButton, MouseCommand, MouseInteraction, MouseMoveOrigin,
    NavigationCommand, PageLoadStrategy, ScreenshotCommand, ScreenshotData, ViewportPoint,
    VirtualKey, encode_cancel_interaction_sequence, encode_get_browsing_contexts,
    encode_interaction_sequence, encode_keyboard_interactions, encode_mouse_interaction,
    encode_navigate, encode_take_screenshot,
};
use super::{AgentAutomation, AutomationError};

type TestObserver = Rc<dyn Fn(AutomationEvent)>;

/// Pure fake transport harness that mirrors `AgentState`'s `RefCell<Dispatcher>` borrow discipline
/// without touching `AppKit`, `WKWebView`, or any OS window.
struct FakeHarness {
    dispatcher: RefCell<Dispatcher>,
    sent: RefCell<Vec<String>>,
    observer: RefCell<Option<TestObserver>>,
}

impl FakeHarness {
    fn new(max_in_flight: usize) -> Rc<Self> {
        Rc::new(Self {
            dispatcher: RefCell::new(Dispatcher::new(max_in_flight)),
            sent: RefCell::new(Vec::new()),
            observer: RefCell::new(None),
        })
    }

    fn set_observer(&self, observer: impl Fn(AutomationEvent) + 'static) {
        *self.observer.borrow_mut() = Some(Rc::new(observer));
    }

    fn send(
        &self,
        command: &EncodedCommand,
        is_navigation: bool,
        done: impl FnOnce(Result<CommandResponse, AutomationError>) + 'static,
    ) -> Result<AutomationRequestId, AutomationError> {
        let (id, wire) = self
            .dispatcher
            .borrow_mut()
            .begin(command, is_navigation, Box::new(done))?;
        self.sent.borrow_mut().push(wire);
        Ok(id)
    }

    fn deliver(&self, raw: &str) {
        let action = self.dispatcher.borrow_mut().receive(raw);
        match action {
            InboundDispatch::Reply {
                completion,
                outcome,
            } => completion.finish(outcome),
            InboundDispatch::Event(event) => {
                let observer = self.observer.borrow().clone();
                if let Some(observer) = observer {
                    observer(event);
                }
            }
            InboundDispatch::Ignored => {}
        }
    }

    fn cancel(&self, id: AutomationRequestId) -> bool {
        let completion = self.dispatcher.borrow_mut().cancel(id);
        let Some(completion) = completion else {
            return false;
        };
        completion.finish(Err(AutomationError::Cancelled));
        true
    }

    fn timeout(&self, id: AutomationRequestId) -> bool {
        let completion = self.dispatcher.borrow_mut().timeout(id);
        let Some(completion) = completion else {
            return false;
        };
        completion.finish(Err(AutomationError::Timeout));
        true
    }

    fn close(&self) {
        let _ = self.observer.borrow_mut().take();
        let pending = self.dispatcher.borrow_mut().close();
        for completion in pending {
            completion.finish(Err(AutomationError::Disconnected));
        }
    }
}

#[test]
fn matches_independent_out_of_order_replies_with_unique_ids() {
    // Arrange
    let harness = FakeHarness::new(4);
    let log = Rc::new(RefCell::new(Vec::<String>::new()));

    let log_a = Rc::clone(&log);
    let id_contexts = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            let contexts = match result {
                Ok(CommandResponse::Contexts(contexts)) => contexts,
                other => panic!("expected contexts, got {other:?}"),
            };
            log_a
                .borrow_mut()
                .push(format!("contexts:{}", contexts[0].handle()));
        })
        .expect("contexts request accepted");

    let click = MouseCommand::single_click("ctx-1", ViewportPoint { x: 12.5, y: 34.0 });
    let click_cmd = encode_mouse_interaction(&click).expect("valid click command");
    let log_b = Rc::clone(&log);
    let id_click = harness
        .send(&click_cmd, false, move |result| {
            let point = match result {
                Ok(CommandResponse::Position(point)) => point,
                other => panic!("expected position, got {other:?}"),
            };
            log_b
                .borrow_mut()
                .push(format!("click:{},{}", point.x, point.y));
        })
        .expect("click request accepted");

    let shot_cmd = encode_take_screenshot(&ScreenshotCommand::viewport("ctx-1"))
        .expect("valid screenshot command");
    let log_c = Rc::clone(&log);
    let id_shot = harness
        .send(&shot_cmd, false, move |result| {
            let data = match result {
                Ok(CommandResponse::Screenshot(ScreenshotData { png_base64 })) => png_base64,
                other => panic!("expected screenshot, got {other:?}"),
            };
            log_c.borrow_mut().push(format!("shot:{data}"));
        })
        .expect("screenshot request accepted");

    // Act: reply in reverse order (shot -> contexts -> click).
    harness.deliver(&json!({"id": id_shot.get(), "result": {"data": "iVBORw0KGgo="}}).to_string());
    harness.deliver(
        &json!({
            "id": id_contexts.get(),
            "result": {"contexts": [{"handle": "ctx-1", "url": "https://example.test/"}]}
        })
        .to_string(),
    );
    harness.deliver(
        &json!({"id": id_click.get(), "result": {"position": {"x": 12.5, "y": 34.0}}})
            .to_string(),
    );

    // Assert
    assert!(id_contexts.get() < id_click.get() && id_click.get() < id_shot.get());
    assert_eq!(harness.dispatcher.borrow().in_flight_count(), 0);
    assert_eq!(
        *log.borrow(),
        vec!["shot:iVBORw0KGgo=", "contexts:ctx-1", "click:12.5,34"]
    );
}

#[test]
fn routes_unsolicited_events_and_drops_raw_inspector_and_bidi_messages() {
    // Arrange
    let harness = FakeHarness::new(4);
    let events = Rc::new(RefCell::new(Vec::<AutomationEvent>::new()));
    let observed = Rc::clone(&events);
    harness.set_observer(move |event| observed.borrow_mut().push(event));

    // Act
    harness.deliver(
        r#"{"method":"Automation.browsingContextCleared","params":{"browsingContextHandle":"ctx-7"}}"#,
    );
    harness.deliver(
        r#"{"method":"Automation.fileChooserDismissed","params":{"browsingContextHandle":"ctx-7","selectionCancelled":false,"selectedFiles":["/Users/secret/a.txt","/Users/secret/b.txt"]}}"#,
    );
    harness.deliver(
        r#"{"method":"Automation.receiveInspectorMessage","params":{"browsingContextHandle":"ctx-7","message":"{\"method\":\"Runtime.evaluate\"}"}}"#,
    );
    harness.deliver(
        r#"{"method":"Automation.bidiMessageSent","params":{"message":"{\"type\":\"event\"}"}}"#,
    );
    harness.deliver(r#"{"method":"Automation.unknownEvent","params":{}}"#);

    // Assert
    assert_eq!(
        *events.borrow(),
        vec![
            AutomationEvent::BrowsingContextCleared {
                browsing_context_handle: "ctx-7".to_owned(),
            },
            AutomationEvent::FileChooserDismissed {
                browsing_context_handle: "ctx-7".to_owned(),
                selection_cancelled: false,
                selected_files_count: 2,
            },
        ]
    );
}

#[test]
fn cancellation_completes_only_target_and_ignores_late_reply() {
    // Arrange
    let harness = FakeHarness::new(4);
    let first_outcome = Rc::new(RefCell::new(None));
    let second_outcome = Rc::new(RefCell::new(None));

    let first_slot = Rc::clone(&first_outcome);
    let id1 = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            *first_slot.borrow_mut() = Some(result);
        })
        .expect("first accepted");

    let second_slot = Rc::clone(&second_outcome);
    let id2 = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            *second_slot.borrow_mut() = Some(result);
        })
        .expect("second accepted");

    // Act
    let cancelled = harness.cancel(id1);
    let cancelled_again = harness.cancel(id1);
    harness.deliver(&json!({"id": id1.get(), "result": {"contexts": []}}).to_string());
    harness.deliver(
        &json!({"id": id2.get(), "result": {"contexts": [{"handle": "ctx-2", "url": "https://example.test/"}]}})
            .to_string(),
    );

    // Assert
    assert!(cancelled);
    assert!(!cancelled_again);
    assert!(matches!(
        first_outcome.borrow().as_ref(),
        Some(Err(AutomationError::Cancelled))
    ));
    assert!(matches!(
        second_outcome.borrow().as_ref(),
        Some(Ok(CommandResponse::Contexts(contexts))) if contexts.len() == 1 && contexts[0].handle() == "ctx-2"
    ));
}

#[test]
fn timeout_completes_only_timed_out_operation_and_keeps_session_open() {
    // Arrange
    let harness = FakeHarness::new(4);
    let timed_out_slot = Rc::new(RefCell::new(None));
    let healthy_slot = Rc::new(RefCell::new(None));

    let slot1 = Rc::clone(&timed_out_slot);
    let id1 = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            *slot1.borrow_mut() = Some(result);
        })
        .expect("first accepted");

    let slot2 = Rc::clone(&healthy_slot);
    let id2 = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            *slot2.borrow_mut() = Some(result);
        })
        .expect("second accepted");

    // Act
    assert!(harness.timeout(id1));
    harness.deliver(&json!({"id": id1.get(), "result": {"contexts": []}}).to_string());
    harness.deliver(&json!({"id": id2.get(), "result": {"contexts": []}}).to_string());

    // Assert
    assert!(matches!(
        timed_out_slot.borrow().as_ref(),
        Some(Err(AutomationError::Timeout))
    ));
    assert!(matches!(
        healthy_slot.borrow().as_ref(),
        Some(Ok(CommandResponse::Contexts(contexts))) if contexts.is_empty()
    ));
    assert!(!harness.dispatcher.borrow().is_closed());
}

#[test]
fn close_completes_all_pending_replies_and_rejects_further_requests() {
    // Arrange
    let harness = FakeHarness::new(4);
    let completed = Rc::new(RefCell::new(Vec::<(u64, bool)>::new()));

    let log1 = Rc::clone(&completed);
    let id1 = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            log1.borrow_mut()
                .push((1, matches!(result, Err(AutomationError::Disconnected))));
        })
        .expect("first accepted");
    let log2 = Rc::clone(&completed);
    let id2 = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            log2.borrow_mut()
                .push((2, matches!(result, Err(AutomationError::Disconnected))));
        })
        .expect("second accepted");

    // Act
    harness.close();
    harness.deliver(&json!({"id": id1.get(), "result": {"contexts": []}}).to_string());
    harness.deliver(&json!({"id": id2.get(), "result": {"contexts": []}}).to_string());
    let after_close = harness.send(&encode_get_browsing_contexts(), false, |_| {});

    // Assert
    assert_eq!(*completed.borrow(), vec![(1, true), (2, true)]);
    assert!(matches!(after_close, Err(AutomationError::Disconnected)));
}

#[test]
fn callbacks_do_not_hold_refcell_borrows_and_support_reentrancy() {
    // Arrange
    let harness = FakeHarness::new(4);
    let followup_done = Rc::new(RefCell::new(false));
    let followup_id = Rc::new(RefCell::new(None));

    let nested_harness = Rc::clone(&harness);
    let nested_done = Rc::clone(&followup_done);
    let nested_id_slot = Rc::clone(&followup_id);
    let id1 = harness
        .send(&encode_get_browsing_contexts(), false, move |_| {
            let id = nested_harness
                .send(&encode_get_browsing_contexts(), false, move |result| {
                    *nested_done.borrow_mut() = result.is_ok();
                })
                .expect("re-entrant dispatch must not panic on RefCell borrow");
            *nested_id_slot.borrow_mut() = Some(id);
        })
        .expect("first accepted");

    // Act
    harness.deliver(&json!({"id": id1.get(), "result": {"contexts": []}}).to_string());
    let id2 = followup_id
        .borrow()
        .expect("re-entrant request was registered");
    harness.deliver(&json!({"id": id2.get(), "result": {"contexts": []}}).to_string());

    // Assert
    assert!(id2.get() > id1.get());
    assert!(*followup_done.borrow());
}

#[test]
fn ignores_stale_duplicate_unknown_and_oversized_inbound_messages_without_reusing_ids() {
    // Arrange
    let harness = FakeHarness::new(2);
    let hits = Rc::new(RefCell::new(0_usize));

    let (rolled_back_id, _) = harness
        .dispatcher
        .borrow_mut()
        .begin(&encode_get_browsing_contexts(), false, Box::new(|_| {}))
        .expect("reserved");
    harness
        .dispatcher
        .borrow_mut()
        .rollback_dispatch(rolled_back_id);

    let hits_clone = Rc::clone(&hits);
    let active_id = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            if result.is_ok() {
                *hits_clone.borrow_mut() += 1;
            }
        })
        .expect("active accepted");

    // Act
    harness.deliver(&json!({"id": rolled_back_id.get(), "result": {"contexts": []}}).to_string());
    harness.deliver(
        &json!({"id": AutomationRequestId::from_raw(999).get(), "result": {"contexts": []}})
            .to_string(),
    );
    harness.deliver("not valid json");
    let oversized = " ".repeat(MAX_INBOUND_MESSAGE_BYTES + 1);
    harness.deliver(&oversized);
    harness.deliver(&json!({"id": active_id.get(), "result": {"contexts": []}}).to_string());
    // Duplicate reply for already-settled ID must be ignored.
    harness.deliver(&json!({"id": active_id.get(), "error": {"message": "Timeout"}}).to_string());

    // Assert
    assert!(active_id.get() > rolled_back_id.get());
    assert_eq!(*hits.borrow(), 1);
}

#[test]
fn enforces_in_flight_request_bound() {
    // Arrange
    let harness = FakeHarness::new(2);
    let id1 = harness
        .send(&encode_get_browsing_contexts(), false, |_| {})
        .expect("slot 1");
    let _id2 = harness
        .send(&encode_get_browsing_contexts(), false, |_| {})
        .expect("slot 2");

    // Act
    let third = harness.send(&encode_get_browsing_contexts(), false, |_| {});
    assert!(harness.cancel(id1));
    let fourth = harness.send(&encode_get_browsing_contexts(), false, |_| {});

    // Assert
    assert!(matches!(third, Err(AutomationError::Busy)));
    assert!(fourth.is_ok());
}

#[test]
fn enforces_typed_navigation_origin_and_url_security() {
    // Arrange
    let approved_url = Url::parse("https://example.test/start").expect("valid URL");
    let approved = ApprovedOrigin::from_url(&approved_url).expect("valid origin");
    let same_origin = Url::parse("https://example.test/next?q=1").expect("valid URL");
    let cross_host = Url::parse("https://evil.test/next").expect("valid URL");
    let cross_scheme = Url::parse("http://example.test/next").expect("valid URL");
    let cross_port = Url::parse("https://example.test:8443/next").expect("valid URL");
    let with_creds = Url::parse("https://user:pass@example.test/next").expect("valid URL");
    let file_url = Url::parse("file:///etc/passwd").expect("valid URL");

    // Act
    let valid_cmd = NavigationCommand {
        handle: "ctx-1".to_owned(),
        url: same_origin,
        approved_origin: approved.clone(),
        page_load_strategy: PageLoadStrategy::Eager,
        page_load_timeout_ms: 4_500,
    };
    let encoded = encode_navigate(&valid_cmd).expect("same-origin navigation accepted");

    // Assert
    assert_eq!(encoded.method, "Automation.navigateBrowsingContext");
    assert_eq!(
        encoded.params,
        json!({
            "handle": "ctx-1",
            "url": "https://example.test/next?q=1",
            "pageLoadStrategy": "Eager",
            "pageLoadTimeout": 4_500,
        })
    );
    assert!(matches!(
        encode_navigate(&NavigationCommand::new("ctx-1", cross_host, approved.clone())),
        Err(AutomationError::OriginMismatch)
    ));
    assert!(matches!(
        encode_navigate(&NavigationCommand::new(
            "ctx-1",
            cross_scheme,
            approved.clone()
        )),
        Err(AutomationError::OriginMismatch)
    ));
    assert!(matches!(
        encode_navigate(&NavigationCommand::new(
            "ctx-1",
            cross_port,
            approved.clone()
        )),
        Err(AutomationError::OriginMismatch)
    ));
    assert!(matches!(
        encode_navigate(&NavigationCommand::new("ctx-1", with_creds, approved)),
        Err(AutomationError::InvalidUrl)
    ));
    assert!(matches!(
        ApprovedOrigin::from_url(&file_url),
        Err(AutomationError::InvalidUrl)
    ));
    assert!(matches!(
        encode_navigate(&NavigationCommand {
            handle: "ctx-1".to_owned(),
            url: approved_url,
            approved_origin: ApprovedOrigin::from_url(
                &Url::parse("https://example.test/").expect("valid")
            )
            .expect("origin"),
            page_load_strategy: PageLoadStrategy::Normal,
            page_load_timeout_ms: MAX_PAGE_LOAD_TIMEOUT_MS + 1,
        }),
        Err(AutomationError::InvalidCommand)
    ));
}

#[test]
fn encodes_exact_automation_json_shapes() {
    // Arrange
    let mouse = MouseCommand {
        handle: "ctx-1".to_owned(),
        position: ViewportPoint { x: 40.0, y: 80.0 },
        button: MouseButton::Left,
        interaction: MouseInteraction::DoubleClick,
        modifiers: vec![KeyModifier::Shift, KeyModifier::Meta],
    };
    let keyboard = KeyboardCommand {
        handle: "ctx-1".to_owned(),
        interactions: vec![
            KeyboardStep::PressKey(VirtualKey::Shift),
            KeyboardStep::InsertText("hi".to_owned()),
            KeyboardStep::ReleaseKey(VirtualKey::Shift),
            KeyboardStep::InsertKey(VirtualKey::Enter),
        ],
    };
    let sequence = InteractionSequenceCommand {
        handle: "ctx-1".to_owned(),
        frame_handle: Some("frame-1".to_owned()),
        input_sources: vec![InputSource {
            source_id: "mouse-1".to_owned(),
            source_type: InputSourceType::Mouse,
        }],
        steps: vec![InteractionStep {
            states: vec![InputSourceState {
                origin: Some(MouseMoveOrigin::Viewport),
                location: Some(ViewportPoint { x: 10.0, y: 20.0 }),
                duration_ms: Some(100),
                ..InputSourceState::new("mouse-1")
            }],
        }],
    };
    let cancel_seq = CancelInteractionSequenceCommand {
        handle: "ctx-1".to_owned(),
        frame_handle: Some("frame-1".to_owned()),
    };

    // Act
    let mouse_encoded = encode_mouse_interaction(&mouse).expect("valid mouse");
    let kb_encoded = encode_keyboard_interactions(&keyboard).expect("valid keyboard");
    let seq_encoded = encode_interaction_sequence(&sequence).expect("valid sequence");
    let cancel_encoded =
        encode_cancel_interaction_sequence(&cancel_seq).expect("valid cancel sequence");

    // Assert exact Automation.json method and parameter names.
    assert_eq!(mouse_encoded.method, "Automation.performMouseInteraction");
    assert_eq!(
        mouse_encoded.params,
        json!({
            "handle": "ctx-1",
            "position": {"x": 40.0, "y": 80.0},
            "button": "Left",
            "interaction": "DoubleClick",
            "modifiers": ["Shift", "Meta"],
        })
    );
    assert_eq!(kb_encoded.method, "Automation.performKeyboardInteractions");
    assert_eq!(
        kb_encoded.params,
        json!({
            "handle": "ctx-1",
            "interactions": [
                {"type": "KeyPress", "key": "Shift"},
                {"type": "InsertByKey", "text": "hi"},
                {"type": "KeyRelease", "key": "Shift"},
                {"type": "InsertByKey", "key": "Enter"},
            ],
        })
    );
    assert_eq!(seq_encoded.method, "Automation.performInteractionSequence");
    assert_eq!(
        seq_encoded.params,
        json!({
            "handle": "ctx-1",
            "frameHandle": "frame-1",
            "inputSources": [{"sourceId": "mouse-1", "sourceType": "Mouse"}],
            "steps": [{"states": [{
                "sourceId": "mouse-1",
                "origin": "Viewport",
                "location": {"x": 10.0, "y": 20.0},
                "duration": 100,
            }]}],
        })
    );
    assert_eq!(cancel_encoded.method, "Automation.cancelInteractionSequence");
    assert_eq!(
        cancel_encoded.params,
        json!({"handle": "ctx-1", "frameHandle": "frame-1"})
    );
}

#[test]
fn rejects_out_of_bounds_command_parameters() {
    // Arrange
    let dup_modifiers = MouseCommand {
        modifiers: vec![KeyModifier::Alt, KeyModifier::Alt],
        ..MouseCommand::single_click("ctx-1", ViewportPoint { x: 40.0, y: 80.0 })
    };
    let too_many_keys = KeyboardCommand {
        handle: "ctx-1".to_owned(),
        interactions: vec![KeyboardStep::InsertKey(VirtualKey::Tab); MAX_KEYBOARD_INTERACTIONS + 1],
    };
    let too_much_text = KeyboardCommand {
        handle: "ctx-1".to_owned(),
        interactions: vec![KeyboardStep::InsertText(
            "a".repeat(MAX_KEYBOARD_TEXT_BYTES + 1),
        )],
    };
    let unknown_source = InteractionSequenceCommand {
        handle: "ctx-1".to_owned(),
        frame_handle: None,
        input_sources: vec![InputSource {
            source_id: "src-1".to_owned(),
            source_type: InputSourceType::Null,
        }],
        steps: vec![InteractionStep {
            states: vec![InputSourceState {
                duration_ms: Some(MAX_SEQUENCE_DURATION_MS),
                ..InputSourceState::new("unknown-src")
            }],
        }],
    };

    // Act & Assert
    assert!(matches!(
        encode_mouse_interaction(&MouseCommand::single_click(
            "x".repeat(MAX_HANDLE_BYTES + 1),
            ViewportPoint { x: 1.0, y: 1.0 }
        )),
        Err(AutomationError::InvalidCommand)
    ));
    assert!(matches!(
        encode_mouse_interaction(&MouseCommand::single_click(
            "ctx-1",
            ViewportPoint {
                x: MAX_VIEWPORT_COORD + 1.0,
                y: 1.0
            }
        )),
        Err(AutomationError::InvalidCommand)
    ));
    assert!(matches!(
        encode_mouse_interaction(&dup_modifiers),
        Err(AutomationError::InvalidCommand)
    ));
    assert!(matches!(
        encode_keyboard_interactions(&too_many_keys),
        Err(AutomationError::InvalidCommand)
    ));
    assert!(matches!(
        encode_keyboard_interactions(&too_much_text),
        Err(AutomationError::InvalidCommand)
    ));
    assert!(matches!(
        encode_interaction_sequence(&unknown_source),
        Err(AutomationError::InvalidCommand)
    ));
}

#[test]
fn enforces_bounded_screenshot_limits_and_rejects_oversized_or_invalid_replies() {
    // Arrange
    let harness = FakeHarness::new(4);
    let mut command = ScreenshotCommand::viewport("ctx-1");
    command.max_base64_bytes = 8;
    let encoded = encode_take_screenshot(&command).expect("valid bounded screenshot");

    // Verify exact Automation.json method and params.
    assert_eq!(encoded.method, "Automation.takeScreenshot");
    assert_eq!(
        encoded.params,
        json!({"handle": "ctx-1", "clipToViewport": true})
    );

    let too_large = Rc::new(RefCell::new(None));
    let slot_large = Rc::clone(&too_large);
    let id_large = harness
        .send(&encoded, false, move |result| {
            *slot_large.borrow_mut() = Some(result);
        })
        .expect("dispatched");

    let bad_chars = Rc::new(RefCell::new(None));
    let slot_bad = Rc::clone(&bad_chars);
    let id_bad = harness
        .send(&encoded, false, move |result| {
            *slot_bad.borrow_mut() = Some(result);
        })
        .expect("dispatched");

    // Act
    harness.deliver(&json!({"id": id_large.get(), "result": {"data": "QUJDREVGR0hJ"}}).to_string());
    harness.deliver(&json!({"id": id_bad.get(), "result": {"data": "not*b64!"}}).to_string());

    // Assert
    assert!(matches!(
        too_large.borrow().as_ref(),
        Some(Err(AutomationError::ResponseTooLarge))
    ));
    assert!(matches!(
        bad_chars.borrow().as_ref(),
        Some(Err(AutomationError::InvalidReply))
    ));
    assert!(matches!(
        encode_take_screenshot(&ScreenshotCommand {
            max_base64_bytes: MAX_SCREENSHOT_BASE64_BYTES + 1,
            ..command
        }),
        Err(AutomationError::InvalidCommand)
    ));
}

#[test]
fn maps_webkit_protocol_errors_to_typed_automation_errors() {
    // Arrange
    let harness = FakeHarness::new(4);
    let timeout_out = Rc::new(RefCell::new(None));
    let failed_out = Rc::new(RefCell::new(None));

    let slot1 = Rc::clone(&timeout_out);
    let id1 = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            *slot1.borrow_mut() = Some(result);
        })
        .expect("id1");
    let slot2 = Rc::clone(&failed_out);
    let id2 = harness
        .send(&encode_get_browsing_contexts(), false, move |result| {
            *slot2.borrow_mut() = Some(result);
        })
        .expect("id2");

    // Act
    harness.deliver(&json!({"id": id1.get(), "error": {"message": "Timeout"}}).to_string());
    harness.deliver(
        &json!({"id": id2.get(), "error": {"message": "NotImplemented: screenshot"}}).to_string(),
    );

    // Assert
    assert!(matches!(
        timeout_out.borrow().as_ref(),
        Some(Err(AutomationError::Timeout))
    ));
    assert!(matches!(
        failed_out.borrow().as_ref(),
        Some(Err(AutomationError::CommandFailed))
    ));
}

#[test]
fn stock_webkit_reports_honest_capabilities_without_main_tab_fallback() {
    // Arrange & Act
    let caps = AgentAutomation::capabilities();

    // Assert: on stock macOS WebKit (no fork loaded in unit tests), isolated automation and
    // local diagnostic SPI are honestly reported as unavailable without touching any fork
    // selector or Main tab.
    if std::env::var("QUICKIE_AUTOMATION_FRAMEWORK").ok().as_deref() != Some("fork") {
        assert!(!caps.isolated_automation);
        assert!(!caps.observations.console);
        assert!(!caps.observations.pending_network);
        assert!(!caps.observations.completed_network);
        assert!(!caps.observations.lifecycle);
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            assert!(matches!(
                AgentAutomation::open(mtm),
                Err(AutomationError::Unsupported)
            ));
        }
    }
}

#[test]
fn wire_payload_contains_no_raw_inspector_or_javascript_methods() {
    // Arrange
    let harness = FakeHarness::new(4);
    let url = Url::parse("https://example.test/page").expect("valid");
    let origin = ApprovedOrigin::from_url(&url).expect("origin");
    let nav = encode_navigate(&NavigationCommand::new("ctx-1", url, origin)).expect("nav");

    // Act
    let _ = harness
        .send(&encode_get_browsing_contexts(), false, |_| {})
        .expect("contexts");
    let _ = harness.send(&nav, true, |_| {}).expect("nav");

    // Assert
    for raw in harness.sent.borrow().iter() {
        let parsed: Value = serde_json::from_str(raw).expect("valid JSON");
        let method = parsed["method"].as_str().expect("method string");
        assert!(method.starts_with("Automation."));
        assert!(!method.contains("Inspector"));
        assert!(!method.contains("JavaScript"));
        assert!(!method.contains("Bidi"));
    }
}
