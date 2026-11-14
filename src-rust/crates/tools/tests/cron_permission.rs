//! A scheduled prompt runs tools later, so scheduling one needs the same
//! permission as running a command now. Each case runs in a child process with
//! its own engine home, because the cron store and its file are process-wide.
use claurst_core::config::{Config, PermissionMode, Settings};
use claurst_core::permissions::{AutoPermissionHandler, PermissionManager};
use claurst_tools::{
    cron::{CronCreateTool, CronDeleteTool, CronListTool, CronTask},
    Tool, ToolContext,
};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn run_case(case: &str) {
    let home = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cron_permission_child",
            "--ignored",
            "--nocapture",
        ])
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("COVEN_CRON_PERMISSION_CASE", case)
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("cron-permission-fixture-complete"));
}

/// The context a headless run builds: a manager-backed auto handler in `mode`.
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
        session_id: "cron-permission-fixture".into(),
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

fn planted_task() -> CronTask {
    CronTask {
        id: "planted".into(),
        cron: "* * * * *".into(),
        prompt: "fixture".into(),
        recurring: true,
        durable: true,
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    }
}

#[tokio::test]
#[ignore = "run by isolated parent with temporary engine home"]
async fn cron_permission_child() {
    let case = std::env::var("COVEN_CRON_PERMISSION_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let path = home.join("scheduled_tasks.json");
    let create = serde_json::json!({"cron":"* * * * *", "prompt":"fixture", "durable":true});
    match case.as_str() {
        "read-only" | "default" => {
            let mode = if case == "read-only" {
                PermissionMode::Plan
            } else {
                PermissionMode::Default
            };
            let ctx = headless_context(mode, &home);
            for input in [
                create.clone(),
                serde_json::json!({"cron":"* * * * *", "prompt":"fixture"}),
            ] {
                let result = CronCreateTool.execute(input, &ctx).await;
                assert!(result.is_error, "scheduled in {case}: {}", result.content);
                assert!(
                    result.content.contains("Permission denied"),
                    "{}",
                    result.content
                );
            }
            assert!(!path.exists(), "a refused schedule must not reach disk");
            let list = CronListTool.execute(serde_json::json!({}), &ctx).await;
            assert!(!list.is_error, "listing stays allowed: {}", list.content);
            assert_eq!(list.content, "No scheduled cron tasks.");
        }
        "read-only-delete" => {
            let saved = serde_json::to_vec(&vec![planted_task()]).unwrap();
            std::fs::write(&path, &saved).unwrap();
            let ctx = headless_context(PermissionMode::Plan, &home);
            let list = CronListTool.execute(serde_json::json!({}), &ctx).await;
            assert!(list.content.contains("planted"), "{}", list.content);
            let result = CronDeleteTool
                .execute(serde_json::json!({"id":"planted"}), &ctx)
                .await;
            assert!(result.is_error, "deleted in read-only: {}", result.content);
            assert!(
                result.content.contains("Permission denied"),
                "{}",
                result.content
            );
            assert_eq!(std::fs::read(&path).unwrap(), saved);
            let list = CronListTool.execute(serde_json::json!({}), &ctx).await;
            assert!(list.content.contains("planted"), "{}", list.content);
        }
        "full" => {
            let ctx = headless_context(PermissionMode::BypassPermissions, &home);
            let result = CronCreateTool.execute(create, &ctx).await;
            assert!(!result.is_error, "{}", result.content);
            let stored: Vec<CronTask> =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(stored.len(), 1);
            let deleted = CronDeleteTool
                .execute(serde_json::json!({"id": stored[0].id}), &ctx)
                .await;
            assert!(!deleted.is_error, "{}", deleted.content);
            let stored: Vec<CronTask> =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert!(stored.is_empty());
        }
        _ => panic!("unknown fixture case"),
    }
    println!("cron-permission-fixture-complete");
}

#[test]
fn read_only_turn_cannot_schedule_a_prompt() {
    run_case("read-only");
}

#[test]
fn headless_default_turn_cannot_schedule_a_prompt() {
    run_case("default");
}

#[test]
fn read_only_turn_cannot_delete_a_scheduled_prompt() {
    run_case("read-only-delete");
}

#[test]
fn full_permission_turn_schedules_and_deletes() {
    run_case("full");
}
