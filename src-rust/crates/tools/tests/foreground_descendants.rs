//! #373: the workload must remain a descendant of the ordinary Bash wrapper.
#![cfg(unix)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{BashTool, PtyBashTool, Tool, ToolContext};
use nix::sys::signal::kill;
use nix::unistd::Pid;
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Duration;

struct Fixture {
    directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // The owned Python worker also expires after ten seconds, even if a
        // failed fixture loses its release file during temporary-dir cleanup.
        let _ = std::fs::write(self.directory.path().join("release"), "stop");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn context(directory: &std::path::Path) -> ToolContext {
    ToolContext {
        working_dir: directory.to_path_buf(),
        permission_mode: PermissionMode::BypassPermissions,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::BypassPermissions,
        }),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: uuid::Uuid::new_v4().to_string(),
        goal_turn: Default::default(),
        file_history: Default::default(),
        current_turn: Arc::new(AtomicUsize::new(0)),
        non_interactive: true,
        mcp_manager: None,
        config: Config::default(),
        managed_agent_config: None,
        completion_notifier: None,
        pending_permissions: None,
        permission_manager: None,
        user_question_tx: None,
    }
}

async fn check_descendant(cancel: bool, closed_output: bool, pty: bool) {
    let fixture = Fixture {
        directory: tempfile::tempdir().unwrap(),
    };
    let directory = fixture.directory.path();
    std::fs::write(directory.join("worker.py"), r#"import json, os, time
from pathlib import Path
Path('started.tmp').write_text(json.dumps({'pid': os.getpid(), 'parent': os.getppid(), 'group': os.getpgrp()}))
Path('started.tmp').replace('started')
start = time.monotonic()
while time.monotonic() - start < 10:
    if Path('release').exists(): break
    if time.monotonic() - start >= 3: Path('delayed-marker').write_text('descendant survived')
    time.sleep(0.01)
"#).unwrap();
    let ctx = context(directory);
    let mut control = tokio::process::Command::new("python3")
        .args(["-c", "import time; time.sleep(10)"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let command = if closed_output {
        // This exec only closes the wrapper's descriptors. It never replaces
        // the shell with the workload; the recorded parent PID proves that.
        r#"printf '%s' "$$" > wrapper-pid; exec 1>&- 2>&-; python3 worker.py; :"#
    } else {
        r#"printf '%s' "$$" > wrapper-pid; python3 worker.py; :"#
    };
    let call = tokio::spawn(async move {
        let tool: Box<dyn Tool> = if pty {
            Box::new(PtyBashTool)
        } else {
            Box::new(BashTool)
        };
        tool.execute(
            serde_json::json!({
                "command": command, "timeout": 1500,
            }),
            &ctx,
        )
        .await
    });
    let started: serde_json::Value = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(directory.join("started")) {
                break serde_json::from_str(&text).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned non-exec descendant starts");
    let worker = i32::try_from(started["pid"].as_i64().unwrap()).unwrap();
    let wrapper: i64 = std::fs::read_to_string(directory.join("wrapper-pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        started["parent"].as_i64(),
        Some(wrapper),
        "workload did not remain under the Bash wrapper"
    );
    let isolated = started["group"].as_i64() != Some(i64::from(nix::unistd::getpgrp().as_raw()));
    assert!(
        !call.is_finished(),
        "output closure must still wait for the owned workload"
    );
    if cancel {
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
    } else {
        let result = call.await.unwrap();
        assert!(
            result.is_error && result.content.contains("timed out"),
            "{}",
            result.content
        );
    }
    // The marker is an observable forbidden side effect, not merely a PID
    // probe. Keep the original worker deadline; do not replace it with exec.
    tokio::time::sleep(Duration::from_millis(3200)).await;
    let control_alive = control.try_wait().unwrap().is_none();
    if control_alive {
        control.kill().await.unwrap();
    }
    let marker = directory.join("delayed-marker").exists();
    let alive = kill(Pid::from_raw(worker), None).is_ok();
    std::fs::write(directory.join("release"), "stop").unwrap();
    let cleanup = tokio::time::timeout(Duration::from_secs(2), async {
        while kill(Pid::from_raw(worker), None).is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        cleanup.is_ok(),
        "owned fixture worker did not acknowledge release"
    );
    assert!(
        control_alive,
        "foreground cleanup signaled an unrelated caller-group process"
    );
    assert!(!marker && !alive && isolated, "non-exec descendant {worker} survived or shared the caller group: delayed_marker={marker}, alive={alive}, isolated={isolated}, cancel={cancel}, parent={}, group={}", started["parent"], started["group"]);
}

#[tokio::test]
async fn cancelled_foreground_wrapper_owns_non_exec_descendant() {
    check_descendant(true, false, false).await;
}

#[tokio::test]
async fn timed_out_foreground_wrapper_owns_non_exec_descendant() {
    check_descendant(false, false, false).await;
}

#[tokio::test]
async fn cancelled_wrapper_after_output_closure_owns_descendant() {
    check_descendant(true, true, false).await;
}
#[tokio::test]
async fn timed_out_wrapper_after_output_closure_owns_descendant() {
    check_descendant(false, true, false).await;
}
#[tokio::test]
async fn cancelled_pty_wrapper_owns_non_exec_descendant() {
    check_descendant(true, false, true).await;
}
#[tokio::test]
async fn timed_out_pty_wrapper_owns_non_exec_descendant() {
    check_descendant(false, false, true).await;
}
#[tokio::test]
async fn cancelled_pty_wrapper_after_output_closure_owns_descendant() {
    check_descendant(true, true, true).await;
}
#[tokio::test]
async fn timed_out_pty_wrapper_after_output_closure_owns_descendant() {
    check_descendant(false, true, true).await;
}

#[tokio::test]
async fn ordinary_completion_preserves_shell_cwd_and_exports() {
    for pty in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("sub")).unwrap();
        let ctx = context(directory.path());
        let tool: Box<dyn Tool> = if pty {
            Box::new(PtyBashTool)
        } else {
            Box::new(BashTool)
        };
        let first = tool.execute(serde_json::json!({"command":"cd sub; export OWNED_FOREGROUND_STATE='retained'; printf 'first-output'", "timeout":5000}), &ctx).await;
        assert!(
            !first.is_error && first.content.contains("first-output"),
            "{}",
            first.content
        );
        let second = tool.execute(serde_json::json!({"command":r#"pwd; printf '%s' "$OWNED_FOREGROUND_STATE""#, "timeout":5000}), &ctx).await;
        assert!(!second.is_error, "{}", second.content);
        assert!(
            second.content.contains("/sub") && second.content.contains("retained"),
            "{}",
            second.content
        );
    }
}
