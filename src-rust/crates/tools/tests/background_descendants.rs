//! #373: on Unix, cancelling a background task with the monitor tool, or the
//! task timing out, stops what its shell started, not just the shell. An
//! ordinary exit, and the process that started the task exiting, leave alone
//! what the command started to outlive them.
#![cfg(unix)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_core::tasks::{global_registry, TaskStatus};
use claurst_tools::{BashTool, MonitorTool, PtyBashTool, Tool, ToolContext};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::path::Path;
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Duration;

/// The shell starts `sleep` and waits for it, so the task runs until it is
/// stopped. `sleep` records its PID.
const DESCENDANT: &str = "sleep 120 & echo $! > child-pid.tmp; mv child-pid.tmp child-pid; wait";

/// The shell starts `sleep` to outlive it, away from the task's pipes, and
/// exits with code 3.
const DETACHED: &str = "sleep 120 >/dev/null 2>&1 & echo $! > child-pid.tmp; \
    mv child-pid.tmp child-pid; exit 3";

const COPY_DIRECTORY: &str = "COVEN_BACKGROUND_DESCENDANT_DIRECTORY";

/// A descendant the test recorded by PID. Dropping it kills the descendant if
/// it is still running, so a failing case leaves nothing behind.
struct Descendant(Pid);

impl Drop for Descendant {
    fn drop(&mut self) {
        if !gone(self.0) {
            let _ = kill(self.0, Signal::SIGKILL);
        }
    }
}

/// Whether `pid` has exited. A zombie that its new parent has not reaped yet
/// counts as exited.
fn gone(pid: Pid) -> bool {
    match kill(pid, None) {
        Err(nix::errno::Errno::ESRCH) => true,
        Err(error) => panic!("liveness probe of {pid}: {error}"),
        Ok(()) => {
            let state = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            let state = String::from_utf8_lossy(&state.stdout);
            state.trim().is_empty() || state.trim_start().starts_with('Z')
        }
    }
}

/// Whether `pid` exits within `bound`.
async fn gone_within(pid: Pid, bound: Duration) -> bool {
    tokio::time::timeout(bound, async {
        while !gone(pid) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok()
}

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

/// Start `command` as a background task and return the task's ID and the
/// descendant the command recorded.
async fn start_background(
    pty: bool,
    command: &str,
    timeout_ms: u64,
    ctx: &ToolContext,
) -> (String, Descendant) {
    let result = tool(pty)
        .execute(
            serde_json::json!({ "command": command, "run_in_background": true, "timeout": timeout_ms }),
            ctx,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    let id = result
        .content
        .lines()
        .find_map(|line| line.strip_prefix("Task ID: "))
        .unwrap_or_else(|| panic!("no task ID in {}", result.content))
        .to_owned();
    let marker = ctx.working_dir.join("child-pid");
    let pid = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&marker) {
                break text.trim().parse::<i32>().unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the background command never started its descendant");
    assert!(pid >= 2);
    (id, Descendant(Pid::from_raw(pid)))
}

/// The background task's status once it has ended, within 30 s.
async fn ended(id: &str) -> TaskStatus {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let status = global_registry().get(id).unwrap().status;
            if status != TaskStatus::Running {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the background task never ended")
}

async fn check_cancel(pty: bool) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let (id, descendant) = start_background(pty, DESCENDANT, 120_000, &ctx).await;
    let result = MonitorTool
        .execute(
            serde_json::json!({ "action": "cancel", "task_id": id }),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert!(
        gone_within(descendant.0, Duration::from_secs(5)).await,
        "the background shell's descendant {} kept running 5 s after the task was cancelled (pty={pty})",
        descendant.0
    );
    assert_eq!(
        global_registry().get(&id).unwrap().status,
        TaskStatus::Cancelled
    );
}

async fn check_timeout(pty: bool) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let (id, descendant) = start_background(pty, DESCENDANT, 5000, &ctx).await;
    assert_eq!(
        ended(&id).await,
        TaskStatus::Failed("timed out after 5000ms".into())
    );
    assert!(
        gone_within(descendant.0, Duration::from_secs(5)).await,
        "the background shell's descendant {} kept running 5 s after the task timed out (pty={pty})",
        descendant.0
    );
}

async fn check_detached(pty: bool) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let (id, descendant) = start_background(pty, DETACHED, 120_000, &ctx).await;
    assert_eq!(ended(&id).await, TaskStatus::Failed("exit code 3".into()));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !gone(descendant.0),
        "the descendant the command started to outlive it was stopped when the task exited (pty={pty})"
    );
}

#[tokio::test]
async fn cancelled_bash_background_task_stops_its_descendant() {
    check_cancel(false).await;
}

#[tokio::test]
async fn cancelled_pty_background_task_stops_its_descendant() {
    check_cancel(true).await;
}

#[tokio::test]
async fn timed_out_bash_background_task_stops_its_descendant() {
    check_timeout(false).await;
}

#[tokio::test]
async fn timed_out_pty_background_task_stops_its_descendant() {
    check_timeout(true).await;
}

#[tokio::test]
async fn bash_background_exit_keeps_its_code_and_a_detached_process() {
    check_detached(false).await;
}

#[tokio::test]
async fn pty_background_exit_keeps_its_code_and_a_detached_process() {
    check_detached(true).await;
}

/// A background task outlives the turn, and coven-code too, until it is
/// stopped. A copy of this test binary starts one, drops its runtime, which
/// drops the task, and exits. What the task's shell started must keep
/// running.
#[test]
fn background_task_outlives_the_process_that_started_it() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("copy.log");
    let file = std::fs::File::create(&log).unwrap();
    let mut copy = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "in_copy_start_a_background_task_and_exit",
            "--exact",
            "--ignored",
        ])
        .env(COPY_DIRECTORY, directory.path())
        .stdin(std::process::Stdio::null())
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = copy.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            let _ = copy.kill();
            let _ = copy.wait();
            panic!("the copy did not exit");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = std::fs::read_to_string(&log).unwrap_or_default();
    let recorded = std::fs::read_to_string(directory.path().join("child-pid"));
    let descendant = recorded
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
        .map(|pid| Descendant(Pid::from_raw(pid)));
    assert!(
        status.success() && directory.path().join("copy-ready").exists(),
        "the copy did not start its background task and exit cleanly ({status}):\n{output}"
    );
    let descendant = descendant.expect("the copy recorded its task's descendant");
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !gone(descendant.0),
        "the background task's descendant {} stopped when the process that started it exited",
        descendant.0
    );
}

#[test]
#[ignore = "background_task_outlives_the_process_that_started_it runs it"]
fn in_copy_start_a_background_task_and_exit() {
    let directory = std::path::PathBuf::from(
        std::env::var_os(COPY_DIRECTORY)
            .expect("background_task_outlives_the_process_that_started_it runs this"),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let id = runtime.block_on(async {
        let ctx = context(&directory);
        let (id, descendant) = start_background(false, DESCENDANT, 120_000, &ctx).await;
        // The outer test owns the descendant from here.
        std::mem::forget(descendant);
        id
    });
    assert_eq!(
        global_registry().get(&id).unwrap().status,
        TaskStatus::Running
    );
    drop(runtime);
    std::fs::write(directory.join("copy-ready"), "").unwrap();
}
