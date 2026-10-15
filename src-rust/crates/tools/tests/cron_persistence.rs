//! Exercise persistence errors and successful writes through public cron tools.
use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
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
            "cron_persistence_child",
            "--ignored",
            "--nocapture",
        ])
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("COVEN_CRON_PERSISTENCE_CASE", case)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("cron-persistence-fixture-complete"));
}

#[tokio::test]
#[ignore = "run by isolated parent with temporary engine home"]
async fn cron_persistence_child() {
    let case = std::env::var("COVEN_CRON_PERSISTENCE_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let ctx = ToolContext {
        working_dir: home.clone(),
        permission_mode: PermissionMode::Default,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::Default,
        }),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: "cron-persistence-fixture".into(),
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

    let path = home.join("scheduled_tasks.json");
    match case.as_str() {
        "create-failure" => {
            // Load the absent file first so the fixture isolates a write failure.
            let initial = CronListTool.execute(serde_json::json!({}), &ctx).await;
            assert!(!initial.is_error, "{}", initial.content);
            assert_eq!(initial.content, "No scheduled cron tasks.");
            std::fs::create_dir(&path).unwrap();
            let io_cause = std::fs::write(&path, b"fixture").unwrap_err().to_string();
            let result = CronCreateTool
                .execute(
                    serde_json::json!({"cron":"* * * * *", "prompt":"fixture", "durable":true}),
                    &ctx,
                )
                .await;
            assert!(result.is_error, "{}", result.content);
            assert!(result.content.contains(&io_cause), "{}", result.content);
            assert!(result.content.contains("create"), "{}", result.content);
            assert!(
                result.content.contains(path.to_str().unwrap()),
                "{}",
                result.content
            );
            assert_eq!(
                CronListTool
                    .execute(serde_json::json!({}), &ctx)
                    .await
                    .content,
                "No scheduled cron tasks."
            );
            assert!(path.is_dir());
        }
        "delete-failure" => {
            let task = CronTask {
                id: "owned-task".into(),
                cron: "* * * * *".into(),
                prompt: "fixture".into(),
                recurring: true,
                durable: true,
                created_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            };
            std::fs::write(&path, serde_json::to_vec(&vec![task]).unwrap()).unwrap();
            let before = CronListTool.execute(serde_json::json!({}), &ctx).await;
            assert!(before.content.contains("owned-task"));
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
            let io_cause = std::fs::write(&path, b"fixture").unwrap_err().to_string();
            let result = CronDeleteTool
                .execute(serde_json::json!({"id":"owned-task"}), &ctx)
                .await;
            assert!(result.is_error, "{}", result.content);
            assert!(result.content.contains(&io_cause), "{}", result.content);
            assert!(result.content.contains("delete"), "{}", result.content);
            assert!(
                result.content.contains(path.to_str().unwrap()),
                "{}",
                result.content
            );
            assert_eq!(
                CronListTool
                    .execute(serde_json::json!({}), &ctx)
                    .await
                    .content,
                before.content
            );
            assert!(path.is_dir());
        }
        "durable-success" | "session-only" => {
            let durable = case == "durable-success";
            let result = CronCreateTool
                .execute(
                    serde_json::json!({"cron":"* * * * *", "prompt":"fixture", "durable":durable}),
                    &ctx,
                )
                .await;
            assert!(!result.is_error, "{}", result.content);
            let list = CronListTool.execute(serde_json::json!({}), &ctx).await;
            assert!(list.content.contains("prompt: fixture"));
            let id = list
                .content
                .lines()
                .nth(2)
                .unwrap()
                .split(" | ")
                .next()
                .unwrap();
            if durable {
                let stored: Vec<CronTask> =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                assert_eq!(stored.len(), 1);
                assert_eq!(stored[0].id, id);
                assert_eq!(stored[0].prompt, "fixture");
                assert_eq!(stored[0].cron, "* * * * *");
                assert!(stored[0].durable && stored[0].recurring);
            } else {
                assert!(!path.exists());
            }
            let deleted = CronDeleteTool
                .execute(serde_json::json!({"id":id}), &ctx)
                .await;
            assert!(!deleted.is_error, "{}", deleted.content);
            assert_eq!(
                CronListTool
                    .execute(serde_json::json!({}), &ctx)
                    .await
                    .content,
                "No scheduled cron tasks."
            );
            if durable {
                let stored: Vec<CronTask> =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                assert!(stored.is_empty());
            } else {
                assert!(!path.exists());
            }
        }
        _ => panic!("unknown fixture case"),
    }
    println!("cron-persistence-fixture-complete");
}

#[test]
fn create_failure_rolls_back() {
    run_case("create-failure");
}
#[test]
fn delete_failure_restores_task() {
    run_case("delete-failure");
}
#[test]
fn durable_success_round_trips() {
    run_case("durable-success");
}
#[test]
fn session_only_does_not_write() {
    run_case("session-only");
}
