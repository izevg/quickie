//! Requests as `NSURLSession` data tasks whose delegate hands each piece of the body to the
//! calling thread as it arrives, so an LLM's tokens reach the answer panel one by one.
//!
//! The delegate runs on the session's own serial queue, never on the main thread. It only
//! forwards what it gets through a channel; the calling worker thread does the rest, and checks
//! the stop signal between arrivals.
//!
//! The delegate refuses every redirect, so the 3xx itself is the answer. Following one would
//! re-send the request, an LLM prompt with the page in it, to whatever host the `Location`
//! names, in the clear if it says `http:`, past every check of the endpoint's URL.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::time::Duration;

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL, NSURLRequest,
    NSURLResponse, NSURLSession, NSURLSessionConfiguration, NSURLSessionDataDelegate,
    NSURLSessionDataTask, NSURLSessionDelegate, NSURLSessionResponseDisposition, NSURLSessionTask,
    NSURLSessionTaskDelegate,
};
use quickie_application::{Ended, Flow, HttpError, HttpEvent, HttpMethod, HttpRequest, StopSignal};

/// How often the waiting thread looks at the stop signal while nothing arrives.
const POLL: Duration = Duration::from_millis(50);

/// How long a request may take.
#[derive(Clone, Copy, Debug)]
pub(super) struct Timeouts {
    /// The longest wait for the next bytes.
    pub idle: Duration,
    /// The longest a whole request may take.
    pub total: Duration,
}

/// What the delegate forwards, in the order it happens.
enum Arrival {
    /// The response's status code; `None` for a response that is not http.
    Status(Option<u16>, Option<String>),
    /// The next piece of the body.
    Data(Vec<u8>),
    /// The task ended: completely, or with the system's description of what failed.
    Finished(Option<String>),
}

struct Ivars {
    arrivals: SyncSender<Arrival>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `StreamDelegate` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "QuickieWebKitStreamDelegate"]
    #[ivars = Ivars]
    struct StreamDelegate;

    unsafe impl NSObjectProtocol for StreamDelegate {}

    unsafe impl NSURLSessionDelegate for StreamDelegate {}

    // SAFETY: both methods match the protocol's signatures.
    unsafe impl NSURLSessionTaskDelegate for StreamDelegate {
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        fn will_redirect(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionTask,
            _response: &NSHTTPURLResponse,
            _request: &NSURLRequest,
            completion: &DynBlock<dyn Fn(*mut NSURLRequest)>,
        ) {
            // No new request: the redirect response itself is delivered as the answer.
            completion.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn did_complete(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionTask,
            error: Option<&NSError>,
        ) {
            let failure = error.map(|error| error.localizedDescription().to_string());
            self.forward(Arrival::Finished(failure));
        }
    }

    // SAFETY: both methods match the protocol's signatures.
    unsafe impl NSURLSessionDataDelegate for StreamDelegate {
        #[unsafe(method(URLSession:dataTask:didReceiveResponse:completionHandler:))]
        fn did_receive_response(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionDataTask,
            response: &NSURLResponse,
            completion: &DynBlock<dyn Fn(NSURLSessionResponseDisposition)>,
        ) {
            let http = response.downcast_ref::<NSHTTPURLResponse>();
            let status = http.and_then(|response| u16::try_from(response.statusCode()).ok());
            let location = http
                .and_then(|response| {
                    response.valueForHTTPHeaderField(&NSString::from_str("Location"))
                })
                .map(|value| value.to_string());
            self.forward(Arrival::Status(status, location));
            completion.call((NSURLSessionResponseDisposition::Allow,));
        }

        #[unsafe(method(URLSession:dataTask:didReceiveData:))]
        fn did_receive_data(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionDataTask,
            data: &NSData,
        ) {
            self.forward(Arrival::Data(data.to_vec()));
        }
    }
);

impl StreamDelegate {
    fn new(arrivals: SyncSender<Arrival>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { arrivals });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn forward(&self, arrival: Arrival) {
        // The caller may have stopped waiting; then nobody needs it.
        let _ = self.ivars().arrivals.send(arrival);
    }
}

/// Sends `request` over a session of its own, ephemeral like every request of the browser's,
/// and hands what arrives to `on_event` until the body ends, `stop` is given, or `on_event`
/// has had enough.
pub(super) fn stream(
    request: &HttpRequest,
    timeouts: Timeouts,
    stop: &StopSignal,
    on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
) -> Result<Ended, HttpError> {
    stream_with_location(request, timeouts, stop, on_event, &mut |_| {})
}

pub(super) fn stream_with_location(
    request: &HttpRequest,
    timeouts: Timeouts,
    stop: &StopSignal,
    on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    on_location: &mut dyn FnMut(String),
) -> Result<Ended, HttpError> {
    let target = ns_request(request)?;
    let (sender, arrivals) = mpsc::sync_channel(4);
    let delegate = StreamDelegate::new(sender);
    let configuration = NSURLSessionConfiguration::ephemeralSessionConfiguration();
    configuration.setTimeoutIntervalForRequest(timeouts.idle.as_secs_f64());
    configuration.setTimeoutIntervalForResource(timeouts.total.as_secs_f64());
    configuration.setHTTPShouldSetCookies(false);
    // SAFETY: no queue is given, so the session makes its own serial one; the delegate is
    // thread-safe (it only sends on a channel), and the session keeps it alive until it is
    // invalidated below.
    let session = unsafe {
        NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
            &configuration,
            Some(ProtocolObject::from_ref(&*delegate)),
            None,
        )
    };
    let task = session.dataTaskWithRequest(&target);
    task.resume();
    let ended = pump(&arrivals, stop, on_event, on_location);
    // Disconnect first so a delegate blocked by backpressure can finish.
    drop(arrivals);
    // Cancels the task if it still runs, and lets go of the delegate.
    session.invalidateAndCancel();
    ended
}

/// Hands arrivals to `on_event` until the task ends, `stop` is given, or `on_event` stops.
fn pump(
    arrivals: &Receiver<Arrival>,
    stop: &StopSignal,
    on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    on_location: &mut dyn FnMut(String),
) -> Result<Ended, HttpError> {
    loop {
        if stop.is_stopped() {
            return Ok(Ended::Stopped);
        }
        let flow = match arrivals.recv_timeout(POLL) {
            Ok(Arrival::Status(Some(status), location)) => {
                if let Some(location) = location {
                    on_location(location);
                }
                on_event(HttpEvent::Status(status))
            }
            Ok(Arrival::Status(None, _)) => {
                return Err(HttpError("the answer is not http".to_owned()));
            }
            Ok(Arrival::Data(bytes)) => on_event(HttpEvent::Body(&bytes)),
            Ok(Arrival::Finished(None)) => return Ok(Ended::Complete),
            Ok(Arrival::Finished(Some(failure))) => return Err(HttpError(failure)),
            Err(RecvTimeoutError::Timeout) => Flow::Continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(HttpError("the request ended without an answer".to_owned()));
            }
        };
        if flow == Flow::Stop {
            return Ok(Ended::Stopped);
        }
    }
}

/// The system's request for `request`.
fn ns_request(request: &HttpRequest) -> Result<Retained<NSMutableURLRequest>, HttpError> {
    let url = NSURL::URLWithString(&NSString::from_str(request.url.as_str()))
        .ok_or_else(|| HttpError(format!("not a URL the system can load: {}", request.url)))?;
    let target = NSMutableURLRequest::requestWithURL(&url);
    match &request.method {
        HttpMethod::Get => target.setHTTPMethod(&NSString::from_str("GET")),
        HttpMethod::Post(body) => {
            target.setHTTPMethod(&NSString::from_str("POST"));
            target.setHTTPBody(Some(&NSData::with_bytes(body)));
        }
    }
    for (name, value) in &request.headers {
        target.setValue_forHTTPHeaderField(
            Some(&NSString::from_str(value)),
            &NSString::from_str(name),
        );
    }
    Ok(target)
}
