//! A cancelled foreground tool call must not leave its shell child running.
#![cfg(unix)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{BashTool, Tool, ToolContext};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Duration;

#[tokio::test]
async fn cancelled_foreground_bash_kills_child() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("wait.py"),
        "import os, time\nfrom pathlib import Path\nPath('started.tmp').write_text(str(os.getpid()))\nPath('started.tmp').replace('started')\nwhile True: time.sleep(0.1)\n",
    )
    .unwrap();
    let ctx = ToolContext {
        working_dir: directory.path().to_path_buf(),
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
    };
    let call = tokio::spawn(async move {
        BashTool
            .execute(
                serde_json::json!({"command": "exec python3 wait.py", "timeout": 10000}),
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
    .expect("child starts before cancellation");
    call.abort();
    let _ = call.await;

    let exited = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
                Err(nix::errno::Errno::ESRCH) => break true,
                Ok(()) => tokio::time::sleep(Duration::from_millis(10)).await,
                other => panic!("child liveness probe: {other:?}"),
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
    assert!(
        exited,
        "cancelled foreground command left child {pid} running"
    );
}
