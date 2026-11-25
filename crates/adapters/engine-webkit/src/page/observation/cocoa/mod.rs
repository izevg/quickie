//! Private macOS Cocoa leaf for read-only `WebKit` Inspector diagnostic observation.
//!
//! Guards the fork's `_WKInspector` selectors (`connectLocalDiagnosticClient:`,
//! `dispatchDiagnosticMessage:`, `disconnectLocalDiagnosticClient`) before touching any SPI,
//! reports unavailable on Apple's system `WebKit` without launching a window or activating visual
//! Web Inspector, and binds weakly to `WKWebView` so observation leases never outlive or retain
//! their page's view.

pub(crate) mod snapshot;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak as RcWeak};

use block2::RcBlock;
use objc2::rc::{Retained, Weak as ObjCWeak};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{ClassType, msg_send, sel};
use objc2_foundation::NSString;
use objc2_web_kit::WKWebView;
use quickie_application::mcp::DiagnosticCapabilities;
use quickie_application::{DiagnosticGapReason, DiagnosticLease, DiagnosticSignal, EngineError};

use super::ObservationDecoder;

type SignalCallback = Rc<dyn Fn(DiagnosticSignal) + 'static>;

/// Whether the loaded `WebKit` exposes Quickie's local read-only diagnostic SPI on `_WKInspector`.
///
/// Inspects Objective-C runtime metadata only; calls nothing on Apple's system `WebKit`.
#[must_use]
pub(crate) fn has_local_diagnostic_spi() -> bool {
    let has_inspector_getter = WKWebView::class()
        .instance_method(sel!(_inspector))
        .is_some();
    let has_diagnostic_methods = AnyClass::get(c"_WKInspector").is_some_and(|class| {
        class
            .instance_method(sel!(connectLocalDiagnosticClient:))
            .is_some()
            && class
                .instance_method(sel!(dispatchDiagnosticMessage:))
                .is_some()
            && class
                .instance_method(sel!(disconnectLocalDiagnosticClient))
                .is_some()
    });
    has_inspector_getter && has_diagnostic_methods
}

/// Diagnostic observation capabilities supported by `WebKitEngine` on the running `WebKit`.
///
/// `accessibility` stays `false` until native accessibility tree data is consumed directly.
#[must_use]
pub(crate) fn capabilities() -> DiagnosticCapabilities {
    let spi = has_local_diagnostic_spi();
    DiagnosticCapabilities {
        dom: true,
        accessibility: false,
        console: spi,
        runtime_errors: spi,
        pending_network: spi,
        completed_network: spi,
        lifecycle: spi,
    }
}

/// Shared main-thread state for one active local diagnostic observation lease.
pub(crate) struct ObserverState {
    view: ObjCWeak<WKWebView>,
    decoder: RefCell<ObservationDecoder>,
    on_signal: RefCell<Option<SignalCallback>>,
    active: Cell<bool>,
}

impl ObserverState {
    /// Connects a read-only local diagnostic observer to `view`'s `_WKInspector`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the running `WebKit` is not the fork with the local diagnostic
    /// SPI, if `view` has no `_WKInspector`, or if another inspector or diagnostic client is
    /// already attached.
    pub(crate) fn connect(
        view: &WKWebView,
        on_signal: Box<dyn Fn(DiagnosticSignal) + 'static>,
    ) -> Result<(DiagnosticLease, RcWeak<Self>), EngineError> {
        if !has_local_diagnostic_spi() {
            return Err(EngineError(
                "local diagnostic observation requires Quickie's WebKit fork".to_owned(),
            ));
        }

        let inspector = inspector_of(view)?;
        let state = Rc::new(Self {
            view: ObjCWeak::new(view),
            decoder: RefCell::new(ObservationDecoder::new()),
            on_signal: RefCell::new(Some(Rc::from(on_signal))),
            active: Cell::new(true),
        });

        let weak_state = Rc::downgrade(&state);
        let handler = RcBlock::new(move |message: NonNull<NSString>| {
            // SAFETY: the fork passes a live `NSString` for the duration of this main-thread
            // callback.
            let raw = unsafe { message.as_ref() }.to_string();
            if let Some(state) = weak_state.upgrade() {
                state.receive(&raw);
            }
        });

        // SAFETY: `has_local_diagnostic_spi()` verified the selector on `_WKInspector`; the fork
        // copies the `void (^)(NSString *)` block and invokes it asynchronously on the main loop.
        let connected: Bool =
            unsafe { msg_send![&*inspector, connectLocalDiagnosticClient: &*handler] };
        if !connected.as_bool() {
            state.active.set(false);
            let _ = state.on_signal.borrow_mut().take();
            return Err(EngineError(
                "local diagnostic client could not attach to this page".to_owned(),
            ));
        }

        // Direct `Network.enable` / `Page.enable` succeed only when the page controller proxies
        // those agents; returning `NO` here is expected when those agents live on target frames.
        let initial_commands = state.decoder.borrow_mut().initial_direct_commands();
        for command in initial_commands {
            let _ = dispatch_on_inspector(&inspector, &command);
        }

        let handle = Rc::downgrade(&state);
        let lease = DiagnosticLease::new(move || {
            state.disconnect();
        });
        Ok((lease, handle))
    }

    /// Invalidates the channel when its owning page closes or native dispatch fails, emitting an
    /// explicit [`DiagnosticGapReason::ChannelClosed`] signal once.
    pub(crate) fn invalidate(&self) {
        if !self.active.replace(false) {
            return;
        }
        self.disconnect_native();
        let closed = self.decoder.borrow_mut().on_channel_closed();
        if let Some(callback) = self.on_signal.borrow_mut().take() {
            callback(closed);
        }
    }

    fn disconnect(&self) {
        if !self.active.replace(false) {
            return;
        }
        let _ = self.on_signal.borrow_mut().take();
        self.disconnect_native();
    }

    fn disconnect_native(&self) {
        if let Some(view) = self.view.load()
            && let Ok(inspector) = inspector_of(&view)
        {
            // SAFETY: `has_local_diagnostic_spi()` verified the selector on `_WKInspector`.
            let () = unsafe { msg_send![&*inspector, disconnectLocalDiagnosticClient] };
        }
    }

    fn receive(&self, raw: &str) {
        if !self.active.get() {
            return;
        }

        let outcome = self.decoder.borrow_mut().decode_message(raw);
        for command in outcome.commands {
            if !self.dispatch_command(&command) {
                self.invalidate();
                return;
            }
        }

        for signal in outcome.signals {
            let closed = matches!(
                signal,
                DiagnosticSignal::Gap(DiagnosticGapReason::ChannelClosed)
            );
            let callback = self.on_signal.borrow().clone();
            if let Some(callback) = callback {
                callback(signal);
            }
            if closed {
                self.active.set(false);
                let _ = self.on_signal.borrow_mut().take();
                return;
            }
            if !self.active.get() {
                return;
            }
        }
    }

    fn dispatch_command(&self, command: &str) -> bool {
        let Some(view) = self.view.load() else {
            return false;
        };
        let Ok(inspector) = inspector_of(&view) else {
            return false;
        };
        dispatch_on_inspector(&inspector, command)
    }
}

fn inspector_of(view: &WKWebView) -> Result<Retained<AnyObject>, EngineError> {
    // SAFETY: `has_local_diagnostic_spi()` verified `-[WKWebView _inspector]` exists.
    let inspector: Option<Retained<AnyObject>> = unsafe { msg_send![view, _inspector] };
    inspector.ok_or_else(|| EngineError("the page has no Web Inspector".to_owned()))
}

fn dispatch_on_inspector(inspector: &AnyObject, message: &str) -> bool {
    let payload = NSString::from_str(message);
    // SAFETY: `has_local_diagnostic_spi()` verified `dispatchDiagnosticMessage:` on `_WKInspector`
    // and the message is a valid UTF-8 `NSString` dispatched on the main thread.
    let accepted: Bool = unsafe { msg_send![inspector, dispatchDiagnosticMessage: &*payload] };
    accepted.as_bool()
}
