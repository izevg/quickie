use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Duration;

use super::{Cancellation, SoupTransport};
use quickie_updater::Transport;
use url::Url;

fn fixture(response: &'static str) -> (Url, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback fixture");
    let address = listener.local_addr().expect("loopback address");
    let fixture = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("loopback request");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set fixture timeout");
        let mut request = BufReader::new(&socket);
        let mut line = String::new();
        loop {
            line.clear();
            if request.read_line(&mut line).expect("HTTP header") == 0 || line == "\r\n" {
                break;
            }
        }
        socket
            .write_all(response.as_bytes())
            .expect("loopback response");
    });
    (
        Url::parse(&format!("http://{address}/asset")).expect("loopback URL"),
        fixture,
    )
}

#[test]
fn exposes_a_redirect_without_delivering_its_body_or_following_it() {
    let (url, server) = fixture(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/elsewhere\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody",
    );
    let cancel = Cancellation::new().expect("cancellation handle");
    let response = SoupTransport(&cancel)
        .get(&url, "application/octet-stream", &mut |_| {
            Err(io::Error::other(
                "redirect body must not reach the receiver",
            ))
        })
        .expect("redirect response");
    assert_eq!(response.status, 302);
    assert_eq!(
        response.location.as_deref(),
        Some("http://127.0.0.1:1/elsewhere")
    );
    server.join().expect("fixture server");
}

#[test]
fn delivers_successful_bytes_to_the_bounded_receiver() {
    let (url, server) =
        fixture("HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
    let cancel = Cancellation::new().expect("cancellation handle");
    let mut actual = Vec::new();
    SoupTransport(&cancel)
        .get(&url, "application/octet-stream", &mut |bytes| {
            actual.extend_from_slice(bytes);
            Ok(())
        })
        .expect("complete response");
    assert_eq!(actual, b"hello");
    server.join().expect("fixture server");
}
