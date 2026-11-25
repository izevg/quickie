//! Snapshot bounds, UTF-8 text truncation, and network metadata redaction tests.

use url::Url;

use super::super::{
    BoundedSnapshot, ConsoleLevel, DiagnosticEventData, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_DEPTH,
    MAX_SNAPSHOT_NODES, MAX_TEXT_EVENT_BYTES, NetworkEndpoint, NetworkEvent, NetworkPhase,
    SnapshotKind, SnapshotRequest,
};

#[test]
fn snapshot_request_clamps_nodes_depth_and_bytes_to_ceilings() {
    // Arrange & Act
    let clamped = SnapshotRequest::new(
        SnapshotKind::Accessibility,
        Some("main".to_owned()),
        99_999,
        128,
        512 * 1024,
    );
    let small = SnapshotRequest::new(SnapshotKind::Text, None, 10, 4, 256);
    let maxed = SnapshotRequest::with_max_bounds(SnapshotKind::Dom, Some("#root".to_owned()));

    // Assert
    assert_eq!(clamped.kind(), SnapshotKind::Accessibility);
    assert_eq!(clamped.selector(), Some("main"));
    assert_eq!(clamped.max_nodes(), MAX_SNAPSHOT_NODES);
    assert_eq!(clamped.max_depth(), MAX_SNAPSHOT_DEPTH);
    assert_eq!(clamped.max_bytes(), MAX_SNAPSHOT_BYTES);

    assert_eq!(small.kind(), SnapshotKind::Text);
    assert_eq!(small.selector(), None);
    assert_eq!(small.max_nodes(), 10);
    assert_eq!(small.max_depth(), 4);
    assert_eq!(small.max_bytes(), 256);

    assert_eq!(maxed.kind(), SnapshotKind::Dom);
    assert_eq!(maxed.selector(), Some("#root"));
    assert_eq!(maxed.max_nodes(), MAX_SNAPSHOT_NODES);
    assert_eq!(maxed.max_depth(), MAX_SNAPSHOT_DEPTH);
    assert_eq!(maxed.max_bytes(), MAX_SNAPSHOT_BYTES);
}

#[test]
fn bounded_snapshot_clamps_multibyte_utf8_at_64_kib_and_marks_truncated() {
    // Arrange
    let oversized = format!("a{}", "é".repeat(MAX_SNAPSHOT_BYTES));

    // Act
    let snapshot = BoundedSnapshot::new(7, &oversized, false);
    let exact = BoundedSnapshot::new(7, "hello", false);
    let already_truncated = BoundedSnapshot::new(7, "hello", true);

    // Assert
    assert_eq!(MAX_SNAPSHOT_BYTES, 65_536);
    assert_eq!(snapshot.epoch(), 7);
    assert_eq!(snapshot.data().len(), 65_535);
    assert!(snapshot.is_truncated());

    assert_eq!(exact.data(), "hello");
    assert!(!exact.is_truncated());
    assert!(already_truncated.is_truncated());
}

#[test]
fn textual_diagnostic_events_clamp_multibyte_utf8_at_4_kib() {
    // Arrange
    let long_emoji = format!("a{}", "🦀".repeat(2_000));

    // Act
    let console = DiagnosticEventData::console(ConsoleLevel::Error, &long_emoji);
    let runtime = DiagnosticEventData::runtime_error(&long_emoji);

    // Assert
    assert_eq!(MAX_TEXT_EVENT_BYTES, 4_096);
    let DiagnosticEventData::Console {
        level,
        text,
        truncated,
    } = console
    else {
        panic!("expected console payload");
    };
    assert_eq!(level, ConsoleLevel::Error);
    assert_eq!(text.len(), 4_093);
    assert!(truncated);

    let DiagnosticEventData::RuntimeError {
        message,
        truncated: rt_truncated,
    } = runtime
    else {
        panic!("expected runtime error payload");
    };
    assert_eq!(message.len(), 4_093);
    assert!(rt_truncated);
}

#[test]
fn network_metadata_strips_userinfo_query_and_fragment_and_rejects_non_http_schemes() {
    // Arrange
    let sensitive = Url::parse(
        "https://alice:supersecret@gitlab.example.com:8443/api/graphql?private_token=glpat-12345&code=xyz#access_token=abc",
    )
    .unwrap();
    let expected_origin = Url::parse("https://gitlab.example.com:8443/")
        .unwrap()
        .origin();
    let file_url = Url::parse("file:///etc/passwd").unwrap();
    let data_url = Url::parse("data:text/plain,secret").unwrap();
    let ws_url = Url::parse("wss://gitlab.example.com/cable?token=secret").unwrap();
    let blob_url =
        Url::parse("blob:https://gitlab.example.com/550e8400-e29b-41d4-a716-446655440000").unwrap();

    // Act
    let event = NetworkEvent::new(42, &sensitive, "post", Some(200), NetworkPhase::Completed)
        .expect("valid HTTPS endpoint must be accepted");
    let rejected_file = NetworkEndpoint::from_url(&file_url);
    let rejected_data = NetworkEndpoint::from_url(&data_url);
    let rejected_ws = NetworkEvent::new(43, &ws_url, "GET", None, NetworkPhase::Pending);
    let rejected_blob = NetworkEndpoint::from_url(&blob_url);
    let rejected_empty_method =
        NetworkEvent::new(44, &sensitive, "   ", None, NetworkPhase::Pending);
    let http_url = Url::parse("http://localhost:8080/health").unwrap();
    let http_event = NetworkEvent::new(45, &http_url, "get", Some(204), NetworkPhase::Completed)
        .expect("valid HTTP endpoint must be accepted");
    let long_path = format!(
        "https://example.com/{}",
        "a".repeat(MAX_TEXT_EVENT_BYTES + 10)
    );
    let long_url = Url::parse(&long_path).unwrap();
    let truncated_ep = NetworkEndpoint::from_url(&long_url).unwrap();

    // Assert
    assert_eq!(event.request_id(), 42);
    assert_eq!(event.endpoint().origin(), &expected_origin);
    assert_eq!(event.endpoint().path(), "/api/graphql");
    assert!(!event.endpoint().is_path_truncated());
    assert_eq!(event.method(), "POST");
    assert_eq!(event.status(), Some(200));
    assert_eq!(event.phase(), NetworkPhase::Completed);
    let serialized_debug = format!("{event:?}");
    assert!(!serialized_debug.contains("alice"));
    assert!(!serialized_debug.contains("supersecret"));
    assert!(!serialized_debug.contains("glpat-12345"));
    assert!(!serialized_debug.contains("access_token"));
    assert_eq!(rejected_file, None);
    assert_eq!(rejected_data, None);
    assert_eq!(rejected_ws, None);
    assert_eq!(rejected_blob, None);
    assert_eq!(rejected_empty_method, None);
    assert_eq!(http_event.endpoint().path(), "/health");
    assert!(truncated_ep.is_path_truncated());
    assert_eq!(truncated_ep.path().len(), MAX_TEXT_EVENT_BYTES);
}
