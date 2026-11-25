//! Unix socket security, permissions, lock/reclaim, and 16-client concurrency tests for
//! `quickie-mcp`.
#![expect(
    clippy::expect_used,
    reason = "integration tests panic on unexpected setup or I/O failure"
)]

mod support;

use std::fs::{self, Permissions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use quickie_mcp::{
    MAX_CONCURRENT_CONNECTIONS, MAX_FRAME_BYTES, McpError, SocketEndpoint, connect_socket,
    serve_socket,
};
use serde_json::Value;
use tempfile::TempDir;

use crate::support::{
    EchoHandler, initialize_frame, initialized_notification_frame, tools_call_echo_frame,
    tools_list_frame,
};

fn private_temp_root() -> TempDir {
    let dir = TempDir::new().expect("create temp dir");
    fs::set_permissions(dir.path(), Permissions::from_mode(0o700)).expect("chmod 0700 temp root");
    dir
}

struct SyncMcpClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl SyncMcpClient {
    fn connect(endpoint: &SocketEndpoint) -> Self {
        let stream = UnixStream::connect(endpoint.socket_path()).expect("connect unix socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("set read timeout");
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .expect("set write timeout");
        let writer = stream.try_clone().expect("clone unix stream");
        Self {
            reader: BufReader::new(stream),
            writer,
        }
    }

    fn send_raw(&mut self, data: &[u8]) {
        self.writer.write_all(data).expect("write to socket");
        self.writer.flush().expect("flush socket");
    }

    fn read_json(&mut self) -> Value {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).expect("read json line");
        assert!(n > 0, "unexpected EOF reading JSON line");
        serde_json::from_str(&line).expect("parse server JSON")
    }

    fn handshake(&mut self) -> Value {
        self.send_raw(initialize_frame(1).as_bytes());
        let init = self.read_json();
        self.send_raw(initialized_notification_frame().as_bytes());
        init
    }
}

#[test]
fn serve_socket_creates_0700_dir_0600_socket_and_cleans_up_on_shutdown() {
    // Arrange
    let root = private_temp_root();
    let sock_path = root.path().join("quickie").join("mcp.sock");
    let endpoint =
        SocketEndpoint::in_trusted_root(root.path(), &sock_path).expect("build validated endpoint");

    // Act
    let handle = serve_socket(&endpoint, || EchoHandler::new(1)).expect("start socket server");
    let dir_meta = fs::symlink_metadata(endpoint.parent_dir()).expect("stat parent dir");
    let sock_meta = fs::symlink_metadata(endpoint.socket_path()).expect("stat socket");
    handle.shutdown();
    let exists_after_shutdown = endpoint.socket_path().exists();

    // Assert
    assert_eq!(dir_meta.mode() & 0o777, 0o700);
    assert_eq!(sock_meta.mode() & 0o777, 0o600);
    assert!(
        !exists_after_shutdown,
        "socket must be unlinked on shutdown"
    );
}

#[test]
fn endpoint_validation_rejects_unsafe_dir_mode_symlinks_and_escape() {
    // Arrange
    let root = private_temp_root();
    let outside = root.path().parent().expect("parent").join("outside.sock");
    let escape_path = root.path().join("quickie").join("..").join("mcp.sock");
    let unsafe_dir = root.path().join("wide");
    fs::create_dir(&unsafe_dir).expect("create wide dir");
    fs::set_permissions(&unsafe_dir, Permissions::from_mode(0o755)).expect("chmod 0755");
    let link_dir = root.path().join("linkdir");
    symlink(&unsafe_dir, &link_dir).expect("create symlink dir");

    // Act
    let outside_res = SocketEndpoint::in_trusted_root(root.path(), outside);
    let escape_res = SocketEndpoint::in_trusted_root(root.path(), escape_path);
    let wide_ep = SocketEndpoint::in_trusted_root(root.path(), unsafe_dir.join("mcp.sock"))
        .expect("path syntax valid");
    let wide_bind = serve_socket(&wide_ep, || EchoHandler::new(1));
    let link_ep = SocketEndpoint::in_trusted_root(root.path(), link_dir.join("mcp.sock"))
        .expect("path syntax valid");
    let link_bind = serve_socket(&link_ep, || EchoHandler::new(1));

    // Assert
    assert!(matches!(outside_res, Err(McpError::InvalidEndpoint(_))));
    assert!(matches!(escape_res, Err(McpError::InvalidEndpoint(_))));
    assert!(matches!(wide_bind, Err(McpError::InvalidEndpoint(_))));
    assert!(matches!(link_bind, Err(McpError::InvalidEndpoint(_))));
}

#[test]
fn connect_socket_classifies_missing_without_creating_files_and_stale_separately() {
    // Arrange
    let root = private_temp_root();
    let dir = root.path().join("quickie");
    let sock_path = dir.join("mcp.sock");
    let endpoint =
        SocketEndpoint::in_trusted_root(root.path(), &sock_path).expect("validated endpoint");

    // Act: missing endpoint must not create parent directory.
    let missing_err = connect_socket(&endpoint).expect_err("missing socket must fail");
    let dir_created = dir.exists();

    // Create a stale socket file (bound and listener dropped without unlinking).
    fs::create_dir(&dir).expect("create quickie dir");
    fs::set_permissions(&dir, Permissions::from_mode(0o700)).expect("chmod 0700");
    let stale_listener = UnixListener::bind(&sock_path).expect("bind stale socket");
    fs::set_permissions(&sock_path, Permissions::from_mode(0o600)).expect("chmod 0600");
    drop(stale_listener);
    let stale_err = connect_socket(&endpoint).expect_err("stale socket must fail");

    // Assert
    assert!(missing_err.is_missing_endpoint());
    assert!(!dir_created, "connect_socket must never create directories");
    assert!(matches!(stale_err, McpError::EndpointStale(_)));
    assert!(!stale_err.is_missing_endpoint());
}

#[test]
fn serve_socket_enforces_lock_exclusivity_and_reclaims_stale_socket() {
    // Arrange
    let root = private_temp_root();
    let dir = root.path().join("quickie");
    fs::create_dir(&dir).expect("create dir");
    fs::set_permissions(&dir, Permissions::from_mode(0o700)).expect("chmod 0700");
    let sock_path = dir.join("mcp.sock");
    let stale_listener = UnixListener::bind(&sock_path).expect("bind stale socket");
    fs::set_permissions(&sock_path, Permissions::from_mode(0o600)).expect("chmod 0600");
    drop(stale_listener);
    let endpoint =
        SocketEndpoint::in_trusted_root(root.path(), &sock_path).expect("validated endpoint");

    // Act
    let first = serve_socket(&endpoint, || EchoHandler::new(1))
        .expect("stale socket with no lock must be reclaimed");
    let second = serve_socket(&endpoint, || EchoHandler::new(2));
    let mut client = SyncMcpClient::connect(&endpoint);
    let init = client.handshake();
    drop(client);
    first.shutdown();

    // Assert
    assert!(matches!(second, Err(McpError::EndpointInUse(_))));
    assert_eq!(init["id"], 1);
}

#[test]
fn serve_socket_supports_16_independent_sessions_and_isolates_bad_peers() {
    // Arrange
    let root = private_temp_root();
    let sock_path = root.path().join("quickie").join("mcp.sock");
    let endpoint =
        SocketEndpoint::in_trusted_root(root.path(), &sock_path).expect("validated endpoint");
    let next_id = Arc::new(AtomicUsize::new(1));
    let factory_counter = Arc::clone(&next_id);
    let handle = serve_socket(&endpoint, move || {
        let id = factory_counter.fetch_add(1, Ordering::Relaxed);
        EchoHandler::new(id)
    })
    .expect("start socket server");

    // Act: open 16 simultaneous independent MCP sessions.
    let mut clients: Vec<SyncMcpClient> = (0..MAX_CONCURRENT_CONNECTIONS)
        .map(|_| {
            let mut c = SyncMcpClient::connect(&endpoint);
            let init = c.handshake();
            assert_eq!(init["id"], 1);
            c
        })
        .collect();

    // 17th client should be dropped by the 16-session semaphore cap.
    let mut overflow = SyncMcpClient::connect(&endpoint);
    let _ = overflow.writer.write_all(initialize_frame(1).as_bytes());
    let mut overflow_line = String::new();
    let overflow_read = overflow.reader.read_line(&mut overflow_line);

    // Send malformed JSON on client 0 and oversized frame on client 1.
    clients[0].send_raw(b"not-json\n");
    let mut malformed_eof = String::new();
    let malformed_read = clients[0]
        .reader
        .read_line(&mut malformed_eof)
        .expect("read after malformed");

    let mut huge = vec![b'z'; MAX_FRAME_BYTES];
    huge.push(b'\n');
    let _ = clients[1].writer.write_all(&huge);
    let mut oversize_eof = String::new();
    let oversize_read = clients[1]
        .reader
        .read_line(&mut oversize_eof)
        .expect("read after oversize");

    // Healthy client 2 must remain unaffected and have its own independent session state.
    clients[2].send_raw(tools_list_frame(2).as_bytes());
    let list_resp = clients[2].read_json();
    clients[2].send_raw(tools_call_echo_frame(3, "alpha").as_bytes());
    let call_a = clients[2].read_json();
    clients[3].send_raw(tools_call_echo_frame(4, "beta").as_bytes());
    let call_b = clients[3].read_json();

    drop(clients);
    drop(overflow);

    // After slots free up, a new client can connect and serve cleanly.
    let mut recovered = SyncMcpClient::connect(&endpoint);
    let recovered_init = recovered.handshake();
    drop(recovered);
    handle.shutdown();

    // Assert
    assert!(
        matches!(overflow_read, Ok(0) | Err(_)),
        "17th concurrent client must be rejected while 16 sessions are active"
    );
    assert_eq!(malformed_read, 0, "malformed peer must be disconnected");
    assert_eq!(oversize_read, 0, "oversized peer must be disconnected");
    assert_eq!(list_resp["result"]["tools"][0]["name"], "echo");
    let text_a = call_a["result"]["content"][0]["text"]
        .as_str()
        .expect("call_a text");
    let text_b = call_b["result"]["content"][0]["text"]
        .as_str()
        .expect("call_b text");
    assert!(text_a.contains("call=1 text=alpha"));
    assert!(text_b.contains("call=1 text=beta"));
    assert_ne!(
        text_a, text_b,
        "sessions must have independent handler state"
    );
    assert_eq!(recovered_init["id"], 1);
}

#[test]
fn socket_shutdown_does_not_unlink_replaced_inode() {
    // Arrange
    let root = private_temp_root();
    let sock_path = root.path().join("quickie").join("mcp.sock");
    let endpoint =
        SocketEndpoint::in_trusted_root(root.path(), &sock_path).expect("validated endpoint");
    let handle = serve_socket(&endpoint, || EchoHandler::new(1)).expect("start server");

    // Simulate an external replacement of the socket path with a different inode.
    fs::remove_file(&sock_path).expect("remove original socket");
    let replacement = UnixListener::bind(&sock_path).expect("bind replacement socket");
    let replacement_ino = fs::symlink_metadata(&sock_path)
        .expect("stat replacement")
        .ino();

    // Act
    handle.shutdown();
    let after_meta = fs::symlink_metadata(&sock_path).expect("replacement must remain");
    drop(replacement);
    let _ = fs::remove_file(&sock_path);

    // Assert
    assert_eq!(after_meta.ino(), replacement_ino);
}
