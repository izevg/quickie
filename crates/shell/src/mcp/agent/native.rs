//! Native macOS `WebKit` [`AgentBackend`] and [`AgentSession`] backed by
//! [`quickie_engine_webkit::AgentAutomation`].

use std::rc::Rc;

use objc2::MainThreadMarker;

use super::policy::{AgentCapabilities, AgentEngineError};
use super::{AgentBackend, AgentSession};

/// Native macOS Agent backend probing and opening [`quickie_engine_webkit::AgentAutomation`].
#[derive(Clone, Copy)]
pub(crate) struct NativeAgentBackend {
    #[cfg(feature = "main-webkit")]
    mtm: MainThreadMarker,
}

impl NativeAgentBackend {
    #[must_use]
    pub(crate) const fn new(mtm: MainThreadMarker) -> Self {
        #[cfg(not(feature = "main-webkit"))]
        let _ = mtm;
        Self {
            #[cfg(feature = "main-webkit")]
            mtm,
        }
    }

    #[must_use]
    pub(crate) fn probe_capabilities() -> AgentCapabilities {
        #[cfg(feature = "main-webkit")]
        {
            let caps = quickie_engine_webkit::AgentAutomation::capabilities();
            AgentCapabilities {
                isolated_automation: caps.isolated_automation,
                observations: caps.observations,
            }
        }
        #[cfg(not(feature = "main-webkit"))]
        {
            AgentCapabilities::default()
        }
    }
}

impl AgentBackend for NativeAgentBackend {
    fn agent_capabilities(&self) -> AgentCapabilities {
        Self::probe_capabilities()
    }

    fn open_session(&self) -> Result<Rc<dyn AgentSession>, AgentEngineError> {
        #[cfg(feature = "main-webkit")]
        {
            let automation = quickie_engine_webkit::AgentAutomation::open(self.mtm)
                .map_err(|e| map_error(&e))?;
            Ok(Rc::new(webkit::WebKitAgentSession::new(automation)))
        }
        #[cfg(not(feature = "main-webkit"))]
        {
            Err(AgentEngineError::Unsupported(
                "isolated agent automation requires the Quickie WebKit fork",
            ))
        }
    }
}

#[cfg(feature = "main-webkit")]
fn map_error(error: &quickie_engine_webkit::AutomationError) -> AgentEngineError {
    use quickie_engine_webkit::AutomationError;
    match error {
        AutomationError::Unsupported => AgentEngineError::Unsupported(
            "isolated agent automation requires the Quickie WebKit fork",
        ),
        AutomationError::InvalidUrl | AutomationError::InvalidCommand => {
            AgentEngineError::InvalidCommand("invalid agent automation command")
        }
        AutomationError::OriginMismatch => AgentEngineError::OriginMismatch,
        AutomationError::Busy => AgentEngineError::Busy,
        AutomationError::Timeout => AgentEngineError::Timeout,
        AutomationError::Cancelled => AgentEngineError::Cancelled,
        AutomationError::Disconnected => AgentEngineError::Disconnected,
        AutomationError::ResponseTooLarge => {
            AgentEngineError::BoundsExceeded("agent response exceeded size limit")
        }
        AutomationError::Transport
        | AutomationError::InvalidReply
        | AutomationError::CommandFailed => AgentEngineError::Failed(error.to_string()),
    }
}

#[cfg(feature = "main-webkit")]
mod webkit {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use quickie_application::mcp::SnapshotRequest;
    use quickie_application::{DiagnosticLease, DiagnosticSignal, DiagnosticSnapshot, EngineError};
    use quickie_engine_webkit::{
        AgentAutomation, ApprovedOrigin, AutomationEvent, ElementSignals, InputSource,
        InputSourceState, InputSourceType, InteractionDelta, InteractionSequenceCommand,
        InteractionStep, KeyModifier, KeyboardCommand, KeyboardStep, LocatorQuery,
        MAX_PAGE_LOAD_TIMEOUT_MS, MAX_SCREENSHOT_BASE64_BYTES, MouseButton, MouseCommand,
        MouseInteraction, MouseMoveOrigin, NavigationCommand, PageLoadStrategy, ScreenshotCommand,
        ViewportPoint, VirtualKey, WaitOutcome, WaitQuery,
    };
    use quickie_mcp::browser::{
        AgentKey, AgentKeyModifier, AgentLocator, AgentMouseButton, AgentViewport,
        AgentWaitCondition, AgentWaitStage, AgentWaitStatus, ElementBounds, SelectOptionBy,
    };
    use url::{Origin, Url};

    use super::map_error;
    use crate::mcp::agent::policy::{
        AgentEngineError, AgentLocateOutput, AgentLocatedItem, AgentObservedOutput, AgentSignals,
    };
    use crate::mcp::agent::{AgentSession, AgentSessionEvent, MouseActionKind, MouseActionRequest};

    pub(super) struct WebKitAgentSession {
        automation: Rc<AgentAutomation>,
        handle: Rc<RefCell<Option<String>>>,
    }

    impl WebKitAgentSession {
        pub(super) fn new(automation: AgentAutomation) -> Self {
            Self {
                automation: Rc::new(automation),
                handle: Rc::new(RefCell::new(None)),
            }
        }

        fn with_handle(
            &self,
            timeout: Duration,
            then: impl FnOnce(Result<String, AgentEngineError>) + 'static,
        ) {
            if let Some(existing) = self.handle.borrow().clone() {
                then(Ok(existing));
                return;
            }
            let handle_slot = Rc::clone(&self.handle);
            let res = self
                .automation
                .list_contexts(timeout, move |contexts| match contexts {
                    Ok(list) => {
                        let Some(first) = list.first() else {
                            then(Err(AgentEngineError::Busy));
                            return;
                        };
                        let handle = first.handle().to_owned();
                        *handle_slot.borrow_mut() = Some(handle.clone());
                        then(Ok(handle));
                    }
                    Err(err) => then(Err(map_error(&err))),
                });
            if let Err(err) = res {
                let _ = err;
            }
        }
    }

    impl AgentSession for WebKitAgentSession {
        fn is_connected(&self) -> bool {
            self.automation.is_connected()
        }

        fn set_viewport(&self, viewport: AgentViewport) {
            self.automation
                .set_viewport(viewport.width, viewport.height);
        }

        fn current_url(&self) -> Option<Url> {
            self.automation.current_url()
        }

        fn set_event_observer(&self, observer: Option<Box<dyn Fn(AgentSessionEvent) + 'static>>) {
            let mapped = observer.map(|obs| {
                Box::new(move |event: AutomationEvent| match event {
                    AutomationEvent::BrowsingContextCleared { .. } => {
                        obs(AgentSessionEvent::ContextCleared);
                    }
                    AutomationEvent::FileChooserDismissed { .. } => {}
                }) as Box<dyn Fn(AutomationEvent) + 'static>
            });
            self.automation.set_event_observer(mapped);
        }

        fn observe_diagnostics(
            &self,
            on_signal: Box<dyn Fn(DiagnosticSignal) + 'static>,
        ) -> Result<DiagnosticLease, EngineError> {
            self.automation.observe_diagnostics(on_signal)
        }

        fn diagnostic_snapshot(
            &self,
            request: &SnapshotRequest,
            done: Box<dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static>,
        ) -> Result<(), EngineError> {
            self.automation.diagnostic_snapshot(request, done)
        }

        fn navigate(
            &self,
            url: Url,
            approved_origin: Origin,
            wait_stage: AgentWaitStage,
            timeout: Duration,
            done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
        ) {
            let Ok(approved) = ApprovedOrigin::from_url(&url) else {
                done(Err(AgentEngineError::InvalidCommand(
                    "invalid navigation URL",
                )));
                return;
            };
            if approved.origin() != &approved_origin {
                done(Err(AgentEngineError::OriginMismatch));
                return;
            }
            let page_load_strategy = match wait_stage {
                AgentWaitStage::None => PageLoadStrategy::None,
                AgentWaitStage::DomContentLoaded => PageLoadStrategy::Eager,
                AgentWaitStage::Load => PageLoadStrategy::Normal,
            };
            let page_load_timeout_ms = u32::try_from(timeout.as_millis())
                .unwrap_or(MAX_PAGE_LOAD_TIMEOUT_MS)
                .clamp(1, MAX_PAGE_LOAD_TIMEOUT_MS);

            let automation = Rc::clone(&self.automation);
            self.with_handle(timeout, move |handle_res| match handle_res {
                Ok(handle) => {
                    let cmd = NavigationCommand {
                        handle,
                        url,
                        approved_origin: approved,
                        page_load_strategy,
                        page_load_timeout_ms,
                    };
                    if let Err(err) = automation.navigate(&cmd, timeout, move |res| {
                        done(res.map_err(|e| map_error(&e)));
                    }) {
                        let _ = err;
                    }
                }
                Err(err) => done(Err(err)),
            });
        }

        fn locate_elements(
            &self,
            locator: &AgentLocator,
            max_results: usize,
            done: Box<dyn FnOnce(Result<AgentLocateOutput, AgentEngineError>) + 'static>,
        ) {
            let query = map_locator(locator);
            if let Err(err) = self
                .automation
                .locate_elements(&query, max_results, move |res| {
                    done(res.map_err(|e| map_error(&e)).map(|snap| {
                        AgentLocateOutput {
                            elements: snap
                                .elements
                                .into_iter()
                                .map(|el| AgentLocatedItem {
                                    element_id: el.element_id,
                                    role: el.role,
                                    name: el.name,
                                    tag: el.tag,
                                    bounds: el.bounds.map(|b| ElementBounds {
                                        x: b.x,
                                        y: b.y,
                                        width: b.width,
                                        height: b.height,
                                    }),
                                    signals: map_signals(el.signals),
                                })
                                .collect(),
                            truncated: snap.truncated,
                        }
                    }));
                })
            {
                let _ = err;
            }
        }

        fn resolve_element(
            &self,
            element_id: &str,
            scroll_into_view: bool,
            done: Box<dyn FnOnce(Result<AgentSignals, AgentEngineError>) + 'static>,
        ) {
            if let Err(err) =
                self.automation
                    .resolve_element(element_id, scroll_into_view, move |res| {
                        done(res.map_err(|e| map_error(&e)).map(map_signals));
                    })
            {
                let _ = err;
            }
        }

        fn inspect_focused(
            &self,
            done: Box<dyn FnOnce(Result<AgentSignals, AgentEngineError>) + 'static>,
        ) {
            if let Err(err) = self.automation.inspect_focused(move |res| {
                done(res.map_err(|e| map_error(&e)).map(map_signals));
            }) {
                let _ = err;
            }
        }

        fn mouse_action(
            &self,
            request: MouseActionRequest,
            done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
        ) {
            let automation = Rc::clone(&self.automation);
            self.with_handle(request.timeout, move |handle_res| match handle_res {
                Ok(handle) => {
                    let interaction = match request.kind {
                        MouseActionKind::Click => MouseInteraction::SingleClick,
                        MouseActionKind::DoubleClick => MouseInteraction::DoubleClick,
                        MouseActionKind::Hover => MouseInteraction::Move,
                    };
                    let cmd = MouseCommand {
                        handle,
                        position: ViewportPoint {
                            x: request.x,
                            y: request.y,
                        },
                        button: if request.kind == MouseActionKind::Hover {
                            MouseButton::None
                        } else {
                            map_mouse_button(request.button)
                        },
                        interaction,
                        modifiers: request.modifiers.into_iter().map(map_modifier).collect(),
                    };
                    if let Err(err) =
                        automation.perform_mouse_interaction(&cmd, request.timeout, move |res| {
                            done(res.map(|_| ()).map_err(|e| map_error(&e)));
                        })
                    {
                        let _ = err;
                    }
                }
                Err(err) => done(Err(err)),
            });
        }

        fn type_text(
            &self,
            text: String,
            timeout: Duration,
            done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
        ) {
            let automation = Rc::clone(&self.automation);
            self.with_handle(timeout, move |handle_res| match handle_res {
                Ok(handle) => {
                    let cmd = KeyboardCommand {
                        handle,
                        interactions: vec![KeyboardStep::InsertText(text)],
                    };
                    if let Err(err) =
                        automation.perform_keyboard_interactions(&cmd, timeout, move |res| {
                            done(res.map_err(|e| map_error(&e)));
                        })
                    {
                        let _ = err;
                    }
                }
                Err(err) => done(Err(err)),
            });
        }

        fn fill_text(
            &self,
            text: String,
            timeout: Duration,
            done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
        ) {
            let automation = Rc::clone(&self.automation);
            self.with_handle(timeout, move |handle_res| match handle_res {
                Ok(handle) => {
                    let mut steps = vec![
                        KeyboardStep::PressKey(VirtualKey::Command),
                        KeyboardStep::InsertText("a".to_owned()),
                        KeyboardStep::ReleaseKey(VirtualKey::Command),
                        KeyboardStep::InsertKey(VirtualKey::Backspace),
                    ];
                    if !text.is_empty() {
                        steps.push(KeyboardStep::InsertText(text));
                    }
                    let cmd = KeyboardCommand {
                        handle,
                        interactions: steps,
                    };
                    if let Err(err) =
                        automation.perform_keyboard_interactions(&cmd, timeout, move |res| {
                            done(res.map_err(|e| map_error(&e)));
                        })
                    {
                        let _ = err;
                    }
                }
                Err(err) => done(Err(err)),
            });
        }

        fn press_key(
            &self,
            key: AgentKey,
            modifiers: Vec<AgentKeyModifier>,
            timeout: Duration,
            done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
        ) {
            let automation = Rc::clone(&self.automation);
            self.with_handle(timeout, move |handle_res| match handle_res {
                Ok(handle) => {
                    let vmods: Vec<VirtualKey> = modifiers
                        .into_iter()
                        .map(map_modifier_to_virtual_key)
                        .collect();
                    let mut steps = Vec::with_capacity(vmods.len() * 2 + 1);
                    for &m in &vmods {
                        steps.push(KeyboardStep::PressKey(m));
                    }
                    steps.push(map_agent_key(key));
                    for &m in vmods.iter().rev() {
                        steps.push(KeyboardStep::ReleaseKey(m));
                    }
                    let cmd = KeyboardCommand {
                        handle,
                        interactions: steps,
                    };
                    if let Err(err) =
                        automation.perform_keyboard_interactions(&cmd, timeout, move |res| {
                            done(res.map_err(|e| map_error(&e)));
                        })
                    {
                        let _ = err;
                    }
                }
                Err(err) => done(Err(err)),
            });
        }

        fn scroll_wheel(
            &self,
            x: f64,
            y: f64,
            delta_x: i32,
            delta_y: i32,
            timeout: Duration,
            done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
        ) {
            let automation = Rc::clone(&self.automation);
            self.with_handle(timeout, move |handle_res| match handle_res {
                Ok(handle) => {
                    let mut state = InputSourceState::new("wheel-1");
                    state.origin = Some(MouseMoveOrigin::Viewport);
                    state.location = Some(ViewportPoint { x, y });
                    state.delta = Some(InteractionDelta {
                        width: f64::from(delta_x),
                        height: f64::from(delta_y),
                    });
                    state.duration_ms = Some(0);
                    let cmd = InteractionSequenceCommand {
                        handle,
                        frame_handle: None,
                        input_sources: vec![InputSource {
                            source_id: "wheel-1".to_owned(),
                            source_type: InputSourceType::Wheel,
                        }],
                        steps: vec![InteractionStep {
                            states: vec![state],
                        }],
                    };
                    if let Err(err) =
                        automation.perform_interaction_sequence(&cmd, timeout, move |res| {
                            done(res.map_err(|e| map_error(&e)));
                        })
                    {
                        let _ = err;
                    }
                }
                Err(err) => done(Err(err)),
            });
        }

        fn select_option(
            &self,
            element_id: &str,
            option: &SelectOptionBy,
            done: Box<dyn FnOnce(Result<(), AgentEngineError>) + 'static>,
        ) {
            let (by_value, by_label) = match option {
                SelectOptionBy::Value(v) => (Some(v.as_str()), None),
                SelectOptionBy::Label(l) => (None, Some(l.as_str())),
            };
            if let Err(err) =
                self.automation
                    .select_option(element_id, by_value, by_label, move |res| {
                        done(res.map_err(|e| map_error(&e)));
                    })
            {
                let _ = err;
            }
        }

        fn observe_state(
            &self,
            done: Box<dyn FnOnce(Result<AgentObservedOutput, AgentEngineError>) + 'static>,
        ) {
            if let Err(err) = self.automation.observe_state(move |res| {
                done(res.map_err(|e| map_error(&e)).map(|o| AgentObservedOutput {
                    focused_role: o.focused_role,
                    value_length: o.value_length,
                    scroll_x: o.scroll_x,
                    scroll_y: o.scroll_y,
                }));
            }) {
                let _ = err;
            }
        }

        fn wait_for(
            &self,
            condition: &AgentWaitCondition,
            timeout: Duration,
            done: Box<dyn FnOnce(Result<AgentWaitStatus, AgentEngineError>) + 'static>,
        ) {
            let Some(query) = map_wait_condition(condition) else {
                done(Err(AgentEngineError::InvalidCommand(
                    "unsupported wait stage",
                )));
                return;
            };
            if let Err(err) = self
                .automation
                .wait_for_condition(&query, timeout, move |res| {
                    done(res.map_err(|e| map_error(&e)).map(|outcome| match outcome {
                        WaitOutcome::Matched => AgentWaitStatus::Matched,
                        WaitOutcome::TimedOut => AgentWaitStatus::TimedOut,
                    }));
                })
            {
                let _ = err;
            }
        }

        fn screenshot_viewport(
            &self,
            max_bytes: usize,
            timeout: Duration,
            done: Box<dyn FnOnce(Result<String, AgentEngineError>) + 'static>,
        ) {
            let max_base64_bytes = max_bytes
                .saturating_add(2)
                .saturating_div(3)
                .saturating_mul(4)
                .clamp(1, MAX_SCREENSHOT_BASE64_BYTES);
            let automation = Rc::clone(&self.automation);
            self.with_handle(timeout, move |handle_res| match handle_res {
                Ok(handle) => {
                    let mut cmd = ScreenshotCommand::viewport(handle);
                    cmd.max_base64_bytes = max_base64_bytes;
                    if let Err(err) = automation.take_screenshot(&cmd, timeout, move |res| {
                        done(res.map(|data| data.png_base64).map_err(|e| map_error(&e)));
                    }) {
                        let _ = err;
                    }
                }
                Err(err) => done(Err(err)),
            });
        }

        fn close(&self) {
            self.automation.close();
        }
    }

    fn map_locator(locator: &AgentLocator) -> LocatorQuery {
        match locator {
            AgentLocator::Css { selector } => LocatorQuery::Css {
                selector: selector.clone(),
            },
            AgentLocator::Text { text, exact } => LocatorQuery::Text {
                text: text.clone(),
                exact: *exact,
            },
            AgentLocator::Role { role, name, exact } => LocatorQuery::Role {
                role: role.clone(),
                name: name.clone(),
                exact: *exact,
            },
        }
    }

    fn map_wait_condition(condition: &AgentWaitCondition) -> Option<WaitQuery> {
        match condition {
            AgentWaitCondition::LocatorVisible { locator } => {
                Some(WaitQuery::LocatorVisible(map_locator(locator)))
            }
            AgentWaitCondition::LocatorHidden { locator } => {
                Some(WaitQuery::LocatorHidden(map_locator(locator)))
            }
            AgentWaitCondition::TextPresent { text, exact } => Some(WaitQuery::TextPresent {
                text: text.clone(),
                exact: *exact,
            }),
            AgentWaitCondition::LoadStage {
                stage: AgentWaitStage::DomContentLoaded,
            } => Some(WaitQuery::DomContentLoaded),
            AgentWaitCondition::LoadStage {
                stage: AgentWaitStage::Load,
            } => Some(WaitQuery::Load),
            AgentWaitCondition::LoadStage {
                stage: AgentWaitStage::None,
            } => None,
        }
    }

    fn map_signals(sig: ElementSignals) -> AgentSignals {
        AgentSignals {
            connected: sig.connected,
            visible: sig.visible,
            is_submit_control: sig.is_submit_control,
            in_form: sig.in_form,
            is_file_input: sig.is_file_input,
            is_editable: sig.is_editable,
            is_select: sig.is_select,
            link_url: sig.link_url,
            link_opens_new_window: sig.link_opens_new_window,
            action_point: sig.action_point.map(|p| (p.x, p.y)),
        }
    }

    const fn map_mouse_button(button: AgentMouseButton) -> MouseButton {
        match button {
            AgentMouseButton::Left => MouseButton::Left,
            AgentMouseButton::Middle => MouseButton::Middle,
            AgentMouseButton::Right => MouseButton::Right,
        }
    }

    const fn map_modifier(modifier: AgentKeyModifier) -> KeyModifier {
        match modifier {
            AgentKeyModifier::Shift => KeyModifier::Shift,
            AgentKeyModifier::Control => KeyModifier::Control,
            AgentKeyModifier::Alt => KeyModifier::Alt,
            AgentKeyModifier::Meta => KeyModifier::Meta,
        }
    }

    const fn map_modifier_to_virtual_key(modifier: AgentKeyModifier) -> VirtualKey {
        match modifier {
            AgentKeyModifier::Shift => VirtualKey::Shift,
            AgentKeyModifier::Control => VirtualKey::Control,
            AgentKeyModifier::Alt => VirtualKey::Alternate,
            AgentKeyModifier::Meta => VirtualKey::Command,
        }
    }

    fn map_agent_key(key: AgentKey) -> KeyboardStep {
        match key {
            AgentKey::Enter => KeyboardStep::InsertKey(VirtualKey::Return),
            AgentKey::Tab => KeyboardStep::InsertKey(VirtualKey::Tab),
            AgentKey::Escape => KeyboardStep::InsertKey(VirtualKey::Escape),
            AgentKey::Backspace => KeyboardStep::InsertKey(VirtualKey::Backspace),
            AgentKey::Delete => KeyboardStep::InsertKey(VirtualKey::Delete),
            AgentKey::Space => KeyboardStep::InsertKey(VirtualKey::Space),
            AgentKey::ArrowUp => KeyboardStep::InsertKey(VirtualKey::UpArrow),
            AgentKey::ArrowDown => KeyboardStep::InsertKey(VirtualKey::DownArrow),
            AgentKey::ArrowLeft => KeyboardStep::InsertKey(VirtualKey::LeftArrow),
            AgentKey::ArrowRight => KeyboardStep::InsertKey(VirtualKey::RightArrow),
            AgentKey::Home => KeyboardStep::InsertKey(VirtualKey::Home),
            AgentKey::End => KeyboardStep::InsertKey(VirtualKey::End),
            AgentKey::PageUp => KeyboardStep::InsertKey(VirtualKey::PageUp),
            AgentKey::PageDown => KeyboardStep::InsertKey(VirtualKey::PageDown),
            AgentKey::Character(ch) => KeyboardStep::InsertText(ch.to_string()),
        }
    }
}
