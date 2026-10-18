#[path = "common/goal.rs"]
mod goal_fixture;

use claurst_commands::{CommandContext, CommandResult, CovenCommand, GoalCommand, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker, Goal, GoalStore};
use goal_fixture::run_cases;

#[test]
fn failed_goal_deletion_is_reported_without_mutating_goals() {
    run_cases("goal_clear_worker", &["direct_failure", "coven_failure"]);
}

#[test]
fn successful_and_absent_goal_deletion_keep_existing_behavior() {
    run_cases("goal_clear_worker", &["direct_success", "coven_success"]);
}

fn assert_goal_unchanged(before: &Goal, after: &Goal) {
    assert_eq!(after.id, before.id);
    assert_eq!(after.session_id, before.session_id);
    assert_eq!(after.objective, before.objective);
    assert_eq!(after.status, before.status);
    assert_eq!(after.token_budget, before.token_budget);
    assert_eq!(after.tokens_used, before.tokens_used);
    assert_eq!(after.time_used_secs, before.time_used_secs);
    assert_eq!(after.turns_used, before.turns_used);
    assert_eq!(after.created_at_ms, before.created_at_ms);
    assert_eq!(after.updated_at_ms, before.updated_at_ms);
}

async fn clear(coven: bool, context: &mut CommandContext) -> CommandResult {
    if coven {
        CovenCommand.execute("goal clear", context).await
    } else {
        GoalCommand.execute("clear", context).await
    }
}

#[test]
#[ignore = "owned process configuration and goal database"]
fn goal_clear_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("GOAL_FIXTURE_PROJECT").unwrap());
    let case = std::env::var("GOAL_FIXTURE_CASE").unwrap();
    assert_eq!(claurst_core::config::config_home(), home);
    assert_eq!(GoalStore::default_path(), home.join("goals.sqlite"));
    let store = GoalStore::open_default().unwrap();
    store
        .set_goal("fixture-session", "preserve this goal", Some(100))
        .unwrap();
    store.add_tokens("fixture-session", 23).unwrap();
    store.record_turn("fixture-session", 7).unwrap();
    let before = store.try_get_goal("fixture-session").unwrap().unwrap();
    let other = store
        .set_goal("other-session", "unrelated goal", None)
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
    let coven = case.starts_with("coven_");
    if case.ends_with("_failure") {
        database
            .execute_batch(
                "CREATE TRIGGER reject_goal_delete BEFORE DELETE ON goals
            BEGIN SELECT RAISE(ABORT, 'fixture delete blocked'); END;",
            )
            .unwrap();
        let result = runtime.block_on(clear(coven, &mut context));
        assert_goal_unchanged(
            &before,
            &store.try_get_goal("fixture-session").unwrap().unwrap(),
        );
        assert_goal_unchanged(
            &other,
            &store.try_get_goal("other-session").unwrap().unwrap(),
        );
        match result {
            CommandResult::Error(message) => {
                assert!(message.contains("Failed to clear goal"), "{message}");
                assert!(message.contains("fixture delete blocked"), "{message}");
            }
            result => panic!("failed delete reported success: {result:?}"),
        }
        database
            .execute_batch("DROP TRIGGER reject_goal_delete;")
            .unwrap();
    } else {
        assert!(case.ends_with("_success"));
    }
    for _ in 0..2 {
        let result = runtime.block_on(clear(coven, &mut context));
        assert!(
            matches!(result, CommandResult::Message(ref message) if message == "Goal cleared."),
            "{result:?}"
        );
        assert!(store.try_get_goal("fixture-session").unwrap().is_none());
        assert_goal_unchanged(
            &other,
            &store.try_get_goal("other-session").unwrap().unwrap(),
        );
    }
    assert_eq!(context.session_id, "fixture-session");
    assert!(context.messages.is_empty());
    std::fs::write(std::env::var_os("GOAL_FIXTURE_DONE").unwrap(), b"checked").unwrap();
}
