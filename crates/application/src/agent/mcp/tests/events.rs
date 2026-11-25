//! Event ring replay, cursor validation, filtered paging, and eviction boundary tests.

use url::Url;

use super::super::{
    ClientId, ConsoleLevel, DiagnosticError, DiagnosticEventData, DiagnosticEventFilter,
    DiagnosticEventKind, DiagnosticState, GrantScope, LifecycleEventKind,
    MAX_EVENTS_PER_ATTACHMENT, NetworkEvent, NetworkPhase,
};
use super::{binding, main_tab};

#[test]
fn cursor_zero_replays_empty_and_mixed_stream_while_ahead_cursors_fail() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let live = binding(main_tab(1), "https://app.example.com/", 3);
    let att = state
        .enable(
            client,
            live.clone(),
            Some(&live),
            GrantScope::SingleDocument,
        )
        .unwrap()
        .attachment;

    // Act
    let empty_batch = state
        .read_events(client, att, 0, &DiagnosticEventFilter::all(), 10)
        .unwrap();
    let ahead_on_empty = state.read_events(client, att, 1, &DiagnosticEventFilter::all(), 10);
    let req_url = Url::parse("https://api.example.com/v1/items").unwrap();
    let net = NetworkEvent::new(1, &req_url, "GET", Some(200), NetworkPhase::Completed).unwrap();
    let s1 = state
        .record_event(
            att,
            &live,
            DiagnosticEventData::console(ConsoleLevel::Warn, "slow"),
        )
        .unwrap();
    let s2 = state
        .record_event(att, &live, DiagnosticEventData::Network(net))
        .unwrap();
    let s3 = state
        .record_event(
            att,
            &live,
            DiagnosticEventData::lifecycle(LifecycleEventKind::DomContentLoaded),
        )
        .unwrap();
    let full = state
        .read_events(client, att, 0, &DiagnosticEventFilter::all(), 10)
        .unwrap();
    let ahead_after_push = state.read_events(client, att, 4, &DiagnosticEventFilter::all(), 10);
    let oversized = state.read_events(client, att, u64::MAX, &DiagnosticEventFilter::all(), 10);

    // Assert
    assert_eq!(empty_batch.events, []);
    assert_eq!(empty_batch.next_cursor, 0);
    assert!(!empty_batch.has_more);
    assert_eq!(empty_batch.dropped_before, None);
    assert_eq!(ahead_on_empty, Err(DiagnosticError::InvalidCursor));
    assert_eq!((s1, s2, s3), (1, 2, 3));
    assert_eq!(full.events.len(), 3);
    assert_eq!(full.events[0].epoch, 3);
    assert_eq!(full.events[0].kind(), DiagnosticEventKind::Console);
    assert_eq!(full.events[1].kind(), DiagnosticEventKind::Network);
    assert_eq!(full.events[2].kind(), DiagnosticEventKind::Lifecycle);
    assert_eq!(full.next_cursor, 3);
    assert!(!full.has_more);
    assert_eq!(full.dropped_before, None);
    assert_eq!(ahead_after_push, Err(DiagnosticError::InvalidCursor));
    assert_eq!(oversized, Err(DiagnosticError::InvalidCursor));
}

#[test]
fn filtered_paging_advances_across_excluded_entries() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let live = binding(main_tab(1), "https://app.example.com/", 1);
    let att = state
        .enable(
            client,
            live.clone(),
            Some(&live),
            GrantScope::SingleDocument,
        )
        .unwrap()
        .attachment;
    let url = Url::parse("https://api.example.com/items").unwrap();
    let net = || {
        DiagnosticEventData::Network(
            NetworkEvent::new(1, &url, "GET", None, NetworkPhase::Pending).unwrap(),
        )
    };
    state.record_event(att, &live, net()).unwrap(); // seq 1 (excluded)
    state
        .record_event(
            att,
            &live,
            DiagnosticEventData::console(ConsoleLevel::Error, "e1"),
        )
        .unwrap(); // seq 2 (match)
    state.record_event(att, &live, net()).unwrap(); // seq 3 (excluded)
    state
        .record_event(att, &live, DiagnosticEventData::runtime_error("boom"))
        .unwrap(); // seq 4 (match)
    state.record_event(att, &live, net()).unwrap(); // seq 5 (trailing excluded)
    let filter = DiagnosticEventFilter::of_kinds([
        DiagnosticEventKind::Console,
        DiagnosticEventKind::RuntimeError,
    ]);

    // Act
    let page1 = state.read_events(client, att, 0, &filter, 1).unwrap();
    let page2 = state
        .read_events(client, att, page1.next_cursor, &filter, 1)
        .unwrap();
    let page3 = state
        .read_events(client, att, page2.next_cursor, &filter, 1)
        .unwrap();
    let only_lifecycle = DiagnosticEventFilter::of_kinds([DiagnosticEventKind::Lifecycle]);
    let skipped_all = state
        .read_events(client, att, 0, &only_lifecycle, 5)
        .unwrap();
    let zero_limit = state.read_events(client, att, 0, &filter, 0).unwrap();

    // Assert
    assert_eq!(page1.events.len(), 1);
    assert_eq!(page1.events[0].seq, 2);
    assert!(page1.has_more);
    assert_eq!(page1.next_cursor, 2);

    assert_eq!(page2.events.len(), 1);
    assert_eq!(page2.events[0].seq, 4);
    assert!(!page2.has_more);
    assert_eq!(page2.next_cursor, 5);

    assert_eq!(page3.events, []);
    assert!(!page3.has_more);
    assert_eq!(page3.next_cursor, 5);

    assert_eq!(skipped_all.events, []);
    assert!(!skipped_all.has_more);
    assert_eq!(skipped_all.next_cursor, 5);

    assert_eq!(zero_limit.events, []);
    assert!(zero_limit.has_more);
    assert_eq!(zero_limit.next_cursor, 0);
}

#[test]
fn ring_overflow_caps_at_1024_and_exposes_oldest_retained_cursor_boundary() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let live = binding(main_tab(1), "https://app.example.com/", 2);
    let att = state
        .enable(
            client,
            live.clone(),
            Some(&live),
            GrantScope::SingleDocument,
        )
        .unwrap()
        .attachment;
    let total = (MAX_EVENTS_PER_ATTACHMENT as u64) + 5;
    for index in 1..=total {
        state
            .record_event(
                att,
                &live,
                DiagnosticEventData::console(ConsoleLevel::Info, &format!("msg-{index}")),
            )
            .unwrap();
    }

    // Act
    let from_start = state
        .read_events(client, att, 0, &DiagnosticEventFilter::all(), 2_000)
        .unwrap();
    let from_boundary = state
        .read_events(client, att, 5, &DiagnosticEventFilter::all(), 2)
        .unwrap();

    // Assert
    assert_eq!(from_start.events.len(), MAX_EVENTS_PER_ATTACHMENT);
    assert_eq!(from_start.events[0].seq, 6);
    assert_eq!(from_start.events.last().unwrap().seq, total);
    assert_eq!(from_start.dropped_before, Some(6));
    assert_eq!(from_start.next_cursor, total);
    assert!(!from_start.has_more);

    assert_eq!(from_boundary.events.len(), 2);
    assert_eq!(from_boundary.events[0].seq, 6);
    assert_eq!(from_boundary.dropped_before, None);
    assert!(from_boundary.has_more);
    assert_eq!(from_boundary.next_cursor, 7);
}
