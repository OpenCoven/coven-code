//! An interactive session starts in the saved permission mode, so the `Config`
//! tool may only change that mode with the approval any other write needs.
//! Each case runs in a child process with its own engine home, because the
//! settings file is found through process-wide environment.
use claurst_core::config::{Config, PermissionMode, Settings};
use claurst_core::permissions::{
    AutoPermissionHandler, PermissionDecision, PermissionHandler, PermissionManager,
    PermissionRequest,
};
use claurst_tools::{config_tool::ConfigTool, Tool, ToolContext};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::{Duration, Instant};

fn run_case(case: &str) {
    let home = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "config_permission_child",
            "--ignored",
            "--nocapture",
        ])
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("COVEN_CONFIG_PERMISSION_CASE", case)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "fixture timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("config-permission-fixture-complete"));
}

/// The context a headless run builds: a manager-backed auto handler in `mode`,
/// which refuses whatever an interactive session would ask about.
fn headless_context(mode: PermissionMode, home: &std::path::Path) -> ToolContext {
    let manager = Arc::new(std::sync::Mutex::new(PermissionManager::new(
        mode.clone(),
        &Settings::default(),
    )));
    ToolContext {
        working_dir: home.to_path_buf(),
        permission_mode: mode,
        permission_handler: Arc::new(AutoPermissionHandler::with_manager(manager.clone())),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: "config-permission-fixture".into(),
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
        permission_manager: Some(manager),
        user_question_tx: None,
    }
}

/// Approves, and changes the settings file while the tool waits for the
/// answer, as "always allow" does when it saves its rule.
struct SavesWhileAsked(std::path::PathBuf);

impl PermissionHandler for SavesWhileAsked {
    fn check_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.request_permission(request)
    }

    fn request_permission(&self, _request: &PermissionRequest) -> PermissionDecision {
        std::fs::write(
            self.0.join("settings.json"),
            r#"{"config": {"model": "saved-during-approval"}}"#,
        )
        .unwrap();
        PermissionDecision::Allow
    }
}

fn set(setting: &str, value: &str) -> serde_json::Value {
    serde_json::json!({"setting": setting, "value": value})
}

#[tokio::test]
#[ignore = "run by isolated parent with temporary engine home"]
async fn config_permission_child() {
    let case = std::env::var("COVEN_CONFIG_PERMISSION_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let saved_mode = || Settings::load_sync().unwrap().config.permission_mode;
    match case.as_str() {
        "unapproved" => {
            for mode in [
                PermissionMode::Default,
                PermissionMode::AcceptEdits,
                PermissionMode::Plan,
            ] {
                let ctx = headless_context(mode.clone(), &home);
                for value in ["accept_edits", "bypass_permissions", "plan", "default"] {
                    let result = ConfigTool
                        .execute(set("permission_mode", value), &ctx)
                        .await;
                    assert!(
                        result.is_error && result.content.contains("Permission denied"),
                        "{value} in {mode:?}: {}",
                        result.content
                    );
                }
                assert_eq!(saved_mode(), PermissionMode::Default, "in {mode:?}");
            }
            assert!(
                !home.join("settings.json").exists(),
                "a refused change must not reach disk"
            );
            // Reading the mode, and the other settings, are as before.
            let ctx = headless_context(PermissionMode::Default, &home);
            let read = ConfigTool
                .execute(serde_json::json!({"setting": "permission_mode"}), &ctx)
                .await;
            assert_eq!(read.content, "permission_mode = \"default\"");
            let model = ConfigTool
                .execute(set("model", "fixture-model"), &ctx)
                .await;
            assert!(!model.is_error, "{}", model.content);
            // A value that is not a mode is still reported as such.
            let unknown = ConfigTool
                .execute(set("permission_mode", "yolo"), &ctx)
                .await;
            assert!(
                unknown.content.contains("Unknown permission_mode"),
                "{}",
                unknown.content
            );
        }
        "approved" => {
            let ctx = headless_context(PermissionMode::BypassPermissions, &home);
            let result = ConfigTool
                .execute(set("permission_mode", "accept_edits"), &ctx)
                .await;
            assert!(!result.is_error, "{}", result.content);
            assert_eq!(saved_mode(), PermissionMode::AcceptEdits);
        }
        "saved-during-approval" => {
            let mut ctx = headless_context(PermissionMode::Default, &home);
            ctx.permission_handler = Arc::new(SavesWhileAsked(home.clone()));
            let result = ConfigTool
                .execute(set("permission_mode", "accept_edits"), &ctx)
                .await;
            assert!(!result.is_error, "{}", result.content);
            let settings = Settings::load_sync().unwrap();
            assert_eq!(settings.config.permission_mode, PermissionMode::AcceptEdits);
            assert_eq!(
                settings.config.model.as_deref(),
                Some("saved-during-approval"),
                "what was saved during the approval must survive the change"
            );
        }
        _ => panic!("unknown fixture case"),
    }
    println!("config-permission-fixture-complete");
}

#[test]
fn the_saved_permission_mode_cannot_be_changed_without_approval() {
    run_case("unapproved");
}

#[test]
fn an_approved_change_to_the_saved_permission_mode_is_saved() {
    run_case("approved");
}

#[test]
fn an_approved_change_keeps_what_was_saved_while_it_waited_for_approval() {
    run_case("saved-during-approval");
}
