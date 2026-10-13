// goal_loop.rs — Goal continuation engine for the /goal feature.
//
// `check_and_continue_goal` is called by the CLI REPL after each query loop
// turn completes.  When an active goal exists it:
//   1. Records the turn in the GoalStore
//   2. Checks runaway / budget guards
//   3. Returns `GoalContinuation::Continue { message }` with the continuation
//      user message to inject, signalling the caller to dispatch another turn.
//
// The caller (cli/src/main.rs) is responsible for the actual dispatch so that
// TUI event handling and cancellation tokens stay in the right place.

use std::path::Path;

use claurst_core::{
    goal_continuation_message, Goal, GoalError, GoalStatus, GoalStore, MAX_GOAL_TURNS,
};

#[cfg(test)]
use std::cell::RefCell;

#[cfg(test)]
thread_local! {
    static BEFORE_COMPLETED_TURN_RECORD: RefCell<Option<Box<dyn Fn()>>> =
        const { RefCell::new(None) };
    static BEFORE_AUTOMATIC_TERMINAL_TRANSITION: RefCell<Option<Box<dyn Fn()>>> =
        const { RefCell::new(None) };
    static BEFORE_BUDGET_LIMIT_TRANSITION: RefCell<Option<Box<dyn Fn()>>> =
        const { RefCell::new(None) };
}

/// Result returned to the caller after a completed query loop turn.
#[derive(Debug)]
pub enum GoalContinuation {
    /// Inject this user message and run another turn.
    Continue { message: String },
    /// Goal is done (complete, paused, cleared, budget hit, runaway).
    Stop { reason: StopReason },
    /// No goal is set for this session.
    NoGoal,
}

#[derive(Debug, Clone)]
pub enum StopReason {
    GoalComplete,
    Paused,
    BudgetLimited,
    RunawayGuard { turns_used: u32 },
    Error(String),
}

impl StopReason {
    pub fn user_message(&self) -> Option<String> {
        match self {
            StopReason::GoalComplete => Some("Goal marked complete by the model.".to_string()),
            StopReason::Paused => None, // user-initiated, no extra message needed
            StopReason::BudgetLimited => Some(
                "Soft token budget reached — goal paused. Start a new goal with a new budget."
                    .to_string(),
            ),
            StopReason::RunawayGuard { turns_used } => Some(format!(
                "Goal paused after {} turns (runaway guard). Start a new goal to continue.",
                turns_used
            )),
            StopReason::Error(msg) => Some(format!("Goal error: {}", msg)),
        }
    }
}

/// Inspect the current goal for `session_id` after a completed turn and decide
/// whether to continue.
///
/// `total_tokens_used` is the goal-wide cumulative token count (used to
/// enforce soft budgets).
/// `turn_elapsed_secs` is how long this turn took (for time accounting).
pub fn check_and_continue_goal(
    session_id: &str,
    total_tokens_used: u64,
    turn_elapsed_secs: u64,
) -> GoalContinuation {
    match GoalStore::open(&GoalStore::default_path()).and_then(|store| {
        check_and_continue_goal_in_store(&store, session_id, total_tokens_used, turn_elapsed_secs)
    }) {
        Ok(decision) => decision,
        Err(GoalError::NotFound { .. }) => GoalContinuation::NoGoal,
        Err(error) => GoalContinuation::Stop {
            reason: StopReason::Error(error.to_string()),
        },
    }
}

/// Expected-ID-aware continuation check used by turn accounting baselines.
pub fn check_and_continue_goal_for_goal(
    session_id: &str,
    expected_goal_id: &str,
    total_tokens_used: u64,
    turn_elapsed_secs: u64,
) -> GoalContinuation {
    match GoalStore::open(&GoalStore::default_path()).and_then(|store| {
        check_and_continue_goal_in_store_for_goal(
            &store,
            session_id,
            expected_goal_id,
            total_tokens_used,
            turn_elapsed_secs,
        )
    }) {
        Ok(decision) => decision,
        Err(GoalError::NotFound { .. }) => GoalContinuation::NoGoal,
        Err(error) => GoalContinuation::Stop {
            reason: StopReason::Error(error.to_string()),
        },
    }
}

fn stop_for_terminal_status(goal: &Goal) -> Option<GoalContinuation> {
    let reason = match goal.status {
        GoalStatus::Complete => StopReason::GoalComplete,
        GoalStatus::Paused => StopReason::Paused,
        GoalStatus::BudgetLimited => StopReason::BudgetLimited,
        GoalStatus::Active => return None,
    };
    Some(GoalContinuation::Stop { reason })
}

fn current_goal(
    store: &GoalStore,
    session_id: &str,
    expected_goal_id: Option<&str>,
) -> Result<Goal, GoalError> {
    let goal = store
        .try_get_goal(session_id)?
        .ok_or_else(|| GoalError::NotFound {
            session_id: session_id.to_string(),
        })?;
    if let Some(expected_goal_id) = expected_goal_id {
        if goal.id != expected_goal_id {
            return Err(GoalError::Replaced {
                session_id: session_id.to_string(),
                expected_goal_id: expected_goal_id.to_string(),
                actual_goal_id: goal.id,
            });
        }
    }
    Ok(goal)
}

fn reload_after_racing_transition(
    store: &GoalStore,
    session_id: &str,
    expected_goal_id: Option<&str>,
    transition_error: GoalError,
) -> Result<GoalContinuation, GoalError> {
    let goal = current_goal(store, session_id, expected_goal_id)?;
    stop_for_terminal_status(&goal).ok_or(transition_error)
}

fn check_and_continue_goal_in_store(
    store: &GoalStore,
    session_id: &str,
    total_tokens_used: u64,
    turn_elapsed_secs: u64,
) -> Result<GoalContinuation, GoalError> {
    let expected_goal_id = current_goal(store, session_id, None)?.id;
    check_and_continue_goal_after_turn(
        store,
        session_id,
        Some(&expected_goal_id),
        total_tokens_used,
        turn_elapsed_secs,
    )
}

fn check_and_continue_goal_in_store_for_goal(
    store: &GoalStore,
    session_id: &str,
    expected_goal_id: &str,
    total_tokens_used: u64,
    turn_elapsed_secs: u64,
) -> Result<GoalContinuation, GoalError> {
    check_and_continue_goal_after_turn(
        store,
        session_id,
        Some(expected_goal_id),
        total_tokens_used,
        turn_elapsed_secs,
    )
}

fn check_and_continue_goal_after_turn(
    store: &GoalStore,
    session_id: &str,
    expected_goal_id: Option<&str>,
    total_tokens_used: u64,
    turn_elapsed_secs: u64,
) -> Result<GoalContinuation, GoalError> {
    let goal = current_goal(store, session_id, expected_goal_id)?;
    if let Some(decision) = stop_for_terminal_status(&goal) {
        return Ok(decision);
    }

    #[cfg(test)]
    BEFORE_COMPLETED_TURN_RECORD.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });

    let record = match expected_goal_id {
        Some(expected_goal_id) => store.record_completed_turn_for_goal(
            session_id,
            expected_goal_id,
            total_tokens_used,
            turn_elapsed_secs,
        ),
        None => store.record_completed_turn(session_id, total_tokens_used, turn_elapsed_secs),
    };
    match record {
        Ok(()) => check_goal_guards(store, session_id, expected_goal_id),
        Err(error @ GoalError::NotActive { .. }) => {
            reload_after_racing_transition(store, session_id, expected_goal_id, error)
        }
        Err(error) => Err(error),
    }
}

fn check_goal_guards(
    store: &GoalStore,
    session_id: &str,
    expected_goal_id: Option<&str>,
) -> Result<GoalContinuation, GoalError> {
    let goal = current_goal(store, session_id, expected_goal_id)?;

    if let Some(decision) = stop_for_terminal_status(&goal) {
        return Ok(decision);
    }

    if goal.turns_used >= MAX_GOAL_TURNS {
        #[cfg(test)]
        BEFORE_AUTOMATIC_TERMINAL_TRANSITION.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook();
            }
        });

        let transition = match expected_goal_id {
            Some(expected_goal_id) => {
                store.pause_active_goal_for_goal_with_outcome(session_id, expected_goal_id)
            }
            None => store.pause_active_goal_with_outcome(session_id),
        };
        match transition {
            Ok(true) => {}
            Ok(false) => {
                return reload_after_racing_transition(
                    store,
                    session_id,
                    expected_goal_id,
                    GoalError::NotActive {
                        session_id: session_id.to_string(),
                    },
                );
            }
            Err(error) => {
                return match error {
                    error @ GoalError::NotActive { .. } => {
                        reload_after_racing_transition(store, session_id, expected_goal_id, error)
                    }
                    other => Err(other),
                };
            }
        }
        return Ok(GoalContinuation::Stop {
            reason: StopReason::RunawayGuard {
                turns_used: goal.turns_used,
            },
        });
    }

    if goal.is_over_budget(goal.tokens_used) {
        #[cfg(test)]
        BEFORE_BUDGET_LIMIT_TRANSITION.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook();
            }
        });

        let transition = match expected_goal_id {
            Some(expected_goal_id) => {
                store.budget_limit_active_goal_for_goal(session_id, expected_goal_id)
            }
            None => store.budget_limit_active_goal(session_id),
        };
        if let Err(error) = transition {
            return match error {
                error @ GoalError::NotActive { .. } => {
                    reload_after_racing_transition(store, session_id, expected_goal_id, error)
                }
                other => Err(other),
            };
        }
        return Ok(GoalContinuation::Stop {
            reason: StopReason::BudgetLimited,
        });
    }

    Ok(GoalContinuation::Continue {
        message: goal_continuation_message(&goal),
    })
}

pub fn check_and_continue_goal_at_path(
    goal_db_path: &Path,
    session_id: &str,
    total_tokens_used: u64,
    turn_elapsed_secs: u64,
) -> Result<GoalContinuation, GoalError> {
    let store = GoalStore::open(goal_db_path)?;
    check_and_continue_goal_in_store(&store, session_id, total_tokens_used, turn_elapsed_secs)
}

pub fn check_and_continue_goal_at_path_for_goal(
    goal_db_path: &Path,
    session_id: &str,
    expected_goal_id: &str,
    total_tokens_used: u64,
    turn_elapsed_secs: u64,
) -> Result<GoalContinuation, GoalError> {
    let store = GoalStore::open(goal_db_path)?;
    check_and_continue_goal_in_store_for_goal(
        &store,
        session_id,
        expected_goal_id,
        total_tokens_used,
        turn_elapsed_secs,
    )
}

/// Called by GoalCompleteTool to mark the goal complete.
pub fn mark_goal_complete(session_id: &str) -> Result<(), String> {
    let store = GoalStore::open_default().ok_or_else(|| "Could not open goal store".to_string())?;
    store
        .complete_active_goal(session_id)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Mutex};

    use super::*;
    use claurst_core::{GoalError, GoalStore};

    static DEFAULT_GOAL_STORE_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn goal_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("goals.sqlite")
    }

    fn assert_terminal_stop(decision: GoalContinuation, status: GoalStatus) {
        match (decision, status) {
            (
                GoalContinuation::Stop {
                    reason: StopReason::GoalComplete,
                },
                GoalStatus::Complete,
            )
            | (
                GoalContinuation::Stop {
                    reason: StopReason::Paused,
                },
                GoalStatus::Paused,
            )
            | (
                GoalContinuation::Stop {
                    reason: StopReason::BudgetLimited,
                },
                GoalStatus::BudgetLimited,
            ) => {}
            (decision, status) => {
                panic!("expected terminal stop for {status:?}, got {decision:?}");
            }
        }
    }

    fn assert_progress_unchanged(store: &GoalStore, session_id: &str) {
        let goal = store.try_get_goal(session_id).unwrap().unwrap();
        assert_eq!(goal.tokens_used, 42);
        assert_eq!(goal.time_used_secs, 5);
        assert_eq!(goal.turns_used, 1);
    }

    fn set_terminal_goal(store: &GoalStore, session_id: &str, status: GoalStatus) -> String {
        let goal = store.set_goal(session_id, "finish", None).unwrap();
        store.record_completed_turn(session_id, 42, 5).unwrap();
        store.set_status(session_id, status).unwrap();
        goal.id
    }

    fn setup_runaway_goal(store: &GoalStore, session_id: &str) -> String {
        let goal = store.set_goal(session_id, "finish", None).unwrap();
        for turn in 1..MAX_GOAL_TURNS {
            store
                .record_completed_turn(session_id, u64::from(turn), 1)
                .unwrap();
        }
        goal.id
    }

    fn pause_before_automatic_terminal_transition(path: &Path, session_id: &str) {
        let path = path.to_path_buf();
        let session_id = session_id.to_string();
        BEFORE_AUTOMATIC_TERMINAL_TRANSITION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                GoalStore::open(&path)
                    .unwrap()
                    .pause_active_goal(&session_id)
                    .unwrap();
            }));
        });
    }

    fn replace_before_automatic_terminal_transition(path: &Path, session_id: &str) {
        let path = path.to_path_buf();
        let session_id = session_id.to_string();
        BEFORE_AUTOMATIC_TERMINAL_TRANSITION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                GoalStore::open(&path)
                    .unwrap()
                    .set_goal(&session_id, "replacement objective", None)
                    .unwrap();
            }));
        });
    }

    fn replace_before_completed_turn_record(path: &Path, session_id: &str) {
        let path = path.to_path_buf();
        let session_id = session_id.to_string();
        BEFORE_COMPLETED_TURN_RECORD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                GoalStore::open(&path)
                    .unwrap()
                    .set_goal(&session_id, "replacement objective", None)
                    .unwrap();
            }));
        });
    }

    fn replace_before_budget_limit_transition(path: &Path, session_id: &str) {
        let path = path.to_path_buf();
        let session_id = session_id.to_string();
        BEFORE_BUDGET_LIMIT_TRANSITION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                GoalStore::open(&path)
                    .unwrap()
                    .set_goal(&session_id, "replacement objective", None)
                    .unwrap();
            }));
        });
    }

    fn assert_replacement_is_active(store: &GoalStore, session_id: &str) {
        let replacement = store.try_get_goal(session_id).unwrap().unwrap();
        assert_eq!(replacement.objective, "replacement objective");
        assert_eq!(replacement.status, GoalStatus::Active);
        assert_eq!(replacement.tokens_used, 0);
        assert_eq!(replacement.time_used_secs, 0);
        assert_eq!(replacement.turns_used, 0);
    }

    #[test]
    fn explicit_path_records_progress_before_budget_stop() {
        let dir = tempfile::tempdir().unwrap();
        let path = goal_path(&dir);
        GoalStore::open(&path)
            .unwrap()
            .set_goal("session", "finish", Some(100))
            .unwrap();

        let decision = check_and_continue_goal_at_path(&path, "session", 100, 9).unwrap();
        assert!(matches!(
            decision,
            GoalContinuation::Stop {
                reason: StopReason::BudgetLimited
            }
        ));
        let goal = GoalStore::open(&path)
            .unwrap()
            .try_get_goal("session")
            .unwrap()
            .unwrap();
        assert_eq!(goal.tokens_used, 100);
        assert_eq!(goal.time_used_secs, 9);
        assert_eq!(goal.turns_used, 1);
    }

    #[test]
    fn explicit_path_stops_at_the_recorded_runaway_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = goal_path(&dir);
        let store = GoalStore::open(&path).unwrap();
        store.set_goal("session", "finish", None).unwrap();
        for turn in 1..MAX_GOAL_TURNS {
            store
                .record_completed_turn("session", u64::from(turn), 1)
                .unwrap();
        }

        let decision = check_and_continue_goal_at_path(&path, "session", 999, 1).unwrap();
        assert!(matches!(
            decision,
            GoalContinuation::Stop {
                reason: StopReason::RunawayGuard {
                    turns_used: MAX_GOAL_TURNS
                }
            }
        ));
    }

    #[test]
    fn default_wrapper_reports_concurrent_pause_at_runaway_boundary() {
        let _env_lock = DEFAULT_GOAL_STORE_ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("COVEN_CODE_HOME");
        std::env::set_var("COVEN_CODE_HOME", dir.path());

        let path = GoalStore::default_path();
        let store = GoalStore::open(&path).unwrap();
        setup_runaway_goal(&store, "session");
        pause_before_automatic_terminal_transition(&path, "session");

        let decision = check_and_continue_goal("session", 999, 1);
        assert_terminal_stop(decision, GoalStatus::Paused);

        match previous_home {
            Some(home) => std::env::set_var("COVEN_CODE_HOME", home),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }
    }

    #[test]
    fn explicit_path_wrapper_reports_concurrent_pause_at_runaway_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = goal_path(&dir);
        let store = GoalStore::open(&path).unwrap();
        setup_runaway_goal(&store, "session");
        pause_before_automatic_terminal_transition(&path, "session");

        let decision = check_and_continue_goal_at_path(&path, "session", 999, 1).unwrap();
        assert_terminal_stop(decision, GoalStatus::Paused);
    }

    #[test]
    fn expected_id_wrapper_reports_concurrent_pause_at_runaway_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = goal_path(&dir);
        let store = GoalStore::open(&path).unwrap();
        let goal_id = setup_runaway_goal(&store, "session");
        pause_before_automatic_terminal_transition(&path, "session");

        let decision =
            check_and_continue_goal_at_path_for_goal(&path, "session", &goal_id, 999, 1).unwrap();
        assert_terminal_stop(decision, GoalStatus::Paused);
    }

    #[test]
    fn default_wrapper_does_not_pause_a_replacement_at_runaway_boundary() {
        let _env_lock = DEFAULT_GOAL_STORE_ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("COVEN_CODE_HOME");
        std::env::set_var("COVEN_CODE_HOME", dir.path());

        let path = GoalStore::default_path();
        let store = GoalStore::open(&path).unwrap();
        setup_runaway_goal(&store, "session");
        replace_before_automatic_terminal_transition(&path, "session");

        let decision = check_and_continue_goal("session", 999, 1);

        match previous_home {
            Some(home) => std::env::set_var("COVEN_CODE_HOME", home),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }

        assert!(matches!(
            decision,
            GoalContinuation::Stop {
                reason: StopReason::Error(message),
            } if message.contains("replaced")
        ));
        assert_replacement_is_active(&GoalStore::open(&path).unwrap(), "session");
    }

    #[test]
    fn explicit_path_wrapper_does_not_pause_a_replacement_at_runaway_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = goal_path(&dir);
        let store = GoalStore::open(&path).unwrap();
        setup_runaway_goal(&store, "session");
        replace_before_automatic_terminal_transition(&path, "session");

        let error = check_and_continue_goal_at_path(&path, "session", 999, 1).unwrap_err();

        assert!(matches!(error, GoalError::Replaced { .. }));
        assert_replacement_is_active(&GoalStore::open(&path).unwrap(), "session");
    }

    #[test]
    fn default_wrapper_does_not_record_progress_on_a_replacement() {
        let _env_lock = DEFAULT_GOAL_STORE_ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("COVEN_CODE_HOME");
        std::env::set_var("COVEN_CODE_HOME", dir.path());

        let path = GoalStore::default_path();
        let store = GoalStore::open(&path).unwrap();
        store
            .set_goal("session", "original objective", None)
            .unwrap();
        replace_before_completed_turn_record(&path, "session");

        let decision = check_and_continue_goal("session", 123, 7);

        match previous_home {
            Some(home) => std::env::set_var("COVEN_CODE_HOME", home),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }

        assert!(matches!(
            decision,
            GoalContinuation::Stop {
                reason: StopReason::Error(message),
            } if message.contains("replaced")
        ));
        assert_replacement_is_active(&GoalStore::open(&path).unwrap(), "session");
    }

    #[test]
    fn explicit_path_wrapper_does_not_record_progress_on_a_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = goal_path(&dir);
        let store = GoalStore::open(&path).unwrap();
        store
            .set_goal("session", "original objective", None)
            .unwrap();
        replace_before_completed_turn_record(&path, "session");

        let error = check_and_continue_goal_at_path(&path, "session", 123, 7).unwrap_err();

        assert!(matches!(error, GoalError::Replaced { .. }));
        assert_replacement_is_active(&GoalStore::open(&path).unwrap(), "session");
    }

    #[test]
    fn default_wrapper_does_not_budget_limit_a_replacement() {
        let _env_lock = DEFAULT_GOAL_STORE_ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("COVEN_CODE_HOME");
        std::env::set_var("COVEN_CODE_HOME", dir.path());

        let path = GoalStore::default_path();
        let store = GoalStore::open(&path).unwrap();
        store
            .set_goal("session", "original objective", Some(100))
            .unwrap();
        replace_before_budget_limit_transition(&path, "session");

        let decision = check_and_continue_goal("session", 100, 7);

        match previous_home {
            Some(home) => std::env::set_var("COVEN_CODE_HOME", home),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }

        assert!(matches!(
            decision,
            GoalContinuation::Stop {
                reason: StopReason::Error(message),
            } if message.contains("replaced")
        ));
        assert_replacement_is_active(&GoalStore::open(&path).unwrap(), "session");
    }

    #[test]
    fn explicit_path_wrapper_does_not_budget_limit_a_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = goal_path(&dir);
        let store = GoalStore::open(&path).unwrap();
        store
            .set_goal("session", "original objective", Some(100))
            .unwrap();
        replace_before_budget_limit_transition(&path, "session");

        let error = check_and_continue_goal_at_path(&path, "session", 100, 7).unwrap_err();

        assert!(matches!(error, GoalError::Replaced { .. }));
        assert_replacement_is_active(&GoalStore::open(&path).unwrap(), "session");
    }

    #[test]
    fn explicit_path_reports_missing_goal() {
        let dir = tempfile::tempdir().unwrap();
        let error = check_and_continue_goal_at_path(&goal_path(&dir), "missing", 0, 0).unwrap_err();
        assert!(matches!(error, GoalError::NotFound { .. }));
    }

    #[test]
    fn explicit_path_terminal_goals_stop_without_recording_progress() {
        let dir = tempfile::tempdir().unwrap();
        let path = goal_path(&dir);
        let store = GoalStore::open(&path).unwrap();

        for (session_id, status) in [
            ("complete", GoalStatus::Complete),
            ("paused", GoalStatus::Paused),
            ("budget-limited", GoalStatus::BudgetLimited),
        ] {
            let goal_id = set_terminal_goal(&store, session_id, status.clone());

            let decision = check_and_continue_goal_at_path(&path, session_id, 999, 8).unwrap();
            assert_terminal_stop(decision, status.clone());
            assert_progress_unchanged(&store, session_id);

            let decision =
                check_and_continue_goal_at_path_for_goal(&path, session_id, &goal_id, 999, 8)
                    .unwrap();
            assert_terminal_stop(decision, status);
            assert_progress_unchanged(&store, session_id);
        }
    }

    #[test]
    fn default_wrappers_terminal_goals_stop_without_recording_progress() {
        let _env_lock = DEFAULT_GOAL_STORE_ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("COVEN_CODE_HOME");
        std::env::set_var("COVEN_CODE_HOME", dir.path());

        let path = GoalStore::default_path();
        let store = GoalStore::open(&path).unwrap();

        for (session_id, status) in [
            ("complete", GoalStatus::Complete),
            ("paused", GoalStatus::Paused),
            ("budget-limited", GoalStatus::BudgetLimited),
        ] {
            let goal_id = set_terminal_goal(&store, session_id, status.clone());

            let decision = check_and_continue_goal(session_id, 999, 8);
            assert_terminal_stop(decision, status.clone());
            assert_progress_unchanged(&store, session_id);

            let decision = check_and_continue_goal_for_goal(session_id, &goal_id, 999, 8);
            assert_terminal_stop(decision, status);
            assert_progress_unchanged(&store, session_id);
        }

        match previous_home {
            Some(home) => std::env::set_var("COVEN_CODE_HOME", home),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }
    }

    #[test]
    fn expected_goal_id_rejects_replacement_without_recording_progress() {
        let dir = tempfile::tempdir().unwrap();
        let path = goal_path(&dir);
        let store = GoalStore::open(&path).unwrap();
        let original = store
            .set_goal("session", "original objective", None)
            .unwrap();

        let replacement = store
            .set_goal("session", "replacement objective", None)
            .unwrap();
        assert_ne!(original.id, replacement.id);

        let error =
            check_and_continue_goal_at_path_for_goal(&path, "session", &original.id, 123, 7)
                .unwrap_err();

        assert!(error.to_string().contains("replaced"));
        let current = GoalStore::open(&path)
            .unwrap()
            .try_get_goal("session")
            .unwrap()
            .unwrap();
        assert_eq!(current.id, replacement.id);
        assert_eq!(current.tokens_used, 0);
        assert_eq!(current.time_used_secs, 0);
        assert_eq!(current.turns_used, 0);
    }
}
