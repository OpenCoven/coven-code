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

fn complete_goal(
    input: Value,
    ctx: &ToolContext,
    scoped_path: Option<&std::path::Path>,
) -> ToolResult {
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

    let Some(turn) = ctx.goal_turn.captured() else {
        return ToolResult::error("No active goal was captured for this turn.");
    };
    if turn.goal().session_id != ctx.session_id {
        return ToolResult::error("Goal completion session does not match the captured turn.");
    }
    if let Some(path) = scoped_path {
        match std::fs::canonicalize(path) {
            Ok(path) if path == turn.store_path() => {}
            _ => {
                return ToolResult::error("Goal completion store does not match the captured turn.")
            }
        }
    }
    let store = match claurst_core::GoalStore::open(turn.store_path()) {
        Ok(store) => store,
        Err(error) => return ToolResult::error(format!("Could not open goal store: {error}")),
    };
    match store.complete_goal_turn(turn) {
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
        complete_goal(input, ctx, None)
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
        complete_goal(input, ctx, Some(&self.goal_store_path))
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
            goal_turn: Default::default(),
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
        let schema = tool.input_schema();
        assert_eq!(schema["properties"].as_object().unwrap().len(), 2);
        assert_eq!(schema["required"], json!(["audit_summary", "evidence"]));
    }

    #[tokio::test]
    async fn old_turn_cannot_complete_replacement_goal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = claurst_core::GoalStore::open(&path).unwrap();
        let original = store.set_goal("target", "finish A", Some(500)).unwrap();
        let mut ctx = test_tool_context(&original.session_id);
        ctx.goal_turn = claurst_core::goal::GoalTurnContext::Captured(
            store.capture_active_goal("target").unwrap(),
        );
        store.set_goal("target", "preserve B", Some(900)).unwrap();
        store.record_completed_turn("target", 120, 7).unwrap();
        let before = store.try_get_goal("target").unwrap().unwrap();

        let result = GoalCompleteTool::at_path(path)
            .execute(
                serde_json::json!({
                    "audit_summary": "finished A",
                    "evidence": "A tests passed"
                }),
                &ctx,
            )
            .await;

        assert!(
            result.is_error,
            "old turn completed replacement: {}",
            result.content
        );
        assert_eq!(
            store.try_get_goal("target").unwrap().unwrap(),
            before,
            "every replacement field must remain unchanged"
        );
    }

    #[tokio::test]
    async fn explicit_path_completes_only_the_matching_active_goal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.sqlite");
        let store = claurst_core::GoalStore::open(&path).unwrap();
        store.set_goal("target", "finish", None).unwrap();
        store.set_goal("other", "stay active", None).unwrap();

        let mut ctx = test_tool_context("target");
        ctx.goal_turn = claurst_core::goal::GoalTurnContext::Captured(
            store.capture_active_goal("target").unwrap(),
        );
        let result = GoalCompleteTool::at_path(path.clone())
            .execute(
                serde_json::json!({
                    "audit_summary": "finished",
                    "evidence": "tests passed"
                }),
                &ctx,
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
    async fn both_tools_reject_every_inactive_or_missing_captured_goal() {
        for scoped in [false, true] {
            for status in [
                Some(claurst_core::GoalStatus::Paused),
                Some(claurst_core::GoalStatus::Complete),
                Some(claurst_core::GoalStatus::BudgetLimited),
                None,
            ] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("goals.sqlite");
                let store = claurst_core::GoalStore::open(&path).unwrap();
                store.set_goal("target", "finish", None).unwrap();
                let other = store.set_goal("other", "preserve", Some(800)).unwrap();
                let mut ctx = test_tool_context("target");
                ctx.goal_turn = claurst_core::goal::GoalTurnContext::Captured(
                    store.capture_active_goal("target").unwrap(),
                );
                match status {
                    Some(status) => store.set_status("target", status).unwrap(),
                    None => store.clear_goal("target").unwrap(),
                }
                let before = store.try_get_goal("target").unwrap();
                let tool: Box<dyn Tool> = if scoped {
                    Box::new(GoalCompleteTool::at_path(path))
                } else {
                    Box::new(GoalCompleteTool)
                };
                let result = tool.execute(valid_input(), &ctx).await;
                assert!(result.is_error, "{}", result.content);
                assert_eq!(store.try_get_goal("target").unwrap(), before);
                assert_eq!(store.try_get_goal("other").unwrap().unwrap(), other);
            }
        }
    }

    fn valid_input() -> Value {
        json!({"audit_summary": "finished A", "evidence": "A tests passed"})
    }

    #[tokio::test]
    async fn missing_authority_and_mismatched_session_deny_without_opening_scoped_store() {
        let dir = tempfile::tempdir().unwrap();
        let unopened = dir.path().join("must-not-exist.sqlite");
        let tool = GoalCompleteTool::at_path(unopened.clone());
        let mut ctx = test_tool_context("target");
        for context in [
            claurst_core::goal::GoalTurnContext::Captured(None),
            claurst_core::goal::GoalTurnContext::Pending(unopened.clone()),
        ] {
            ctx.goal_turn = context;
            assert!(tool.execute(valid_input(), &ctx).await.is_error);
            assert!(GoalCompleteTool.execute(valid_input(), &ctx).await.is_error);
            assert!(!unopened.exists());
        }
        let path = dir.path().join("goals.sqlite");
        let store = claurst_core::GoalStore::open(&path).unwrap();
        let original = store.set_goal("target", "finish", None).unwrap();
        ctx.goal_turn = claurst_core::goal::GoalTurnContext::Captured(
            store.capture_active_goal("target").unwrap(),
        );
        ctx.session_id = "different".to_string();
        assert!(GoalCompleteTool.execute(valid_input(), &ctx).await.is_error);
        assert_eq!(store.try_get_goal("target").unwrap().unwrap(), original);
        assert!(store.try_get_goal("different").unwrap().is_none());
    }

    #[tokio::test]
    async fn scoped_tool_cannot_reuse_a_captured_identity_in_a_copied_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original.sqlite");
        let copied_path = dir.path().join("copied.sqlite");
        let store = claurst_core::GoalStore::open(&path).unwrap();
        let original = store.set_goal("target", "finish", None).unwrap();
        let mut ctx = test_tool_context("target");
        ctx.goal_turn = claurst_core::goal::GoalTurnContext::Captured(
            store.capture_active_goal("target").unwrap(),
        );
        std::fs::copy(&path, &copied_path).unwrap();
        let result = GoalCompleteTool::at_path(copied_path.clone())
            .execute(valid_input(), &ctx)
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("store does not match"));
        let copied = claurst_core::GoalStore::open(&copied_path).unwrap();
        assert_eq!(copied.try_get_goal("target").unwrap().unwrap(), original);
        assert_eq!(store.try_get_goal("target").unwrap().unwrap(), original);
        assert!(!GoalCompleteTool.execute(valid_input(), &ctx).await.is_error);
        assert_eq!(copied.try_get_goal("target").unwrap().unwrap(), original);
    }

    #[test]
    fn default_path_change_cannot_redirect_captured_completion() {
        // Run environment changes in an isolated child, never the test process.
        let dir = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "goal_complete::tests::default_path_change_child",
                "--nocapture",
            ])
            .env("COVEN_CODE_GOAL_COMPLETION_FIXTURE", dir.path())
            .env("COVEN_CODE_HOME", dir.path().join("first"))
            .env("COVEN_CODE_GOALS", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn default_path_change_child() {
        let Some(root) = std::env::var_os("COVEN_CODE_GOAL_COMPLETION_FIXTURE") else {
            return;
        };
        let root = PathBuf::from(root);
        std::fs::create_dir(root.join("first")).unwrap();
        std::fs::create_dir(root.join("second")).unwrap();
        let first_path = claurst_core::GoalStore::default_path();
        assert_eq!(first_path, root.join("first/goals.sqlite"));
        let first = claurst_core::GoalStore::open(&first_path).unwrap();
        first.set_goal("target", "A", None).unwrap();
        let mut ctx = test_tool_context("target");
        ctx.goal_turn = claurst_core::goal::GoalTurnContext::pending_default();
        ctx.goal_turn.capture("target").unwrap();
        std::env::set_var("COVEN_CODE_HOME", root.join("second"));
        let second_path = claurst_core::GoalStore::default_path();
        assert_ne!(first_path, second_path);
        let second = claurst_core::GoalStore::open(&second_path).unwrap();
        let replacement = second.set_goal("target", "B", Some(900)).unwrap();
        let result = GoalCompleteTool.execute(valid_input(), &ctx).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(
            first.try_get_goal("target").unwrap().unwrap().status,
            claurst_core::GoalStatus::Complete
        );
        assert_eq!(second.try_get_goal("target").unwrap().unwrap(), replacement);
        for disabled in ["0", "false"] {
            std::env::set_var("COVEN_CODE_GOALS", disabled);
            assert!(matches!(
                claurst_core::goal::GoalTurnContext::pending_default(),
                claurst_core::goal::GoalTurnContext::Captured(None)
            ));
        }
        std::env::remove_var("COVEN_CODE_GOALS");
        assert!(matches!(
            claurst_core::goal::GoalTurnContext::pending_default(),
            claurst_core::goal::GoalTurnContext::Pending(_)
        ));
    }
}
