use quickie_application::mcp::{
    ConsoleLevel, DiagnosticEventData, LifecycleEventKind, NetworkPhase,
};
use quickie_application::{DiagnosticGapReason, DiagnosticSignal};
use serde_json::{Value, json};
use url::Url;

use super::{MAX_PROTOCOL_MESSAGE_BYTES, ObservationDecoder, ObservationDomain, ObserveCommand};

fn target_created(target_id: &str, target_type: &str, provisional: bool) -> String {
    json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": target_id,
                "type": target_type,
                "isProvisional": provisional,
            }
        }
    })
    .to_string()
}

fn target_destroyed(target_id: &str) -> String {
    json!({
        "method": "Target.targetDestroyed",
        "params": {
            "targetId": target_id,
        }
    })
    .to_string()
}

fn provisional_committed(old_target_id: &str, new_target_id: &str) -> String {
    json!({
        "method": "Target.didCommitProvisionalTarget",
        "params": {
            "oldTargetId": old_target_id,
            "newTargetId": new_target_id,
        }
    })
    .to_string()
}

fn from_target(target_id: &str, inner: &Value) -> String {
    json!({
        "method": "Target.dispatchMessageFromTarget",
        "params": {
            "targetId": target_id,
            "message": inner.to_string(),
        }
    })
    .to_string()
}

#[test]
fn target_creation_emits_only_allowlisted_enable_commands_and_enable_replies_emit_no_events() {
    // Arrange
    let mut decoder = ObservationDecoder::new();

    // Act
    let created = decoder.decode_message(&target_created("page-1", "page", false));
    let outer_reply = decoder.decode_message(&json!({ "id": 1, "result": {} }).to_string());
    let inner_reply =
        decoder.decode_message(&from_target("page-1", &json!({ "id": 2, "result": {} })));

    // Assert
    assert!(decoder.is_active_target("page-1"));
    assert_eq!(created.signals, Vec::new());
    assert_eq!(created.commands.len(), 4);
    let inner_methods: Vec<String> = created
        .commands
        .iter()
        .map(|cmd| {
            let outer: Value = serde_json::from_str(cmd).expect("valid outer JSON");
            assert_eq!(outer["method"], "Target.sendMessageToTarget");
            assert_eq!(outer["params"]["targetId"], "page-1");
            let inner: Value =
                serde_json::from_str(outer["params"]["message"].as_str().expect("inner string"))
                    .expect("valid inner JSON");
            inner["method"].as_str().expect("method").to_owned()
        })
        .collect();
    assert_eq!(
        inner_methods,
        vec![
            "Console.enable",
            "Runtime.enable",
            "Network.enable",
            "Page.enable",
        ]
    );
    assert_eq!(outer_reply.signals, Vec::new());
    assert_eq!(inner_reply.signals, Vec::new());
}

#[test]
fn direct_commands_allow_only_network_and_page_domains() {
    // Arrange
    let mut decoder = ObservationDecoder::new();

    // Act
    let direct = decoder.initial_direct_commands();
    let console = decoder.encode_direct_command(ObserveCommand::Enable(ObservationDomain::Console));
    let runtime = decoder.encode_direct_command(ObserveCommand::Enable(ObservationDomain::Runtime));

    // Assert
    assert_eq!(direct.len(), 2);
    assert!(console.is_none());
    assert!(runtime.is_none());
}

#[test]
fn console_warning_and_javascript_source_error_map_to_distinct_event_variants() {
    // Arrange
    let mut decoder = ObservationDecoder::new();
    let _ = decoder.decode_message(&target_created("page-1", "page", false));
    let warning_msg = from_target(
        "page-1",
        &json!({
            "method": "Console.messageAdded",
            "params": {
                "message": {
                    "source": "console-api",
                    "level": "warning",
                    "text": "deprecated call"
                }
            }
        }),
    );
    let js_error_msg = from_target(
        "page-1",
        &json!({
            "method": "Console.messageAdded",
            "params": {
                "message": {
                    "source": "javascript",
                    "level": "error",
                    "text": "TypeError: undefined is not an object"
                }
            }
        }),
    );

    // Act
    let warning = decoder.decode_message(&warning_msg);
    let runtime_error = decoder.decode_message(&js_error_msg);

    // Assert
    assert_eq!(
        warning.signals,
        vec![DiagnosticSignal::Event(DiagnosticEventData::console(
            ConsoleLevel::Warn,
            "deprecated call",
        ))]
    );
    assert_eq!(
        runtime_error.signals,
        vec![DiagnosticSignal::Event(DiagnosticEventData::runtime_error(
            "TypeError: undefined is not an object",
        ))]
    );
}

#[test]
fn page_dom_ready_and_load_events_decode_in_both_direct_and_target_forms() {
    // Arrange
    let mut decoder = ObservationDecoder::new();
    let _ = decoder.decode_message(&target_created("page-1", "page", false));
    let dom_ready_direct = json!({
        "method": "Page.domContentEventFired",
        "params": { "timestamp": 12.5 }
    })
    .to_string();
    let load_targeted = from_target(
        "page-1",
        &json!({
            "method": "Page.loadEventFired",
            "params": { "timestamp": 18.2 }
        }),
    );

    // Act
    let dom_ready = decoder.decode_message(&dom_ready_direct);
    let load = decoder.decode_message(&load_targeted);

    // Assert
    assert_eq!(
        dom_ready.signals,
        vec![DiagnosticSignal::Event(DiagnosticEventData::lifecycle(
            LifecycleEventKind::DomContentLoaded,
        ))]
    );
    assert_eq!(
        load.signals,
        vec![DiagnosticSignal::Event(DiagnosticEventData::lifecycle(
            LifecycleEventKind::Load,
        ))]
    );
}

#[test]
fn network_lifecycle_correlates_pending_response_completion_failure_and_redacts_url() {
    // Arrange
    let mut decoder = ObservationDecoder::new();
    let _ = decoder.decode_message(&target_created("page-1", "page", false));
    let sensitive_url = "https://alice:s3cr3t@example.com:8443/api/items?token=abc#frag";

    // Act
    let sent = decoder.decode_message(&from_target(
        "page-1",
        &json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": "req-1",
                "frameId": "frame-1",
                "loaderId": "loader-1",
                "documentURL": "https://example.com:8443/",
                "request": {
                    "url": sensitive_url,
                    "method": "post",
                    "headers": { "Authorization": "Bearer secret", "Cookie": "sid=xyz" },
                    "postData": "{\"password\":\"hunter2\"}"
                },
                "timestamp": 1.0,
                "walltime": 100.0,
                "initiator": { "type": "script" }
            }
        }),
    ));
    let response = decoder.decode_message(&from_target(
        "page-1",
        &json!({
            "method": "Network.responseReceived",
            "params": {
                "requestId": "req-1",
                "frameId": "frame-1",
                "loaderId": "loader-1",
                "timestamp": 1.5,
                "type": "Fetch",
                "response": {
                    "url": sensitive_url,
                    "status": 201,
                    "statusText": "Created",
                    "headers": { "Set-Cookie": "sid=new" },
                    "mimeType": "application/json",
                    "source": "network"
                }
            }
        }),
    ));
    let finished = decoder.decode_message(&from_target(
        "page-1",
        &json!({
            "method": "Network.loadingFinished",
            "params": { "requestId": "req-1", "timestamp": 2.0 }
        }),
    ));

    // Assert
    assert_eq!(
        response.signals,
        Vec::new(),
        "response receipt is not completion"
    );
    let [DiagnosticSignal::Event(DiagnosticEventData::Network(pending))] = sent.signals.as_slice()
    else {
        panic!(
            "expected single pending network event, got {:?}",
            sent.signals
        );
    };
    let [DiagnosticSignal::Event(DiagnosticEventData::Network(done))] = finished.signals.as_slice()
    else {
        panic!(
            "expected single completed network event, got {:?}",
            finished.signals
        );
    };
    let expected_origin = Url::parse("https://example.com:8443")
        .expect("origin")
        .origin();
    assert_eq!(pending.request_id(), done.request_id());
    assert_eq!(pending.phase(), NetworkPhase::Pending);
    assert_eq!(pending.status(), None);
    assert_eq!(pending.method(), "POST");
    assert_eq!(pending.endpoint().origin(), &expected_origin);
    assert_eq!(pending.endpoint().path(), "/api/items");
    assert_eq!(done.phase(), NetworkPhase::Completed);
    assert_eq!(done.status(), Some(201));
    assert_eq!(done.endpoint().path(), "/api/items");
    assert_eq!(decoder.tracked_request_count(), 0);
}

#[test]
fn network_redirect_completes_previous_hop_and_tracks_redirect_target_until_failed() {
    // Arrange
    let mut decoder = ObservationDecoder::new();
    let _ = decoder.decode_message(&target_created("page-1", "page", false));
    let _ = decoder.decode_message(&from_target(
        "page-1",
        &json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": "req-redir",
                "request": { "url": "https://example.com/old?secret=1", "method": "GET" }
            }
        }),
    ));

    // Act
    let redirected = decoder.decode_message(&from_target(
        "page-1",
        &json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": "req-redir",
                "request": { "url": "https://cdn.example.com/new?key=2", "method": "GET" },
                "redirectResponse": {
                    "url": "https://example.com/old?secret=1",
                    "status": 302,
                    "statusText": "Found",
                    "headers": { "Location": "https://cdn.example.com/new?key=2" },
                    "mimeType": "text/html",
                    "source": "network"
                }
            }
        }),
    ));
    let failed = decoder.decode_message(&from_target(
        "page-1",
        &json!({
            "method": "Network.loadingFailed",
            "params": {
                "requestId": "req-redir",
                "timestamp": 3.0,
                "errorText": "Timed out",
                "canceled": false
            }
        }),
    ));

    // Assert
    assert_eq!(redirected.signals.len(), 2);
    let DiagnosticSignal::Event(DiagnosticEventData::Network(hop_done)) = &redirected.signals[0]
    else {
        panic!("expected completed redirect hop");
    };
    let DiagnosticSignal::Event(DiagnosticEventData::Network(hop_next)) = &redirected.signals[1]
    else {
        panic!("expected pending redirected request");
    };
    let [DiagnosticSignal::Event(DiagnosticEventData::Network(terminal))] =
        failed.signals.as_slice()
    else {
        panic!("expected failed terminal event");
    };
    assert_eq!(hop_done.phase(), NetworkPhase::Completed);
    assert_eq!(hop_done.status(), Some(302));
    assert_eq!(hop_done.endpoint().path(), "/old");
    assert_ne!(hop_done.request_id(), hop_next.request_id());
    assert_eq!(hop_next.phase(), NetworkPhase::Pending);
    assert_eq!(hop_next.endpoint().path(), "/new");
    assert_eq!(terminal.request_id(), hop_next.request_id());
    assert_eq!(terminal.phase(), NetworkPhase::Failed);
    assert_eq!(terminal.status(), None);
}

#[test]
fn identical_protocol_request_ids_across_separate_targets_never_collide() {
    // Arrange
    let mut decoder = ObservationDecoder::new();
    let _ = decoder.decode_message(&target_created("page-1", "page", false));
    let _ = decoder.decode_message(&target_created("frame-2", "frame", false));

    // Act
    let first = decoder.decode_message(&from_target(
        "page-1",
        &json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": "42",
                "request": { "url": "https://a.example/one", "method": "GET" }
            }
        }),
    ));
    let second = decoder.decode_message(&from_target(
        "frame-2",
        &json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": "42",
                "request": { "url": "https://b.example/two", "method": "GET" }
            }
        }),
    ));
    let finished_second = decoder.decode_message(&from_target(
        "frame-2",
        &json!({
            "method": "Network.loadingFinished",
            "params": { "requestId": "42" }
        }),
    ));

    // Assert
    let [DiagnosticSignal::Event(DiagnosticEventData::Network(req_a))] = first.signals.as_slice()
    else {
        panic!("expected req_a");
    };
    let [DiagnosticSignal::Event(DiagnosticEventData::Network(req_b))] = second.signals.as_slice()
    else {
        panic!("expected req_b");
    };
    let [DiagnosticSignal::Event(DiagnosticEventData::Network(done_b))] =
        finished_second.signals.as_slice()
    else {
        panic!("expected done_b");
    };
    assert_ne!(req_a.request_id(), req_b.request_id());
    assert_eq!(done_b.request_id(), req_b.request_id());
    assert_eq!(done_b.endpoint().path(), "/two");
    assert_eq!(decoder.tracked_request_count(), 1);
}

#[test]
fn stale_provisional_destroyed_targets_and_disallowed_command_output_produce_no_events() {
    // Arrange
    let mut decoder = ObservationDecoder::new();
    let _ = decoder.decode_message(&target_created("page-1", "page", false));
    let _ = decoder.decode_message(&target_created("page-2", "page", true));
    let _ = decoder.decode_message(&target_destroyed("page-1"));

    // Act
    let from_destroyed = decoder.decode_message(&from_target(
        "page-1",
        &json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 } }),
    ));
    let from_provisional = decoder.decode_message(&from_target(
        "page-2",
        &json!({ "method": "Page.loadEventFired", "params": { "timestamp": 2.0 } }),
    ));
    let disallowed_result = decoder.decode_message(
        &json!({
            "id": 999,
            "result": { "result": { "type": "string", "value": "secret-cookie" } }
        })
        .to_string(),
    );
    let oversized = decoder.decode_message(&"x".repeat(MAX_PROTOCOL_MESSAGE_BYTES + 1));

    // Assert
    assert_eq!(from_destroyed.signals, Vec::new());
    assert_eq!(from_provisional.signals, Vec::new());
    assert_eq!(disallowed_result.signals, Vec::new());
    assert_eq!(oversized.signals, Vec::new());
}

#[test]
fn provisional_commit_promotes_new_target_and_retires_old_target() {
    // Arrange
    let mut decoder = ObservationDecoder::new();
    let _ = decoder.decode_message(&target_created("page-1", "page", false));
    let _ = decoder.decode_message(&target_created("page-2", "page", true));

    // Act
    let committed = decoder.decode_message(&provisional_committed("page-1", "page-2"));
    let old_event = decoder.decode_message(&from_target(
        "page-1",
        &json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 } }),
    ));
    let new_event = decoder.decode_message(&from_target(
        "page-2",
        &json!({ "method": "Page.loadEventFired", "params": { "timestamp": 2.0 } }),
    ));

    // Assert
    assert!(!decoder.is_active_target("page-1"));
    assert!(decoder.is_active_target("page-2"));
    assert_eq!(committed.commands.len(), 4);
    assert_eq!(old_event.signals, Vec::new());
    assert_eq!(
        new_event.signals,
        vec![DiagnosticSignal::Event(DiagnosticEventData::lifecycle(
            LifecycleEventKind::Load,
        ))]
    );
}

#[test]
fn bounded_request_map_evicts_oldest_with_explicit_gap_and_native_overflow_clears_pending() {
    // Arrange
    let mut decoder = ObservationDecoder::with_max_tracked_requests(2);
    let _ = decoder.decode_message(&target_created("page-1", "page", false));
    for id in ["r1", "r2"] {
        let _ = decoder.decode_message(&from_target(
            "page-1",
            &json!({
                "method": "Network.requestWillBeSent",
                "params": {
                    "requestId": id,
                    "request": { "url": format!("https://example.com/{id}"), "method": "GET" }
                }
            }),
        ));
    }

    // Act
    let third = decoder.decode_message(&from_target(
        "page-1",
        &json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": "r3",
                "request": { "url": "https://example.com/r3", "method": "GET" }
            }
        }),
    ));
    let native_overflow = decoder.decode_message(
        &json!({
            "method": "LocalDiagnostic.queueOverflow",
            "params": {
                "droppedCount": 7,
                "totalDroppedCount": 11,
                "maxPendingCallbacks": 512
            }
        })
        .to_string(),
    );
    let late_finish = decoder.decode_message(&from_target(
        "page-1",
        &json!({
            "method": "Network.loadingFinished",
            "params": { "requestId": "r3" }
        }),
    ));

    // Assert
    assert_eq!(
        third.signals.first(),
        Some(&DiagnosticSignal::Gap(
            DiagnosticGapReason::RequestMapOverflow { evicted: 1 }
        ))
    );
    assert_eq!(
        native_overflow.signals,
        vec![DiagnosticSignal::Gap(
            DiagnosticGapReason::NativeQueueOverflow {
                dropped: 7,
                total_dropped: 11,
            }
        )]
    );
    assert_eq!(decoder.tracked_request_count(), 0);
    assert_eq!(late_finish.signals, Vec::new());
}

#[test]
fn native_channel_closed_notification_emits_gap_and_retires_active_targets() {
    // Arrange
    let mut decoder = ObservationDecoder::new();
    let _ = decoder.decode_message(&target_created("page-1", "page", false));

    // Act
    let closed = decoder.decode_message(
        &json!({
            "method": "LocalDiagnostic.closed",
            "params": {}
        })
        .to_string(),
    );
    let after_close = decoder.decode_message(&from_target(
        "page-1",
        &json!({ "method": "Page.loadEventFired", "params": { "timestamp": 4.0 } }),
    ));

    // Assert
    assert_eq!(
        closed.signals,
        vec![DiagnosticSignal::Gap(DiagnosticGapReason::ChannelClosed)]
    );
    assert!(!decoder.is_active_target("page-1"));
    assert_eq!(after_close.signals, Vec::new());
}
