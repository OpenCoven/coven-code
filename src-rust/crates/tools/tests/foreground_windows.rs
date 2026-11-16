//! #373 on Windows: the foreground shell tools run `cmd /C` or `powershell`.
//! A cancelled or timed-out call must stop the shell it started and every
//! process below it. A finished call still returns its output and exit code,
//! and leaves alone a process the command started on purpose to outlive it.
#![cfg(windows)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{BashTool, PowerShellTool, PtyBashTool, Tool, ToolContext};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Duration;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// A loop that only `cmd.exe` runs: each pass appends a line, then waits about
/// a second in `ping`. The file stops growing once that `cmd.exe` is gone. The
/// echoed word has no digits, since `echo 1>>file` redirects handle 1.
const TICKER: &str = "for /L %i in (1,1,60) do @(echo tick>>ticks.txt & ping -n 2 127.0.0.1 >nul)";

/// The same loop run by `powershell.exe` itself, with no child process.
const PS_TICKER: &str =
    "for ($i = 0; $i -lt 60; $i++) { Add-Content -Path ticks.txt -Value tick; Start-Sleep -Seconds 1 }";

/// [`TICKER`] two processes below the shell: the shell starts `cmd` (the
/// child), which starts the `cmd` that runs `ticker.cmd` (the grandchild).
/// `cmd.exe` and `powershell.exe` read this line the same way.
const GRANDCHILD: &str = "cmd /D /C cmd /D /C ticker.cmd";

/// Starts `ticker.cmd` through `Start-Process`, which neither waits for it nor
/// hands it the shell's pipes, prints its process ID, then exits with code 3.
const PS_DETACH: &str =
    "(Start-Process -WindowStyle Hidden cmd -ArgumentList '/D /C ticker.cmd' -PassThru).Id; exit 3";

/// The same from `cmd.exe`, which has no such command of its own.
const DETACH: &str = "powershell -NoProfile -NonInteractive -Command (Start-Process -WindowStyle Hidden cmd -ArgumentList '/D /C ticker.cmd' -PassThru).Id& exit 3";

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

/// A directory holding `ticker.cmd`, a batch file that runs [`TICKER`].
fn workspace() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    // A batch file needs `%%i` where the command line takes `%i`.
    let script = format!("@{}\r\n", TICKER.replace('%', "%%"));
    std::fs::write(directory.path().join("ticker.cmd"), script).unwrap();
    directory
}

#[derive(Clone, Copy, Debug)]
enum Shell {
    Bash,
    PtyBash,
    PowerShell,
}

/// Which process runs the loop.
#[derive(Clone, Copy, Debug)]
enum Ticker {
    Shell,
    Grandchild,
}

impl Shell {
    fn tool(self) -> Box<dyn Tool> {
        match self {
            Shell::Bash => Box::new(BashTool),
            Shell::PtyBash => Box::new(PtyBashTool),
            Shell::PowerShell => Box::new(PowerShellTool),
        }
    }

    fn ticker(self, ticker: Ticker) -> &'static str {
        match (self, ticker) {
            (_, Ticker::Grandchild) => GRANDCHILD,
            (Shell::PowerShell, Ticker::Shell) => PS_TICKER,
            (Shell::Bash | Shell::PtyBash, Ticker::Shell) => TICKER,
        }
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

/// Whether the file reaches `count` lines within 20 s.
async fn reaches_ticks(directory: &Path, count: usize) -> bool {
    tokio::time::timeout(Duration::from_secs(20), async {
        while ticks(directory) < count {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .is_ok()
}

/// The tick count 3 s after the call ended, when a pass already under way has
/// landed, and again 3 s later, when a live loop would have added about three.
async fn ticks_after_stop(directory: &Path) -> (usize, usize) {
    tokio::time::sleep(Duration::from_secs(3)).await;
    let settled = ticks(directory);
    tokio::time::sleep(Duration::from_secs(3)).await;
    (settled, ticks(directory))
}

async fn check_cancel(shell: Shell, ticker: Ticker) {
    let directory = workspace();
    let ctx = context(directory.path());
    let call = tokio::spawn(async move {
        shell
            .tool()
            .execute(
                serde_json::json!({ "command": shell.ticker(ticker), "timeout": 120000 }),
                &ctx,
            )
            .await
    });
    assert!(
        reaches_ticks(directory.path(), 2).await,
        "the loop never started"
    );
    assert!(!call.is_finished(), "the loop ended on its own");
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    let stopped = ticks(directory.path());
    let (settled, later) = ticks_after_stop(directory.path()).await;
    assert_eq!(
        settled, later,
        "the {ticker:?} loop kept running after the call was cancelled ({shell:?}): \
         {stopped} ticks at the cancel, {settled} 3 s later, {later} 6 s later"
    );
}

async fn check_timeout(shell: Shell, ticker: Ticker) {
    let directory = workspace();
    let ctx = context(directory.path());
    let result = shell
        .tool()
        .execute(
            // Long enough for a cold powershell.exe to start and tick.
            serde_json::json!({ "command": shell.ticker(ticker), "timeout": 8000 }),
            &ctx,
        )
        .await;
    assert!(
        result.is_error && result.content.contains("timed out"),
        "{}",
        result.content
    );
    let stopped = ticks(directory.path());
    assert!(stopped >= 1, "the loop never started");
    let (settled, later) = ticks_after_stop(directory.path()).await;
    assert_eq!(
        settled, later,
        "the {ticker:?} loop kept running after the call timed out ({shell:?}): \
         {stopped} ticks at the timeout, {settled} 3 s later, {later} 6 s later"
    );
}

async fn check_completion(shell: Shell) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let result = shell
        .tool()
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

/// A command that exits with code 3 after starting a loop meant to outlive it.
/// The call reports the code and the output, and the loop keeps going.
async fn check_detached(shell: Shell) {
    let directory = workspace();
    let ctx = context(directory.path());
    let command = match shell {
        Shell::PowerShell => PS_DETACH,
        Shell::Bash | Shell::PtyBash => DETACH,
    };
    let result = shell
        .tool()
        .execute(serde_json::json!({ "command": command }), &ctx)
        .await;
    assert!(
        result.is_error && result.content.contains("exited with code 3"),
        "{}",
        result.content
    );
    let pid: u32 = result
        .content
        .lines()
        .find_map(|line| line.trim().parse().ok())
        .unwrap_or_else(|| panic!("no process ID in {}", result.content));
    let returned = ticks(directory.path());
    let kept = reaches_ticks(directory.path(), returned + 2).await;
    let stopped = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output()
        .unwrap();
    assert!(
        kept,
        "the detached loop stopped when the call ended ({shell:?}): {returned} ticks then, {} now",
        ticks(directory.path())
    );
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
}

#[tokio::test]
async fn cancelled_pty_fallback_stops_its_cmd() {
    check_cancel(Shell::PtyBash, Ticker::Shell).await;
}

#[tokio::test]
async fn cancelled_bash_fallback_stops_its_cmd() {
    check_cancel(Shell::Bash, Ticker::Shell).await;
}

#[tokio::test]
async fn timed_out_pty_fallback_stops_its_cmd() {
    check_timeout(Shell::PtyBash, Ticker::Shell).await;
}

#[tokio::test]
async fn timed_out_bash_fallback_stops_its_cmd() {
    check_timeout(Shell::Bash, Ticker::Shell).await;
}

#[tokio::test]
async fn pty_fallback_returns_output_of_a_finished_command() {
    check_completion(Shell::PtyBash).await;
}

#[tokio::test]
async fn bash_fallback_returns_output_of_a_finished_command() {
    check_completion(Shell::Bash).await;
}

#[tokio::test]
async fn cancelled_powershell_stops_its_process() {
    check_cancel(Shell::PowerShell, Ticker::Shell).await;
}

#[tokio::test]
async fn timed_out_powershell_stops_its_process() {
    check_timeout(Shell::PowerShell, Ticker::Shell).await;
}

#[tokio::test]
async fn cancelled_pty_fallback_stops_its_grandchild() {
    check_cancel(Shell::PtyBash, Ticker::Grandchild).await;
}

#[tokio::test]
async fn cancelled_bash_fallback_stops_its_grandchild() {
    check_cancel(Shell::Bash, Ticker::Grandchild).await;
}

#[tokio::test]
async fn cancelled_powershell_stops_its_grandchild() {
    check_cancel(Shell::PowerShell, Ticker::Grandchild).await;
}

#[tokio::test]
async fn timed_out_pty_fallback_stops_its_grandchild() {
    check_timeout(Shell::PtyBash, Ticker::Grandchild).await;
}

#[tokio::test]
async fn timed_out_bash_fallback_stops_its_grandchild() {
    check_timeout(Shell::Bash, Ticker::Grandchild).await;
}

#[tokio::test]
async fn timed_out_powershell_stops_its_grandchild() {
    check_timeout(Shell::PowerShell, Ticker::Grandchild).await;
}

#[tokio::test]
async fn pty_fallback_exit_keeps_its_code_and_a_detached_process() {
    check_detached(Shell::PtyBash).await;
}

#[tokio::test]
async fn bash_fallback_exit_keeps_its_code_and_a_detached_process() {
    check_detached(Shell::Bash).await;
}

#[tokio::test]
async fn powershell_exit_keeps_its_code_and_a_detached_process() {
    check_detached(Shell::PowerShell).await;
}

/// CI runners and some terminals start processes inside a job already. Run a
/// grandchild case in a copy of this test binary that is in a job, so the
/// tool's own job is nested in it.
#[test]
fn nested_job_stops_the_grandchild() {
    // SAFETY: null attributes and name are allowed; they make an unnamed job.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    assert!(!job.is_null(), "{}", std::io::Error::last_os_error());
    // SAFETY: `job` is a new, open handle that nothing else owns.
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut copy = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "in_job_cancelled_bash_fallback_stops_its_grandchild",
            "--exact",
            "--ignored",
        ])
        .spawn()
        .unwrap();
    // The copy checks that it is in the job before it starts a shell.
    // SAFETY: both handles are open.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle(), copy.as_raw_handle()) } == 0 {
        let error = std::io::Error::last_os_error();
        let _ = copy.kill();
        panic!("could not put the copy in a job: {error}");
    }
    assert!(
        copy.wait().unwrap().success(),
        "the run inside a job failed; its output is above"
    );
}

#[tokio::test]
#[ignore = "nested_job_stops_the_grandchild runs it inside a job"]
async fn in_job_cancelled_bash_fallback_stops_its_grandchild() {
    let mut in_job = 0;
    // SAFETY: the pseudo handle is always valid, a null job means any job, and
    // `in_job` outlives the call.
    let asked = unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) };
    assert!(
        asked != 0 && in_job != 0,
        "not in a job: run nested_job_stops_the_grandchild instead"
    );
    check_cancel(Shell::Bash, Ticker::Grandchild).await;
}
