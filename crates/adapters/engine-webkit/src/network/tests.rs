//! `NSURLSession` against a one-shot server on 127.0.0.1: no request leaves the machine.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use quickie_application::{
    Ended, Flow, HttpError, HttpEvent, HttpMethod, HttpPort, HttpRequest, StopSignal,
};
use url::Url;

use super::UrlSessionHttp;

/// A request as the server read it: the request line, the headers in lowercase, the body.
#[derive(Debug)]
struct Received {
    line: String,
    headers: Vec<String>,
    body: Vec<u8>,
}

/// Serves one connection: answers `status`, then writes each piece of `pieces` on its own
/// after a pause, then keeps the connection open for `hold` before closing it. Returns the URL
/// and what the server received.
fn serve_stream(
    status: &'static str,
    pieces: Vec<&'static [u8]>,
    hold: Duration,
) -> (Url, Receiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (report, received) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut headers = Vec::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            if header.trim().is_empty() {
                break;
            }
            headers.push(header.trim().to_lowercase());
        }
        let length = headers
            .iter()
            .find_map(|header| header.strip_prefix("content-length: "))
            .map_or(0, |length| length.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        // A test that does not look at the request has dropped the receiver.
        let _ = report.send(Received {
            line: line.trim().to_owned(),
            headers,
            body,
        });
        let mut stream = reader.into_inner();
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        for piece in pieces {
            thread::sleep(Duration::from_millis(30));
            // The client may have stopped and gone.
            if stream
                .write_all(piece)
                .and_then(|()| stream.flush())
                .is_err()
            {
                return;
            }
        }
        thread::sleep(hold);
    });
    let url = Url::parse(&format!("http://127.0.0.1:{port}/v1/chat/completions")).unwrap();
    (url, received)
}

/// Streams `request`, collecting the status and each body piece as it came.
fn collect(
    request: &HttpRequest,
    stop: &StopSignal,
) -> (Result<Ended, HttpError>, Option<u16>, Vec<Vec<u8>>) {
    let mut status = None;
    let mut pieces = Vec::new();
    let ended = UrlSessionHttp::new().stream(request, stop, &mut |event| {
        match event {
            HttpEvent::Status(code) => status = Some(code),
            HttpEvent::Body(bytes) => pieces.push(bytes.to_vec()),
        }
        Flow::Continue
    });
    (ended, status, pieces)
}

/// A listener that only counts the connections it gets.
fn bystander() -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    thread::spawn(move || {
        for _ in listener.incoming() {
            count.fetch_add(1, Ordering::SeqCst);
        }
    });
    (port, accepted)
}

/// Answers one request with a redirect to `location`.
fn serve_redirect(status: &'static str, location: String) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let mut length = 0;
        while reader.read_line(&mut line).unwrap() > 2 {
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
            line.clear();
        }
        // Read the POST body too: closing a socket with unread input sends a reset, which can
        // reach the client before the response and make the test flaky.
        let mut request_body = vec![0; length];
        let _ = reader.read_exact(&mut request_body);
        let mut stream = reader.into_inner();
        let body = b"moved";
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nLocation: {location}\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            body.len()
        )
        .and_then(|()| stream.write_all(body))
        .and_then(|()| stream.shutdown(std::net::Shutdown::Write));
    });
    Url::parse(&format!("http://127.0.0.1:{port}/v1/chat/completions")).unwrap()
}

#[test]
fn a_streamed_redirect_is_the_answer_and_never_followed() {
    for status in [
        "301 Moved Permanently",
        "302 Found",
        "307 Temporary Redirect",
        "308 Permanent Redirect",
    ] {
        let (elsewhere, accepted) = bystander();
        let url = serve_redirect(status, format!("http://127.0.0.1:{elsewhere}/steal"));

        let (ended, code, body) = collect(&post(url), &StopSignal::new());

        assert_eq!(ended, Ok(Ended::Complete), "{status}");
        assert_eq!(
            code.map(|code| code.to_string()),
            Some(status[..3].to_owned())
        );
        assert_eq!(body.concat(), b"moved");
        assert_eq!(accepted.load(Ordering::SeqCst), 0, "{status} was followed");
    }
}

#[test]
fn a_whole_request_never_follows_a_redirect() {
    let (elsewhere, accepted) = bystander();
    let url = serve_redirect(
        "302 Found",
        format!("http://127.0.0.1:{elsewhere}/list.txt"),
    );

    let body = UrlSessionHttp::new().get(&url);

    assert_eq!(body, Err(HttpError("status 302".to_owned())));
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
}

#[test]
fn a_receiver_that_has_enough_ends_the_stream() {
    let pieces: Vec<&'static [u8]> = vec![b"data: first\n\n", b"data: second\n\n"];
    let (url, _) = serve_stream("200 OK", pieces, Duration::from_secs(10));
    let mut seen = Vec::new();
    let started = Instant::now();

    let ended = UrlSessionHttp::new().stream(&post(url), &StopSignal::new(), &mut |event| {
        seen.push(format!("{event:?}"));
        if matches!(event, HttpEvent::Body(_)) {
            Flow::Stop
        } else {
            Flow::Continue
        }
    });

    assert_eq!(ended, Ok(Ended::Stopped));
    assert_eq!(seen.len(), 2);
    assert!(started.elapsed() < Duration::from_secs(2));
}

fn post(url: Url) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post(b"{\"stream\":true}".to_vec()),
        url,
        headers: vec![
            ("Accept".to_owned(), "text/event-stream".to_owned()),
            ("Authorization".to_owned(), "Bearer sk-test".to_owned()),
        ],
    }
}

#[test]
fn a_stream_arrives_piece_by_piece_after_its_status() {
    let pieces: Vec<&'static [u8]> = vec![b"data: one\n\n", b"data: two\n\n"];
    let (url, _) = serve_stream("200 OK", pieces, Duration::ZERO);

    let (ended, status, body) = collect(&post(url), &StopSignal::new());

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(status, Some(200));
    // Under load the system may hand both pieces over at once; the first one always leads.
    assert!(body[0].starts_with(b"data: one\n\n"), "{body:?}");
    assert_eq!(body.concat(), b"data: one\n\ndata: two\n\n");
}

#[test]
fn a_streamed_post_carries_its_body_and_headers() {
    let (url, received) = serve_stream("200 OK", Vec::new(), Duration::ZERO);

    let _ = collect(&post(url), &StopSignal::new());

    let request = received.recv().unwrap();
    assert_eq!(request.line, "POST /v1/chat/completions HTTP/1.1");
    assert!(
        request
            .headers
            .contains(&"authorization: bearer sk-test".to_owned())
    );
    assert!(
        request
            .headers
            .contains(&"accept: text/event-stream".to_owned())
    );
    assert!(
        !request
            .headers
            .iter()
            .any(|header| header.starts_with("cookie"))
    );
    assert_eq!(request.body, b"{\"stream\":true}");
}

#[test]
fn a_streamed_get_sends_no_body() {
    let (url, received) = serve_stream("200 OK", Vec::new(), Duration::ZERO);
    let request = HttpRequest {
        method: HttpMethod::Get,
        url,
        headers: Vec::new(),
    };

    let _ = collect(&request, &StopSignal::new());

    let request = received.recv().unwrap();
    assert_eq!(request.line, "GET /v1/chat/completions HTTP/1.1");
    assert_eq!(request.body, b"");
}

#[test]
fn an_error_status_still_delivers_its_body() {
    let pieces: Vec<&'static [u8]> = vec![b"{\"error\":\"bad key\"}"];
    let (url, _) = serve_stream("401 Unauthorized", pieces, Duration::ZERO);

    let (ended, status, body) = collect(&post(url), &StopSignal::new());

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(status, Some(401));
    assert_eq!(body.concat(), b"{\"error\":\"bad key\"}");
}

#[test]
fn a_stop_while_the_server_is_silent_ends_the_stream_at_once() {
    let pieces: Vec<&'static [u8]> = vec![b"data: first\n\n"];
    let (url, _) = serve_stream("200 OK", pieces, Duration::from_secs(10));
    let stop = StopSignal::new();
    let stopper = stop.clone();
    let mut first = None;
    let started = Instant::now();

    let ended = UrlSessionHttp::new().stream(&post(url), &stop, &mut |event| {
        if let HttpEvent::Body(bytes) = event {
            first = Some(bytes.to_vec());
            stopper.stop();
        }
        Flow::Continue
    });

    assert_eq!(ended, Ok(Ended::Stopped));
    assert_eq!(first, Some(b"data: first\n\n".to_vec()));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_stream_stopped_before_it_starts_sends_nothing_back() {
    let (url, _) = serve_stream("200 OK", vec![b"data: x\n\n"], Duration::ZERO);
    let stop = StopSignal::new();
    stop.stop();

    let (ended, status, body) = collect(&post(url), &stop);

    assert_eq!(ended, Ok(Ended::Stopped));
    assert_eq!(status, None);
    assert_eq!(body, Vec::<Vec<u8>>::new());
}

#[test]
fn a_stream_to_a_host_that_refuses_fails() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();

    let (ended, status, _) = collect(&post(url), &StopSignal::new());

    assert!(ended.is_err());
    assert_eq!(status, None);
}

#[test]
fn a_stream_whose_answer_is_not_http_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("answer.txt");
    std::fs::write(&path, "local").unwrap();
    let request = HttpRequest {
        method: HttpMethod::Get,
        url: Url::from_file_path(&path).unwrap(),
        headers: Vec::new(),
    };

    let (ended, _, _) = collect(&request, &StopSignal::new());

    assert_eq!(ended, Err(HttpError("the answer is not http".to_owned())));
}

/// Serves one connection with `status` and `body`, and returns its URL.
fn serve_once(status: &'static str, body: &'static [u8]) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 2 {
            line.clear();
        }
        let mut stream = reader.into_inner();
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    });
    Url::parse(&format!("http://127.0.0.1:{port}/list.txt")).unwrap()
}

#[test]
fn a_successful_answer_gives_its_body() {
    let url = serve_once("200 OK", b"||tracker.test^\n");

    let body = UrlSessionHttp::new().get(&url);

    assert_eq!(body, Ok(b"||tracker.test^\n".to_vec()));
}

#[test]
fn an_empty_answer_is_an_empty_body() {
    let url = serve_once("204 No Content", b"");

    let body = UrlSessionHttp.get(&url);

    assert_eq!(body, Ok(Vec::new()));
}

#[test]
fn a_status_outside_2xx_is_a_failure() {
    let url = serve_once("404 Not Found", b"gone");

    let body = UrlSessionHttp::new().get(&url);

    assert_eq!(body, Err(HttpError("status 404".to_owned())));
}

#[test]
fn a_host_that_refuses_the_connection_is_a_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();

    let body = UrlSessionHttp::new().get(&url);

    assert!(body.is_err());
}

#[test]
fn an_answer_that_is_not_http_is_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("list.txt");
    std::fs::write(&path, "local").unwrap();
    let url = Url::from_file_path(&path).unwrap();

    let body = UrlSessionHttp::new().get(&url);

    assert_eq!(body, Err(HttpError("the answer is not http".to_owned())));
}
