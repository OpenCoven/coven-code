#![cfg(unix)]

use claurst_core::config::{HookEntry, HookEvent};
use claurst_core::hooks::{run_hooks, HookContext, HookOutcome};
use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MODE: &str = "COVEN_TEST_CORE_HOOK_MODE";
const WORKING_DIR: &str = "COVEN_TEST_CORE_HOOK_WORKING_DIR";
const PAYLOAD_SIZE: usize = 1024 * 1024;

fn run_case(mode: &str) {
    let working_dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "core_hook_runner_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env(MODE, mode)
        .env(WORKING_DIR, working_dir.path())
        .stdout(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(error) => break Err(error),
        }
        if Instant::now() >= deadline {
            break Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "hook deadline exceeded",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    match result {
        Ok(status) => assert!(status.success(), "core hook {mode} failed: {status}"),
        Err(error) => {
            // Kill the owned group, including the shell and pipeline, before failing.
            let killed = Command::new("/bin/kill")
                .args(["-KILL", &format!("-{}", child.id())])
                .status();
            let _ = child.kill();
            let reaped = child.wait();
            assert!(
                killed.is_ok_and(|status| status.success()),
                "fixture group cleanup failed"
            );
            assert!(reaped.is_ok(), "fixture reap failed");
            panic!("core hook {mode}: {error}");
        }
    }
}

#[test]
fn core_hook_stdout_before_input() {
    run_case("stdout");
}

#[test]
fn core_hook_stderr_before_input() {
    run_case("stderr");
}

#[test]
fn core_hook_exit_outcomes() {
    for mode in [
        "quiet",
        "deny_stderr",
        "deny_stdout",
        "nonblocking_output",
        "nonblocking_empty",
        "early",
    ] {
        run_case(mode);
    }
}

#[test]
#[ignore = "invoked by the bounded subprocess driver"]
fn core_hook_runner_fixture() {
    let mode = std::env::var(MODE).unwrap();
    let working_dir = std::path::PathBuf::from(std::env::var_os(WORKING_DIR).unwrap());
    let command = match mode.as_str() {
        "stdout" => r"head -c 1048576 /dev/zero | tr '\000' x; cat > input.json",
        "stderr" => r"head -c 1048576 /dev/zero | tr '\000' x >&2; cat > input.json",
        "quiet" => "cat > input.json",
        "deny_stderr" => "printf 'fallback output'; printf 'fixture denial' >&2; exit 7",
        "deny_stdout" => "printf 'fixture denial'; exit 7",
        "nonblocking_output" => "printf '  fixture modification  '; exit 7",
        "nonblocking_empty" => "exit 7",
        "early" => "exit 0",
        _ => panic!("unknown fixture mode"),
    };
    let hooks = HashMap::from([(
        HookEvent::PreToolUse,
        vec![HookEntry {
            command: command.into(),
            tool_filter: None,
            blocking: !mode.starts_with("nonblocking"),
        }],
    )]);
    let context = HookContext {
        event: "PreToolUse".into(),
        tool_name: Some("Read".into()),
        tool_input: Some(serde_json::json!({ "payload": "x".repeat(PAYLOAD_SIZE) })),
        tool_output: None,
        is_error: None,
        session_id: Some("fixture".into()),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = runtime.block_on(run_hooks(
        &hooks,
        HookEvent::PreToolUse,
        &context,
        &working_dir,
    ));
    match mode.as_str() {
        "stdout" => assert!(
            matches!(outcome, HookOutcome::Modified(ref text) if text == &"x".repeat(PAYLOAD_SIZE)),
            "{outcome:?}"
        ),
        "deny_stderr" | "deny_stdout" => assert!(
            matches!(outcome, HookOutcome::Blocked(ref reason) if reason.ends_with(": fixture denial")),
            "{outcome:?}"
        ),
        "nonblocking_output" => assert!(
            matches!(outcome, HookOutcome::Modified(ref text) if text == "fixture modification"),
            "{outcome:?}"
        ),
        _ => assert!(matches!(outcome, HookOutcome::Allowed), "{outcome:?}"),
    }
    if matches!(mode.as_str(), "stdout" | "stderr" | "quiet") {
        let received: serde_json::Value =
            serde_json::from_slice(&std::fs::read(working_dir.join("input.json")).unwrap())
                .unwrap();
        assert_eq!(received, serde_json::to_value(context).unwrap());
    }
}
