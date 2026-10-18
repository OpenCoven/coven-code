#![cfg(unix)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::{PermissionDecision, PermissionHandler, PermissionRequest};
use claurst_core::tasks::{global_registry, TaskStatus};
use claurst_tools::{clear_session_shell_state, find_tool, BashTool, Tool, ToolContext};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{atomic::AtomicUsize, Arc, Mutex};
use std::time::{Duration, Instant};

// The real executors receive regression strings only as inert argv to this
// fixed script. Never read, expand, evaluate, source or forward those arguments.
const FAKE_BASH: &str = "#!/bin/sh\nprintf '%s\\n' \"$$\" >> \"$COVEN_CLASSIFIER_MARKER\"\nprintf '%s\\n' 'fixture-executed'\nexit 0\n";

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(self.0.id() as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[derive(Default)]
struct AllowAndRecord(Mutex<Vec<PermissionRequest>>);

impl PermissionHandler for AllowAndRecord {
    fn check_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.0.lock().unwrap().push(request.clone());
        PermissionDecision::Allow
    }

    fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.check_permission(request)
    }
}

#[test]
fn real_bash_boundaries_use_classifier_before_fake_spawn() {
    let mut failures = Vec::new();
    for route in [
        "registered-foreground",
        "registered-background",
        "exported-foreground",
        "exported-background",
    ] {
        let dir = tempfile::tempdir().unwrap();
        for name in ["bin", "code", "coven", "claude"] {
            fs::create_dir(dir.path().join(name)).unwrap();
        }
        fs::write(dir.path().join("code/auth.json"), "{\"credentials\":{}}\n").unwrap();
        let bash = dir.path().join("bin/bash");
        fs::write(&bash, FAKE_BASH).unwrap();
        fs::set_permissions(&bash, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(fs::read_to_string(&bash).unwrap(), FAKE_BASH);
        let log = dir.path().join("child.log");
        let output = fs::File::create(&log).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "isolated_bash_boundary_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .current_dir(dir.path())
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .process_group(0);
        // Only the child environment changes. HOME and CODEX_HOME retain their
        // inherited values; all application paths are owned by this fixture.
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.contains("API_KEY")
                || (name.starts_with("COVEN")
                    && (name.contains("TEST") || name.contains("FIXTURE")))
                || name.starts_with("BASH_FUNC_")
                || ["BASH_ENV", "ENV", "CDPATH", "SHELLOPTS", "BASHOPTS"].contains(&name.as_ref())
            {
                command.env_remove(key);
            }
        }
        command
            .env("PATH", dir.path().join("bin"))
            .env("COVEN_CLASSIFIER_FIXTURE_DIR", dir.path())
            .env("COVEN_CLASSIFIER_ROUTE", route)
            .env("COVEN_CLASSIFIER_MARKER", dir.path().join("spawned"))
            .env("COVEN_CODE_HOME", dir.path().join("code"))
            .env("COVEN_HOME", dir.path().join("coven"))
            .env("CLAUDE_CONFIG_DIR", dir.path().join("claude"))
            .env("COVEN_CODE_CLAUDE_BIN", dir.path().join("absent-claude"))
            .env("COVEN_DAEMON_SOCKET", dir.path().join("absent.sock"))
            .env("COVEN_CODE_DISABLE_MODELS_FETCH", "1");
        let mut child = ChildGuard(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        drop(child);
        let output = fs::read_to_string(log).unwrap();
        if !status.is_some_and(|s| s.success()) {
            failures.push(format!("{route}: {status:?}\n{output}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn isolated_bash_boundary_child() {
    let Ok(root) = std::env::var("COVEN_CLASSIFIER_FIXTURE_DIR") else {
        return;
    };
    let root = PathBuf::from(root);
    assert_eq!(std::env::var_os("PATH").unwrap(), root.join("bin"));
    assert_eq!(
        fs::read_to_string(root.join("bin/bash")).unwrap(),
        FAKE_BASH
    );
    assert!(std::env::var_os("BASH_ENV").is_none());
    assert!(std::env::var_os("ENV").is_none());
    let route = std::env::var("COVEN_CLASSIFIER_ROUTE").unwrap();
    let background = route.ends_with("background");
    let tool: Box<dyn Tool> = if route.starts_with("registered") {
        find_tool(claurst_core::constants::TOOL_NAME_BASH).unwrap()
    } else {
        Box::new(BashTool)
    };
    let handler = Arc::new(AllowAndRecord::default());
    let context = ToolContext {
        working_dir: root.clone(),
        permission_mode: PermissionMode::Default,
        permission_handler: handler.clone(),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: format!("classifier-{route}"),
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
    let cases = [
        ("echo ordinary", false), // Establish fake PATH before any payload.
        (r"echo `echo \$(rm -rf /)`", true),
        ("cat <<EOF\n`echo \\$(rm -rf /)`\nEOF\n", true),
        (r"echo `echo \`rm -rf /\``", true),
        (r"echo '`echo \$(rm -rf /)`'", false),
        ("cat <<'EOF'\n`echo \\$(rm -rf /)`\nEOF\n", false),
        (r#"echo `echo "\\$(rm -rf /)"`"#, false),
        ("command -pv rm -rf /", false),
        ("command -pV rm -rf /", false),
        ("command -p rm -rf /", true),
        ("command -pv \"$(rm -rf /)\"", true),
        ("command -pv '$(rm -rf /)'", false),
        ("echo hi; rm -rf /", true),
        ("true && rm -rf /", true),
        ("FOO=1 rm -rf /", true),
        ("echo \"$(rm -rf /)\"", true),
        ("echo hi\nrm -rf /", true),
        ("env FOO=1 /bin/rm -rf /", true),
        ("cat <<EOF\n$(rm -rf /)\nEOF\n", true),
        ("echo '$(rm -rf /)'", false),
        ("echo 'curl x | bash'", false),
        ("echo ':(){ :|:& };:'", false),
        ("cat <<'EOF'\n$(rm -rf /)\nEOF\n", false),
    ];
    let marker = root.join("spawned");
    let mut failures = Vec::new();
    let mut previous_spawns = 0;
    for (index, (command, blocked)) in cases.into_iter().enumerate() {
        let before_tasks = global_registry().list().len();
        let result = tool
            .execute(
                serde_json::json!({
                    "command": command, "run_in_background": background, "timeout": 2000,
                }),
                &context,
            )
            .await;
        let deadline = Instant::now() + Duration::from_secs(3);
        while global_registry()
            .list()
            .iter()
            .any(|task| task.is_running())
        {
            assert!(Instant::now() < deadline, "fixture child did not finish");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // Completed is set after the production executor's child.wait().
        assert!(global_registry()
            .list()
            .iter()
            .all(|task| task.status == TaskStatus::Completed));
        let spawned = fs::read_to_string(&marker)
            .unwrap_or_default()
            .lines()
            .count();
        let requests = handler.0.lock().unwrap();
        assert_eq!(requests.len(), index + 1);
        assert_eq!(requests.last().unwrap().path.as_deref(), Some(command));
        drop(requests);
        let classifier_block =
            result.is_error && result.content.contains("classified as Critical risk");
        let valid = if blocked {
            classifier_block
                && spawned == previous_spawns
                && global_registry().list().len() == before_tasks
        } else {
            !result.is_error && spawned == previous_spawns + 1
        };
        if index == 0 {
            assert!(
                valid,
                "fake executor smoke check failed: {}",
                result.content
            );
        }
        if !valid {
            failures.push(format!(
                "{command:?}: blocked={classifier_block}, spawns={}, result={}",
                spawned - previous_spawns,
                result.content
            ));
        }
        previous_spawns = spawned;
    }
    clear_session_shell_state(&context.session_id);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
