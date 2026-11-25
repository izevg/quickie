//! Lifecycle, ownership, navigation, capacity, and stale-callback tests for `DiagnosticState`.

use url::{Host, Origin};

use super::super::{
    AgentContextId, AttachmentId, ClientId, ConsoleLevel, DiagnosticCapabilities, DiagnosticError,
    DiagnosticEventData, DiagnosticEventFilter, DiagnosticState, DocumentBinding, GrantScope,
    MAX_CLIENTS, NavigationKind, ObservationTarget, RevokeReason,
};
use super::{agent_tab, binding, main_tab};

#[test]
fn enable_rejects_non_main_tab_without_creating_state() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let target = binding(agent_tab(1), "https://example.com/work", 1);

    // Act
    let result = state.enable(
        client,
        target.clone(),
        Some(&target),
        GrantScope::SingleDocument,
    );

    // Assert
    assert_eq!(result, Err(DiagnosticError::InvalidApproval));
    assert_eq!(state.active_count(), 0);
    assert!(!state.has_attachments_for_tab(agent_tab(1)));
}

#[test]
fn enable_rejects_opaque_and_non_http_origins_without_creating_state() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let opaque = binding(main_tab(1), "about:blank", 1);
    let file_url = binding(main_tab(2), "file:///tmp/page.html", 1);
    let empty_host = DocumentBinding {
        tab: main_tab(3),
        origin: Origin::Tuple("https".to_owned(), Host::Domain(String::new()), 443),
        epoch: 1,
    };

    // Act
    let r1 = state.enable(
        client,
        opaque.clone(),
        Some(&opaque),
        GrantScope::SingleDocument,
    );
    let r2 = state.enable(
        client,
        file_url.clone(),
        Some(&file_url),
        GrantScope::SingleDocument,
    );
    let r3 = state.enable(
        client,
        empty_host.clone(),
        Some(&empty_host),
        GrantScope::SingleDocument,
    );

    // Assert
    assert_eq!(r1, Err(DiagnosticError::InvalidApproval));
    assert_eq!(r2, Err(DiagnosticError::InvalidApproval));
    assert_eq!(r3, Err(DiagnosticError::InvalidApproval));
    assert_eq!(state.active_count(), 0);
}

#[test]
fn enable_rejects_stale_or_missing_live_binding_without_mutating_existing_attachment() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let initial = binding(main_tab(1), "https://example.com/a", 1);
    let enabled = state
        .enable(
            client,
            initial.clone(),
            Some(&initial),
            GrantScope::SingleDocument,
        )
        .unwrap();
    let approved = binding(main_tab(2), "https://example.com/b", 2);
    let moved_live = binding(main_tab(2), "https://example.com/b", 3);

    // Act
    let stale_epoch = state.enable(
        client,
        approved.clone(),
        Some(&moved_live),
        GrantScope::SingleDocument,
    );
    let missing_live = state.enable(client, approved, None, GrantScope::SingleDocument);

    // Assert
    assert_eq!(stale_epoch, Err(DiagnosticError::InvalidApproval));
    assert_eq!(missing_live, Err(DiagnosticError::InvalidApproval));
    assert_eq!(state.active_count(), 1);
    assert_eq!(state.binding_for(client, enabled.attachment), Ok(&initial));
}

#[test]
fn cross_client_and_unissued_attachment_ids_return_identical_error() {
    // Arrange
    let mut state = DiagnosticState::new();
    let owner = ClientId::new(10);
    let intruder = ClientId::new(20);
    let live = binding(main_tab(1), "https://gitlab.example.com/items", 1);
    let enabled = state
        .enable(owner, live.clone(), Some(&live), GrantScope::SingleDocument)
        .unwrap();
    let wire_id = AttachmentId::from_wire(enabled.attachment.get());
    let unissued = AttachmentId::from_wire(999);

    // Act
    let wrong_owner_lookup = state.binding_for(intruder, wire_id).cloned();
    let unissued_lookup = state.binding_for(owner, unissued).cloned();
    let wrong_owner_read =
        state.read_events(intruder, wire_id, 0, &DiagnosticEventFilter::all(), 10);
    let wrong_owner_detach = state.detach(intruder, wire_id);

    // Assert
    assert_eq!(
        wrong_owner_lookup,
        Err(DiagnosticError::AttachmentUnavailable)
    );
    assert_eq!(unissued_lookup, Err(DiagnosticError::AttachmentUnavailable));
    assert_eq!(
        wrong_owner_read,
        Err(DiagnosticError::AttachmentUnavailable)
    );
    assert_eq!(
        wrong_owner_detach,
        Err(DiagnosticError::AttachmentUnavailable)
    );
    assert_eq!(state.binding_for(owner, wire_id), Ok(&live));
}

#[test]
fn same_document_navigation_keeps_epoch_and_grant() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let tab = main_tab(5);
    let live = binding(tab, "https://app.example.com/issues/1", 4);
    let enabled = state
        .enable(
            client,
            live.clone(),
            Some(&live),
            GrantScope::SingleDocument,
        )
        .unwrap();
    let spa_nav = binding(tab, "https://app.example.com/issues/2", 4);

    // Act
    let revoked = state.observe_navigation(tab, &spa_nav, NavigationKind::SameDocument);

    // Assert
    assert_eq!(revoked, []);
    assert_eq!(state.binding_for(client, enabled.attachment), Ok(&live));
}

#[test]
fn single_document_revokes_on_reload_and_new_document() {
    // Arrange
    let mut state = DiagnosticState::new();
    let c1 = ClientId::new(1);
    let c2 = ClientId::new(2);
    let tab1 = main_tab(1);
    let tab2 = main_tab(2);
    let b1 = binding(tab1, "https://app.example.com/a", 1);
    let b2 = binding(tab2, "https://app.example.com/b", 1);
    let e1 = state
        .enable(c1, b1.clone(), Some(&b1), GrantScope::SingleDocument)
        .unwrap();
    let e2 = state
        .enable(c2, b2.clone(), Some(&b2), GrantScope::SingleDocument)
        .unwrap();

    // Act
    let revoked_reload = state.observe_navigation(
        tab1,
        &binding(tab1, "https://app.example.com/a", 2),
        NavigationKind::Reload,
    );
    let revoked_new = state.observe_navigation(
        tab2,
        &binding(tab2, "https://app.example.com/c", 2),
        NavigationKind::NewDocument,
    );

    // Assert
    assert_eq!(revoked_reload.len(), 1);
    assert_eq!(revoked_reload[0].attachment, e1.attachment);
    assert_eq!(revoked_reload[0].reason, RevokeReason::NavigatedAway);
    assert_eq!(revoked_new.len(), 1);
    assert_eq!(revoked_new[0].attachment, e2.attachment);
    assert_eq!(revoked_new[0].reason, RevokeReason::NavigatedAway);
    assert_eq!(state.active_count(), 0);
}

#[test]
fn same_origin_reload_survives_only_higher_epoch_reload_and_updates_binding() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let tab = main_tab(3);
    let b1 = binding(tab, "https://app.example.com/work", 10);
    let enabled = state
        .enable(client, b1.clone(), Some(&b1), GrantScope::SameOriginReload)
        .unwrap();
    let reloaded = binding(tab, "https://app.example.com/work", 11);

    // Act
    let revoked = state.observe_navigation(tab, &reloaded, NavigationKind::Reload);

    // Assert
    assert_eq!(revoked, []);
    assert_eq!(state.binding_for(client, enabled.attachment), Ok(&reloaded));
}

#[test]
fn same_origin_reload_revokes_on_non_increasing_epoch_or_non_reload_navigation() {
    // Arrange
    let mut state = DiagnosticState::new();
    let c1 = ClientId::new(1);
    let c2 = ClientId::new(2);
    let tab1 = main_tab(1);
    let tab2 = main_tab(2);
    let b1 = binding(tab1, "https://app.example.com/work", 5);
    let b2 = binding(tab2, "https://app.example.com/work", 5);
    let e1 = state
        .enable(c1, b1.clone(), Some(&b1), GrantScope::SameOriginReload)
        .unwrap();
    let e2 = state
        .enable(c2, b2.clone(), Some(&b2), GrantScope::SameOriginReload)
        .unwrap();

    // Act
    let revoked_same_epoch = state.observe_navigation(
        tab1,
        &binding(tab1, "https://app.example.com/work", 5),
        NavigationKind::Reload,
    );
    let revoked_new_doc = state.observe_navigation(
        tab2,
        &binding(tab2, "https://app.example.com/other", 6),
        NavigationKind::NewDocument,
    );

    // Assert
    assert_eq!(revoked_same_epoch.len(), 1);
    assert_eq!(revoked_same_epoch[0].attachment, e1.attachment);
    assert_eq!(revoked_same_epoch[0].reason, RevokeReason::NavigatedAway);
    assert_eq!(revoked_new_doc.len(), 1);
    assert_eq!(revoked_new_doc[0].attachment, e2.attachment);
    assert_eq!(revoked_new_doc[0].reason, RevokeReason::NavigatedAway);
}

#[test]
fn cross_origin_navigation_always_revokes_even_for_same_origin_reload_scope() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let tab = main_tab(7);
    let live = binding(tab, "https://app.example.com/work", 1);
    let enabled = state
        .enable(
            client,
            live.clone(),
            Some(&live),
            GrantScope::SameOriginReload,
        )
        .unwrap();
    let foreign = binding(tab, "https://evil.example.org/work", 2);

    // Act
    let revoked = state.observe_navigation(tab, &foreign, NavigationKind::Reload);

    // Assert
    assert_eq!(revoked.len(), 1);
    assert_eq!(revoked[0].attachment, enabled.attachment);
    assert_eq!(revoked[0].reason, RevokeReason::CrossOriginNavigation);
    assert!(!state.has_attachments_for_tab(tab));
}

#[test]
fn wrong_tab_navigation_does_not_revoke_other_tabs_attachments() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let watched_tab = main_tab(1);
    let other_tab = main_tab(2);
    let live = binding(watched_tab, "https://app.example.com/work", 1);
    let enabled = state
        .enable(
            client,
            live.clone(),
            Some(&live),
            GrantScope::SingleDocument,
        )
        .unwrap();
    let foreign_other_tab = binding(other_tab, "https://other.example.org/", 2);

    // Act
    let revoked =
        state.observe_navigation(other_tab, &foreign_other_tab, NavigationKind::NewDocument);

    // Assert
    assert_eq!(revoked, []);
    assert_eq!(state.binding_for(client, enabled.attachment), Ok(&live));
}

#[test]
fn tab_close_process_crash_disconnect_and_manual_revoke_release_attachments_and_never_reuse_ids() {
    // Arrange
    let mut state = DiagnosticState::new();
    let c1 = ClientId::new(1);
    let c2 = ClientId::new(2);
    let c3 = ClientId::new(3);
    let c4 = ClientId::new(4);
    let b1 = binding(main_tab(1), "https://a.example.com/", 1);
    let b2 = binding(main_tab(2), "https://b.example.com/", 1);
    let b3 = binding(main_tab(3), "https://c.example.com/", 1);
    let b4 = binding(main_tab(4), "https://d.example.com/", 1);
    let e1 = state
        .enable(c1, b1.clone(), Some(&b1), GrantScope::SingleDocument)
        .unwrap();
    let e2 = state
        .enable(c2, b2.clone(), Some(&b2), GrantScope::SingleDocument)
        .unwrap();
    let e3 = state
        .enable(c3, b3.clone(), Some(&b3), GrantScope::SingleDocument)
        .unwrap();
    let e4 = state
        .enable(c4, b4.clone(), Some(&b4), GrantScope::SingleDocument)
        .unwrap();

    // Act
    let closed = state.on_tab_closed(main_tab(1));
    let crashed = state.on_process_terminated(main_tab(2));
    let disconnected = state.on_client_disconnected(c3);
    let manual = state.revoke(e4.attachment);
    let reissued = state
        .enable(c1, b1.clone(), Some(&b1), GrantScope::SingleDocument)
        .unwrap();

    // Assert
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].attachment, e1.attachment);
    assert_eq!(closed[0].reason, RevokeReason::TabClosed);
    assert_eq!(crashed.len(), 1);
    assert_eq!(crashed[0].attachment, e2.attachment);
    assert_eq!(crashed[0].reason, RevokeReason::ProcessCrashed);
    let disconnected = disconnected.unwrap();
    assert_eq!(disconnected.attachment, e3.attachment);
    assert_eq!(disconnected.reason, RevokeReason::ClientDisconnected);
    assert_eq!(manual.unwrap().reason, RevokeReason::UserRevoked);
    assert!(reissued.attachment.get() > e4.attachment.get());
    assert_ne!(reissued.attachment, e1.attachment);
    assert_eq!(state.active_count(), 1);
}

#[test]
fn client_capacity_is_capped_at_sixteen_and_replacing_same_client_stays_within_cap() {
    // Arrange
    let mut state = DiagnosticState::new();
    let tab = main_tab(1);
    let live = binding(tab, "https://app.example.com/", 1);
    let mut first_attachment = None;
    for raw in 1..=(MAX_CLIENTS as u64) {
        let enabled = state
            .enable(
                ClientId::new(raw),
                live.clone(),
                Some(&live),
                GrantScope::SingleDocument,
            )
            .unwrap();
        if raw == 1 {
            first_attachment = Some(enabled.attachment);
        }
    }

    // Act
    let over_cap = state.enable(
        ClientId::new((MAX_CLIENTS as u64) + 1),
        live.clone(),
        Some(&live),
        GrantScope::SingleDocument,
    );
    let replaced = state
        .enable(
            ClientId::new(1),
            live.clone(),
            Some(&live),
            GrantScope::SingleDocument,
        )
        .unwrap();
    let freed = state.on_client_disconnected(ClientId::new(2));
    let admitted = state.enable(
        ClientId::new((MAX_CLIENTS as u64) + 1),
        live.clone(),
        Some(&live),
        GrantScope::SingleDocument,
    );

    // Assert
    assert_eq!(over_cap, Err(DiagnosticError::ClientCapacityReached));
    assert_eq!(
        replaced.replaced.unwrap().attachment,
        first_attachment.unwrap()
    );
    assert_eq!(replaced.replaced.unwrap().reason, RevokeReason::Replaced);
    assert!(freed.is_some());
    assert!(admitted.is_ok());
    assert_eq!(state.active_count(), MAX_CLIENTS);
}

#[test]
fn stale_native_callbacks_cannot_append_to_reloaded_document_or_reissued_attachment() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let tab = main_tab(1);
    let epoch1 = binding(tab, "https://app.example.com/", 1);
    let first = state
        .enable(
            client,
            epoch1.clone(),
            Some(&epoch1),
            GrantScope::SameOriginReload,
        )
        .unwrap();
    let epoch2 = binding(tab, "https://app.example.com/", 2);
    assert_eq!(
        state.observe_navigation(tab, &epoch2, NavigationKind::Reload),
        []
    );

    // Act
    let stale_epoch_event = state.record_event(
        first.attachment,
        &epoch1,
        DiagnosticEventData::console(ConsoleLevel::Info, "from old document"),
    );
    let stale_epoch_snapshot = state.complete_snapshot(first.attachment, &epoch1, "<html>", false);
    let live_snapshot = state
        .complete_snapshot(first.attachment, &epoch2, "<html>ok</html>", false)
        .unwrap();
    let live_epoch_event = state.record_event(
        first.attachment,
        &epoch2,
        DiagnosticEventData::console(ConsoleLevel::Info, "from reloaded document"),
    );
    let second = state
        .enable(
            client,
            epoch2.clone(),
            Some(&epoch2),
            GrantScope::SingleDocument,
        )
        .unwrap();
    let retired_attachment_event = state.record_event(
        first.attachment,
        &epoch2,
        DiagnosticEventData::console(ConsoleLevel::Warn, "from retired attachment"),
    );
    let batch = state
        .read_events(
            client,
            second.attachment,
            0,
            &DiagnosticEventFilter::all(),
            10,
        )
        .unwrap();

    // Assert
    assert_eq!(
        stale_epoch_event,
        Err(DiagnosticError::AttachmentUnavailable)
    );
    assert_eq!(
        stale_epoch_snapshot,
        Err(DiagnosticError::AttachmentUnavailable)
    );
    assert_eq!(live_snapshot.epoch(), 2);
    assert_eq!(live_snapshot.data(), "<html>ok</html>");
    assert!(!live_snapshot.is_truncated());
    assert_eq!(live_epoch_event, Ok(1));
    assert_eq!(
        retired_attachment_event,
        Err(DiagnosticError::AttachmentUnavailable)
    );
    assert_eq!(batch.events, []);
}

#[test]
fn observation_target_and_capabilities_keep_planes_disjoint() {
    // Arrange
    let agent_ctx = AgentContextId::new(42);
    let att = AttachmentId::from_wire(42);
    let caps = DiagnosticCapabilities {
        dom: true,
        accessibility: true,
        console: false,
        runtime_errors: false,
        pending_network: false,
        completed_network: true,
        lifecycle: true,
    };

    // Act
    let diag_target = ObservationTarget::Diagnostic(att);
    let agent_target = ObservationTarget::Agent(agent_ctx);

    // Assert
    assert_ne!(diag_target, agent_target);
    assert_eq!(agent_ctx.get(), 42);
    assert_eq!(ClientId::new(7).get(), 7);
    assert_eq!(ClientId::new(7).to_string(), "client#7");
    assert_eq!(att.to_string(), "attachment#42");
    assert_eq!(agent_ctx.to_string(), "agent-context#42");
    assert!(caps.dom && caps.accessibility && caps.completed_network && caps.lifecycle);
    assert!(!caps.console && !caps.runtime_errors && !caps.pending_network);
}

#[test]
fn client_detach_and_shutdown_release_attachments_and_clear_tab_tracking() {
    // Arrange
    let mut state = DiagnosticState::new();
    let c1 = ClientId::new(1);
    let c2 = ClientId::new(2);
    let tab1 = main_tab(1);
    let tab2 = main_tab(2);
    let b1 = binding(tab1, "https://a.example.com/", 1);
    let b2 = binding(tab2, "https://b.example.com/", 1);
    let e1 = state
        .enable(c1, b1.clone(), Some(&b1), GrantScope::SingleDocument)
        .unwrap();
    let e2 = state
        .enable(c2, b2.clone(), Some(&b2), GrantScope::SingleDocument)
        .unwrap();

    // Act
    let has_before = state.has_attachments_for_tab(tab1);
    let detached = state.detach(c1, e1.attachment).unwrap();
    let has_after_detach = state.has_attachments_for_tab(tab1);
    let shutdown = state.revoke_all_for_shutdown();

    // Assert
    assert!(has_before);
    assert_eq!(detached.attachment, e1.attachment);
    assert_eq!(detached.reason, RevokeReason::ClientDetached);
    assert!(!has_after_detach);
    assert_eq!(shutdown.len(), 1);
    assert_eq!(shutdown[0].attachment, e2.attachment);
    assert_eq!(shutdown[0].reason, RevokeReason::BrowserShuttingDown);
    assert_eq!(state.active_count(), 0);
}

#[test]
fn mismatched_observed_tab_revokes_bound_tab_conservatively() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let tab1 = main_tab(1);
    let tab2 = main_tab(2);
    let b1 = binding(tab1, "https://a.example.com/", 1);
    let e1 = state
        .enable(client, b1.clone(), Some(&b1), GrantScope::SameOriginReload)
        .unwrap();
    let mismatched = binding(tab2, "https://a.example.com/", 2);

    // Act
    let revoked = state.observe_navigation(tab1, &mismatched, NavigationKind::Reload);

    // Assert
    assert_eq!(revoked.len(), 1);
    assert_eq!(revoked[0].attachment, e1.attachment);
    assert_eq!(revoked[0].reason, RevokeReason::NavigatedAway);
}

#[test]
fn same_document_navigation_with_mismatched_epoch_revokes_stale_grant() {
    // Arrange
    let mut state = DiagnosticState::new();
    let c1 = ClientId::new(1);
    let c2 = ClientId::new(2);
    let tab1 = main_tab(1);
    let tab2 = main_tab(2);
    let b1 = binding(tab1, "https://app.example.com/a", 4);
    let b2 = binding(tab2, "https://app.example.com/b", 4);
    let e1 = state
        .enable(c1, b1.clone(), Some(&b1), GrantScope::SingleDocument)
        .unwrap();
    let e2 = state
        .enable(c2, b2.clone(), Some(&b2), GrantScope::SameOriginReload)
        .unwrap();
    let higher_epoch = binding(tab1, "https://app.example.com/a#next", 5);
    let lower_epoch = binding(tab2, "https://app.example.com/b#prev", 3);

    // Act
    let revoked_higher =
        state.observe_navigation(tab1, &higher_epoch, NavigationKind::SameDocument);
    let revoked_lower = state.observe_navigation(tab2, &lower_epoch, NavigationKind::SameDocument);

    // Assert
    assert_eq!(revoked_higher.len(), 1);
    assert_eq!(revoked_higher[0].attachment, e1.attachment);
    assert_eq!(revoked_higher[0].reason, RevokeReason::NavigatedAway);
    assert_eq!(revoked_lower.len(), 1);
    assert_eq!(revoked_lower[0].attachment, e2.attachment);
    assert_eq!(revoked_lower[0].reason, RevokeReason::NavigatedAway);
    assert_eq!(state.active_count(), 0);
}

#[test]
fn enable_fails_without_revoking_existing_attachment_when_attachment_ids_exhaust() {
    // Arrange
    let mut state = DiagnosticState::new();
    let client = ClientId::new(1);
    let initial = binding(main_tab(1), "https://app.example.com/a", 1);
    let enabled = state
        .enable(
            client,
            initial.clone(),
            Some(&initial),
            GrantScope::SingleDocument,
        )
        .unwrap();
    let replacement = binding(main_tab(2), "https://app.example.com/b", 2);
    state.set_next_attachment_id(u64::MAX);

    // Act
    let exhausted = state.enable(
        client,
        replacement.clone(),
        Some(&replacement),
        GrantScope::SingleDocument,
    );

    // Assert
    assert_eq!(exhausted, Err(DiagnosticError::AttachmentIdExhausted));
    assert_eq!(state.active_count(), 1);
    assert_eq!(state.binding_for(client, enabled.attachment), Ok(&initial));
}
