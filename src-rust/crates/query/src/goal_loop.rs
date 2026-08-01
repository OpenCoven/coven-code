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

fn reload_after_racing_transition(
    store: &GoalStore,
    session_id: &str,
    transition_error: GoalError,
) -> Result<GoalContinuation, GoalError> {
    let goal = store
        .try_get_goal(session_id)?
        .ok_or_else(|| GoalError::NotFound {
            session_id: session_id.to_string(),
        })?;
    stop_for_terminal_status(&goal).ok_or(transition_error)
}

fn check_and_continue_goal_in_store(
    store: &GoalStore,
    session_id: &str,
    total_tokens_used: u64,
    turn_elapsed_secs: u64,
) -> Result<GoalContinuation, GoalError> {
    store.record_completed_turn(session_id, total_tokens_used, turn_elapsed_secs)?;
    check_goal_guards(store, session_id, None)
}

fn check_and_continue_goal_in_store_for_goal(
    store: &GoalStore,
    session_id: &str,
    expected_goal_id: &str,
    total_tokens_used: u64,
    turn_elapsed_secs: u64,
) -> Result<GoalContinuation, GoalError> {
    store.record_completed_turn_for_goal(
        session_id,
        expected_goal_id,
        total_tokens_used,
        turn_elapsed_secs,
    )?;
    check_goal_guards(store, session_id, Some(expected_goal_id))
}

fn check_goal_guards(
    store: &GoalStore,
    session_id: &str,
    expected_goal_id: Option<&str>,
) -> Result<GoalContinuation, GoalError> {
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

    if let Some(decision) = stop_for_terminal_status(&goal) {
        return Ok(decision);
    }

    if goal.turns_used >= MAX_GOAL_TURNS {
        let transition = match expected_goal_id {
            Some(expected_goal_id) => {
                store.pause_active_goal_for_goal(session_id, expected_goal_id)
            }
            None => store.pause_active_goal(session_id),
        };
        if let Err(error) = transition {
            return match error {
                error @ GoalError::NotActive { .. } => {
                    reload_after_racing_transition(store, session_id, error)
                }
                other => Err(other),
            };
        }
        return Ok(GoalContinuation::Stop {
            reason: StopReason::RunawayGuard {
                turns_used: goal.turns_used,
            },
        });
    }

    if goal.is_over_budget(goal.tokens_used) {
        let transition = match expected_goal_id {
            Some(expected_goal_id) => {
                store.budget_limit_active_goal_for_goal(session_id, expected_goal_id)
            }
            None => store.budget_limit_active_goal(session_id),
        };
        if let Err(error) = transition {
            return match error {
                error @ GoalError::NotActive { .. } => {
                    reload_after_racing_transition(store, session_id, error)
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
    use std::path::PathBuf;

    use super::*;
    use claurst_core::{GoalError, GoalStore};

    fn goal_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("goals.sqlite")
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
    fn explicit_path_reports_missing_goal() {
        let dir = tempfile::tempdir().unwrap();
        let error = check_and_continue_goal_at_path(&goal_path(&dir), "missing", 0, 0).unwrap_err();
        assert!(matches!(error, GoalError::NotFound { .. }));
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
