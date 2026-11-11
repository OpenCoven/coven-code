//! End-to-end smoke test: spawn the `coven-code` binary in ACP mode, send a
//! short JSON-RPC conversation over its stdin, and verify the responses on
//! stdout match what the Agent Client Protocol spec mandates.
//!
//! This guards the wire-format and capability surface that registry-listed
//! ACP clients (Zed, Neovim, JetBrains, …) rely on. Runs against the
//! debug binary produced by `cargo build` — Cargo provides the path via
//! `CARGO_BIN_EXE_coven-code` (internal Cargo env for the binary target).

use std::io::Read;
use std::process::{Command, ExitStatus};
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
    let mut child = command.spawn().expect("spawn coven-code acp");

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let reaped = child.wait();
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
    let request = "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"session/load\"}\n";
    let (status, stdout, stderr) =
        run_with_input(&request.repeat(4096), Duration::from_secs(20), None);
    assert!(status.success(), "ACP failed: {stderr}");
    let responses: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON-RPC response"))
        .collect();
    assert_eq!(responses.len(), 4096);
    for response in responses {
        assert_eq!(response["id"], 7);
        assert_eq!(response["error"]["code"], -32601);
    }
}
