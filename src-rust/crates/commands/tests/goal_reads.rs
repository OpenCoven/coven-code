use claurst_commands::{CommandContext, CommandResult, CovenCommand, GoalCommand, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker, GoalError, GoalStatus, GoalStore};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn corrupt_goal_reads_are_errors_without_mutation() {
    run_cases(&["status", "counter"]);
}

#[test]
fn missing_and_valid_goal_reads_keep_existing_behavior() {
    run_cases(&["missing", "valid"]);
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
            .args(["--exact", "goal_read_worker", "--ignored", "--nocapture"])
            .env_clear()
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("COVEN_CODE_HOME", &home)
            .env("COVEN_HOME", temp.path().join("coven"))
            .env("COVEN_CAVE_HOME", temp.path().join("cave"))
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawGoal {
    id: String,
    session_id: String,
    objective: String,
    status: String,
    token_budget: Option<i64>,
    tokens_used: i64,
    time_used_secs: i64,
    turns_used: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

fn snapshot(database: &rusqlite::Connection) -> Vec<RawGoal> {
    database
        .prepare(
            "SELECT id, session_id, objective, status, token_budget,
        tokens_used, time_used_secs, turns_used, created_at_ms, updated_at_ms
        FROM goals ORDER BY session_id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok(RawGoal {
                id: row.get(0)?,
                session_id: row.get(1)?,
                objective: row.get(2)?,
                status: row.get(3)?,
                token_budget: row.get(4)?,
                tokens_used: row.get(5)?,
                time_used_secs: row.get(6)?,
                turns_used: row.get(7)?,
                created_at_ms: row.get(8)?,
                updated_at_ms: row.get(9)?,
            })
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

async fn execute(coven: bool, argument: &str, context: &mut CommandContext) -> CommandResult {
    if coven {
        CovenCommand
            .execute(&format!("goal {argument}"), context)
            .await
    } else {
        GoalCommand.execute(argument, context).await
    }
}

fn missing_message(argument: &str) -> &'static str {
    match argument {
        "" | "status" => "No active goal. Set one with:\n  /coven goal <objective>",
        "pause" => "No active goal.",
        "resume" => "No goal to resume.",
        "complete" => "No active goal. Set one with /coven goal <objective>.",
        _ => panic!("unexpected argument"),
    }
}

#[test]
#[ignore = "owned process configuration and goal database"]
fn goal_read_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("GOAL_FIXTURE_PROJECT").unwrap());
    let case = std::env::var("GOAL_FIXTURE_CASE").unwrap();
    assert_eq!(claurst_core::config::config_home(), home);
    assert_eq!(GoalStore::default_path(), home.join("goals.sqlite"));
    let store = GoalStore::open_default().unwrap();
    store
        .set_goal("unrelated", "unrelated objective", None)
        .unwrap();
    let database = rusqlite::Connection::open(GoalStore::default_path()).unwrap();
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
    let mut failures = Vec::new();
    for coven in [false, true] {
        for argument in ["", "status", "pause", "resume", "complete"] {
            if case == "valid" {
                for status in [
                    GoalStatus::Active,
                    GoalStatus::Paused,
                    GoalStatus::Complete,
                    GoalStatus::BudgetLimited,
                ] {
                    store
                        .set_goal("fixture-session", "fixture objective", Some(100))
                        .unwrap();
                    store.add_tokens("fixture-session", 23).unwrap();
                    store.record_turn("fixture-session", 7).unwrap();
                    store.set_status("fixture-session", status.clone()).unwrap();
                    let mut before = snapshot(&database);
                    assert_eq!(before[0].session_id, "fixture-session");
                    let result = runtime.block_on(execute(coven, argument, &mut context));
                    let after = snapshot(&database);
                    let changed = (argument == "pause"
                        && matches!(status, GoalStatus::Active | GoalStatus::BudgetLimited))
                        || (argument == "resume"
                            && matches!(status, GoalStatus::Paused | GoalStatus::BudgetLimited));
                    if changed {
                        before[0].status = if argument == "pause" {
                            "paused"
                        } else {
                            "active"
                        }
                        .into();
                        before[0].updated_at_ms = after[0].updated_at_ms;
                    }
                    assert_eq!(after, before);
                    let expected = match argument {
                        "" | "status" => format!("Goal status\n───────────\nStatus:  {}\nTurns:   1\nElapsed: 7s\nBudget:  100 tokens\nObjective:\n  fixture objective", status.as_str()),
                        "pause" => match status {
                            GoalStatus::Complete => "Goal is already complete.",
                            GoalStatus::Paused => "Goal is already paused. Use /coven goal resume to continue.",
                            _ => "Goal paused. Use /coven goal resume to continue.",
                        }.into(),
                        "resume" => match status {
                            GoalStatus::Active => "Goal is already active.",
                            GoalStatus::Complete => "Goal is complete. Use /coven goal <objective> to set a new one.",
                            _ => "Goal resumed. Coven will continue on the next message.",
                        }.into(),
                        "complete" if status == GoalStatus::Active => {
                            assert!(matches!(result, CommandResult::GoalUserMessage { ref message, .. }
                                if message.starts_with("[User requested goal completion audit]")
                                && message.contains("<objective>\nfixture objective\n</objective>")
                                && message.contains("GoalComplete")), "{result:?}");
                            continue;
                        }
                        "complete" => missing_message(argument).into(),
                        _ => unreachable!(),
                    };
                    assert!(
                        matches!(result, CommandResult::Message(ref message) if message == &expected),
                        "{argument}/{status:?}: {result:?}; expected {expected:?}"
                    );
                }
                continue;
            }
            if case != "missing" {
                store
                    .set_goal("fixture-session", "fixture objective", Some(100))
                    .unwrap();
                let field = if case == "status" {
                    database.execute("UPDATE goals SET status = 'fixture-invalid' WHERE session_id = 'fixture-session'", []).unwrap();
                    "status"
                } else {
                    assert_eq!(case, "counter");
                    database.execute("UPDATE goals SET tokens_used = -1 WHERE session_id = 'fixture-session'", []).unwrap();
                    "tokens_used"
                };
                assert!(
                    matches!(store.try_get_goal("fixture-session"), Err(GoalError::InvalidStoredValue { field: actual, .. }) if actual == field)
                );
            }
            let before = snapshot(&database);
            let result = runtime.block_on(execute(coven, argument, &mut context));
            assert_eq!(snapshot(&database), before);
            if case == "missing" {
                assert!(
                    matches!(result, CommandResult::Message(ref message) if message == missing_message(argument)),
                    "{result:?}"
                );
            } else {
                let field = if case == "status" {
                    "status"
                } else {
                    "tokens_used"
                };
                let error_prefix = if argument == "complete" {
                    "Could not capture goal"
                } else {
                    "Failed to read goal"
                };
                if !matches!(result, CommandResult::Error(ref message) if message.contains(error_prefix) && message.contains(field))
                {
                    failures.push(format!("coven={coven} {argument:?}: {result:?}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(context.session_id, "fixture-session");
    assert!(context.messages.is_empty());
    std::fs::write(std::env::var_os("GOAL_FIXTURE_DONE").unwrap(), b"checked").unwrap();
}
