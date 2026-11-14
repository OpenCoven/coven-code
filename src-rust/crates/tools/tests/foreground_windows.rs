//! #373 on Windows: both foreground Bash tools run `cmd /C` with no owner for
//! the process tree yet. A cancelled call must at least stop the `cmd.exe` it
//! started, as a timed-out call already does.
#![cfg(windows)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{BashTool, PtyBashTool, Tool, ToolContext};
use std::path::Path;
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Duration;

/// A loop that only `cmd.exe` runs: each pass appends a line, then waits about
/// a second in `ping`. The file stops growing once that `cmd.exe` is gone. The
/// echoed word has no digits, since `echo 1>>file` redirects handle 1.
const TICKER: &str = "for /L %i in (1,1,60) do @(echo tick>>ticks.txt & ping -n 2 127.0.0.1 >nul)";

fn context(directory: &Path) -> ToolContext {
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

fn tool(pty: bool) -> Box<dyn Tool> {
    if pty {
        Box::new(PtyBashTool)
    } else {
        Box::new(BashTool)
    }
}

/// Lines in the ticker's file. A read that collides with `cmd.exe` appending
/// is retried rather than counted as empty.
fn ticks(directory: &Path) -> usize {
    let path = directory.join("ticks.txt");
    for _ in 0..40 {
        match std::fs::read_to_string(&path) {
            Ok(text) => return text.lines().count(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return 0,
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    panic!("could not read {}", path.display());
}

async fn wait_for_ticks(directory: &Path, count: usize) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while ticks(directory) < count {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the command started its loop");
}

/// The tick count 3 s after the call ended, when a pass already under way has
/// landed, and again 3 s later, when a live loop would have added about three.
async fn ticks_after_stop(directory: &Path) -> (usize, usize) {
    tokio::time::sleep(Duration::from_secs(3)).await;
    let settled = ticks(directory);
    tokio::time::sleep(Duration::from_secs(3)).await;
    (settled, ticks(directory))
}

async fn check_cancel(pty: bool) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let call = tokio::spawn(async move {
        tool(pty)
            .execute(
                serde_json::json!({ "command": TICKER, "timeout": 120000 }),
                &ctx,
            )
            .await
    });
    wait_for_ticks(directory.path(), 2).await;
    assert!(!call.is_finished(), "the loop ended on its own");
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    let (settled, later) = ticks_after_stop(directory.path()).await;
    assert_eq!(
        settled, later,
        "cmd.exe kept running after the call was cancelled (pty={pty})"
    );
}

async fn check_timeout(pty: bool) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let result = tool(pty)
        .execute(
            serde_json::json!({ "command": TICKER, "timeout": 2500 }),
            &ctx,
        )
        .await;
    assert!(
        result.is_error && result.content.contains("timed out"),
        "{}",
        result.content
    );
    assert!(ticks(directory.path()) >= 1, "the loop never started");
    let (settled, later) = ticks_after_stop(directory.path()).await;
    assert_eq!(
        settled, later,
        "cmd.exe kept running after the call timed out (pty={pty})"
    );
}

async fn check_completion(pty: bool) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let result = tool(pty)
        .execute(
            serde_json::json!({ "command": "echo finished& echo warned 1>&2" }),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.contains("finished") && result.content.contains("warned"),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn cancelled_pty_fallback_stops_its_cmd() {
    check_cancel(true).await;
}

#[tokio::test]
async fn cancelled_bash_fallback_stops_its_cmd() {
    check_cancel(false).await;
}

#[tokio::test]
async fn timed_out_pty_fallback_stops_its_cmd() {
    check_timeout(true).await;
}

#[tokio::test]
async fn timed_out_bash_fallback_stops_its_cmd() {
    check_timeout(false).await;
}

#[tokio::test]
async fn pty_fallback_returns_output_of_a_finished_command() {
    check_completion(true).await;
}

#[tokio::test]
async fn bash_fallback_returns_output_of_a_finished_command() {
    check_completion(false).await;
}
