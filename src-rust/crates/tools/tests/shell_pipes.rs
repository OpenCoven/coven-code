//! Pipe-drain regressions through registered tools and the public Bash SDK.
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
    if case == "powershell" {
        if !available("pwsh", "-Version") {
            return;
        }
    } else if !available("python3", "--version") {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "pipe_child", "--ignored", "--nocapture"])
        .env("COVEN_PIPE_CASE", case)
        .env("COVEN_PIPE_HOME", home.path())
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let group = OwnedGroup(child);
    let result_path = home.path().join("result.json");
    let deadline =
        Instant::now() + Duration::from_secs(if case == "powershell" { 135 } else { 30 });
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
fn registered_powershell_drains_both_pipes() {
    run_case("powershell");
}
#[test]
fn registered_background_bash_drains_both_pipes() {
    run_case("background");
}
#[test]
fn public_background_bash_drains_both_pipes() {
    run_case("sdk-background");
}

#[test]
fn registered_background_stderr_stays_live() {
    run_case("background-live");
}

#[test]
fn public_background_stderr_stays_live() {
    run_case("sdk-background-live");
}

#[test]
fn public_bash_drains_both_pipes() {
    run_case("bash");
}

#[test]
#[ignore = "owned process group driver invoked by parent"]
fn pipe_child() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_PIPE_HOME").unwrap());
    let case = std::env::var("COVEN_PIPE_CASE").unwrap();
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
    let background = matches!(case, "background" | "sdk-background");
    let timeout = if case == "powershell" { 30_000 } else { 10_000 };
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

    std::fs::write(
        home.join("pipe_fixture.py"),
        r#"import os
payload = b'e' * (1024 * 1024) + b'\n'
while payload:
    payload = payload[os.write(2, payload):]
os.write(1, b'stdout-after-stderr\n')
"#,
    )
    .unwrap();
    let tool: Box<dyn Tool> = match case {
        "powershell" => claurst_tools::find_tool("PowerShell").unwrap(),
        "background" | "background-live" => {
            claurst_tools::find_tool(claurst_core::constants::TOOL_NAME_BASH).unwrap()
        }
        "bash" | "sdk-background" | "sdk-background-live" => Box::new(BashTool),
        _ => panic!("unknown case"),
    };
    if case.ends_with("-live") {
        check_live_stderr(tool.as_ref(), &ctx, home).await;
        claurst_tools::clear_session_shell_state(&ctx.session_id);
        return;
    }
    let command = if case == "powershell" {
        "[Console]::Error.WriteLine(('e' * 1048576)); [Console]::Error.Flush(); [Console]::Out.WriteLine('stdout-after-stderr')"
    } else {
        "python3 pipe_fixture.py"
    };
    let result = tool
        .execute(
            serde_json::json!({"command":command, "timeout":timeout, "run_in_background":background}),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    if background {
        let id = result
            .content
            .lines()
            .find_map(|line| line.strip_prefix("Task ID: "))
            .expect("background task ID");
        let task = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let task = global_registry().get(id).unwrap();
                if task.status != TaskStatus::Running {
                    break task;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("background completion deadline");
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(
            task.output,
            vec![
                "stdout-after-stderr".to_owned(),
                format!("STDERR: {}", "e".repeat(1048576))
            ]
        );
    } else {
        assert!(result.content.starts_with("stdout-after-stderr\nSTDERR:\n"));
        assert!(result.content.ends_with(&"e".repeat(50000)));
        let controls = if case == "powershell" {
            [
                (
                    "[Console]::Out.WriteLine('out'); [Console]::Error.WriteLine('err')",
                    "out\nSTDERR:\nerr",
                    false,
                ),
                ("$null", "(no output)", false),
                (
                    "Write-Output 'out'; exit 7",
                    "PowerShell exited with code 7\nout",
                    true,
                ),
            ]
        } else {
            [
                (
                    "printf 'out\n'; printf 'err\n' >&2",
                    "out\nSTDERR:\nerr",
                    false,
                ),
                ("true", "(no output)", false),
                (
                    "printf 'out\n'; exit 7",
                    "Command exited with code 7\nout",
                    true,
                ),
            ]
        };
        for (command, expected, error) in controls {
            let result = tool
                .execute(
                    serde_json::json!({"command":command,"timeout":timeout}),
                    &ctx,
                )
                .await;
            assert_eq!(result.is_error, error, "{command}: {}", result.content);
            assert_eq!(result.content, expected);
        }
        if case == "bash" {
            std::fs::create_dir(home.join("child")).unwrap();
            let changed = tool.execute(serde_json::json!({"command":"cd child; export PIPE_FIXTURE=value","timeout":10000}), &ctx).await;
            assert!(!changed.is_error, "{}", changed.content);
            let state = tool.execute(serde_json::json!({"command":"printf '%s\n' \"$PIPE_FIXTURE\"; pwd","timeout":10000}), &ctx).await;
            assert_eq!(
                state.content,
                format!("value\n{}", home.join("child").display())
            );
        }
    }
    claurst_tools::clear_session_shell_state(&ctx.session_id);
}

async fn check_live_stderr(tool: &dyn Tool, ctx: &ToolContext, home: &Path) {
    std::fs::write(
        home.join("live_fixture.py"),
        r#"import os
import time
from pathlib import Path
os.close(1)
os.write(2, b'live-stderr\n')
while not Path('release').exists():
    time.sleep(0.01)
"#,
    )
    .unwrap();
    let live = tool.execute(serde_json::json!({"command":"exec python3 live_fixture.py", "timeout":10000, "run_in_background":true}), ctx).await;
    assert!(!live.is_error, "{}", live.content);
    let live_id = live
        .content
        .lines()
        .find_map(|line| line.strip_prefix("Task ID: "))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let task = global_registry().get(live_id).unwrap();
            assert_eq!(task.status, TaskStatus::Running);
            if task.output.iter().any(|line| line == "STDERR: live-stderr") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("stderr must stream after stdout closes, before process completion");
    assert!(!home.join("release").exists());
    std::fs::write(home.join("release"), b"continue").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let task = global_registry().get(live_id).unwrap();
            if task.status != TaskStatus::Running {
                assert_eq!(task.status, TaskStatus::Completed);
                assert_eq!(task.output, ["STDERR: live-stderr"]);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("released background fixture must complete");
}
