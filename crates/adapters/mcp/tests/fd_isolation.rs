//! Harness-free subprocess test proving `IsolatedStdio::isolate` keeps protocol stdout 100% free
//! of any pre- or post-init fd 1 chatter.
#![expect(
    clippy::expect_used,
    reason = "integration test panics on unexpected setup or I/O failure"
)]

mod support;

use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};

use quickie_mcp::{IsolatedStdio, serve_stdio};
use serde_json::Value;

use crate::support::{
    EchoHandler, initialize_frame, initialized_notification_frame, tools_call_echo_frame,
    tools_list_frame,
};

const ISOLATION_CHILD_ENV: &str = "QUICKIE_MCP_TEST_FD_ISOLATION_CHILD";

fn run_child() {
    // Isolate stdout BEFORE any write to fd 1.
    let isolated = IsolatedStdio::isolate().expect("isolate stdio");
    #[expect(
        clippy::print_stdout,
        reason = "intentionally writes to fd 1 to prove IsolatedStdio redirects it to stderr"
    )]
    {
        println!("native-toolkit-chatter-on-fd-1");
    }
    std::io::stdout().flush().expect("flush redirected stdout");

    let handle = serve_stdio(isolated, EchoHandler::new(55)).expect("serve isolated stdio");
    handle.wait();
}

fn main() {
    if std::env::var_os(ISOLATION_CHILD_ENV).is_some() {
        run_child();
        return;
    }

    // Arrange
    let exe = std::env::current_exe().expect("current test binary path");
    let mut child = Command::new(exe)
        .env(ISOLATION_CHILD_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn harness-free fd-isolation child");

    let mut child_in = child.stdin.take().expect("child stdin");
    let child_out = child.stdout.take().expect("child stdout");
    let mut child_err = child.stderr.take().expect("child stderr");

    // Act
    child_in
        .write_all(initialize_frame(1).as_bytes())
        .expect("write initialize");
    child_in
        .write_all(initialized_notification_frame().as_bytes())
        .expect("write initialized");
    child_in
        .write_all(tools_list_frame(2).as_bytes())
        .expect("write tools/list");
    child_in
        .write_all(tools_call_echo_frame(3, "clean-stdout").as_bytes())
        .expect("write tools/call");
    child_in.flush().expect("flush child stdin");
    drop(child_in);

    let mut stdout_bytes = String::new();
    BufReader::new(child_out)
        .read_to_string(&mut stdout_bytes)
        .expect("read child stdout");
    let mut stderr_bytes = String::new();
    child_err
        .read_to_string(&mut stderr_bytes)
        .expect("read child stderr");
    let status = child.wait().expect("wait for child");

    // Assert
    assert!(status.success(), "child failed: stderr={stderr_bytes}");
    assert!(
        stderr_bytes.contains("native-toolkit-chatter-on-fd-1"),
        "fd 1 chatter must be redirected to stderr; stderr={stderr_bytes:?}"
    );
    assert!(
        !stdout_bytes.contains("native-toolkit-chatter-on-fd-1"),
        "protocol stdout must never contain fd 1 chatter; stdout={stdout_bytes:?}"
    );
    assert!(
        stdout_bytes.ends_with('\n'),
        "protocol stdout must be newline-terminated NDJSON; stdout={stdout_bytes:?}"
    );

    // Every single line on stdout — with zero filtering — must parse as a valid JSON-RPC frame.
    let frames: Vec<Value> = stdout_bytes
        .lines()
        .map(|line| {
            serde_json::from_str(line).expect("every stdout line must be pure JSON-RPC frame")
        })
        .collect();
    assert_eq!(
        frames.len(),
        3,
        "expected initialize + tools/list + tools/call responses"
    );
    assert_eq!(frames[0]["id"], 1);
    assert_eq!(frames[1]["id"], 2);
    assert_eq!(frames[1]["result"]["tools"][0]["name"], "echo");
    assert_eq!(frames[2]["id"], 3);
    assert_eq!(
        frames[2]["result"]["content"][0]["text"],
        "session=55 call=1 text=clean-stdout"
    );
}
