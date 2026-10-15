//! Exercise atomic storage replacement through public cron tools.
use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{
    cron::{CronCreateTool, CronListTool, CronTask},
    Tool, ToolContext,
};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn run_case(case: &str) {
    let fixture = tempfile::tempdir().unwrap();
    let home = fixture.path().join("home");
    std::fs::create_dir(&home).unwrap();
    #[cfg(unix)]
    let home = if case == "home-alias" {
        let alias = fixture.path().join("home-alias");
        std::os::unix::fs::symlink(&home, &alias).unwrap();
        alias
    } else {
        home
    };
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cron_atomic_child", "--ignored", "--nocapture"])
        .env("COVEN_CODE_HOME", &home)
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("COVEN_CRON_ATOMIC_CASE", case)
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("cron-atomic-fixture-complete"));
}

#[tokio::test]
#[ignore = "run by isolated parent with temporary engine home"]
async fn cron_atomic_child() {
    let case = std::env::var("COVEN_CRON_ATOMIC_CASE").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let ctx = ToolContext {
        working_dir: home.clone(),
        permission_mode: PermissionMode::Default,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::Default,
        }),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: "cron-atomic-fixture".into(),
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
    let original = serde_json::to_vec(&vec![CronTask {
        id: "old-task".into(),
        cron: "* * * * *".into(),
        prompt: "old".into(),
        recurring: true,
        durable: true,
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    }])
    .unwrap();
    let alias = home.join("old-alias.json");
    match case.as_str() {
        "new" => {}
        "existing" | "hardlink" => {
            std::fs::write(&path, &original).unwrap();
            if case == "hardlink" {
                std::fs::hard_link(&path, &alias).unwrap();
            }
        }
        "directory" => {
            // Initialize first so this case exercises failed snapshot replacement.
            let initial = CronListTool.execute(serde_json::json!({}), &ctx).await;
            assert!(!initial.is_error, "{}", initial.content);
            assert_eq!(initial.content, "No scheduled cron tasks.");
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("sentinel"), b"untouched").unwrap();
        }
        #[cfg(unix)]
        "symlink" => {
            std::fs::write(&alias, &original).unwrap();
            std::os::unix::fs::symlink(&alias, &path).unwrap();
        }
        #[cfg(unix)]
        "home-alias" => {
            // The parent supplies a symlinked engine home for this case.
            assert!(std::fs::symlink_metadata(&home)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        _ => panic!("unknown case"),
    }
    let result = CronCreateTool
        .execute(
            serde_json::json!({
                "cron":"* * * * *", "prompt":"new", "durable":true
            }),
            &ctx,
        )
        .await;
    if case == "directory" {
        // Error reporting is a separate unit; this tests storage and staging cleanup.
        assert!(path.is_dir());
        assert_eq!(std::fs::read(path.join("sentinel")).unwrap(), b"untouched");
    } else {
        assert!(!result.is_error, "{}", result.content);
        let tasks: Vec<CronTask> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            tasks.len(),
            if matches!(case.as_str(), "existing" | "hardlink" | "symlink") {
                2
            } else {
                1
            }
        );
        assert!(tasks
            .iter()
            .any(|task| task.prompt == "new" && task.durable));
        if matches!(case.as_str(), "hardlink" | "symlink") {
            assert_eq!(
                std::fs::read(&alias).unwrap(),
                original,
                "replacement must not overwrite aliases"
            );
        }
        assert!(!std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    let mut names: Vec<_> = std::fs::read_dir(&home)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    let mut expected = vec![std::ffi::OsString::from("scheduled_tasks.json")];
    if matches!(case.as_str(), "hardlink" | "symlink") {
        expected.push("old-alias.json".into());
    }
    expected.sort();
    assert_eq!(names, expected, "staging files must be removed");
    println!("cron-atomic-fixture-complete");
}

#[test]
fn new_file() {
    run_case("new");
}
#[test]
fn existing_file() {
    run_case("existing");
}
#[test]
fn hardlink_preserves_old_bytes() {
    run_case("hardlink");
}
#[test]
fn directory_failure_cleans_staging() {
    run_case("directory");
}
#[cfg(unix)]
#[test]
fn destination_symlink_is_replaced() {
    run_case("symlink");
}
#[cfg(unix)]
#[test]
fn configured_home_symlink_is_supported() {
    run_case("home-alias");
}
