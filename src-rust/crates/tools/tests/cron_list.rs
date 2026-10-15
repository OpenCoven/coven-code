//! Exercise persisted prompt display through the public CronList tool.
use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{
    cron::{CronListTool, CronTask},
    Tool, ToolContext,
};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn cron_list_preserves_utf8_boundaries() {
    let home = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cron_list_child", "--ignored", "--nocapture"])
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("COVEN_CRON_LIST_FIXTURE", "1")
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("cron-list-fixture-complete"));
}

#[tokio::test]
#[ignore = "run by isolated parent with temporary engine home"]
async fn cron_list_child() {
    assert_eq!(std::env::var("COVEN_CRON_LIST_FIXTURE").unwrap(), "1");
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let ctx = ToolContext {
        working_dir: home.clone(),
        permission_mode: PermissionMode::Default,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::Default,
        }),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: "cron-list-fixture".into(),
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
    let mut cases = vec![
        (String::new(), String::new()),
        ("short".into(), "short".into()),
        ("a".repeat(60), "a".repeat(60)),
        ("a".repeat(61), format!("{}…", "a".repeat(60))),
    ];
    for character in ['\u{e9}', '\u{20ac}', '\u{1f600}'] {
        let width = character.len_utf8();
        let exact = format!("{}{character}", "a".repeat(60 - width));
        cases.push((exact.clone(), exact.clone()));
        cases.push((format!("{exact}z"), format!("{exact}…")));
        for offset in 1..width {
            let prefix = "a".repeat(60 - offset);
            cases.push((format!("{prefix}{character}tail"), format!("{prefix}…")));
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let tasks: Vec<_> = cases
        .iter()
        .enumerate()
        .map(|(index, (prompt, _))| CronTask {
            id: format!("fixture-{index}"),
            cron: "* * * * *".into(),
            prompt: prompt.clone(),
            recurring: true,
            durable: true,
            created_at: now,
        })
        .collect();
    let stored = serde_json::to_vec(&tasks).unwrap();
    let path = home.join("scheduled_tasks.json");
    std::fs::write(&path, &stored).unwrap();
    let result = CronListTool.execute(serde_json::json!({}), &ctx).await;
    assert!(!result.is_error, "{}", result.content);
    assert!(result
        .content
        .starts_with(&format!("Scheduled tasks ({}):\n\n", tasks.len())));
    for (index, (_, expected)) in cases.iter().enumerate() {
        let line = format!("fixture-{index} | * * * * * | every minute | recurring=true | durable=true | prompt: {expected}");
        assert!(
            result.content.lines().any(|actual| actual == line),
            "missing {line:?} in {}",
            result.content
        );
    }
    assert_eq!(
        std::fs::read(path).unwrap(),
        stored,
        "listing must not mutate persisted prompts"
    );
    println!("cron-list-fixture-complete");
}
