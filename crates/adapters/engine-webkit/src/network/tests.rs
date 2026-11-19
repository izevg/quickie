//! `NSURLSession` against a one-shot server on 127.0.0.1: no request leaves the machine.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;

use quickie_application::{HttpError, HttpPort};
use url::Url;

use super::UrlSessionHttp;

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

    let body = UrlSessionHttp::default().get(&url);

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
