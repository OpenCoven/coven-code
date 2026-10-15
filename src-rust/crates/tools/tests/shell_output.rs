//! Foreground shell truncation through the public tool contract.
#![cfg(unix)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{BashTool, PowerShellTool, PtyBashTool, Tool, ToolContext};
use std::sync::{atomic::AtomicUsize, Arc};

#[tokio::test]
async fn bash_unicode_output() {
    check_tool(BashTool, false).await;
}

#[tokio::test]
async fn pty_unicode_output() {
    check_tool(PtyBashTool, false).await;
}

#[tokio::test]
async fn powershell_unicode_output() {
    match std::process::Command::new("pwsh").arg("-Version").output() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping PowerShell runtime fixture: pwsh is not installed");
            return;
        }
        result => assert!(result.unwrap().status.success()),
    }
    check_tool(PowerShellTool, true).await;
}

async fn check_tool(tool: impl Tool, powershell: bool) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        working_dir: directory.path().to_path_buf(),
        permission_mode: PermissionMode::BypassPermissions,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::BypassPermissions,
        }),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: uuid::Uuid::new_v4().to_string(),
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
    // Native PowerShell invocations can exceed five seconds on a busy host.
    let timeout = if powershell { 30_000 } else { 5_000 };
    let prefix = "a".repeat(49_999);
    let suffix = "b".repeat(60_000);
    std::fs::write(
        directory.path().join("output.txt"),
        format!("{prefix}\u{e9}{suffix}\n"),
    )
    .unwrap();
    let command = if powershell {
        "Get-Content -Encoding utf8 output.txt"
    } else {
        "cat output.txt"
    };
    let result = tool
        .execute(
            serde_json::json!({"command": command, "timeout": timeout}),
            &ctx,
        )
        .await;
    claurst_tools::clear_session_shell_state(&ctx.session_id);
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.starts_with(&prefix));
    assert!(result.content.ends_with(&"b".repeat(50_000)));
    let marker = &result.content[prefix.len()..result.content.len() - 50_000];
    // PTY decoding is independently lossy across read boundaries; only discarded
    // bytes may differ here. Exact byte counts are covered by pure helper tests.
    let count = marker
        .strip_prefix("\n\n... (")
        .unwrap()
        .strip_suffix(" bytes truncated) ...\n\n")
        .unwrap();
    assert!(count.parse::<usize>().unwrap() >= 10_002);

    let controls = if powershell {
        [
            ("Write-Output 'short'", "short", false),
            ("$null", "(no output)", false),
            (
                "Write-Output 'short'; exit 7",
                "PowerShell exited with code 7\nshort",
                true,
            ),
        ]
    } else {
        [
            ("printf 'short\\n'", "short", false),
            ("true", "(no output)", false),
            (
                "printf 'short\\n'; false",
                "Command exited with code 1\nshort",
                true,
            ),
        ]
    };
    for (command, expected, is_error) in controls {
        let result = tool
            .execute(
                serde_json::json!({"command": command, "timeout": timeout}),
                &ctx,
            )
            .await;
        assert_eq!(result.is_error, is_error, "{command}: {}", result.content);
        assert_eq!(result.content, expected);
    }
    claurst_tools::clear_session_shell_state(&ctx.session_id);
}
