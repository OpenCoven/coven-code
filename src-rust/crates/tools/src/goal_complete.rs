// GoalCompleteTool — marks the active goal as complete.
//
// This is the tool the model calls after passing a self-audit that verifies
// the goal objective has been fully achieved.  Calling it without a thorough
// audit_summary + evidence is considered a violation of the goal contract.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct GoalCompleteTool;

pub struct PathScopedGoalCompleteTool {
    goal_store_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct GoalCompleteInput {
    /// A concise summary of what was accomplished (the audit).
    audit_summary: String,
    /// Concrete evidence: test output, file diffs, command results, etc.
    evidence: String,
}

impl GoalCompleteTool {
    pub fn at_path(goal_store_path: PathBuf) -> PathScopedGoalCompleteTool {
        PathScopedGoalCompleteTool { goal_store_path }
    }
}

fn complete_goal(input: Value, ctx: &ToolContext, store: claurst_core::GoalStore) -> ToolResult {
    let params: GoalCompleteInput = match serde_json::from_value(input) {
        Ok(params) => params,
        Err(error) => return ToolResult::error(format!("Invalid input: {error}")),
    };

    if params.audit_summary.trim().is_empty() {
        return ToolResult::error(
            "audit_summary cannot be empty. Provide a concise description of what was completed."
                .to_string(),
        );
    }
    if params.evidence.trim().is_empty() {
        return ToolResult::error(
            "evidence cannot be empty. Provide test output, diffs, or command results.".to_string(),
        );
    }

    match store.complete_active_goal(&ctx.session_id) {
        Ok(()) => ToolResult::success(format!(
            "Goal marked complete.\n\nAudit summary: {}\n\nEvidence: {}",
            params.audit_summary, params.evidence,
        )),
        Err(error) => ToolResult::error(format!("Failed to mark goal complete: {error}")),
    }
}

#[async_trait]
impl Tool for GoalCompleteTool {
    fn name(&self) -> &str {
        "GoalComplete"
    }

    fn description(&self) -> &str {
        "Mark the active goal as complete. ONLY call this after a genuine completion audit:\n\
         1. Restate the goal as concrete deliverables.\n\
         2. Check each deliverable against real output, test results, or file diffs.\n\
         3. Confirm all deliverables are satisfied.\n\
         Calling this without a real audit is a goal contract violation."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "audit_summary": {
                    "type": "string",
                    "description": "Concise summary of what was accomplished and verified"
                },
                "evidence": {
                    "type": "string",
                    "description": "Concrete evidence of completion: test output, diffs, command results"
                }
            },
            "required": ["audit_summary", "evidence"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        match claurst_core::GoalStore::open_default() {
            None => ToolResult::error("Could not open goal store.".to_string()),
            Some(store) => complete_goal(input, ctx, store),
        }
    }
}

impl PathScopedGoalCompleteTool {
    fn open_store(&self) -> Result<claurst_core::GoalStore, String> {
        claurst_core::GoalStore::open(&self.goal_store_path).map_err(|error| error.to_string())
    }
}

#[async_trait]
impl Tool for PathScopedGoalCompleteTool {
    fn name(&self) -> &str {
        "GoalComplete"
    }

    fn description(&self) -> &str {
        GoalCompleteTool.description()
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn input_schema(&self) -> Value {
        GoalCompleteTool.input_schema()
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        match self.open_store() {
            Ok(store) => complete_goal(input, ctx, store),
            Err(error) => ToolResult::error(format!("Could not open goal store: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    use super::*;
    use claurst_core::config::{Config, PermissionMode};
    use claurst_core::file_history::FileHistory;
    use claurst_core::permissions::AutoPermissionHandler;

    fn test_tool_context(session_id: &str) -> ToolContext {
        ToolContext {
            working_dir: PathBuf::from("/workspace"),
            permission_mode: PermissionMode::Default,
            permission_handler: Arc::new(AutoPermissionHandler {
                mode: PermissionMode::Default,
            }),
            cost_tracker: claurst_core::cost::CostTracker::new(),
            session_id: session_id.to_string(),
            file_history: Arc::new(parking_lot::Mutex::new(FileHistory::new())),
            current_turn: Arc::new(AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: None,
            config: Config::default(),
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
        }
    }

    #[test]
    fn default_tool_remains_unit_constructible() {
        let tool = GoalCompleteTool;
        assert_eq!(tool.name(), "GoalComplete");
    }

    #[tokio::test]
    async fn explicit_path_completes_only_the_matching_active_goal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = claurst_core::GoalStore::open(&path).unwrap();
        store.set_goal("target", "finish", None).unwrap();
        store.set_goal("other", "stay active", None).unwrap();

        let result = GoalCompleteTool::at_path(path.clone())
            .execute(
                serde_json::json!({
                    "audit_summary": "finished",
                    "evidence": "tests passed"
                }),
                &test_tool_context("target"),
            )
            .await;

        assert!(!result.is_error);
        let reopened = claurst_core::GoalStore::open(&path).unwrap();
        assert_eq!(
            reopened.try_get_goal("target").unwrap().unwrap().status,
            claurst_core::GoalStatus::Complete
        );
        assert_eq!(
            reopened.try_get_goal("other").unwrap().unwrap().status,
            claurst_core::GoalStatus::Active
        );
    }

    #[tokio::test]
    async fn explicit_path_rejects_paused_and_missing_goals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = claurst_core::GoalStore::open(&path).unwrap();
        store.set_goal("paused", "finish", None).unwrap();
        store
            .set_status("paused", claurst_core::GoalStatus::Paused)
            .unwrap();
        let input = serde_json::json!({
            "audit_summary": "finished",
            "evidence": "tests passed"
        });

        assert!(
            GoalCompleteTool::at_path(path.clone())
                .execute(input.clone(), &test_tool_context("paused"))
                .await
                .is_error
        );
        assert!(
            GoalCompleteTool::at_path(path)
                .execute(input, &test_tool_context("missing"))
                .await
                .is_error
        );
    }
}
