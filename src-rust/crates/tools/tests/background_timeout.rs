//! Background timeout regressions through registered tools and the public Bash SDK.
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
        // while this guard owns it. All pipe-tool descendants inherit this group.
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
            eprintln!("skipping shell pipe fixture: {program} is not installed");
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
        .args(["--exact", "timeout_child", "--ignored", "--nocapture"])
        .env("COVEN_TIMEOUT_CASE", case)
        .env("COVEN_TIMEOUT_HOME", home.path())
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
fn registered_timeout_reaps_child() {
    run_case("registered-timeout");
}
#[test]
fn raw_timeout_reaps_child() {
    run_case("raw-timeout");
}
#[test]
fn registered_exit_controls() {
    run_case("registered-controls");
}
#[test]
fn raw_exit_notification_controls() {
    run_case("raw-controls");
}
#[test]
#[ignore = "owned process group driver invoked by parent"]
fn timeout_child() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_TIMEOUT_HOME").unwrap());
    let case = std::env::var("COVEN_TIMEOUT_CASE").unwrap();
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
    let mut ctx = ToolContext {
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
    if case.ends_with("timeout") {
        std::fs::write(home.join("wait.py"), "import os,time\nfrom pathlib import Path\nPath('started.tmp').write_text(str(os.getpid()))\nPath('started.tmp').rename('started')\nwhile True: time.sleep(0.1)\n").unwrap();
        let result = tool.execute(serde_json::json!({"command":"exec python3 wait.py","run_in_background":true,"timeout":5000}), &ctx).await;
        assert!(!result.is_error, "{}", result.content);
        let id = result
            .content
            .lines()
            .find_map(|line| line.strip_prefix("Task ID: "))
            .unwrap();
        let pid = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let task = global_registry().get(id).unwrap();
                assert_eq!(
                    task.status,
                    TaskStatus::Running,
                    "process must start before terminal status"
                );
                if let Ok(marker) = std::fs::read_to_string(home.join("started")) {
                    let pid: u32 = marker.parse().unwrap();
                    assert_eq!(
                        task.pid,
                        Some(pid),
                        "exec must retain the registered child PID"
                    );
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("owned process start marker");
        assert_eq!(
            terminal_status(id).await,
            TaskStatus::Failed("timed out after 5000ms".into())
        );
        let pid = i32::try_from(pid).unwrap();
        assert!(pid >= 2);
        // Probe only: cleanup never signals this potentially reusable individual PID.
        assert_eq!(
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
            Err(nix::errno::Errno::ESRCH),
            "timeout published before child exit/reaping"
        );
    } else {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        ctx.completion_notifier = Some(claurst_tools::CompletionNotifier::new(move |event| {
            tx.send(event).unwrap();
        }));
        for (command, expected, success, exit_info) in [
            ("printf 'control\\n'", TaskStatus::Completed, true, "exit 0"),
            (
                "printf 'control\\n'; exit 7",
                TaskStatus::Failed("exit code 7".into()),
                false,
                "failed: exit code 7",
            ),
        ] {
            let notify = case.starts_with("raw");
            let result = tool.execute(serde_json::json!({"command":command,"run_in_background":true,"notify_on_complete":notify,"timeout":5000}), &ctx).await;
            assert!(!result.is_error, "{}", result.content);
            let id = if notify {
                result
                    .content
                    .lines()
                    .next()
                    .unwrap()
                    .strip_prefix("Started background task ")
                    .unwrap()
                    .strip_suffix('.')
                    .unwrap()
            } else {
                result
                    .content
                    .lines()
                    .find_map(|line| line.strip_prefix("Task ID: "))
                    .unwrap()
            };
            assert_eq!(terminal_status(id).await, expected);
            assert_eq!(global_registry().get(id).unwrap().output, ["control"]);
            if notify {
                let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(event.task_id, id);
                assert_eq!(event.command, command);
                assert_eq!(event.success, success);
                assert_eq!(event.exit_info, exit_info);
                assert_eq!(event.output_tail, "control");
            }
        }
    }
    claurst_tools::clear_session_shell_state(&ctx.session_id);
}
async fn terminal_status(id: &str) -> TaskStatus {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status = global_registry().get(id).unwrap().status;
            if status != TaskStatus::Running {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("background terminal status")
}
