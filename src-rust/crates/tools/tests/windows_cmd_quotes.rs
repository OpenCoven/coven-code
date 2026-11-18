//! #517 on Windows: the shell tools hand a command line to `cmd.exe`. It must
//! get the command's double quotes exactly as written, in a foreground call,
//! a background task and a worktree post-create command.
#![cfg(windows)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_core::tasks::{global_registry, TaskStatus};
use claurst_tools::{BashTool, EnterWorktreeTool, PtyBashTool, Tool, ToolContext};
use std::path::Path;
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Duration;

/// Two commands, each with a quoted argument. `echo` prints its argument with
/// the quotes, so the output shows exactly what `cmd.exe` was given.
const QUOTED: &str = r#"echo "a b"&echo "c""#;

/// What [`QUOTED`] prints, one entry per line.
const QUOTED_LINES: [&str; 2] = [r#""a b""#, r#""c""#];

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

/// A directory whose `with space` folder holds `note.txt` and `show.cmd`, a
/// batch file that prints its first argument, without its quotes, in brackets.
fn workspace() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("with space");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("note.txt"), "from a quoted path\r\n").unwrap();
    std::fs::write(folder.join("show.cmd"), "@echo [%~1]\r\n").unwrap();
    directory
}

/// Run `command` in the foreground and return what it printed.
async fn output_of(tool: &dyn Tool, command: &str, directory: &Path) -> String {
    let result = tool
        .execute(
            serde_json::json!({ "command": command }),
            &context(directory),
        )
        .await;
    assert!(!result.is_error, "{command}: {}", result.content);
    result.content
}

/// A quoted path as an argument.
async fn check_quoted_path(tool: &dyn Tool) {
    let directory = workspace();
    let note = directory.path().join("with space").join("note.txt");
    assert_eq!(
        output_of(
            tool,
            &format!(r#"type "{}""#, note.display()),
            directory.path()
        )
        .await,
        "from a quoted path"
    );
}

/// A quoted program path first and a quoted argument last: the line starts
/// and ends with a quote that belongs to the command.
async fn check_quoted_program(tool: &dyn Tool) {
    let directory = workspace();
    let script = directory.path().join("with space").join("show.cmd");
    assert_eq!(
        output_of(
            tool,
            &format!(r#""{}" "x y""#, script.display()),
            directory.path()
        )
        .await,
        "[x y]"
    );
}

/// Run [`QUOTED`] as a background task of `tool` and return its output lines.
async fn background_output_of(tool: &dyn Tool) -> Vec<String> {
    let directory = tempfile::tempdir().unwrap();
    let result = tool
        .execute(
            serde_json::json!({ "command": QUOTED, "run_in_background": true }),
            &context(directory.path()),
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    let id = result
        .content
        .lines()
        .find_map(|line| line.strip_prefix("Task ID: "))
        .unwrap_or_else(|| panic!("no task ID in {}", result.content));
    let task = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let task = global_registry().get(id).unwrap();
            if task.status != TaskStatus::Running {
                break task;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the background task never ended");
    assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.output);
    task.output
}

#[tokio::test]
async fn bash_fallback_passes_quotes_as_written() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        output_of(&BashTool, QUOTED, directory.path()).await,
        QUOTED_LINES.join("\n")
    );
}

#[tokio::test]
async fn pty_fallback_passes_quotes_as_written() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        output_of(&PtyBashTool, QUOTED, directory.path()).await,
        QUOTED_LINES.join("\n")
    );
}

#[tokio::test]
async fn bash_fallback_passes_a_quoted_path() {
    check_quoted_path(&BashTool).await;
}

#[tokio::test]
async fn pty_fallback_passes_a_quoted_path() {
    check_quoted_path(&PtyBashTool).await;
}

#[tokio::test]
async fn bash_fallback_runs_a_quoted_program_with_a_quoted_argument() {
    check_quoted_program(&BashTool).await;
}

#[tokio::test]
async fn pty_fallback_runs_a_quoted_program_with_a_quoted_argument() {
    check_quoted_program(&PtyBashTool).await;
}

#[tokio::test]
async fn bash_background_task_passes_quotes_as_written() {
    assert_eq!(background_output_of(&BashTool).await, QUOTED_LINES);
}

#[tokio::test]
async fn pty_background_task_passes_quotes_as_written() {
    assert_eq!(background_output_of(&PtyBashTool).await, QUOTED_LINES);
}

/// EnterWorktree keeps one session per process, so this must stay the only
/// EnterWorktree call in this binary.
#[tokio::test]
async fn worktree_post_create_command_passes_quotes_as_written() {
    let repository = tempfile::tempdir().unwrap();
    std::fs::write(repository.path().join("tracked.txt"), "fixture\n").unwrap();
    let commands: [&[&str]; 3] = [
        &["init", "-q"],
        &["add", "tracked.txt"],
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-q",
            "-m",
            "fixture",
        ],
    ];
    for args in commands {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repository.path())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    let result = EnterWorktreeTool
        .execute(
            serde_json::json!({ "branch": "quotes", "post_create_command": QUOTED }),
            &context(repository.path()),
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    // The tool reports the command's output as printed, so with `echo`'s CRLF.
    let report = format!(
        "\nPost-create command '{QUOTED}' completed successfully.\nOutput: {}",
        QUOTED_LINES.join("\r\n")
    );
    assert!(result.content.ends_with(&report), "{}", result.content);
}
