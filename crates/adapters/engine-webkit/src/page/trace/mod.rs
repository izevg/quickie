//! EXPERIMENT (never merged): with `QUICKIE_TRACE=<file>`, every page's frame timing
//! (`trace.js`) is appended to `<file>` as JSON lines, each wrapped with the time it arrived.
//! Without the variable nothing is installed.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSString;
use objc2_web_kit::{
    WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime,
};

const SCRIPT: &str = include_str!("trace.js");
const HANDLER: &str = "quickieTrace";

fn file() -> &'static Mutex<Option<File>> {
    static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        let opened = std::env::var_os("QUICKIE_TRACE")
            .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());
        Mutex::new(opened)
    })
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

/// Appends one line, `{"rx":<ms>,<field>:<json>}`, when tracing.
pub fn write(field: &str, json: &str) {
    if let Ok(mut guard) = file().lock()
        && let Some(out) = guard.as_mut()
    {
        let _ = writeln!(out, "{{\"rx\":{:.2},\"{field}\":{json}}}", now_ms());
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `TraceHandler` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieWebKitTraceHandler"]
    struct TraceHandler;

    unsafe impl NSObjectProtocol for TraceHandler {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl WKScriptMessageHandler for TraceHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: a getter; the body is whatever the script posted.
            let body = unsafe { message.body() };
            if let Some(line) = body.downcast_ref::<NSString>() {
                write("page", &line.to_string());
            }
        }
    }
);

/// Adds the trace script and its handler to `controller`, when `QUICKIE_TRACE` names a file.
pub(crate) fn install(controller: &WKUserContentController, mtm: MainThreadMarker) {
    if file().lock().map_or(true, |guard| guard.is_none()) {
        return;
    }
    let world = super::scripts::world(mtm);
    // SAFETY: the source is a complete script, injected at document start into the main frame.
    let script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(mtm),
            &NSString::from_str(SCRIPT),
            WKUserScriptInjectionTime::AtDocumentStart,
            true,
            &world,
        )
    };
    // SAFETY: the controller retains the script.
    unsafe { controller.addUserScript(&script) };
    let this = TraceHandler::alloc(mtm).set_ivars(());
    // SAFETY: `init` is `NSObject`'s designated initializer.
    let handler: Retained<TraceHandler> = unsafe { msg_send![super(this), init] };
    // SAFETY: the controller retains the handler; one per controller.
    unsafe {
        controller.addScriptMessageHandler_contentWorld_name(
            ProtocolObject::from_ref(&*handler),
            &world,
            &NSString::from_str(HANDLER),
        );
    }
    write("app", "\"trace on\"");
}
