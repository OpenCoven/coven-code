//! Late background completion regressions through registered tools and the public Bash SDK.
#![cfg(unix)]
use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_core::tasks::{global_registry, TaskStatus};
use claurst_tools::{BashTool, Tool, ToolContext};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::{Duration, Instant};

struct OwnedGroup(Child);
impl Drop for OwnedGroup {
    fn drop(&mut self) {
        // The fixture leader parks after reporting; its PGID cannot be reused
        // while this guard owns it. All background-tool descendants inherit this group.
        let pid = i32::try_from(self.0.id()).unwrap();
        assert!(pid >= 2, "fixture must own a positive process-group leader");
        let group = nix::unistd::Pid::from_raw(pid);
        let killed = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
        let _ = self.0.kill();
        let waited = self.0.wait();
        if !std::thread::panicking() {
            assert!(
                matches!(killed, Ok(()) | Err(nix::errno::Errno::ESRCH)),
                "group cleanup: {killed:?}"
            );
            assert!(waited.is_ok(), "driver reap: {waited:?}");
        } else if !matches!(killed, Ok(()) | Err(nix::errno::Errno::ESRCH)) {
            eprintln!("group cleanup failed during unwind: {killed:?}");
        }
    }
}

fn available(program: &str, argument: &str) -> bool {
    match Command::new(program).arg(argument).output() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping background terminal fixture: {program} is not installed");
            false
        }
        result => {
            assert!(result.unwrap().status.success());
            true
        }
    }
}

fn run_case(case: &str) {
    if !available("python3", "--version") {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "terminal_child", "--ignored", "--nocapture"])
        .env("COVEN_TERMINAL_CASE", case)
        .env("COVEN_TERMINAL_HOME", home.path())
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let group = OwnedGroup(child);
    let result_path = home.path().join("result.json");
    let deadline = Instant::now() + Duration::from_secs(30);
    let result: Result<(), String> = loop {
        if result_path.exists() {
            break serde_json::from_slice(&std::fs::read(&result_path).unwrap()).unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "fixture exceeded parent deadline"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    drop(group); // Kill and reap before TempDir cleanup, including baseline orphans.
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

#[test]
fn registered_success_preserves_cancel() {
    run_case("registered-0");
}
#[test]
fn registered_failure_preserves_cancel() {
    run_case("registered-7");
}
#[test]
fn raw_success_preserves_cancel() {
    run_case("raw-0");
}
#[test]
fn raw_failure_preserves_cancel() {
    run_case("raw-7");
}
#[test]
#[ignore = "owned process group driver invoked by parent"]
fn terminal_child() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_TERMINAL_HOME").unwrap());
    let case = std::env::var("COVEN_TERMINAL_CASE").unwrap();
    let result = std::panic::catch_unwind(|| {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(check_tool(&home, &case));
    })
    .map_err(|panic| {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
            .unwrap_or_else(|| "fixture panicked".into())
    });
    // Publish only complete result JSON; the driver remains alive for group cleanup.
    std::fs::write(
        home.join("result.tmp"),
        serde_json::to_vec(&result).unwrap(),
    )
    .unwrap();
    std::fs::rename(home.join("result.tmp"), home.join("result.json")).unwrap();
    loop {
        std::thread::park();
    }
}

async fn check_tool(home: &Path, case: &str) {
    let ctx = ToolContext {
        working_dir: home.to_path_buf(),
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
    let tool: Box<dyn Tool> = if case.starts_with("raw") {
        Box::new(BashTool)
    } else {
        claurst_tools::find_tool(claurst_core::constants::TOOL_NAME_BASH).unwrap()
    };
    let exit_code = if case.ends_with('7') { 7 } else { 0 };
    std::fs::write(home.join("wait.py"), format!("import os,time\nfrom pathlib import Path\nPath('started.tmp').write_text(str(os.getpid()))\nPath('started.tmp').rename('started')\nwhile not Path('release').exists(): time.sleep(0.01)\nos.write(1,b'finished\\n')\nraise SystemExit({exit_code})\n")).unwrap();
    let result = tool.execute(serde_json::json!({"command":"exec python3 wait.py","run_in_background":true,"timeout":10000}), &ctx).await;
    assert!(!result.is_error, "{}", result.content);
    let id = result
        .content
        .lines()
        .find_map(|line| line.strip_prefix("Task ID: "))
        .unwrap();
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let task = global_registry().get(id).unwrap();
            assert_eq!(task.status, TaskStatus::Running);
            if let Ok(marker) = std::fs::read_to_string(home.join("started")) {
                let pid: u32 = marker.parse().unwrap();
                assert_eq!(task.pid, Some(pid));
                break pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned process starts before cancellation");
    // This tests registry terminal-state preservation, not monitor process termination.
    global_registry().cancel(id);
    let cancelled_at = global_registry().get(id).unwrap().completed_at;
    assert!(cancelled_at.is_some());
    std::fs::write(home.join("release"), b"continue").unwrap();
    let pid = i32::try_from(pid).unwrap();
    assert!(pid >= 2);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
                Err(nix::errno::Errno::ESRCH) => break,
                Ok(()) => tokio::time::sleep(Duration::from_millis(10)).await,
                other => panic!("owned child liveness probe: {other:?}"),
            }
        }
    })
    .await
    .expect("worker must reap released child");
    // Allow the worker's synchronous registry update after wait() to finish.
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let task = global_registry().get(id).unwrap();
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert_eq!(task.completed_at, cancelled_at);
        assert_eq!(task.output, ["finished"]);
    }
    claurst_tools::clear_session_shell_state(&ctx.session_id);
}
