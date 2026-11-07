use claurst_core::config::{Config, PermissionMode, Settings};
use claurst_core::permissions::{
    AutoPermissionHandler, ManagedAutoPermissionHandler, PermissionAction, PermissionDecision,
    PermissionHandler, PermissionManager, PermissionRequest, PermissionRule, PermissionScope,
};
use claurst_tools::{PermissionLevel, Tool, ToolContext, WebFetchTool};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

fn context(handler: Arc<dyn PermissionHandler>, mode: PermissionMode) -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("."),
        permission_mode: mode,
        permission_handler: handler,
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: "webfetch-authority-regression".to_string(),
        goal_turn: Default::default(),
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
    }
}

#[derive(Default)]
struct RecordingDenyHandler {
    requests: Mutex<Vec<PermissionRequest>>,
}

impl PermissionHandler for RecordingDenyHandler {
    fn check_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.requests.lock().unwrap().push(request.clone());
        PermissionDecision::Deny
    }

    fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        self.check_permission(request)
    }
}

#[test]
fn webfetch_is_not_advertised_as_an_unrestricted_read() {
    assert!(
        !matches!(
            WebFetchTool.permission_level(),
            PermissionLevel::None | PermissionLevel::ReadOnly
        ),
        "network access must not enter filesystem-read-only tool sets"
    );
}

#[tokio::test]
async fn webfetch_requests_network_authority_before_io() {
    let handler = Arc::new(RecordingDenyHandler::default());
    let ctx = context(handler.clone(), PermissionMode::Default);
    let result = WebFetchTool
        .execute(json!({"url": "https://example.com/"}), &ctx)
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("Permission denied"));
    let requests = handler.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tool_name, "WebFetch");
    assert!(
        !requests[0].is_read_only,
        "legacy permission handlers trust this field"
    );
}

#[tokio::test]
async fn legacy_handlers_do_not_silently_authorize_webfetch() {
    for mode in [
        PermissionMode::Default,
        PermissionMode::Plan,
        PermissionMode::AcceptEdits,
    ] {
        let handler = Arc::new(AutoPermissionHandler { mode: mode.clone() });
        let ctx = context(handler, mode);
        // A rejected permission must win before even an unsupported URL is used.
        // This fixture performs no external network IO on the unpatched backend.
        let result = WebFetchTool
            .execute(json!({"url": "file:///webfetch-permission-fixture"}), &ctx)
            .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("Permission denied"),
            "mode {:?} reached the HTTP client: {}",
            ctx.permission_mode,
            result.content
        );
    }
}

#[tokio::test]
async fn manager_backed_default_rejects_unapproved_network_access() {
    let manager = Arc::new(Mutex::new(PermissionManager::new(
        PermissionMode::Default,
        &Settings::default(),
    )));
    let handler = Arc::new(ManagedAutoPermissionHandler::new(manager));
    let ctx = context(handler, PermissionMode::Default);
    let result = WebFetchTool
        .execute(json!({"url": "https://example.com/"}), &ctx)
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("Permission denied"));
}

#[test]
fn canonical_network_policy_keeps_allow_and_deny_rules() {
    let mut manager = PermissionManager::new(PermissionMode::Default, &Settings::default());
    assert!(matches!(
        manager.evaluate("WebFetch", "fetch fixture", None, None, &[]),
        PermissionDecision::Ask { .. }
    ));
    manager.add_session_allow("WebFetch");
    assert_eq!(
        manager.evaluate("WebFetch", "fetch fixture", None, None, &[]),
        PermissionDecision::Allow
    );
    manager.add_rule(PermissionRule {
        tool_name: Some("WebFetch".to_string()),
        path_pattern: None,
        action: PermissionAction::Deny,
        scope: PermissionScope::Session,
    });
    assert_eq!(
        manager.evaluate("WebFetch", "fetch fixture", None, None, &[]),
        PermissionDecision::Deny
    );
}
