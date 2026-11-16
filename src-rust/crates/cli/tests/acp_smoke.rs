//! End-to-end smoke test: spawn the `coven-code` binary in ACP mode, send a
//! short JSON-RPC conversation over its stdin, and verify the responses on
//! stdout match what the Agent Client Protocol spec mandates.
//!
//! This guards the wire-format and capability surface that registry-listed
//! ACP clients (Zed, Neovim, JetBrains, …) rely on. Runs against the
//! debug binary produced by `cargo build` — Cargo provides the path via
//! `CARGO_BIN_EXE_coven-code` (internal Cargo env for the binary target).

use std::io::Read;
use std::process::{Child, Command, ExitStatus};
use std::time::Duration;

fn binary_path() -> String {
    env!("CARGO_BIN_EXE_coven-code").to_string()
}

fn captured_output(path: &std::path::Path) -> String {
    let file = std::fs::File::open(path).expect("open captured output");
    let mut bytes = Vec::new();
    file.take(16 * 1024)
        .read_to_end(&mut bytes)
        .expect("read captured output");
    String::from_utf8_lossy(&bytes).into_owned()
}

// Own the child across polling errors as well as the ordinary timeout path.
struct AcpChild(Child);

impl Drop for AcpChild {
    fn drop(&mut self) {
        // Successful polling and explicit timeout cleanup already reap the
        // child. Errors or any earlier unwind still retain this owner.
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn poll_child(
    child: &mut AcpChild,
    poll: impl FnOnce(&mut Child) -> std::io::Result<Option<ExitStatus>>,
) -> Option<ExitStatus> {
    poll(&mut child.0).expect("try_wait")
}

fn run_with_input(
    stdin: &str,
    timeout: Duration,
    settings: Option<&str>,
) -> (ExitStatus, String, String) {
    let scratch = tempfile::tempdir().expect("create ACP scratch home");
    let stdin_path = scratch.path().join("stdin.jsonl");
    std::fs::write(&stdin_path, stdin).expect("write ACP input");
    let stdout_path = scratch.path().join("stdout.log");
    let stderr_path = scratch.path().join("stderr.log");
    if let Some(settings) = settings {
        std::fs::write(scratch.path().join("settings.json"), settings)
            .expect("write ACP test settings");
    }
    let mut command = Command::new(binary_path());
    command
        .arg("acp")
        .env("HOME", scratch.path())
        .env("USERPROFILE", scratch.path())
        .env("COVEN_CODE_HOME", scratch.path())
        .env("COVEN_HOME", scratch.path().join("coven"))
        // ACP never reads the interactive renderer selector; an unsupported
        // value must leave every ACP smoke result unchanged.
        .env("COVEN_CODE_UI", "not-a-renderer")
        // File-backed input reaches EOF without blocking the timeout supervisor.
        .stdin(std::fs::File::open(&stdin_path).expect("open ACP input"))
        // Drain into owned files so waiting for exit cannot fill an output pipe.
        .stdout(std::fs::File::create(&stdout_path).expect("create stdout log"))
        .stderr(std::fs::File::create(&stderr_path).expect("create stderr log"));
    for key in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "COVEN_CODE_PROVIDER",
        "COVEN_CODE_API_BASE",
        "COVEN_CODE_CLAUDE_BIN",
        "COVEN_PARENT",
    ] {
        command.env_remove(key);
    }
    let mut child = AcpChild(command.spawn().expect("spawn coven-code acp"));

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match poll_child(&mut child, Child::try_wait) {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.0.kill();
                let reaped = child.0.wait();
                let stdout = captured_output(&stdout_path);
                let stderr = captured_output(&stderr_path);
                panic!("coven-code acp did not exit within {timeout:?}; reap: {reaped:?}\nstdout (first 16 KiB):\n{stdout}\nstderr (first 16 KiB):\n{stderr}");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    (
        status,
        String::from_utf8_lossy(&std::fs::read(stdout_path).expect("read stdout log")).into_owned(),
        String::from_utf8_lossy(&std::fs::read(stderr_path).expect("read stderr log")).into_owned(),
    )
}

fn find_response(stdout: &str, id: i64) -> serde_json::Value {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if v.get("id") == Some(&serde_json::json!(id)) {
                return v;
            }
        }
    }
    panic!("no response for id={id} in stdout:\n{stdout}");
}

#[test]
fn initialize_returns_spec_compliant_response() {
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":false},"clientInfo":{"name":"smoke","version":"0.0.0"}}}
"#;
    let (_status, stdout, stderr) = run_with_input(request, Duration::from_secs(20), None);
    assert!(
        !stderr.contains("COVEN_CODE_UI"),
        "ACP must not read the renderer selector:\n{stderr}"
    );
    let resp = find_response(&stdout, 1);
    assert_eq!(resp["jsonrpc"], "2.0");
    let result = &resp["result"];
    assert_eq!(result["protocolVersion"], 1);
    // Agent identifies itself.
    assert_eq!(result["agentInfo"]["name"], "coven-code");
    assert!(result["agentInfo"]["version"].is_string());
    // authMethods MUST be an array (even if empty).
    assert!(result["authMethods"].is_array());
    // Standard capability blocks present.
    let caps = &result["agentCapabilities"];
    assert!(caps["promptCapabilities"].is_object());
    assert!(caps["mcpCapabilities"].is_object());
    assert!(caps["loadSession"].is_boolean());
}

#[test]
fn session_new_returns_session_id() {
    // Use a path that's absolute on both Unix and Windows.
    let cwd = std::env::current_dir().expect("cwd");
    let cwd_str = cwd.to_string_lossy().replace('\\', "/");
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}},"clientInfo":{{"name":"smoke","version":"0.0.0"}}}}}}
{{"jsonrpc":"2.0","id":2,"method":"session/new","params":{{"cwd":"{cwd_str}","mcpServers":[]}}}}
"#
    );
    let (_status, stdout, _stderr) = run_with_input(&request, Duration::from_secs(20), None);
    let resp = find_response(&stdout, 2);
    let session_id = resp["result"]["sessionId"]
        .as_str()
        .expect("sessionId should be a string");
    assert!(
        session_id.starts_with("acp-"),
        "sessionId not prefixed: {session_id}"
    );
}

#[test]
fn session_load_returns_method_not_found() {
    // We do not advertise loadSession, so well-behaved clients should never
    // call session/load — but the agent must answer correctly if they do.
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"smoke","version":"0.0.0"}}}
{"jsonrpc":"2.0","id":2,"method":"session/load","params":{"sessionId":"x","cwd":"/","mcpServers":[]}}
"#;
    let (_status, stdout, _stderr) = run_with_input(request, Duration::from_secs(20), None);
    let resp = find_response(&stdout, 2);
    assert_eq!(resp["error"]["code"], -32601, "should be MethodNotFound");
}

#[test]
fn cancel_notification_is_silent() {
    // session/cancel is a notification — the agent MUST NOT respond, even
    // for an unknown session.
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"smoke","version":"0.0.0"}}}
{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"nonexistent"}}
"#;
    let (_status, stdout, _stderr) = run_with_input(request, Duration::from_secs(20), None);
    // Only the initialize response (id=1) should appear; no extra responses.
    let response_count = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| serde_json::from_str::<serde_json::Value>(l).is_ok())
        .count();
    assert_eq!(
        response_count, 1,
        "unexpected extra responses in:\n{stdout}"
    );
}

#[test]
fn invalid_settings_fail_acp_startup() {
    let (status, stdout, stderr) =
        run_with_input("", Duration::from_secs(20), Some("{invalid settings"));
    assert!(!status.success(), "ACP ignored invalid settings: {stderr}");
    assert!(stderr.contains("Failed to load ACP settings"), "{stderr}");
    assert!(stdout.is_empty(), "ACP wrote a response: {stdout}");
}

#[test]
fn large_response_stream_does_not_block_child_exit() {
    // Keep ordinary traffic below admission capacity while producing more
    // output than the former 4,096 tiny replies. Overload has separate bounded
    // rejection/cancellation tests; this still proves large output drains.
    let ids: Vec<String> = (0..16)
        .map(|i| format!("{i}-{}", "x".repeat(32 * 1024)))
        .collect();
    let input: String = ids
        .iter()
        .map(|id| {
            format!(
                "{}\n",
                serde_json::json!({
                    "jsonrpc":"2.0", "id":id, "method":"session/load"
                })
            )
        })
        .collect();
    let (status, stdout, stderr) = run_with_input(&input, Duration::from_secs(20), None);
    assert!(status.success(), "ACP failed: {stderr}");
    assert!(
        stdout.len() > 512 * 1024,
        "fixture must exceed an output pipe's capacity"
    );
    let responses: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON-RPC response"))
        .collect();
    assert_eq!(responses.len(), ids.len());
    let mut actual: Vec<&str> = responses
        .iter()
        .map(|response| {
            assert_eq!(response["error"]["code"], -32601);
            response["id"].as_str().expect("original string request ID")
        })
        .collect();
    actual.sort_unstable();
    let mut expected: Vec<&str> = ids.iter().map(String::as_str).collect();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[cfg(unix)]
#[test]
fn saturated_real_stdio_pipes_fail_closed_without_runtime_shutdown_hanging() {
    use std::io::Write;
    use std::process::Stdio;
    let scratch = tempfile::tempdir().expect("owned saturation home");
    let mut child = AcpChild(
        Command::new(binary_path())
            .arg("acp")
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", scratch.path())
            .env("COVEN_CODE_HOME", scratch.path().join("code"))
            .env("COVEN_HOME", scratch.path().join("coven"))
            .current_dir(scratch.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn saturated ACP"),
    );
    let mut input = child.0.stdin.take().expect("owned input pipe");
    let writer = std::thread::spawn(move || {
        // Large correlated IDs fill stdout before handlers can release their
        // permits. Keep stdin open and writing until the connection retires.
        for number in 0..256 {
            let row = serde_json::json!({
                "jsonrpc": "2.0", "id": format!("{number}-{}", "x".repeat(32 * 1024)),
                "method": "session/load"
            });
            if writeln!(input, "{row}")
                .and_then(|_| input.flush())
                .is_err()
            {
                break;
            }
        }
    });
    // Deliberately do not read stdout. A timeout on an async future alone is
    // insufficient if generic Tokio stdio left a blocking kernel I/O task alive.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let observed = loop {
        if let Some(status) = poll_child(&mut child, Child::try_wait) {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            child.0.kill().expect("retire timed-out owned process");
            child.0.wait().expect("reap timed-out owned process");
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    writer.join().expect("owned input writer reaped");
    assert!(
        observed.is_some_and(|status| !status.success()),
        "saturated ACP must retire with an error before the fixture deadline"
    );
}

#[cfg(unix)]
#[test]
fn polling_error_reaps_owned_acp_child_before_unwinding_returns() {
    use nix::errno::Errno;
    use nix::sys::signal::{kill, Signal};
    use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
    use nix::unistd::Pid;

    let child = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "owned_acp_child_worker", "--ignored"])
        .env("ACP_REAP_FIXTURE_WORKER", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn bounded owned worker");
    let pid = Pid::from_raw(i32::try_from(child.id()).expect("child pid"));
    assert_eq!(
        waitpid(pid, Some(WaitPidFlag::WNOHANG)),
        Ok(WaitStatus::StillAlive)
    );
    let panic = std::panic::catch_unwind(move || {
        let mut child = AcpChild(child);
        // Force the exact polling-error branch used by the smoke supervisor.
        poll_child(&mut child, |_| {
            Err(std::io::Error::other("synthetic poll failure"))
        });
    });
    let after = waitpid(pid, Some(WaitPidFlag::WNOHANG));
    // Retain ownership even on the deliberately failing red implementation.
    if after == Ok(WaitStatus::StillAlive) {
        let _ = kill(pid, Signal::SIGKILL);
        let _ = waitpid(pid, None);
    }
    assert!(panic.is_err(), "polling error was not injected");
    assert_eq!(
        after,
        Err(Errno::ECHILD),
        "owned worker was not reaped before unwinding completed"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "bounded child invoked by owned polling-error regression"]
fn owned_acp_child_worker() {
    if std::env::var("ACP_REAP_FIXTURE_WORKER").as_deref() == Ok("1") {
        std::thread::sleep(Duration::from_secs(10));
    }
}
