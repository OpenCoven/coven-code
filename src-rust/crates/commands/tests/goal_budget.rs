use claurst_commands::{CommandContext, CommandResult, GoalCommand, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker, GoalStore};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn invalid_explicit_goal_budgets_preserve_existing_goal() {
    run_cases(&[
        "overflow_k",
        "overflow_m",
        "overflow_integer",
        "invalid",
        "negative",
        "over_storage",
    ]);
}
#[test]
fn valid_goal_budgets_preserve_existing_semantics() {
    run_cases(&["zero", "plain", "k", "m", "none", "max_storage"]);
}

fn run_cases(cases: &[&str]) {
    let mut failures = Vec::new();
    for case in cases {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&project).unwrap();
        let log = temp.path().join("stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "goal_budget_worker", "--ignored", "--nocapture"])
            .env_clear()
            .env("COVEN_CODE_HOME", &home)
            .env("GOAL_FIXTURE_PROJECT", &project)
            .env("GOAL_FIXTURE_CASE", case)
            .env("GOAL_FIXTURE_DONE", temp.path().join("done"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{case}: child wait failed: {result:?}");
                }
            }
        };
        if outcome.success() {
            assert_eq!(std::fs::read(temp.path().join("done")).unwrap(), b"checked");
        } else {
            failures.push(format!("{case}: {}", std::fs::read_to_string(log).unwrap()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "owned process configuration and goal database"]
fn goal_budget_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("GOAL_FIXTURE_PROJECT").unwrap());
    let case = std::env::var("GOAL_FIXTURE_CASE").unwrap();
    assert_eq!(claurst_core::config::config_home(), home);
    assert_eq!(GoalStore::default_path(), home.join("goals.sqlite"));
    let (argument, expected) = match case.as_str() {
        "overflow_k" => ("--tokens 18446744073709552K replacement", None),
        "overflow_m" => ("--tokens 18446744073710M replacement", None),
        "overflow_integer" => ("--tokens 18446744073709551616 replacement", None),
        "invalid" => ("--tokens nonsense replacement", None),
        "negative" => ("--tokens -1 replacement", None),
        "over_storage" => ("--tokens 9223372036854775808 replacement", None),
        "zero" => ("--tokens 0 replacement", Some(Some(0))),
        "plain" => ("--tokens 42 replacement", Some(Some(42))),
        "k" => ("--tokens 25k replacement", Some(Some(25_000))),
        "m" => ("--tokens 2M replacement", Some(Some(2_000_000))),
        "none" => ("replacement", Some(None)),
        "max_storage" => (
            "--tokens 9223372036854775807 replacement",
            Some(Some(i64::MAX as u64)),
        ),
        _ => panic!("unknown case"),
    };
    let store = GoalStore::open_default().unwrap();
    let before = store
        .set_goal("fixture-session", "original", Some(100))
        .unwrap();
    let mut context = CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: vec![],
        working_dir: project,
        session_id: "fixture-session".into(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(GoalCommand.execute(argument, &mut context));
    let after = store.try_get_goal("fixture-session").unwrap().unwrap();
    if case == "zero" {
        // A zero budget is exhausted on creation: the goal is replaced, but no
        // goal turn can be captured for it, so the command reports that.
        assert!(
            matches!(&result, CommandResult::Error(message) if message.contains("Goal is not active")),
            "{result:?}"
        );
        assert_eq!(after.objective, "replacement");
        assert_eq!(after.token_budget, Some(0));
        assert_ne!(after.id, before.id);
    } else if let Some(budget) = expected {
        assert!(
            matches!(
                result,
                CommandResult::UserMessage(_) | CommandResult::GoalUserMessage { .. }
            ),
            "{result:?}"
        );
        assert_eq!(after.objective, "replacement");
        assert_eq!(after.token_budget, budget);
        assert_ne!(after.id, before.id);
    } else {
        assert!(
            matches!(result, CommandResult::Error(_)),
            "invalid budget accepted: {result:?}"
        );
        assert_eq!(after.id, before.id);
        assert_eq!(after.objective, before.objective);
        assert_eq!(after.token_budget, before.token_budget);
        assert_eq!(after.updated_at_ms, before.updated_at_ms);
    }
    std::fs::write(std::env::var_os("GOAL_FIXTURE_DONE").unwrap(), b"checked").unwrap();
}
