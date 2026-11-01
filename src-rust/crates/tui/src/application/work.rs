//! Read-only projection of the engine-local work stores for the Next Work page.
//!
//! Three stores exist in the engine today, and none is canonical orchestration:
//!
//! - the process-global background registry (`claurst_core::tasks`), in memory;
//! - the model's task list (`claurst_tools::TASK_STORE`), in memory;
//! - the session goal (`claurst_core::goal::GoalStore`), in SQLite.
//!
//! The interactive loop snapshots them into [`WorkSources`]; [`WorkView`]
//! turns a snapshot into display rows. Rows keep their origin and full
//! identifier, so two items with the same title are never merged, and no row
//! grants authority to start, stop or retry anything. Canonical tasks, lanes
//! and leases belong to Psyche and Coven (Psyche #11/#12) and are not shown
//! until a qualified producer contract exists.

use super::view::terminal_safe;
use claurst_core::goal::{Goal, GoalStatus};
use claurst_core::tasks::{BackgroundTask, TaskStatus as BackgroundStatus};
use claurst_tools::{Task, TaskStatus as ModelTaskStatus};

/// Which engine-local store a row came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkOrigin {
    /// A background shell or agent task in the process-global registry.
    Background,
    /// A task the model recorded with its task tools.
    ModelTask,
    /// The durable goal of the current session.
    SessionGoal,
}

impl WorkOrigin {
    /// Bracketed label shown at the start of every row.
    pub fn label(self) -> &'static str {
        match self {
            WorkOrigin::Background => "[background]",
            WorkOrigin::ModelTask => "[model task]",
            WorkOrigin::SessionGoal => "[session goal]",
        }
    }
}

/// How settled a row's state is. Only the producer can confirm an outcome;
/// the Work page never upgrades a state on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkStateKind {
    /// Still running or waiting.
    Open,
    /// Finished as the store reports it.
    Done,
    /// Failed, or budget-limited.
    Failed,
    /// A stop was signalled or requested, but exit is not confirmed.
    Stopping,
    /// Stopped at a limit such as a token budget; not a failure.
    Limited,
}

/// One display row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItem {
    pub origin: WorkOrigin,
    /// Full identifier from the store, never shortened.
    pub id: String,
    pub title: String,
    pub state: String,
    pub kind: WorkStateKind,
    pub owner: Option<String>,
    /// Blocking task identifiers as the store records them.
    pub blocked_by: Vec<String>,
    /// Blocking identifiers that match no task in the same snapshot.
    pub missing_blockers: Vec<String>,
    /// Further `(name, value)` details for the inspector.
    pub detail: Vec<(String, String)>,
}

/// A snapshot of the three stores, taken by the interactive loop.
#[derive(Debug, Clone, Default)]
pub struct WorkSources {
    pub background: Vec<BackgroundTask>,
    pub model_tasks: Vec<Task>,
    /// `None` when there is no goal for this session or it was not read yet;
    /// `Some(Err(_))` when reading the goal store failed.
    pub goal: Option<Result<Goal, String>>,
}

/// Display rows for the Work page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkView {
    pub items: Vec<WorkItem>,
    /// Why the session goal could not be read, when it could not.
    pub goal_error: Option<String>,
}

impl WorkView {
    pub fn from_sources(sources: &WorkSources) -> Self {
        let mut items = Vec::new();

        let mut background: Vec<&BackgroundTask> = sources.background.iter().collect();
        background.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.id.cmp(&b.id)));
        for task in background {
            let (state, kind) = match &task.status {
                BackgroundStatus::Running => ("running".to_string(), WorkStateKind::Open),
                BackgroundStatus::Completed => ("completed".to_string(), WorkStateKind::Done),
                BackgroundStatus::Failed(reason) => {
                    (format!("failed: {reason}"), WorkStateKind::Failed)
                }
                // The registry marks a task cancelled when it fires the
                // token, before any process has exited (#373).
                BackgroundStatus::Cancelled => (
                    "cancel signalled; exit not confirmed".to_string(),
                    WorkStateKind::Stopping,
                ),
            };
            let mut detail = vec![(
                "Started".to_string(),
                task.started_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            )];
            if let Some(done) = task.completed_at {
                detail.push((
                    "Ended".to_string(),
                    done.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                ));
            }
            if let Some(pid) = task.pid {
                detail.push(("Process".to_string(), pid.to_string()));
            }
            if let Some(last) = task.output.last() {
                detail.push(("Last output".to_string(), last.clone()));
            }
            items.push(item(
                WorkOrigin::Background,
                &task.id,
                &task.name,
                state,
                kind,
                None,
                &[],
                Vec::new(),
                detail,
            ));
        }

        let listed: Vec<&Task> = {
            let mut tasks: Vec<&Task> = sources
                .model_tasks
                .iter()
                .filter(|task| task.status != ModelTaskStatus::Deleted)
                .collect();
            tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
            tasks
        };
        for task in &listed {
            let (state, kind) = match task.status {
                ModelTaskStatus::Pending => ("pending", WorkStateKind::Open),
                ModelTaskStatus::InProgress => ("in progress", WorkStateKind::Open),
                ModelTaskStatus::Running => ("running", WorkStateKind::Open),
                // TaskStop also writes Completed, so this is the model's
                // report, not a verified outcome.
                ModelTaskStatus::Completed => {
                    ("completed (reported by the model)", WorkStateKind::Done)
                }
                ModelTaskStatus::Failed => ("failed", WorkStateKind::Failed),
                ModelTaskStatus::Deleted => continue,
            };
            let missing: Vec<String> = task
                .blocked_by
                .iter()
                .filter(|id| !listed.iter().any(|other| &other.id == *id))
                .cloned()
                .collect();
            let mut detail = Vec::new();
            if !task.description.is_empty() {
                detail.push(("Description".to_string(), task.description.clone()));
            }
            if !task.blocks.is_empty() {
                detail.push(("Blocks".to_string(), task.blocks.join(", ")));
            }
            detail.push((
                "Updated".to_string(),
                task.updated_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            ));
            items.push(item(
                WorkOrigin::ModelTask,
                &task.id,
                &task.subject,
                state.to_string(),
                kind,
                task.owner.as_deref(),
                &task.blocked_by,
                missing,
                detail,
            ));
        }

        let mut goal_error = None;
        match &sources.goal {
            Some(Ok(goal)) => {
                let (state, kind) = match goal.status {
                    GoalStatus::Active => ("active", WorkStateKind::Open),
                    GoalStatus::Paused => ("paused", WorkStateKind::Open),
                    GoalStatus::BudgetLimited => ("budget limited", WorkStateKind::Limited),
                    GoalStatus::Complete => ("complete (reported)", WorkStateKind::Done),
                };
                let budget = match goal.token_budget {
                    Some(budget) => format!("{} of {budget} tokens", goal.tokens_used),
                    None => format!("{} tokens, no budget", goal.tokens_used),
                };
                let detail = vec![
                    ("Budget".to_string(), budget),
                    (
                        "Used".to_string(),
                        format!("{}, {} turns", goal.elapsed_display(), goal.turns_used),
                    ),
                ];
                items.push(item(
                    WorkOrigin::SessionGoal,
                    &goal.id,
                    &goal.objective,
                    state.to_string(),
                    kind,
                    None,
                    &[],
                    Vec::new(),
                    detail,
                ));
            }
            Some(Err(error)) => goal_error = Some(one_line(error)),
            None => {}
        }

        WorkView { items, goal_error }
    }
}

// Allowed: one call site per store, each naming every display field of the
// row; a builder struct would only restate these parameters.
#[allow(clippy::too_many_arguments)]
fn item(
    origin: WorkOrigin,
    id: &str,
    title: &str,
    state: String,
    kind: WorkStateKind,
    owner: Option<&str>,
    blocked_by: &[String],
    missing_blockers: Vec<String>,
    detail: Vec<(String, String)>,
) -> WorkItem {
    let safe_all = |ids: &[String]| ids.iter().map(|id| one_line(id)).collect();
    WorkItem {
        origin,
        id: one_line(id),
        title: one_line(title),
        state: one_line(&state),
        kind,
        owner: owner.map(one_line),
        blocked_by: safe_all(blocked_by),
        missing_blockers: safe_all(&missing_blockers),
        detail: detail
            .into_iter()
            .map(|(name, value)| (name, one_line(&value)))
            .collect(),
    }
}

/// Terminal-safe text on one row: newlines become spaces.
fn one_line(text: &str) -> String {
    terminal_safe(text).replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn background(name: &str, status: BackgroundStatus, minute: u32) -> BackgroundTask {
        let mut task = BackgroundTask::new(name);
        task.status = status;
        task.started_at = Utc.with_ymd_and_hms(2026, 11, 1, 12, minute, 0).unwrap();
        task
    }

    fn model_task(id: &str, subject: &str, status: ModelTaskStatus, blocked_by: &[&str]) -> Task {
        let at = Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
        Task {
            id: id.to_string(),
            subject: subject.to_string(),
            description: String::new(),
            status,
            owner: None,
            blocks: Vec::new(),
            blocked_by: blocked_by.iter().map(|id| id.to_string()).collect(),
            metadata: None,
            output: None,
            created_at: at,
            updated_at: at,
        }
    }

    fn goal(status: GoalStatus) -> Goal {
        Goal {
            id: "goal-1".to_string(),
            session_id: "session-1".to_string(),
            objective: "Ship the Work view".to_string(),
            status,
            token_budget: Some(50_000),
            tokens_used: 12_000,
            time_used_secs: 90,
            turns_used: 3,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn rows_keep_their_origin_and_identical_titles_are_not_merged() {
        let sources = WorkSources {
            background: vec![background("build", BackgroundStatus::Running, 1)],
            model_tasks: vec![
                model_task("t1", "build", ModelTaskStatus::Pending, &[]),
                model_task("t2", "build", ModelTaskStatus::InProgress, &[]),
            ],
            goal: Some(Ok(goal(GoalStatus::Active))),
        };
        let view = WorkView::from_sources(&sources);
        let origins: Vec<_> = view.items.iter().map(|item| item.origin).collect();
        assert_eq!(
            origins,
            [
                WorkOrigin::Background,
                WorkOrigin::ModelTask,
                WorkOrigin::ModelTask,
                WorkOrigin::SessionGoal
            ]
        );
        let ids: Vec<_> = view.items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(&ids[1..3], ["t1", "t2"]);
        assert_eq!(ids[0], sources.background[0].id, "the full id is kept");
    }

    #[test]
    fn a_blocker_that_matches_no_task_is_flagged_and_nothing_is_inferred() {
        let sources = WorkSources {
            model_tasks: vec![
                model_task("a", "first", ModelTaskStatus::Completed, &[]),
                model_task("b", "second", ModelTaskStatus::Pending, &["a", "ghost"]),
            ],
            ..WorkSources::default()
        };
        let view = WorkView::from_sources(&sources);
        let second = view.items.iter().find(|item| item.id == "b").unwrap();
        assert_eq!(second.blocked_by, ["a", "ghost"]);
        assert_eq!(second.missing_blockers, ["ghost"]);
        assert_eq!(
            second.state, "pending",
            "a finished blocker does not start it"
        );
        assert_eq!(second.kind, WorkStateKind::Open);
    }

    #[test]
    fn stop_and_completion_states_say_who_reported_them() {
        let sources = WorkSources {
            background: vec![
                background("cancelled", BackgroundStatus::Cancelled, 1),
                background("failed", BackgroundStatus::Failed("exit 2".into()), 2),
            ],
            model_tasks: vec![
                model_task("done", "done", ModelTaskStatus::Completed, &[]),
                model_task("gone", "gone", ModelTaskStatus::Deleted, &[]),
            ],
            goal: Some(Ok(goal(GoalStatus::BudgetLimited))),
        };
        let view = WorkView::from_sources(&sources);
        let state = |title: &str| {
            view.items
                .iter()
                .find(|item| item.title == title)
                .map(|item| (item.state.as_str(), item.kind))
        };
        assert_eq!(
            state("cancelled"),
            Some((
                "cancel signalled; exit not confirmed",
                WorkStateKind::Stopping
            ))
        );
        assert_eq!(
            state("failed"),
            Some(("failed: exit 2", WorkStateKind::Failed))
        );
        assert_eq!(
            state("done"),
            Some(("completed (reported by the model)", WorkStateKind::Done))
        );
        assert_eq!(state("gone"), None, "deleted model tasks are not listed");
        assert_eq!(
            state("Ship the Work view"),
            Some(("budget limited", WorkStateKind::Limited))
        );
    }

    #[test]
    fn the_goal_shows_its_budget_and_a_read_failure_is_reported() {
        let view = WorkView::from_sources(&WorkSources {
            goal: Some(Ok(goal(GoalStatus::Active))),
            ..WorkSources::default()
        });
        let goal_row = &view.items[0];
        assert_eq!(goal_row.id, "goal-1");
        assert!(goal_row
            .detail
            .iter()
            .any(|(name, value)| name == "Budget" && value.contains("50000")));

        let failed = WorkView::from_sources(&WorkSources {
            goal: Some(Err("database is locked".into())),
            ..WorkSources::default()
        });
        assert!(failed.items.is_empty());
        assert_eq!(failed.goal_error.as_deref(), Some("database is locked"));
    }

    #[test]
    fn rows_are_ordered_by_start_or_creation_time_within_each_store() {
        let mut late = model_task("b-late", "late", ModelTaskStatus::Pending, &[]);
        late.created_at = Utc.with_ymd_and_hms(2026, 11, 1, 13, 0, 0).unwrap();
        let early = model_task("z-early", "early", ModelTaskStatus::Pending, &[]);
        let view = WorkView::from_sources(&WorkSources {
            background: vec![
                background("second", BackgroundStatus::Running, 30),
                background("first", BackgroundStatus::Running, 10),
            ],
            model_tasks: vec![late, early],
            goal: None,
        });
        let titles: Vec<_> = view.items.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(titles, ["first", "second", "early", "late"]);
    }

    #[test]
    fn a_blocker_that_was_deleted_is_missing() {
        let view = WorkView::from_sources(&WorkSources {
            model_tasks: vec![
                model_task("old", "old", ModelTaskStatus::Deleted, &[]),
                model_task("new", "new", ModelTaskStatus::Pending, &["old"]),
            ],
            ..WorkSources::default()
        });
        assert_eq!(view.items.len(), 1);
        assert_eq!(view.items[0].missing_blockers, ["old"]);
    }

    #[test]
    fn a_multi_line_goal_error_stays_on_one_row() {
        let view = WorkView::from_sources(&WorkSources {
            goal: Some(Err("disk I/O error\nat goals.sqlite".into())),
            ..WorkSources::default()
        });
        assert_eq!(
            view.goal_error.as_deref(),
            Some("disk I/O error at goals.sqlite")
        );
    }

    #[test]
    fn store_text_is_made_terminal_safe() {
        let mut task = model_task(
            "t\u{1b}[2J",
            "evil\u{1b}]0;title\u{7}",
            ModelTaskStatus::Pending,
            &[],
        );
        task.owner = Some("agent\u{202e}x".into());
        let view = WorkView::from_sources(&WorkSources {
            model_tasks: vec![task],
            ..WorkSources::default()
        });
        let item = &view.items[0];
        assert!(!item.id.contains('\u{1b}'));
        assert!(!item.title.contains('\u{1b}') && !item.title.contains('\u{7}'));
        assert!(!item.owner.as_deref().unwrap().contains('\u{202e}'));
    }
}
