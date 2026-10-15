//! Durable one-shot dequeue persistence through public scheduler APIs.
use chrono::{Local, TimeZone};
use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{
    cron::{pop_due_tasks, CronCreateTool, CronListTool, CronTask},
    Tool, ToolContext,
};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn seed(home: &std::path::Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let tasks: Vec<_> = [("once", false), ("recurring", true), ("later", false)]
        .into_iter()
        .map(|(id, recurring)| CronTask {
            id: id.into(),
            cron: if id == "later" {
                "0 13 * * *"
            } else {
                "* * * * *"
            }
            .into(),
            prompt: id.into(),
            recurring,
            durable: true,
            created_at: now,
        })
        .collect();
    std::fs::write(
        home.join("scheduled_tasks.json"),
        serde_json::to_vec(&tasks).unwrap(),
    )
    .unwrap();
}

fn run_case(home: &std::path::Path, case: &str) {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cron_once_child", "--ignored", "--nocapture"])
        .env("COVEN_CODE_HOME", home)
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("COVEN_CRON_ONCE_CASE", case)
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("cron-once-fixture-complete"));
}

#[test]
fn durable_once_does_not_replay_after_restart() {
    let home = tempfile::tempdir().unwrap();
    seed(home.path());
    run_case(home.path(), "first");
    run_case(home.path(), "restart");
}

#[test]
fn failed_removal_retains_once_and_allows_other_due_tasks() {
    let home = tempfile::tempdir().unwrap();
    seed(home.path());
    run_case(home.path(), "failure-retry");
    run_case(home.path(), "restart");
}

#[tokio::test]
#[ignore = "run by isolated parent with temporary engine home"]
async fn cron_once_child() {
    let case = std::env::var("COVEN_CRON_ONCE_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let ctx = ToolContext {
        working_dir: home.clone(),
        permission_mode: PermissionMode::Default,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::Default,
        }),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: "cron-once-fixture".into(),
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
    let date = Local
        .with_ymd_and_hms(2026, 1, 4, 12, 10, 0)
        .single()
        .unwrap();
    match case.as_str() {
        "first" => {
            let mut ids: Vec<_> = pop_due_tasks(&date)
                .await
                .into_iter()
                .map(|task| task.id)
                .collect();
            ids.sort();
            assert_eq!(ids, ["once", "recurring"]);
        }
        "restart" => {
            let tasks = pop_due_tasks(&date).await;
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].id, "recurring");
        }
        "failure-retry" => {
            assert!(CronListTool
                .execute(serde_json::json!({}), &ctx)
                .await
                .content
                .contains("once"));
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("sentinel"), b"untouched").unwrap();
            let created = CronCreateTool.execute(serde_json::json!({"cron":"* * * * *", "prompt":"session-only", "recurring":false}), &ctx).await;
            assert!(!created.is_error, "{}", created.content);
            let tasks = pop_due_tasks(&date).await;
            assert_eq!(
                tasks.len(),
                2,
                "durable once must be withheld on persistence error"
            );
            assert!(tasks.iter().any(|task| task.id == "recurring"));
            assert!(tasks
                .iter()
                .any(|task| task.prompt == "session-only" && !task.durable));
            let remaining = CronListTool
                .execute(serde_json::json!({}), &ctx)
                .await
                .content;
            assert!(remaining.contains("once"));
            assert!(!remaining.contains("session-only"));
            assert_eq!(std::fs::read(path.join("sentinel")).unwrap(), b"untouched");
            std::fs::remove_file(path.join("sentinel")).unwrap();
            std::fs::remove_dir(&path).unwrap();
            let mut ids: Vec<_> = pop_due_tasks(&date)
                .await
                .into_iter()
                .map(|task| task.id)
                .collect();
            ids.sort();
            assert_eq!(ids, ["once", "recurring"]);
        }
        _ => panic!("unknown fixture"),
    }
    if case != "first" {
        let stored: Vec<CronTask> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut stored_ids: Vec<_> = stored.into_iter().map(|task| task.id).collect();
        stored_ids.sort();
        assert_eq!(stored_ids, ["later", "recurring"]);
    }
    println!("cron-once-fixture-complete");
}
