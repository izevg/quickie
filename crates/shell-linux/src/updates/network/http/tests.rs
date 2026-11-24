use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;

fn socket() -> (TcpListener, Url) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let url = Url::parse(&format!("http://{address}/v1/chat/completions")).unwrap();
    (listener, url)
}

fn connection(listener: &TcpListener) -> TcpStream {
    let (stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    stream
}

fn incoming(stream: &TcpStream) -> (String, String, Vec<u8>) {
    let mut read = BufReader::new(stream);
    let mut first_line = String::new();
    read.read_line(&mut first_line).unwrap();
    let mut headers = String::new();
    let mut length = 0;
    loop {
        let mut line = String::new();
        let count = read.read_line(&mut line).unwrap();
        if count == 0 || line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap();
        }
        headers.push_str(&line);
    }
    let mut body = vec![0; length];
    read.read_exact(&mut body).unwrap();
    (first_line, headers, body)
}

#[test]
fn a_redirect_delivers_status_without_resending_authorization() {
    // Arrange
    let (origin, url) = socket();
    let (foreign, destination) = socket();
    foreign.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let mut stream = connection(&origin);
        let request = incoming(&stream);
        write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        request
    });
    let proof = format!("Bearer {}", std::process::id());
    let post = HttpRequest {
        method: HttpMethod::Post(b"{\"model\":\"fixture\"}".to_vec()),
        url,
        headers: vec![
            ("Authorization".into(), proof.clone()),
            ("Content-Type".into(), "application/json".into()),
        ],
    };
    let mut status = None;

    // Act
    let ended = WebKitHttp.stream(&post, &StopSignal::new(), &mut |event| {
        if let HttpEvent::Status(value) = event {
            status = Some(value);
        }
        Flow::Continue
    });

    // Assert
    let (method, headers, body) = server.join().unwrap();
    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(status, Some(307));
    assert_eq!(method, "POST /v1/chat/completions HTTP/1.1\r\n");
    assert!(headers.contains(&proof));
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("content-type: application/json")
    );
    assert_eq!(body, b"{\"model\":\"fixture\"}");
    assert_eq!(
        foreign.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn a_stopped_request_sends_nothing() {
    // Arrange
    let (server, url) = socket();
    server.set_nonblocking(true).unwrap();
    let request = HttpRequest {
        method: HttpMethod::Get,
        url,
        headers: vec![(
            "Authorization".into(),
            format!("Bearer {}", std::process::id()),
        )],
    };
    let stop = StopSignal::new();
    stop.stop();

    // Act
    let ended = WebKitHttp.stream(&request, &stop, &mut |_| {
        panic!("a stopped request emitted an event")
    });

    // Assert
    assert_eq!(ended, Ok(Ended::Stopped));
    assert_eq!(
        server.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn stopping_interrupts_a_silent_response_body() {
    // Arrange
    let (listener, url) = socket();
    let request = HttpRequest {
        method: HttpMethod::Get,
        url,
        headers: Vec::new(),
    };
    let (ready, observed) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut stream = connection(&listener);
        incoming(&stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n")
            .unwrap();
        let mut buffer = [0];
        stream.read(&mut buffer)
    });
    let stop = StopSignal::new();
    let user_stop = stop.clone();
    let worker = thread::spawn(move || {
        WebKitHttp.stream(&request, &stop, &mut |event| {
            if event == HttpEvent::Status(200) {
                ready.send(()).unwrap();
            }
            Flow::Continue
        })
    });
    observed.recv_timeout(Duration::from_secs(4)).unwrap();
    let started = Instant::now();

    // Act
    user_stop.stop();

    // Assert
    assert_eq!(worker.join().unwrap(), Ok(Ended::Stopped));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(server.join().unwrap().unwrap(), 0);
}
