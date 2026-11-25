//! OS-pipe relay, controlled EOF, standalone stdio server, and process-global stdout isolation
//! tests for `quickie-mcp`.

#![expect(
    unsafe_code,
    reason = "OS pipe creation via libc::pipe for real fd relay and stdio isolation tests"
)]
#![expect(
    clippy::expect_used,
    reason = "integration tests panic on unexpected setup or I/O failure"
)]

mod support;

use std::fs::{self, File, Permissions};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use quickie_mcp::{
    IsolatedStdio, RelayConfig, SocketEndpoint, run_relay_fds, serve_socket, serve_stdio,
};
use serde_json::Value;
use tempfile::TempDir;

use crate::support::{
    EchoHandler, initialize_frame, initialized_notification_frame, tools_call_echo_frame,
    tools_list_frame,
};

fn make_os_pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0_i32; 2];
    // SAFETY: `fds` is a valid 2-element array for `pipe`.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(rc, 0, "libc::pipe failed");
    // SAFETY: `pipe` succeeded and `fds[0]` is a newly created, exclusively owned read fd.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: `pipe` succeeded and `fds[1]` is a newly created, exclusively owned write fd.
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    (read_fd, write_fd)
}

fn private_temp_root() -> TempDir {
    let dir = TempDir::new().expect("create temp dir");
    fs::set_permissions(dir.path(), Permissions::from_mode(0o700)).expect("chmod 0700 temp root");
    dir
}

fn read_json_line<R: BufRead>(reader: &mut R) -> Value {
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("read json line");
    assert!(n > 0, "unexpected EOF reading JSON line");
    serde_json::from_str(&line).expect("parse JSON line")
}

#[test]
fn relay_forwards_mcp_handshake_tools_and_drains_on_stdin_eof() {
    // Arrange
    let root = private_temp_root();
    let sock_path = root.path().join("quickie").join("mcp.sock");
    let endpoint =
        SocketEndpoint::in_trusted_root(root.path(), &sock_path).expect("validated endpoint");
    let server = serve_socket(&endpoint, || EchoHandler::new(42)).expect("start socket server");

    let (relay_stdin_rd, client_stdin_wr) = make_os_pipe();
    let (client_stdout_rd, relay_stdout_wr) = make_os_pipe();
    let relay_ep = endpoint.clone();
    let relay_thread = thread::spawn(move || {
        run_relay_fds(
            &relay_ep,
            relay_stdin_rd,
            relay_stdout_wr,
            RelayConfig {
                drain_timeout: Duration::from_secs(2),
            },
        )
    });

    let mut client_in = File::from(client_stdin_wr);
    let mut client_out = BufReader::new(File::from(client_stdout_rd));

    // Act
    client_in
        .write_all(initialize_frame(1).as_bytes())
        .expect("write initialize");
    client_in.flush().expect("flush initialize");
    let init_resp = read_json_line(&mut client_out);

    client_in
        .write_all(initialized_notification_frame().as_bytes())
        .expect("write initialized");
    client_in
        .write_all(tools_list_frame(2).as_bytes())
        .expect("write tools/list");
    client_in
        .write_all(tools_call_echo_frame(3, "via-relay").as_bytes())
        .expect("write tools/call");
    client_in.flush().expect("flush requests");
    // Close client stdin immediately after sending requests to verify half-close drain!
    drop(client_in);

    let list_resp = read_json_line(&mut client_out);
    let call_resp = read_json_line(&mut client_out);
    let relay_res = relay_thread.join().expect("relay thread panicked");
    server.shutdown();

    // Assert
    assert!(relay_res.is_ok(), "relay failed: {relay_res:?}");
    assert_eq!(init_resp["id"], 1);
    assert_eq!(list_resp["result"]["tools"][0]["name"], "echo");
    assert_eq!(
        call_resp["result"]["content"][0]["text"],
        "session=42 call=1 text=via-relay"
    );
}

#[test]
fn relay_exits_promptly_on_server_close_while_stdin_pipe_remains_open() {
    // Arrange
    let root = private_temp_root();
    let sock_path = root.path().join("quickie").join("mcp.sock");
    let endpoint =
        SocketEndpoint::in_trusted_root(root.path(), &sock_path).expect("validated endpoint");
    let server = serve_socket(&endpoint, || EchoHandler::new(7)).expect("start socket server");

    let (relay_stdin_rd, client_stdin_wr) = make_os_pipe();
    let (client_stdout_rd, relay_stdout_wr) = make_os_pipe();
    let relay_ep = endpoint.clone();
    let relay_thread = thread::spawn(move || {
        run_relay_fds(
            &relay_ep,
            relay_stdin_rd,
            relay_stdout_wr,
            RelayConfig::default(),
        )
    });

    let mut client_in = File::from(client_stdin_wr);
    let mut client_out = BufReader::new(File::from(client_stdout_rd));
    client_in
        .write_all(initialize_frame(1).as_bytes())
        .expect("write initialize");
    client_in.flush().expect("flush");
    let _ = read_json_line(&mut client_out);

    // Act: shut down the server while intentionally keeping `client_in` open!
    let start = Instant::now();
    server.shutdown();
    let relay_res = relay_thread.join().expect("relay thread panicked");
    let elapsed = start.elapsed();
    let mut eof_line = String::new();
    let eof_bytes = client_out
        .read_line(&mut eof_line)
        .expect("read client stdout after server shutdown");
    drop(client_in);

    // Assert
    assert!(relay_res.is_ok(), "relay returned error: {relay_res:?}");
    assert_eq!(
        eof_bytes, 0,
        "client stdout must observe EOF after server shutdown even while stdin stays open"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "relay hung waiting on open stdin after server EOF ({elapsed:?})"
    );
}

#[test]
fn serve_stdio_handles_mcp_over_os_pipes_and_exits_on_stdin_eof() {
    // Arrange
    let (server_stdin_rd, client_stdin_wr) = make_os_pipe();
    let (client_stdout_rd, server_stdout_wr) = make_os_pipe();
    let isolated = IsolatedStdio::from_fds(server_stdin_rd, server_stdout_wr);
    let handle = serve_stdio(isolated, EchoHandler::new(99)).expect("start stdio server");

    let mut client_in = File::from(client_stdin_wr);
    let mut client_out = BufReader::new(File::from(client_stdout_rd));

    // Act
    client_in
        .write_all(initialize_frame(1).as_bytes())
        .expect("write initialize");
    client_in.flush().expect("flush");
    let init_resp = read_json_line(&mut client_out);

    client_in
        .write_all(initialized_notification_frame().as_bytes())
        .expect("write initialized");
    client_in
        .write_all(tools_call_echo_frame(2, "stdio-direct").as_bytes())
        .expect("write tools/call");
    client_in.flush().expect("flush");
    let call_resp = read_json_line(&mut client_out);

    drop(client_in);
    let start = Instant::now();
    handle.wait();
    let elapsed = start.elapsed();

    // Assert
    assert_eq!(init_resp["id"], 1);
    assert_eq!(
        call_resp["result"]["content"][0]["text"],
        "session=99 call=1 text=stdio-direct"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "serve_stdio did not exit promptly on stdin EOF ({elapsed:?})"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_default_runtime_dir_ignores_overridden_tmpdir() {
    const CHECK_CHILD_ENV: &str = "QUICKIE_MCP_TEST_DARWIN_DIR_CHILD";
    if std::env::var_os(CHECK_CHILD_ENV).is_some() {
        let dir = quickie_mcp::default_runtime_dir().expect("resolve darwin runtime dir");
        println!("{}", dir.display());
        return;
    }

    // Arrange
    let expected_raw = Command::new("getconf")
        .arg("DARWIN_USER_TEMP_DIR")
        .output()
        .expect("run getconf DARWIN_USER_TEMP_DIR");
    assert!(expected_raw.status.success(), "getconf failed");
    let expected = String::from_utf8(expected_raw.stdout)
        .expect("utf8 getconf output")
        .trim()
        .trim_end_matches('/')
        .to_owned();

    // Act
    let exe = std::env::current_exe().expect("current test binary path");
    let output = Command::new(exe)
        .arg("--exact")
        .arg("macos_default_runtime_dir_ignores_overridden_tmpdir")
        .arg("--nocapture")
        .env(CHECK_CHILD_ENV, "1")
        .env("TMPDIR", "/tmp")
        .output()
        .expect("spawn child with overridden TMPDIR");

    // Assert
    assert!(output.status.success(), "child failed");
    let stdout = String::from_utf8(output.stdout).expect("utf8 child stdout");
    assert!(
        stdout.lines().any(|line| line.trim() == expected),
        "expected default_runtime_dir() to return {expected:?} despite TMPDIR=/tmp; stdout={stdout:?}"
    );
}
