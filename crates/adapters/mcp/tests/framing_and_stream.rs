//! End-to-end framing and generic `AsyncRead`/`AsyncWrite` stream tests for `quickie-mcp`.
#![expect(
    clippy::expect_used,
    reason = "integration tests panic on unexpected setup or I/O failure"
)]

mod support;

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use quickie_mcp::{MAX_FRAME_BYTES, ShutdownTrigger, serve_stream};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Notify;

use crate::support::{
    EchoHandler, initialize_frame, initialized_notification_frame, tools_call_echo_frame,
    tools_list_frame,
};

fn build_current_thread_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("build current-thread tokio runtime")
}

async fn read_json_line<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Value {
    let mut line = String::new();
    let n = tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line))
        .await
        .expect("timed out waiting for server frame")
        .expect("read server frame");
    assert!(n > 0, "unexpected EOF while waiting for server frame");
    serde_json::from_str(&line).expect("server frame must be valid JSON")
}

#[test]
fn serve_stream_completes_initialize_tools_list_and_tools_call() {
    let rt = build_current_thread_rt();
    rt.block_on(async {
        // Arrange
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, mut client_write) = tokio::io::split(client_io);
        let mut client_lines = BufReader::new(client_read);
        let (_trigger, shutdown) = ShutdownTrigger::new();
        let server = tokio::spawn(serve_stream(
            EchoHandler::new(1),
            server_read,
            server_write,
            shutdown,
        ));

        // Act
        client_write
            .write_all(initialize_frame(1).as_bytes())
            .await
            .expect("write initialize");
        let init_resp = read_json_line(&mut client_lines).await;

        client_write
            .write_all(initialized_notification_frame().as_bytes())
            .await
            .expect("write initialized notification");
        client_write
            .write_all(tools_list_frame(2).as_bytes())
            .await
            .expect("write tools/list");
        let list_resp = read_json_line(&mut client_lines).await;

        client_write
            .write_all(tools_call_echo_frame(3, "hello-mcp").as_bytes())
            .await
            .expect("write tools/call");
        let call_resp = read_json_line(&mut client_lines).await;

        client_write
            .shutdown()
            .await
            .expect("shutdown client write");
        drop(client_write);
        let server_res = tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("server join timeout")
            .expect("server task panicked");

        // Assert
        assert!(server_res.is_ok());
        assert_eq!(init_resp["id"], 1);
        assert!(init_resp["result"]["capabilities"]["tools"].is_object());
        assert_eq!(list_resp["id"], 2);
        assert_eq!(list_resp["result"]["tools"][0]["name"], "echo");
        assert_eq!(call_resp["id"], 3);
        assert_eq!(
            call_resp["result"]["content"][0]["text"],
            "session=1 call=1 text=hello-mcp"
        );
    });
}

#[test]
fn serve_stream_disconnects_immediately_on_malformed_json_frame() {
    let rt = build_current_thread_rt();
    rt.block_on(async {
        // Arrange
        let (client_io, server_io) = tokio::io::duplex(8 * 1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, mut client_write) = tokio::io::split(client_io);
        let mut client_lines = BufReader::new(client_read);
        let (_trigger, shutdown) = ShutdownTrigger::new();
        let server = tokio::spawn(serve_stream(
            EchoHandler::new(2),
            server_read,
            server_write,
            shutdown,
        ));

        // Act
        client_write
            .write_all(b"{not-valid-json\n")
            .await
            .expect("write malformed frame");
        let mut buf = String::new();
        let bytes_read =
            tokio::time::timeout(Duration::from_secs(3), client_lines.read_line(&mut buf))
                .await
                .expect("timed out waiting for disconnect")
                .expect("read after malformed frame");
        let server_res = tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("server join timeout")
            .expect("server task panicked");

        // Assert
        assert_eq!(
            bytes_read, 0,
            "server must close write side on malformed frame"
        );
        assert!(server_res.is_ok());
    });
}

#[test]
fn serve_stream_disconnects_on_oversized_frame_above_256_kib() {
    let rt = build_current_thread_rt();
    rt.block_on(async {
        // Arrange
        let (client_io, server_io) = tokio::io::duplex(512 * 1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, mut client_write) = tokio::io::split(client_io);
        let mut client_lines = BufReader::new(client_read);
        let (_trigger, shutdown) = ShutdownTrigger::new();
        let server = tokio::spawn(serve_stream(
            EchoHandler::new(3),
            server_read,
            server_write,
            shutdown,
        ));

        // Build a frame that is exactly MAX_FRAME_BYTES + 1 bytes including `\n`.
        let mut oversized = vec![b'a'; MAX_FRAME_BYTES];
        oversized.push(b'\n');
        assert_eq!(oversized.len(), MAX_FRAME_BYTES + 1);

        // Act
        let _ = client_write.write_all(&oversized).await;
        let mut buf = String::new();
        let bytes_read =
            tokio::time::timeout(Duration::from_secs(3), client_lines.read_line(&mut buf))
                .await
                .expect("timed out waiting for disconnect")
                .expect("read after oversized frame");
        let server_res = tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("server join timeout")
            .expect("server task panicked");

        // Assert
        assert_eq!(
            bytes_read, 0,
            "server must close write side on oversized frame"
        );
        assert!(server_res.is_ok());
    });
}

#[test]
fn serve_stream_accepts_exact_256_kib_frame_boundary() {
    let rt = build_current_thread_rt();
    rt.block_on(async {
        // Arrange
        let (client_io, server_io) = tokio::io::duplex(512 * 1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, mut client_write) = tokio::io::split(client_io);
        let mut client_lines = BufReader::new(client_read);
        let (_trigger, shutdown) = ShutdownTrigger::new();
        let server = tokio::spawn(serve_stream(
            EchoHandler::new(4),
            server_read,
            server_write,
            shutdown,
        ));

        client_write
            .write_all(initialize_frame(1).as_bytes())
            .await
            .expect("write initialize");
        let _ = read_json_line(&mut client_lines).await;
        client_write
            .write_all(initialized_notification_frame().as_bytes())
            .await
            .expect("write initialized");

        let template = tools_call_echo_frame(2, "");
        let pad_len = MAX_FRAME_BYTES - template.len();
        let exact_text = "x".repeat(pad_len);
        let exact_frame = tools_call_echo_frame(2, &exact_text);
        assert_eq!(exact_frame.len(), MAX_FRAME_BYTES);

        // Act
        client_write
            .write_all(exact_frame.as_bytes())
            .await
            .expect("write exact-limit frame");
        let call_resp = read_json_line(&mut client_lines).await;
        client_write
            .shutdown()
            .await
            .expect("shutdown client write");
        drop(client_write);
        let server_res = tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("server join timeout")
            .expect("server task panicked");

        // Assert
        assert!(server_res.is_ok());
        assert_eq!(call_resp["id"], 2);
        let expected = format!("session=4 call=1 text={exact_text}");
        assert_eq!(call_resp["result"]["content"][0]["text"], expected);
    });
}

#[test]
fn serve_stream_exits_cleanly_when_shutdown_trigger_fires() {
    let rt = build_current_thread_rt();
    rt.block_on(async {
        // Arrange
        let (_client_io, server_io) = tokio::io::duplex(8 * 1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (trigger, shutdown) = ShutdownTrigger::new();
        let server = tokio::spawn(serve_stream(
            EchoHandler::new(5),
            server_read,
            server_write,
            shutdown,
        ));

        // Act
        trigger.trigger();
        let server_res = tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("server should exit promptly after shutdown trigger")
            .expect("server task panicked");

        // Assert
        assert!(server_res.is_ok());
    });
}

struct WriteBlockObserver {
    blocked: AtomicBool,
    notify: Notify,
}

impl WriteBlockObserver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            blocked: AtomicBool::new(false),
            notify: Notify::new(),
        })
    }

    fn mark_blocked(&self) {
        self.blocked.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn wait_until_blocked(&self) {
        while !self.blocked.load(Ordering::Acquire) {
            let notified = self.notify.notified();
            if self.blocked.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

struct ObservedWriter<W> {
    inner: W,
    observer: Arc<WriteBlockObserver>,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for ObservedWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        if poll.is_pending() {
            self.observer.mark_blocked();
        }
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[test]
fn serve_stream_joins_promptly_on_shutdown_when_response_write_is_blocked() {
    let rt = build_current_thread_rt();
    rt.block_on(async {
        // Arrange
        const DUPLEX_BUF_BYTES: usize = 4 * 1024;
        let (client_io, server_io) = tokio::io::duplex(DUPLEX_BUF_BYTES);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, mut client_write) = tokio::io::split(client_io);
        let mut client_lines = BufReader::new(client_read);
        let observer = WriteBlockObserver::new();
        let observed_write = ObservedWriter {
            inner: server_write,
            observer: Arc::clone(&observer),
        };
        let (trigger, shutdown) = ShutdownTrigger::new();
        let server = tokio::spawn(serve_stream(
            EchoHandler::new(6),
            server_read,
            observed_write,
            shutdown,
        ));

        client_write
            .write_all(initialize_frame(1).as_bytes())
            .await
            .expect("write initialize");
        let _ = read_json_line(&mut client_lines).await;
        client_write
            .write_all(initialized_notification_frame().as_bytes())
            .await
            .expect("write initialized");

        let large_text = "x".repeat(DUPLEX_BUF_BYTES * 2);
        let large_frame = tools_call_echo_frame(2, &large_text);
        client_write
            .write_all(large_frame.as_bytes())
            .await
            .expect("write large echo request");

        // Verify the response write has started and blocked on the full duplex buffer without
        // draining any response bytes on the client side.
        tokio::time::timeout(Duration::from_secs(3), observer.wait_until_blocked())
            .await
            .expect("server response write did not block on bounded duplex");

        // Act
        trigger.trigger();
        let server_res = tokio::time::timeout(Duration::from_millis(500), server)
            .await
            .expect("server must join promptly on shutdown despite blocked write")
            .expect("server task panicked");

        // Assert
        assert!(server_res.is_ok());
        drop((client_lines, client_write));
    });
}
