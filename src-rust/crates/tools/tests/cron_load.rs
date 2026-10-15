//! Exercise initialization errors and retry through public cron tools.
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
        .args(["--exact", "cron_load_child", "--ignored", "--nocapture"])
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("COVEN_CRON_LOAD_CASE", case)
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("cron-load-fixture-complete"));
}

#[tokio::test]
#[ignore = "run by isolated parent with temporary engine home"]
async fn cron_load_child() {
    let case = std::env::var("COVEN_CRON_LOAD_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let ctx = ToolContext {
        working_dir: home.clone(),
        permission_mode: PermissionMode::Default,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::Default,
        }),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: "cron-load-fixture".into(),
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

    let path = home.join("scheduled_tasks.json");
    let stored = serde_json::to_vec(&vec![CronTask {
        id: "old-task".into(),
        cron: "* * * * *".into(),
        prompt: "stored".into(),
        recurring: true,
        durable: true,
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    }])
    .unwrap();
    if case == "missing" {
        assert_eq!(
            CronListTool
                .execute(serde_json::json!({}), &ctx)
                .await
                .content,
            "No scheduled cron tasks."
        );
        let created = CronCreateTool
            .execute(
                serde_json::json!({"cron":"* * * * *", "prompt":"new", "durable":true}),
                &ctx,
            )
            .await;
        assert!(!created.is_error, "{}", created.content);
        let tasks: Vec<CronTask> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].prompt, "new");
    } else {
        if case == "corrupt" {
            std::fs::write(&path, b"{broken").unwrap();
        } else {
            assert_eq!(case, "directory");
            std::fs::create_dir(&path).unwrap();
        }
        let cause = if case == "corrupt" {
            serde_json::from_slice::<Vec<CronTask>>(b"{broken")
                .unwrap_err()
                .to_string()
        } else {
            std::fs::read_to_string(&path).unwrap_err().to_string()
        };
        for result in [
            CronListTool.execute(serde_json::json!({}), &ctx).await,
            CronDeleteTool
                .execute(serde_json::json!({"id":"old-task"}), &ctx)
                .await,
            CronCreateTool
                .execute(
                    serde_json::json!({"cron":"* * * * *", "prompt":"new", "durable":true}),
                    &ctx,
                )
                .await,
        ] {
            assert!(result.is_error, "{}", result.content);
            assert!(result.content.contains(&cause), "{}", result.content);
            assert!(
                result.content.contains(path.to_str().unwrap()),
                "{}",
                result.content
            );
        }
        assert!(claurst_tools::cron::pop_due_tasks(&chrono::Local::now())
            .await
            .is_empty());
        if case == "corrupt" {
            assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
        } else {
            assert!(path.is_dir());
            std::fs::remove_dir(&path).unwrap();
        }
        std::fs::write(&path, &stored).unwrap();
        let listed = CronListTool.execute(serde_json::json!({}), &ctx).await;
        assert!(!listed.is_error, "{}", listed.content);
        assert!(listed.content.contains("old-task"));
        assert!(!listed.content.contains("prompt: new"));
        let due = claurst_tools::cron::pop_due_tasks(&chrono::Local::now()).await;
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "old-task");
        assert_eq!(std::fs::read(&path).unwrap(), stored);
    }
    println!("cron-load-fixture-complete");
}

#[test]
fn malformed_storage_blocks_mutation_and_retries_after_repair() {
    run_case("corrupt");
}
#[test]
fn read_error_blocks_operations_and_retries_after_repair() {
    run_case("directory");
}
#[test]
fn missing_storage_still_initializes_empty() {
    run_case("missing");
}
