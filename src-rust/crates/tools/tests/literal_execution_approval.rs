#![cfg(unix)]

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::{PermissionDecision, PermissionHandler, PermissionRequest};
use claurst_tools::{find_tool, ToolContext};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{atomic::AtomicUsize, Arc, Mutex};
use std::time::{Duration, Instant};

const FAKE_GIT: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$LITERAL_GIT_LOG"
if [ "$1" = "rev-parse" ]; then
  printf '%s\n' '0123456789abcdef'
elif [ "$1" = "worktree" ] && [ "$2" = "add" ]; then
  /bin/mkdir -p "$5"
fi
"#;

const FAKE_SH: &str = r#"#!/bin/sh
printf '%s' "$2" > "$LITERAL_COMMAND_LOG"
printf '%s\n' 'post-create-ok'
"#;

const FAKE_REPL: &str = r#"#!/bin/sh
printf '%s\n' "$$" >> "$LITERAL_REPL_SPAWN_LOG"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LITERAL_REPL_INPUT_LOG"
  case "$line" in
    *"__REPL_DONE_7f3a9b__"*) printf '%s\n' '__REPL_DONE_7f3a9b__' ;;
  esac
done
"#;

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
struct SequencedHandler {
    decisions: Mutex<Vec<PermissionDecision>>,
    requests: Mutex<Vec<PermissionRequest>>,
}

impl SequencedHandler {
    fn new(decisions: Vec<PermissionDecision>) -> Self {
        Self {
            decisions: Mutex::new(decisions.into_iter().rev().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl PermissionHandler for SequencedHandler {
    fn check_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.requests.lock().unwrap().push(request.clone());
        self.decisions
            .lock()
            .unwrap()
            .pop()
            .unwrap_or(PermissionDecision::Allow)
    }

    fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.check_permission(request)
    }
}

fn context(root: &Path, session: &str, handler: Arc<dyn PermissionHandler>) -> ToolContext {
    ToolContext {
        working_dir: root.to_path_buf(),
        permission_mode: PermissionMode::Default,
        permission_handler: handler,
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: session.to_string(),
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
    }
}

fn run_isolated(child_test: &str) {
    let fixture = tempfile::tempdir().unwrap();
    let bin = fixture.path().join("bin");
    fs::create_dir(&bin).unwrap();
    for (name, contents) in [
        ("git", FAKE_GIT),
        ("sh", FAKE_SH),
        ("bash", FAKE_REPL),
        ("python3", FAKE_REPL),
        ("node", FAKE_REPL),
    ] {
        let path = bin.join(name);
        fs::write(&path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    for name in ["code", "coven", "claude"] {
        fs::create_dir(fixture.path().join(name)).unwrap();
    }
    fs::write(
        fixture.path().join("code/auth.json"),
        "{\"credentials\":{}}\n",
    )
    .unwrap();

    let output_path = fixture.path().join("child.log");
    let output = fs::File::create(&output_path).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", child_test, "--nocapture", "--test-threads=1"])
        .current_dir(fixture.path())
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .process_group(0);
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.contains("API_KEY")
            || (name.starts_with("COVEN") && (name.contains("TEST") || name.contains("FIXTURE")))
            || name.starts_with("BASH_FUNC_")
            || ["BASH_ENV", "ENV", "CDPATH", "SHELLOPTS", "BASHOPTS"].contains(&name.as_ref())
        {
            command.env_remove(key);
        }
    }
    command
        .env("PATH", &bin)
        .env("LITERAL_FIXTURE_ROOT", fixture.path())
        .env("LITERAL_GIT_LOG", fixture.path().join("git.log"))
        .env(
            "LITERAL_COMMAND_LOG",
            fixture.path().join("post-command.log"),
        )
        .env(
            "LITERAL_REPL_SPAWN_LOG",
            fixture.path().join("repl-spawn.log"),
        )
        .env(
            "LITERAL_REPL_INPUT_LOG",
            fixture.path().join("repl-input.log"),
        )
        .env("COVEN_CODE_HOME", fixture.path().join("code"))
        .env("COVEN_HOME", fixture.path().join("coven"))
        .env("CLAUDE_CONFIG_DIR", fixture.path().join("claude"))
        .env(
            "COVEN_CODE_CLAUDE_BIN",
            fixture.path().join("absent-claude"),
        )
        .env("COVEN_DAEMON_SOCKET", fixture.path().join("absent.sock"))
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
    let output = fs::read_to_string(output_path).unwrap();
    assert!(status.is_some_and(|value| value.success()), "{output}");
}

#[test]
fn literal_execution_approval_boundaries() {
    for child in [
        "worktree_execution_denial_child",
        "worktree_execution_allow_child",
        "repl_submission_approval_child",
        "repl_bash_critical_child",
        "repl_non_bash_literal_child",
    ] {
        run_isolated(child);
    }
}

fn fixture_root() -> Option<PathBuf> {
    std::env::var_os("LITERAL_FIXTURE_ROOT").map(PathBuf::from)
}

#[tokio::test]
async fn worktree_execution_denial_child() {
    let Some(root) = fixture_root() else {
        return;
    };
    let command = "printf harmless\nprintf second-line";
    let handler = Arc::new(SequencedHandler::new(vec![
        PermissionDecision::Allow,
        PermissionDecision::Deny,
        PermissionDecision::Allow,
    ]));
    let ctx = context(&root, "worktree-denial", handler.clone());
    let tool = find_tool("EnterWorktree").unwrap();
    let denied = tool
        .execute(
            serde_json::json!({
                "branch": "denied-branch",
                "path": "denied-tree",
                "post_create_command": command,
            }),
            &ctx,
        )
        .await;
    assert!(denied.is_error);
    assert!(!root.join("git.log").exists());
    assert!(!root.join("post-command.log").exists());
    assert!(!root.join("denied-tree").exists());
    {
        let requests = handler.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].tool_name, "EnterWorktree");
        assert_eq!(requests[0].description, "Create a git worktree");
        assert_eq!(requests[1].path.as_deref(), Some(command));
        assert!(requests[1].details.as_deref().unwrap().contains(command));
        assert!(requests[1]
            .details
            .as_deref()
            .unwrap()
            .contains(&root.join("denied-tree").display().to_string()));
    }

    let allowed = tool
        .execute(
            serde_json::json!({"branch": "later-branch", "path": "later-tree"}),
            &ctx,
        )
        .await;
    assert!(!allowed.is_error, "{}", allowed.content);
}

#[tokio::test]
async fn worktree_execution_allow_child() {
    let Some(root) = fixture_root() else {
        return;
    };
    let command = "printf literal-one\nprintf literal-two";
    let handler = Arc::new(SequencedHandler::new(vec![
        PermissionDecision::Allow,
        PermissionDecision::Allow,
    ]));
    let ctx = context(&root, "worktree-allow", handler.clone());
    let result = find_tool("EnterWorktree")
        .unwrap()
        .execute(
            serde_json::json!({
                "branch": "allowed-branch",
                "path": "allowed-tree",
                "post_create_command": command,
            }),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        fs::read_to_string(root.join("post-command.log")).unwrap(),
        command
    );
    let requests = handler.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path.as_deref(), Some(command));
    assert!(requests[1]
        .details
        .as_deref()
        .unwrap()
        .contains(&root.join("allowed-tree").display().to_string()));
}

#[tokio::test]
async fn repl_submission_approval_child() {
    let Some(root) = fixture_root() else {
        return;
    };
    let first = "printf harmless-first\nprintf harmless-second";
    let denied = "printf denied-input";
    let last = "printf harmless-last";
    let handler = Arc::new(SequencedHandler::new(vec![
        PermissionDecision::Allow,
        PermissionDecision::Deny,
        PermissionDecision::Allow,
    ]));
    let ctx = context(&root, "repl-reuse", handler.clone());
    let tool = find_tool("REPL").unwrap();
    assert!(
        !tool
            .execute(serde_json::json!({"language": "bash", "code": first}), &ctx)
            .await
            .is_error
    );
    let before_denial = fs::read(root.join("repl-input.log")).unwrap();
    assert!(
        tool.execute(
            serde_json::json!({"language": "bash", "code": denied}),
            &ctx,
        )
        .await
        .is_error
    );
    assert_eq!(
        fs::read(root.join("repl-input.log")).unwrap(),
        before_denial
    );
    assert!(
        !tool
            .execute(serde_json::json!({"language": "bash", "code": last}), &ctx)
            .await
            .is_error
    );
    assert_eq!(
        fs::read_to_string(root.join("repl-spawn.log"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let requests = handler.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    for (request, code) in requests.iter().zip([first, denied, last]) {
        assert_eq!(request.path.as_deref(), Some(code));
        let details = request.details.as_deref().unwrap();
        assert!(details.contains(code));
        assert!(details.contains("bash"));
        assert!(details.contains("repl-reuse"));
    }
}

#[tokio::test]
async fn repl_bash_critical_child() {
    let Some(root) = fixture_root() else {
        return;
    };
    let handler = Arc::new(SequencedHandler::new(vec![PermissionDecision::Allow]));
    let ctx = context(&root, "repl-critical", handler.clone());
    let result = find_tool("REPL")
        .unwrap()
        .execute(
            serde_json::json!({"language": "bash", "code": "echo harmless; rm -rf /"}),
            &ctx,
        )
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("Critical risk"));
    assert!(!root.join("repl-spawn.log").exists());
    assert!(!root.join("repl-input.log").exists());
}

#[tokio::test]
async fn repl_non_bash_literal_child() {
    let Some(root) = fixture_root() else {
        return;
    };
    let bash_looking = "text = 'echo harmless; rm -rf /'";
    for (index, language) in ["python", "javascript"].into_iter().enumerate() {
        let handler = Arc::new(SequencedHandler::new(vec![PermissionDecision::Allow]));
        let ctx = context(&root, &format!("non-bash-{index}"), handler.clone());
        let result = find_tool("REPL")
            .unwrap()
            .execute(
                serde_json::json!({"language": language, "code": bash_looking}),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.content);
        let requests = handler.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path.as_deref(), Some(bash_looking));
        assert!(requests[0].details.as_deref().unwrap().contains(language));
    }
}
