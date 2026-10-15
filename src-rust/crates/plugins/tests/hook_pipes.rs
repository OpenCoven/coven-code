#![cfg(unix)]

use claurst_plugins::hooks::{run_hook_sync, HookOutcome, HookScope, RegisteredHook};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MODE: &str = "COVEN_TEST_PLUGIN_PIPE_MODE";

// Each case runs in an owned process group so a broken runner cannot hang the suite.
#[test]
fn hook_pipes_and_exit_outcomes() {
    for mode in ["stdout", "stderr", "deny", "nonblocking", "early", "quiet"] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "hook_runner_fixture", "--nocapture"])
            .env(MODE, mode)
            .stdout(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let killed = Command::new("/bin/kill")
                    .args(["-KILL", &format!("-{}", child.id())])
                    .status()
                    .unwrap();
                let _ = child.kill();
                child.wait().unwrap();
                assert!(killed.success(), "failed to terminate fixture group");
                panic!("hook fixture {mode} exceeded its deadline");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success(), "hook fixture {mode} failed: {status}");
    }
}

#[test]
fn hook_runner_fixture() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let binary = std::env::current_exe().unwrap();
    let quoted = binary.to_str().unwrap().replace('\'', "'\\''");
    let hook = RegisteredHook {
        command: format!("'{quoted}' --exact hook_output_fixture --nocapture"),
        matcher: None,
        blocking: mode != "nonblocking",
        plugin_root: ".".into(),
        plugin_name: "pipe-fixture".into(),
        plugin_source: "test".into(),
        plugin_capabilities: Some(vec!["shell".into()]),
        scope: HookScope::Global,
    };
    let outcome = run_hook_sync(
        &hook,
        &"x".repeat(1024 * 1024),
        &std::env::current_dir().unwrap(),
    );
    if mode == "deny" {
        assert!(
            matches!(outcome, HookOutcome::Deny(ref message) if message.contains("fixture denial")),
            "{outcome:?}"
        );
    } else {
        assert!(matches!(outcome, HookOutcome::Allow), "{outcome:?}");
    }
}

#[test]
fn hook_output_fixture() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    match mode.as_str() {
        "stdout" => {
            std::io::stdout()
                .write_all(&vec![b'x'; 1024 * 1024])
                .unwrap();
            std::io::stdout().flush().unwrap();
        }
        "stderr" => {
            std::io::stderr()
                .write_all(&vec![b'x'; 1024 * 1024])
                .unwrap();
            std::io::stderr().flush().unwrap();
        }
        "deny" | "nonblocking" => {
            eprintln!("fixture denial");
            std::process::exit(7);
        }
        "early" => return,
        _ => {}
    }
    let mut payload = Vec::new();
    std::io::stdin().read_to_end(&mut payload).unwrap();
    assert_eq!(payload, vec![b'x'; 1024 * 1024]);
}
