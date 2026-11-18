//! #373 on Windows: the foreground shell tools run `cmd /C` or `powershell`.
//! A cancelled or timed-out call must stop the shell it started and every
//! process below it. A finished call still returns its output and exit code,
//! and leaves alone a process the command started on purpose to outlive it.
//!
//! The same holds for a background task of either Bash tool when it is
//! cancelled or times out, and for a cancelled worktree post-create command.
#![cfg(windows)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_core::tasks::{global_registry, TaskStatus};
use claurst_tools::{
    BashTool, EnterWorktreeTool, MonitorTool, PowerShellTool, PtyBashTool, Tool, ToolContext,
};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::process::Stdio;
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Duration;
use windows_sys::Win32::Foundation::{FILETIME, HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, TerminateProcess, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

/// Holds, for the copy that `nested_job_stops_the_grandchild` runs, the value
/// of the job handle the copy inherits.
const OUTER_JOB: &str = "FOREGROUND_WINDOWS_OUTER_JOB";

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
/// hands it the shell's pipes, then exits with code 3. It prints the started
/// process's ID and creation time (a FILETIME), which together name that
/// process even after its ID is reused.
const PS_DETACH: &str = "$p = Start-Process -WindowStyle Hidden cmd -ArgumentList '/D /C ticker.cmd' -PassThru; '{0} {1}' -f $p.Id, $p.StartTime.ToFileTimeUtc(); exit 3";

/// The same from `cmd.exe`, which has no such command of its own.
const DETACH: &str = "powershell -NoProfile -NonInteractive -Command $p = Start-Process -WindowStyle Hidden cmd -ArgumentList '/D /C ticker.cmd' -PassThru; '{0} {1}' -f $p.Id, $p.StartTime.ToFileTimeUtc()& exit 3";

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

/// A detached loop, held open by handle so that its process ID cannot be
/// reused while the test runs. Dropping the guard ends the loop, even after a
/// failed assertion. The `ping` it may be running ends by itself within a
/// second.
struct Detached(OwnedHandle);

impl Detached {
    /// Open process `pid` only if it is the one created at `created`, so a
    /// process that was given the ID of a loop that already ended is never
    /// touched.
    fn open(pid: u32, created: u64) -> Option<Self> {
        // SAFETY: takes no pointers.
        let process = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        if process.is_null() {
            return None;
        }
        // SAFETY: `process` is a new, open handle that nothing else owns.
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        let [mut creation, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
        // SAFETY: the handle has PROCESS_QUERY_LIMITED_INFORMATION, and each
        // pointer is to a FILETIME that outlives the call.
        let read = unsafe {
            GetProcessTimes(
                process.as_raw_handle(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        };
        let creation =
            (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        (read != 0 && creation == created).then_some(Self(process))
    }
}

impl Drop for Detached {
    fn drop(&mut self) {
        let process = self.0.as_raw_handle();
        // SAFETY: the handle has SYNCHRONIZE. Only a process still running is
        // ended.
        if unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT {
            // SAFETY: the handle has PROCESS_TERMINATE and SYNCHRONIZE.
            unsafe {
                TerminateProcess(process, 1);
                WaitForSingleObject(process, 5_000);
            }
        }
    }
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
    assert_detached_loop_runs(directory.path(), &result.content, shell).await;
}

/// The loop that [`DETACH`] or [`PS_DETACH`] started, named in `output`, is
/// still ticking.
async fn assert_detached_loop_runs(directory: &Path, output: &str, shell: Shell) {
    let (pid, created) = output
        .lines()
        .find_map(|line| {
            let (pid, created) = line.trim().split_once(' ')?;
            Some((pid.parse::<u32>().ok()?, created.parse::<u64>().ok()?))
        })
        .unwrap_or_else(|| panic!("no process ID and creation time in {output}"));
    // `None` if the loop has already ended.
    let detached = Detached::open(pid, created);
    let returned = ticks(directory);
    assert!(
        reaches_ticks(directory, returned + 2).await,
        "the detached loop stopped when the call ended ({shell:?}): {returned} ticks then, {} now",
        ticks(directory)
    );
    assert!(
        detached.is_some(),
        "the detached loop is running, but could not be opened as process {pid} created at {created}"
    );
}

/// Start `command` as a background task of `shell` and return the task's ID.
async fn start_background(
    shell: Shell,
    command: &str,
    timeout_ms: u64,
    ctx: &ToolContext,
) -> String {
    let result = shell
        .tool()
        .execute(
            serde_json::json!({ "command": command, "run_in_background": true, "timeout": timeout_ms }),
            ctx,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    result
        .content
        .lines()
        .find_map(|line| line.strip_prefix("Task ID: "))
        .unwrap_or_else(|| panic!("no task ID in {}", result.content))
        .to_owned()
}

/// The background task's status once it has ended, within 30 s.
async fn ended(id: &str) -> TaskStatus {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let status = global_registry().get(id).unwrap().status;
            if status != TaskStatus::Running {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the background task never ended")
}

/// Cancelling a background task with the monitor tool stops the loop its
/// shell's child started.
async fn check_background_cancel(shell: Shell) {
    let directory = workspace();
    let ctx = context(directory.path());
    let id = start_background(shell, GRANDCHILD, 120_000, &ctx).await;
    assert!(
        reaches_ticks(directory.path(), 2).await,
        "the loop never started"
    );
    let result = MonitorTool
        .execute(
            serde_json::json!({ "action": "cancel", "task_id": id }),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    let stopped = ticks(directory.path());
    let (settled, later) = ticks_after_stop(directory.path()).await;
    assert_eq!(
        settled, later,
        "the grandchild loop kept running after the background task was cancelled ({shell:?}): \
         {stopped} ticks at the cancel, {settled} 3 s later, {later} 6 s later"
    );
    assert_eq!(
        global_registry().get(&id).unwrap().status,
        TaskStatus::Cancelled
    );
}

/// A background task that times out stops the loop its shell's child started.
async fn check_background_timeout(shell: Shell) {
    let directory = workspace();
    let ctx = context(directory.path());
    let id = start_background(shell, GRANDCHILD, 8000, &ctx).await;
    assert_eq!(
        ended(&id).await,
        TaskStatus::Failed("timed out after 8000ms".into())
    );
    let stopped = ticks(directory.path());
    assert!(stopped >= 1, "the loop never started");
    let (settled, later) = ticks_after_stop(directory.path()).await;
    assert_eq!(
        settled, later,
        "the grandchild loop kept running after the background task timed out ({shell:?}): \
         {stopped} ticks at the timeout, {settled} 3 s later, {later} 6 s later"
    );
}

/// A background task that exits with code 3 reports it, and leaves alone the
/// loop it started to outlive it.
async fn check_background_detached(shell: Shell) {
    let directory = workspace();
    let ctx = context(directory.path());
    let id = start_background(shell, DETACH, 120_000, &ctx).await;
    assert_eq!(ended(&id).await, TaskStatus::Failed("exit code 3".into()));
    let output = global_registry().get(&id).unwrap().output.join("\n");
    assert_detached_loop_runs(directory.path(), &output, shell).await;
}

/// A git repository whose one commit holds `ticker.cmd`, so a new worktree of
/// it has the script too.
fn repository() -> tempfile::TempDir {
    let directory = workspace();
    let commands: [&[&str]; 3] = [
        &["init", "-q"],
        &["add", "ticker.cmd"],
        &[
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
            .current_dir(directory.path())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    directory
}

#[tokio::test]
async fn cancelled_bash_background_task_stops_its_grandchild() {
    check_background_cancel(Shell::Bash).await;
}

#[tokio::test]
async fn cancelled_pty_background_task_stops_its_grandchild() {
    check_background_cancel(Shell::PtyBash).await;
}

#[tokio::test]
async fn timed_out_bash_background_task_stops_its_grandchild() {
    check_background_timeout(Shell::Bash).await;
}

#[tokio::test]
async fn timed_out_pty_background_task_stops_its_grandchild() {
    check_background_timeout(Shell::PtyBash).await;
}

#[tokio::test]
async fn bash_background_exit_keeps_its_code_and_a_detached_process() {
    check_background_detached(Shell::Bash).await;
}

#[tokio::test]
async fn pty_background_exit_keeps_its_code_and_a_detached_process() {
    check_background_detached(Shell::PtyBash).await;
}

/// A cancelled EnterWorktree call stops its post-create command's grandchild.
#[tokio::test]
async fn cancelled_worktree_hook_stops_its_grandchild() {
    let repository = repository();
    let worktree = repository.path().join(".worktrees").join("hook-cancel");
    let ctx = context(repository.path());
    let call = tokio::spawn(async move {
        EnterWorktreeTool
            .execute(
                serde_json::json!({ "branch": "hook-cancel", "post_create_command": GRANDCHILD }),
                &ctx,
            )
            .await
    });
    assert!(
        reaches_ticks(&worktree, 2).await,
        "the hook's loop never started"
    );
    assert!(!call.is_finished(), "the hook ended on its own");
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    let stopped = ticks(&worktree);
    let (settled, later) = ticks_after_stop(&worktree).await;
    assert_eq!(
        settled, later,
        "the hook's grandchild loop kept running after the call was cancelled: \
         {stopped} ticks at the cancel, {settled} 3 s later, {later} 6 s later"
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

/// Processes often start inside a job already: on Windows `cargo test` runs
/// tests in one of its own, as do some CI systems and terminals. Run a
/// grandchild case in a copy of this test binary inside one more job, so the
/// tool's job is nested under both.
#[test]
fn nested_job_stops_the_grandchild() {
    // The copy inherits this handle, so it can check that it is in the job.
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: `attributes` outlives the call, and a null name is allowed.
    let job = unsafe { CreateJobObjectW(&attributes, std::ptr::null()) };
    assert!(!job.is_null(), "{}", std::io::Error::last_os_error());
    // SAFETY: `job` is a new, open handle that nothing else owns.
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut copy = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "in_job_cancelled_bash_fallback_stops_its_grandchild",
            "--exact",
            "--ignored",
        ])
        .env(OUTER_JOB, (job.as_raw_handle() as usize).to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The copy checks that it is in the job before it starts a shell.
    // SAFETY: both handles are open.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle(), copy.as_raw_handle()) } == 0 {
        let error = std::io::Error::last_os_error();
        let _ = copy.kill();
        panic!("could not put the copy in a job: {error}");
    }
    let output = copy.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed"),
        "the run inside a job did not pass exactly one test:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "nested_job_stops_the_grandchild runs it inside a job"]
async fn in_job_cancelled_bash_fallback_stops_its_grandchild() {
    let outer: usize = std::env::var(OUTER_JOB)
        .expect("nested_job_stops_the_grandchild runs this")
        .parse()
        .unwrap();
    let mut in_job = 0;
    // SAFETY: `outer` is the inherited job handle, the pseudo handle is always
    // valid, and `in_job` outlives the call.
    let asked = unsafe { IsProcessInJob(GetCurrentProcess(), outer as HANDLE, &mut in_job) };
    assert!(
        asked != 0 && in_job != 0,
        "not in the outer job: {}",
        std::io::Error::last_os_error()
    );
    check_cancel(Shell::Bash, Ticker::Grandchild).await;
}
