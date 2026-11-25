//! Focused fake-ABI unit tests for GTK native Inspector observation and ordinary Inspector.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_ulong, c_void};
use std::ptr;
use std::rc::Rc;

use quickie_domain_browsing::{Profile, TabId};

use crate::gtk::ffi::Object;

use super::observation::{
    ClosedCallback, DestroyCallback, DirectSubscription, InspectorAbi, MAX_TARGET_ID_BYTES,
    MessageCallback, ObservationAvailability, ObservationError, ObservationLease, ObserverCommand,
    SelectedTabGrant, TargetSubscription, availability_with,
};
use super::{TOGGLE_COMMAND, ToggleAction, toggle_action};

#[derive(Debug, Clone, PartialEq, Eq)]
enum AbiEvent {
    InspectorForView(usize),
    RefInspector(usize),
    UnrefInspector(usize),
    ConnectMessageSignal(c_ulong),
    ConnectClosedSignal(c_ulong),
    DisconnectSignal(c_ulong),
    DestroySignalData(c_ulong),
    ConnectNative(usize),
    SendMessage(String),
    DisconnectNative(usize),
}

struct MessageSlot {
    inspector: usize,
    handler: MessageCallback,
    data: Object,
    destroy: DestroyCallback,
}

struct ClosedSlot {
    inspector: usize,
    handler: ClosedCallback,
    data: Object,
    destroy: DestroyCallback,
}

struct FakeState {
    main_thread: bool,
    has_connect: bool,
    has_send: bool,
    has_disconnect: bool,
    inspector: Option<usize>,
    message_signal_ok: bool,
    closed_signal_ok: bool,
    connect_ok: bool,
    send_ok: bool,
    next_handler_id: c_ulong,
    message_slots: BTreeMap<c_ulong, MessageSlot>,
    closed_slots: BTreeMap<c_ulong, ClosedSlot>,
    events: Vec<AbiEvent>,
    ref_count: isize,
}

impl FakeState {
    fn ready(inspector: usize) -> Self {
        Self {
            main_thread: true,
            has_connect: true,
            has_send: true,
            has_disconnect: true,
            inspector: Some(inspector),
            message_signal_ok: true,
            closed_signal_ok: true,
            connect_ok: true,
            send_ok: true,
            next_handler_id: 1,
            message_slots: BTreeMap::new(),
            closed_slots: BTreeMap::new(),
            events: Vec::new(),
            ref_count: 0,
        }
    }
}

#[derive(Clone)]
struct FakeAbi {
    state: Rc<RefCell<FakeState>>,
}

impl FakeAbi {
    fn ready(inspector: usize) -> Self {
        Self {
            state: Rc::new(RefCell::new(FakeState::ready(inspector))),
        }
    }

    fn emit_message(&self, frame: &str) {
        let slots: Vec<_> = {
            let state = self.state.borrow();
            state
                .message_slots
                .values()
                .map(|slot| (slot.inspector, slot.handler, slot.data))
                .collect()
        };
        let payload = CString::new(frame).expect("test frame has no NUL");
        for (inspector, handler, data) in slots {
            handler(
                ptr::without_provenance_mut(inspector),
                payload.as_ptr(),
                data,
            );
        }
    }

    fn emit_closed(&self) {
        let slots: Vec<_> = {
            let state = self.state.borrow();
            state
                .closed_slots
                .values()
                .map(|slot| (slot.inspector, slot.handler, slot.data))
                .collect()
        };
        for (inspector, handler, data) in slots {
            handler(ptr::without_provenance_mut(inspector), data);
        }
    }

    fn events(&self) -> Vec<AbiEvent> {
        self.state.borrow().events.clone()
    }

    fn ref_count(&self) -> isize {
        self.state.borrow().ref_count
    }

    fn connected_signal_count(&self) -> usize {
        let state = self.state.borrow();
        state.message_slots.len() + state.closed_slots.len()
    }
}

impl InspectorAbi for FakeAbi {
    fn is_main_thread(&self) -> bool {
        self.state.borrow().main_thread
    }

    fn has_diagnostic_symbols(&self) -> bool {
        let state = self.state.borrow();
        state.has_connect && state.has_send && state.has_disconnect
    }

    fn inspector_for_view(&self, view: Object) -> Object {
        let mut state = self.state.borrow_mut();
        state.events.push(AbiEvent::InspectorForView(view.addr()));
        state
            .inspector
            .map_or(ptr::null_mut(), ptr::without_provenance_mut)
    }

    fn ref_inspector(&self, inspector: Object) {
        let mut state = self.state.borrow_mut();
        state.ref_count += 1;
        state.events.push(AbiEvent::RefInspector(inspector.addr()));
    }

    fn unref_inspector(&self, inspector: Object) {
        let mut state = self.state.borrow_mut();
        state.ref_count -= 1;
        state
            .events
            .push(AbiEvent::UnrefInspector(inspector.addr()));
    }

    fn connect_message_signal(
        &self,
        inspector: Object,
        handler: MessageCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let mut state = self.state.borrow_mut();
        if !state.message_signal_ok {
            return 0;
        }
        let id = state.next_handler_id;
        state.next_handler_id += 1;
        state.message_slots.insert(
            id,
            MessageSlot {
                inspector: inspector.addr(),
                handler,
                data,
                destroy,
            },
        );
        state.events.push(AbiEvent::ConnectMessageSignal(id));
        id
    }

    fn connect_closed_signal(
        &self,
        inspector: Object,
        handler: ClosedCallback,
        data: Object,
        destroy: DestroyCallback,
    ) -> c_ulong {
        let mut state = self.state.borrow_mut();
        if !state.closed_signal_ok {
            return 0;
        }
        let id = state.next_handler_id;
        state.next_handler_id += 1;
        state.closed_slots.insert(
            id,
            ClosedSlot {
                inspector: inspector.addr(),
                handler,
                data,
                destroy,
            },
        );
        state.events.push(AbiEvent::ConnectClosedSignal(id));
        id
    }

    fn disconnect_signal(&self, _inspector: Object, handler_id: c_ulong) {
        let destroyed = {
            let mut state = self.state.borrow_mut();
            state.events.push(AbiEvent::DisconnectSignal(handler_id));
            if let Some(slot) = state.message_slots.remove(&handler_id) {
                Some((slot.destroy, slot.data))
            } else {
                state
                    .closed_slots
                    .remove(&handler_id)
                    .map(|slot| (slot.destroy, slot.data))
            }
        };
        if let Some((destroy, data)) = destroyed {
            // SAFETY: Matches GObject's destroy-notify contract for our owned test closure data.
            unsafe { destroy(data, ptr::null_mut()) };
            self.state
                .borrow_mut()
                .events
                .push(AbiEvent::DestroySignalData(handler_id));
        }
    }

    fn connect_local_diagnostic_client(&self, inspector: Object) -> bool {
        let mut state = self.state.borrow_mut();
        state
            .events
            .push(AbiEvent::ConnectNative(inspector as usize));
        state.connect_ok
    }

    fn send_diagnostic_message(&self, _inspector: Object, message: &CStr) -> bool {
        let mut state = self.state.borrow_mut();
        state.events.push(AbiEvent::SendMessage(
            message.to_string_lossy().into_owned(),
        ));
        state.send_ok
    }

    fn disconnect_local_diagnostic_client(&self, inspector: Object) {
        let mut state = self.state.borrow_mut();
        state
            .events
            .push(AbiEvent::DisconnectNative(inspector as usize));
    }
}

fn test_grant(tab_number: u64) -> SelectedTabGrant {
    SelectedTabGrant::new(TabId::new(Profile::Main, tab_number), "grant-valid-1")
        .expect("non-empty grant")
}

#[test]
fn availability_requires_all_dynamic_symbols_on_main_thread() {
    // Arrange
    let all_present = FakeAbi::ready(0x1001);
    let missing_connect = FakeAbi::ready(0x1001);
    missing_connect.state.borrow_mut().has_connect = false;
    let missing_send = FakeAbi::ready(0x1001);
    missing_send.state.borrow_mut().has_send = false;
    let missing_disconnect = FakeAbi::ready(0x1001);
    missing_disconnect.state.borrow_mut().has_disconnect = false;
    let wrong_thread = FakeAbi::ready(0x1001);
    wrong_thread.state.borrow_mut().main_thread = false;

    // Act
    let results = (
        availability_with(&all_present),
        availability_with(&missing_connect),
        availability_with(&missing_send),
        availability_with(&missing_disconnect),
        availability_with(&wrong_thread),
    );

    // Assert
    assert_eq!(
        results,
        (
            ObservationAvailability::Available,
            ObservationAvailability::UnsupportedSpi,
            ObservationAvailability::UnsupportedSpi,
            ObservationAvailability::UnsupportedSpi,
            ObservationAvailability::WrongThread,
        )
    );
}

#[test]
fn attach_succeeds_forwards_frames_and_dispatches_validated_commands() {
    // Arrange
    let abi = FakeAbi::ready(0x2001);
    let view = 0x9001usize as Object;
    let grant = test_grant(7);
    let frames = Rc::new(RefCell::new(Vec::<String>::new()));
    let frames_sink = Rc::clone(&frames);
    let direct = ObserverCommand::direct(1, DirectSubscription::NetworkEnable);
    let targeted = ObserverCommand::for_target(2, "page-1", 10, TargetSubscription::ConsoleEnable)
        .expect("valid target command");

    // Act
    let mut lease = ObservationLease::attach_with(
        abi.clone(),
        grant.clone(),
        view,
        move |frame| frames_sink.borrow_mut().push(frame.to_owned()),
        || {},
    )
    .expect("attach succeeds");
    abi.emit_message(r#"{"method":"Console.messageAdded","params":{}}"#);
    let direct_result = lease.dispatch_subscription(&direct);
    let target_result = lease.dispatch_subscription(&targeted);

    // Assert
    assert!(lease.is_open());
    assert_eq!(lease.target(), &grant);
    assert_eq!(direct_result, Ok(()));
    assert_eq!(target_result, Ok(()));
    assert_eq!(
        frames.borrow().as_slice(),
        &[r#"{"method":"Console.messageAdded","params":{}}"#]
    );
    assert_eq!(abi.ref_count(), 1);
    assert!(abi.events().contains(&AbiEvent::InspectorForView(0x9001)));
    assert!(
        abi.events()
            .contains(&AbiEvent::SendMessage(direct.as_json().to_owned()))
    );
    assert!(
        abi.events()
            .contains(&AbiEvent::SendMessage(targeted.as_json().to_owned()))
    );
}

#[test]
fn attach_rejects_unsupported_spi_and_missing_inspector_distinctly() {
    // Arrange
    let unsupported = FakeAbi::ready(0x2101);
    unsupported.state.borrow_mut().has_send = false;
    let missing_inspector = FakeAbi::ready(0x2102);
    missing_inspector.state.borrow_mut().inspector = None;
    let wrong_thread = FakeAbi::ready(0x2103);
    wrong_thread.state.borrow_mut().main_thread = false;

    // Act
    let unsupported_err = ObservationLease::attach_with(
        unsupported.clone(),
        test_grant(1),
        0x9101usize as Object,
        |_| {},
        || {},
    )
    .err();
    let missing_err = ObservationLease::attach_with(
        missing_inspector.clone(),
        test_grant(2),
        0x9102usize as Object,
        |_| {},
        || {},
    )
    .err();
    let thread_err = ObservationLease::attach_with(
        wrong_thread,
        test_grant(3),
        0x9103usize as Object,
        |_| {},
        || {},
    )
    .err();

    // Assert
    assert_eq!(unsupported_err, Some(ObservationError::UnsupportedSpi));
    assert_eq!(missing_err, Some(ObservationError::ClosedStream));
    assert_eq!(thread_err, Some(ObservationError::WrongThread));
    assert!(unsupported.events().is_empty());
    assert_eq!(missing_inspector.ref_count(), 0);
}

#[test]
fn duplicate_attach_for_same_inspector_returns_occupied_without_native_connect() {
    // Arrange
    let abi = FakeAbi::ready(0x2201);
    let _first = ObservationLease::attach_with(
        abi.clone(),
        test_grant(1),
        0x9201usize as Object,
        |_| {},
        || {},
    )
    .expect("first attach succeeds");
    let before = abi.events().len();

    // Act
    let duplicate = ObservationLease::attach_with(
        abi.clone(),
        test_grant(1),
        0x9201usize as Object,
        |_| {},
        || {},
    );

    // Assert
    assert_eq!(duplicate.err(), Some(ObservationError::OccupiedInspector));
    assert_eq!(
        &abi.events()[before..],
        &[AbiEvent::InspectorForView(0x9201)]
    );
    assert_eq!(abi.ref_count(), 1);
}

#[test]
fn native_refused_attachment_retires_callback_and_releases_inspector() {
    // Arrange
    let abi = FakeAbi::ready(0x2301);
    abi.state.borrow_mut().connect_ok = false;
    let dropped = Rc::new(RefCell::new(false));
    let dropped_flag = Rc::clone(&dropped);
    struct DropWatch(Rc<RefCell<bool>>);
    impl Drop for DropWatch {
        fn drop(&mut self) {
            *self.0.borrow_mut() = true;
        }
    }
    let watch = DropWatch(dropped_flag);
    let frames = Rc::new(RefCell::new(0usize));
    let frames_sink = Rc::clone(&frames);

    // Act
    let result = ObservationLease::attach_with(
        abi.clone(),
        test_grant(2),
        0x9301usize as Object,
        move |_| {
            let _keep = &watch;
            *frames_sink.borrow_mut() += 1;
        },
        || {},
    );
    abi.emit_message(r#"{"method":"Console.messageAdded"}"#);

    // Assert
    assert_eq!(result.err(), Some(ObservationError::OccupiedInspector));
    assert!(
        *dropped.borrow(),
        "callback closure must be retired on refusal"
    );
    assert_eq!(*frames.borrow(), 0);
    assert_eq!(abi.connected_signal_count(), 0);
    assert_eq!(abi.ref_count(), 0);
    // Subsequent attach after refusal succeeds because the registry entry was released.
    abi.state.borrow_mut().connect_ok = true;
    let retry =
        ObservationLease::attach_with(abi, test_grant(2), 0x9301usize as Object, |_| {}, || {});
    assert!(retry.is_ok());
}

#[test]
fn late_signals_and_duplicate_close_are_suppressed() {
    // Arrange
    let abi = FakeAbi::ready(0x2401);
    let frames = Rc::new(RefCell::new(Vec::<String>::new()));
    let frames_sink = Rc::clone(&frames);
    let closes = Rc::new(RefCell::new(0usize));
    let closes_sink = Rc::clone(&closes);
    let mut lease = ObservationLease::attach_with(
        abi.clone(),
        test_grant(3),
        0x9401usize as Object,
        move |frame| frames_sink.borrow_mut().push(frame.to_owned()),
        move || *closes_sink.borrow_mut() += 1,
    )
    .expect("attach succeeds");

    // Act
    abi.emit_message("before-close");
    abi.emit_closed();
    abi.emit_closed();
    abi.emit_message("after-close");
    let dispatch_after_close =
        lease.dispatch_subscription(&ObserverCommand::direct(3, DirectSubscription::PageEnable));

    // Assert
    assert!(!lease.is_open());
    assert_eq!(frames.borrow().as_slice(), &["before-close"]);
    assert_eq!(*closes.borrow(), 1);
    assert_eq!(dispatch_after_close, Err(ObservationError::ClosedStream));
    assert_eq!(abi.ref_count(), 0);
}

#[test]
fn reentrant_signal_delivery_is_suppressed_without_panicking() {
    // Arrange
    let abi = FakeAbi::ready(0x2501);
    let reenter_abi = abi.clone();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen_sink = Rc::clone(&seen);
    let _lease = ObservationLease::attach_with(
        abi.clone(),
        test_grant(4),
        0x9501usize as Object,
        move |frame| {
            seen_sink.borrow_mut().push(frame.to_owned());
            if frame == "outer" {
                reenter_abi.emit_message("reentrant-inner");
            }
        },
        || {},
    )
    .expect("attach succeeds");

    // Act
    abi.emit_message("outer");
    abi.emit_message("after-outer");

    // Assert
    assert_eq!(seen.borrow().as_slice(), &["outer", "after-outer"]);
}

#[test]
fn failed_dispatch_returns_dispatch_failed_while_stream_stays_open() {
    // Arrange
    let abi = FakeAbi::ready(0x2601);
    abi.state.borrow_mut().send_ok = false;
    let mut lease =
        ObservationLease::attach_with(abi, test_grant(5), 0x9601usize as Object, |_| {}, || {})
            .expect("attach succeeds");
    let command = ObserverCommand::direct(9, DirectSubscription::PageEnable);

    // Act
    let result = lease.dispatch_subscription(&command);

    // Assert
    assert_eq!(result, Err(ObservationError::DispatchFailed));
    assert!(lease.is_open());
}

#[test]
fn drop_disconnects_native_client_before_signals_and_inspector_unref() {
    // Arrange
    let abi = FakeAbi::ready(0x2701);
    let lease = ObservationLease::attach_with(
        abi.clone(),
        test_grant(6),
        0x9701usize as Object,
        |_| {},
        || {},
    )
    .expect("attach succeeds");
    let before_drop = abi.events().len();

    // Act
    drop(lease);

    // Assert
    assert_eq!(
        &abi.events()[before_drop..],
        &[
            AbiEvent::DisconnectNative(0x2701),
            AbiEvent::DisconnectSignal(1),
            AbiEvent::DestroySignalData(1),
            AbiEvent::DisconnectSignal(2),
            AbiEvent::DestroySignalData(2),
            AbiEvent::UnrefInspector(0x2701),
        ]
    );
    assert_eq!(abi.connected_signal_count(), 0);
    assert_eq!(abi.ref_count(), 0);
}

#[test]
fn observer_command_and_selected_tab_grant_enforce_structural_bounds() {
    // Arrange
    let oversized_target = "a".repeat(MAX_TARGET_ID_BYTES + 1);
    let max_target = "a".repeat(MAX_TARGET_ID_BYTES);

    // Act
    let empty_grant = SelectedTabGrant::new(TabId::new(Profile::Main, 1), "   ");
    let empty_target = ObserverCommand::for_target(1, "", 1, TargetSubscription::RuntimeEnable);
    let control_target =
        ObserverCommand::for_target(1, "page\n1", 1, TargetSubscription::RuntimeEnable);
    let too_long =
        ObserverCommand::for_target(1, &oversized_target, 1, TargetSubscription::RuntimeEnable);
    let accepted =
        ObserverCommand::for_target(1, &max_target, 2, TargetSubscription::RuntimeDisable);

    // Assert
    assert!(empty_grant.is_none());
    assert!(empty_target.is_none());
    assert!(control_target.is_none());
    assert!(too_long.is_none());
    let accepted = accepted.expect("max-length target is accepted");
    assert!(accepted.as_json().contains("Target.sendMessageToTarget"));
    assert!(accepted.as_json().contains("Runtime.disable"));
}

#[test]
fn ordinary_inspector_toggle_behavior_is_preserved() {
    // Arrange
    let no_frontend: Object = ptr::null_mut();
    let mut live_marker = 0u8;
    let open_frontend: Object = ptr::from_mut(&mut live_marker).cast::<c_void>();

    // Act
    let when_closed = toggle_action(no_frontend);
    let when_open = toggle_action(open_frontend);

    // Assert
    assert_eq!(when_closed, ToggleAction::Show);
    assert_eq!(when_open, ToggleAction::Close);
    assert_eq!(TOGGLE_COMMAND, "quickie.devtools.toggle");
}

#[test]
fn explicit_stop_suppresses_late_signals_and_is_idempotent() {
    // Arrange
    let abi = FakeAbi::ready(0x2801);
    let frames = Rc::new(RefCell::new(Vec::<String>::new()));
    let frames_sink = Rc::clone(&frames);
    let mut lease = ObservationLease::attach_with(
        abi.clone(),
        test_grant(8),
        ptr::without_provenance_mut(0x9801),
        move |frame| frames_sink.borrow_mut().push(frame.to_owned()),
        || {},
    )
    .expect("attach succeeds");

    // Act
    lease.stop();
    let events_after_first_stop = abi.events().len();
    lease.stop();
    abi.emit_message("late-after-stop");
    let dispatch = lease.dispatch_subscription(&ObserverCommand::direct(
        11,
        DirectSubscription::NetworkEnable,
    ));

    // Assert
    assert!(!lease.is_open());
    assert_eq!(abi.events().len(), events_after_first_stop);
    assert_eq!(abi.connected_signal_count(), 0);
    assert_eq!(abi.ref_count(), 0);
    assert!(frames.borrow().is_empty());
    assert_eq!(dispatch, Err(ObservationError::ClosedStream));
}

#[test]
fn signal_registration_failure_rolls_back_inspector_and_returns_unsupported_spi() {
    // Arrange
    let abi = FakeAbi::ready(0x2901);
    abi.state.borrow_mut().closed_signal_ok = false;

    // Act
    let result = ObservationLease::attach_with(
        abi.clone(),
        test_grant(9),
        ptr::without_provenance_mut(0x9901),
        |_| {},
        || {},
    );

    // Assert
    assert_eq!(result.err(), Some(ObservationError::UnsupportedSpi));
    assert_eq!(abi.connected_signal_count(), 0);
    assert_eq!(abi.ref_count(), 0);
    assert!(
        !abi.events()
            .iter()
            .any(|event| matches!(event, AbiEvent::ConnectNative(_)))
    );
}
