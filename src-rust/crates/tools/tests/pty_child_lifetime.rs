//! A PTY timeout or cancelled call must terminate its command process.
#![cfg(unix)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{PtyBashTool, Tool, ToolContext};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Duration;

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
        file_history: Arc::new(parking_lot::Mutex::new(
            claurst_core::file_history::FileHistory::new(),
        )),
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

async fn check_lifetime(cancel: bool) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("wait.py"),
        "import os, time\nfrom pathlib import Path\nPath('started.tmp').write_text(str(os.getpid()))\nPath('started.tmp').replace('started')\nwhile True: time.sleep(0.1)\n",
    )
    .unwrap();
    let ctx = context(directory.path());
    let call = tokio::spawn(async move {
        PtyBashTool
            .execute(
                serde_json::json!({"command": "exec python3 wait.py", "timeout": 1000}),
                &ctx,
            )
            .await
    });
    let pid: i32 = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(marker) = std::fs::read_to_string(directory.path().join("started")) {
                break marker.parse().unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("PTY child starts before timeout");
    if cancel {
        call.abort();
        let _ = call.await;
    } else {
        let result = call.await.unwrap();
        assert!(result.is_error, "{}", result.content);
        assert!(result.content.contains("timed out"), "{}", result.content);
    }

    let exited = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
                Err(nix::errno::Errno::ESRCH) => break true,
                Ok(()) => tokio::time::sleep(Duration::from_millis(10)).await,
                other => panic!("PTY child liveness probe: {other:?}"),
            }
        }
    })
    .await
    .unwrap_or(false);
    if !exited {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            Some(nix::sys::signal::Signal::SIGKILL),
        );
    }
    assert!(exited, "PTY child {pid} survived timeout or cancellation");
}

#[tokio::test]
async fn timed_out_pty_child_exits() {
    check_lifetime(false).await;
}

#[tokio::test]
async fn cancelled_pty_child_exits() {
    check_lifetime(true).await;
}
