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
            connect_mcp_manager_arc(&self.config(generation), Some(self.root.path())),
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
    timeout(
        Duration::from_secs(2),
        new.call_tool(None, "new_emit", None),
    )
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
    let old_result = timeout(
        Duration::from_secs(2),
        old.call_tool(None, "old_identity", None),
    )
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
        timeout(
            Duration::from_secs(2),
            old.call_tool(None, "old_emit", None),
        )
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
            retained_cron_owner.call_tool(None, "old_identity", None),
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
        let replacement =
            connect_mcp_manager_arc(&cmd_ctx.config, Some(&cmd_ctx.working_dir)).await;
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

fn plugin_config(fixture: &Fixture) -> Config {
    use claurst_core::plugin_mcp::{PluginMcpContribution, PluginMcpOrigin, PluginMcpSource};
    let mut config = fixture.config("plugin");
    config.project_dir = Some(fixture.root.path().to_path_buf());
    config.plugin_mcp_servers = vec![PluginMcpContribution::new(
        PluginMcpOrigin::new(
            PluginMcpSource::Project,
            fixture.root.path(),
            Some(fixture.root.path()),
        )
        .unwrap(),
        config.mcp_servers.remove(0),
    )];
    config
}

#[tokio::test]
async fn plugin_wrapper_denies_actual_foreign_caller() {
    let fixture = Fixture::new();
    let foreign = tempfile::tempdir().unwrap();
    let config = plugin_config(&fixture);
    let manager = connect_mcp_manager_arc(&config, Some(fixture.root.path()))
        .await
        .unwrap();
    assert_eq!(manager.server_count(), 1);
    let tools = build_tools_with_mcp(Some(manager.clone()));
    let wrapper = tools
        .iter()
        .find(|tool| tool.name() == "plugin_identity")
        .unwrap();
    let (mut ctx, _) = fixture.contexts(manager);
    assert!(!wrapper.execute(serde_json::json!({}), &ctx).await.is_error);
    ctx.working_dir = foreign.path().to_path_buf();
    assert!(
        wrapper.execute(serde_json::json!({}), &ctx).await.is_error,
        "a retained plugin wrapper must use the actual caller working_dir"
    );
}

#[tokio::test]
async fn plugin_project_withdrawal_denies_retained_manager_requests() {
    let fixture = Fixture::new();
    let foreign = tempfile::tempdir().unwrap();
    let mut config = plugin_config(&fixture);
    let manager = connect_mcp_manager_arc(&config, Some(fixture.root.path()))
        .await
        .unwrap();
    assert_eq!(manager.server_count(), 1);
    assert!(manager
        .call_tool(Some(fixture.root.path()), "plugin_identity", None)
        .await
        .is_ok());
    let previous = &config.plugin_mcp_servers[0];
    let unchanged = claurst_core::plugin_mcp::PluginMcpContribution::new(
        previous.origin.clone(),
        previous.config.clone(),
    );
    config
        .reconcile_plugin_mcp_servers(vec![unchanged])
        .await
        .unwrap();
    assert!(
        manager
            .call_tool(Some(fixture.root.path()), "plugin_identity", None)
            .await
            .is_ok(),
        "unchanged rediscovery preserves retained clients"
    );
    config.set_mcp_project(foreign.path()).await.unwrap();
    assert!(
        manager
            .call_tool(Some(fixture.root.path()), "plugin_identity", None)
            .await
            .is_err(),
        "withdrawing a contribution must revoke already retained managers"
    );
}

#[tokio::test]
async fn plugin_project_withdrawal_closes_child_while_manager_is_retained() {
    let fixture = Fixture::new();
    let foreign = tempfile::tempdir().unwrap();
    let marker = fixture.root.path().join("exit");
    std::fs::write(
        fixture.root.path().join("mcp.py"),
        format!("{SERVER}\nopen(sys.argv[2], 'w').write('EXIT')\n"),
    )
    .unwrap();
    let mut config = plugin_config(&fixture);
    config.plugin_mcp_servers[0]
        .config
        .args
        .push(marker.display().to_string());
    let manager = connect_mcp_manager_arc(&config, Some(fixture.root.path()))
        .await
        .unwrap();
    assert_eq!(manager.server_count(), 1);
    config.set_mcp_project(foreign.path()).await.unwrap();
    timeout(Duration::from_secs(5), async {
        while !marker.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect(
        "revocation must explicitly close the real stdio child while old manager remains alive",
    );
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "EXIT");
    assert_eq!(
        manager.server_count(),
        1,
        "retain manager through cleanup proof"
    );
}

#[tokio::test]
async fn plugin_withdrawal_reaps_a_stalled_handshake_before_returning() {
    use nix::{
        sys::signal::{kill, Signal},
        unistd::Pid,
    };
    let fixture = Fixture::new();
    let marker = fixture.root.path().join("pid");
    std::fs::write(
        fixture.root.path().join("mcp.py"),
        r#"
import os, sys, time
open(sys.argv[2], 'w').write(str(os.getpid()))
for line in sys.stdin:
    time.sleep(120)
"#,
    )
    .unwrap();
    let mut config = plugin_config(&fixture);
    config.plugin_mcp_servers[0]
        .config
        .args
        .push(marker.display().to_string());
    let stale = config.clone();
    let project = fixture.root.path().to_path_buf();
    let connection =
        tokio::spawn(async move { connect_mcp_manager_arc(&stale, Some(&project)).await });
    timeout(Duration::from_secs(5), async {
        while std::fs::read_to_string(&marker)
            .unwrap_or_default()
            .is_empty()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let pid = Pid::from_raw(std::fs::read_to_string(marker).unwrap().parse().unwrap());
    let retirement = timeout(
        Duration::from_secs(8),
        config.reconcile_plugin_mcp_servers(Vec::new()),
    )
    .await;
    let reaped_before_return = kill(pid, None).is_err();
    // Clean up our exact fixture child even on RED, before removing its directory.
    if !reaped_before_return {
        let _ = kill(pid, Signal::SIGKILL);
    }
    connection.abort();
    let _ = connection.await;
    assert!(
        retirement.is_ok(),
        "withdrawal must not wait on the unfinished handshake"
    );
    retirement.unwrap().unwrap();
    assert!(
        reaped_before_return,
        "withdrawal completion must include actual child reap during initialize"
    );
}

#[tokio::test]
async fn plugin_last_client_drop_closes_transport_while_authority_remains_configured() {
    let fixture = Fixture::new();
    let marker = fixture.root.path().join("exit");
    std::fs::write(
        fixture.root.path().join("mcp.py"),
        format!("{SERVER}\nopen(sys.argv[2], 'w').write('EXIT')\n"),
    )
    .unwrap();
    let mut config = plugin_config(&fixture);
    config.plugin_mcp_servers[0]
        .config
        .args
        .push(marker.display().to_string());
    let manager = connect_mcp_manager_arc(&config, Some(fixture.root.path()))
        .await
        .unwrap();
    assert_eq!(manager.server_count(), 1);
    drop(manager);
    let exited = timeout(Duration::from_secs(5), async {
        while !marker.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    // Also clean up the RED implementation's retained worker before tempdir drop.
    config
        .reconcile_plugin_mcp_servers(Vec::new())
        .await
        .unwrap();
    assert!(
        exited,
        "authority registration must not retain an otherwise unowned backend"
    );
}

#[tokio::test]
async fn plugin_withdrawal_cancels_blocked_request_and_reaps_child() {
    use nix::{
        sys::signal::{kill, Signal},
        unistd::Pid,
    };
    let fixture = Fixture::new();
    let marker = fixture.root.path().join("blocked");
    let script = SERVER.replace("import json, sys", "import json, sys, os, time").replace(
        "    elif method == 'tools/call':",
        "    elif method == 'tools/call':\n        open(sys.argv[2], 'w').write(str(os.getpid()))\n        time.sleep(120)",
    );
    std::fs::write(fixture.root.path().join("mcp.py"), script).unwrap();
    let mut config = plugin_config(&fixture);
    config.plugin_mcp_servers[0]
        .config
        .args
        .push(marker.display().to_string());
    let manager = connect_mcp_manager_arc(&config, Some(fixture.root.path()))
        .await
        .unwrap();
    let retained = manager.clone();
    let project = fixture.root.path().to_path_buf();
    let request = tokio::spawn(async move {
        retained
            .call_tool(Some(&project), "plugin_identity", None)
            .await
    });
    timeout(Duration::from_secs(5), async {
        while std::fs::read_to_string(&marker)
            .unwrap_or_default()
            .is_empty()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let pid = Pid::from_raw(std::fs::read_to_string(marker).unwrap().parse().unwrap());
    let retirement = timeout(
        Duration::from_secs(8),
        config.reconcile_plugin_mcp_servers(Vec::new()),
    )
    .await;
    let reaped = kill(pid, None).is_err();
    if !reaped {
        let _ = kill(pid, Signal::SIGKILL);
    }
    assert!(
        retirement.is_ok(),
        "a blocked RPC must not prevent transport retirement"
    );
    retirement.unwrap().unwrap();
    assert!(
        reaped,
        "withdrawal must reap the blocked child before completion"
    );
    assert!(timeout(Duration::from_secs(2), request)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert_eq!(manager.server_count(), 1);
}
