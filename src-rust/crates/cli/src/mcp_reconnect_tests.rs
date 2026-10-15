use super::*;
use claurst_commands::{CommandContext, CommandResult, McpCommand, SlashCommand};
use claurst_mcp::{McpManager, ResourceChangedEvent};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

const URI: &str = "test://reconnect";

// A local stdio server exercises the real manager/backend without token-store
// lookups, HTTP, provider calls, or process-wide HOME/config overrides.
const SERVER: &str = r#"
import json, sys
generation = sys.argv[1]
def send(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion': '2024-11-05', 'capabilities': {'tools': {}, 'resources': {}, 'prompts': {}}, 'serverInfo': {'name': generation, 'version': '1'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': 'identity', 'inputSchema': {'type': 'object'}}, {'name': 'emit', 'inputSchema': {'type': 'object'}}]}
    elif method == 'resources/list':
        result = {'resources': [{'uri': 'test://' + generation, 'name': generation}]}
    elif method == 'prompts/list':
        result = {'prompts': [{'name': generation}]}
    elif method == 'prompts/get':
        result = {'messages': [{'role': 'user', 'content': {'type': 'text', 'text': generation}}]}
    elif method == 'tools/call':
        if request['params']['name'] == 'emit':
            send({'jsonrpc': '2.0', 'method': 'notifications/resources/updated', 'params': {'uri': 'test://reconnect'}})
        result = {'content': [{'type': 'text', 'text': generation}]}
    else:
        raise AssertionError('unexpected method ' + method)
    send({'jsonrpc': '2.0', 'id': request['id'], 'result': result})
"#;

struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("mcp.py"), SERVER).unwrap();
        Self { root }
    }

    fn config(&self, generation: &str) -> Config {
        Config {
            mcp_servers: vec![claurst_core::config::McpServerConfig {
                name: generation.to_string(),
                command: Some("python3".to_string()),
                args: vec![
                    "-I".to_string(),
                    self.root.path().join("mcp.py").display().to_string(),
                    generation.to_string(),
                ],
                env: Default::default(),
                url: None,
                server_type: "stdio".to_string(),
            }],
            ..Default::default()
        }
    }

    async fn connect(&self, generation: &str) -> Arc<McpManager> {
        let manager = timeout(
            Duration::from_secs(3),
            connect_mcp_manager_arc(&self.config(generation)),
        )
        .await
        .expect("fixture must initialize promptly")
        .unwrap();
        assert_eq!(manager.server_count(), 1, "{:?}", manager.failed_servers());
        manager
    }

    fn contexts(&self, manager: Arc<McpManager>) -> (ToolContext, CommandContext) {
        let cost_tracker = CostTracker::new();
        let config = self.config("old");
        let tool_ctx = ToolContext {
            working_dir: self.root.path().to_path_buf(),
            permission_mode: PermissionMode::BypassPermissions,
            permission_handler: Arc::new(AutoPermissionHandler {
                mode: PermissionMode::BypassPermissions,
            }),
            cost_tracker: cost_tracker.clone(),
            session_id: "reconnect-test".to_string(),
            file_history: Arc::new(ParkingMutex::new(
                claurst_core::file_history::FileHistory::new(),
            )),
            current_turn: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: Some(manager.clone()),
            config: config.clone(),
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
        };
        let cmd_ctx = CommandContext {
            config,
            cost_tracker,
            messages: vec![],
            working_dir: self.root.path().to_path_buf(),
            session_id: "reconnect-test".to_string(),
            session_title: None,
            remote_session_url: None,
            mcp_manager: Some(manager),
            mcp_auth_runner: None,
        };
        (tool_ctx, cmd_ctx)
    }
}

async fn command(args: &str, ctx: &mut CommandContext) -> String {
    match timeout(Duration::from_secs(2), McpCommand.execute(args, ctx))
        .await
        .unwrap()
    {
        CommandResult::Message(message) => message,
        other => panic!("expected MCP command output, got {other:?}"),
    }
}

#[tokio::test]
async fn reconnect_routes_commands_tools_and_notifications_to_replacement() {
    let fixture = Fixture::new();
    let old = fixture.connect("old").await;
    let new = fixture.connect("new").await;
    let (mut tool_ctx, mut cmd_ctx) = fixture.contexts(old.clone());
    let mut app_manager = Some(old.clone());
    cmd_ctx.config = fixture.config("new");
    let tools = replace_mcp_runtime(
        Some(new.clone()),
        &mut tool_ctx,
        &mut cmd_ctx,
        &mut app_manager,
    );

    assert!(command("resources", &mut cmd_ctx)
        .await
        .contains("test://new"));
    assert!(command("prompts", &mut cmd_ctx).await.contains("[new] new"));
    assert!(command("status", &mut cmd_ctx)
        .await
        .contains("connected (2 tools)"));
    assert!(Arc::ptr_eq(cmd_ctx.mcp_manager.as_ref().unwrap(), &new));
    assert!(Arc::ptr_eq(tool_ctx.mcp_manager.as_ref().unwrap(), &new));
    assert!(Arc::ptr_eq(app_manager.as_ref().unwrap(), &new));
    let wrapper = tools
        .iter()
        .find(|tool| tool.name() == "new_identity")
        .unwrap();
    let result = timeout(
        Duration::from_secs(2),
        wrapper.execute(serde_json::json!({}), &tool_ctx),
    )
    .await
    .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "new");
    assert!(!tools.iter().any(|tool| tool.name() == "old_identity"));

    // Reinstalling the active manager must not retire its notification loop.
    replace_mcp_runtime(
        Some(new.clone()),
        &mut tool_ctx,
        &mut cmd_ctx,
        &mut app_manager,
    );
    let (tx, mut rx) = mpsc::channel(1);
    new.resource_subscriptions
        .insert(("new".to_string(), URI.to_string()), tx);
    timeout(Duration::from_secs(2), new.call_tool("new_emit", None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap()
            .server_name,
        "new"
    );
    // Retained startup/cron owners may still issue ordinary MCP requests.
    let old_result = timeout(Duration::from_secs(2), old.call_tool("old_identity", None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claurst_mcp::mcp_result_to_string(&old_result), "old");
}

#[tokio::test]
async fn reconnect_retires_saturated_notifications_with_retained_owners() {
    let fixture = Fixture::new();
    for has_replacement in [true, false] {
        let old = fixture.connect("old").await;
        let retained_cron_owner = old.clone();
        let new = if has_replacement {
            Some(fixture.connect("new").await)
        } else {
            None
        };
        let (mut tool_ctx, mut cmd_ctx) = fixture.contexts(old.clone());
        let mut app_manager = Some(old.clone());
        let registry = old.resource_subscriptions.clone();
        let (tx, mut rx) = mpsc::channel(1);
        tx.try_send(ResourceChangedEvent {
            server_name: "sentinel".to_string(),
            uri: URI.to_string(),
        })
        .unwrap();
        registry.insert(("old".to_string(), URI.to_string()), tx);
        timeout(Duration::from_secs(2), old.call_tool("old_emit", None))
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(2), async {
            while Arc::strong_count(&registry) != 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("old notification must reach the saturated subscriber");

        replace_mcp_runtime(new, &mut tool_ctx, &mut cmd_ctx, &mut app_manager);
        timeout(Duration::from_secs(2), async {
            while Arc::strong_count(&registry) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("replacement must cancel old delivery despite retained startup/cron owners");
        assert_eq!(rx.try_recv().unwrap().server_name, "sentinel");
        assert!(
            rx.try_recv().is_err(),
            "the retired notification must never be delivered"
        );
        let result = timeout(
            Duration::from_secs(2),
            retained_cron_owner.call_tool("old_identity", None),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(claurst_mcp::mcp_result_to_string(&result), "old");
    }
}

#[tokio::test]
async fn reconnect_to_none_clears_command_runtime_and_preserves_filtering() {
    let fixture = Fixture::new();
    for hosted_denied in [false, true] {
        let old = fixture.connect("old").await;
        let (mut tool_ctx, mut cmd_ctx) = fixture.contexts(old.clone());
        let mut app_manager = Some(old);
        if hosted_denied {
            cmd_ctx.config.hosted_review.enabled = true;
            cmd_ctx.config.hosted_review.allow_mcp_servers = false;
        } else {
            cmd_ctx.config.mcp_servers.clear();
        }
        let replacement = connect_mcp_manager_arc(&cmd_ctx.config).await;
        assert!(replacement.is_none());
        let tools = replace_mcp_runtime(replacement, &mut tool_ctx, &mut cmd_ctx, &mut app_manager);
        assert!(
            cmd_ctx.mcp_manager.is_none(),
            "commands must not retain a retired manager"
        );
        assert!(tool_ctx.mcp_manager.is_none());
        assert!(app_manager.is_none());
        assert!(!tools.iter().any(|tool| tool.name().starts_with("old_")));
        if hosted_denied {
            assert!(!tools.iter().any(|tool| tool.name() == "Bash"));
            assert!(command("status", &mut cmd_ctx)
                .await
                .contains("manager not active"));
        } else {
            assert!(command("resources", &mut cmd_ctx)
                .await
                .contains("No MCP servers configured"));
        }
        let repeated = replace_mcp_runtime(None, &mut tool_ctx, &mut cmd_ctx, &mut app_manager);
        assert_eq!(
            tools.iter().map(|tool| tool.name()).collect::<Vec<_>>(),
            repeated.iter().map(|tool| tool.name()).collect::<Vec<_>>()
        );
    }
}
