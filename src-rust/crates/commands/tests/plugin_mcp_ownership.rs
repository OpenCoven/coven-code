#![cfg(unix)]

use claurst_commands::{
    CommandContext, CommandResult, McpCommand, PluginCommand, ReloadPluginsCommand, SlashCommand,
};
use claurst_core::{
    config::{Config, McpServerConfig, Settings},
    cost::CostTracker,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

static STATE: Mutex<()> = Mutex::new(());

struct Fixture {
    root: tempfile::TempDir,
    previous_engine: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl Fixture {
    fn new() -> Self {
        let lock = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let root = tempfile::tempdir().unwrap();
        let previous_engine = std::env::var_os("COVEN_CODE_HOME");
        std::env::set_var("COVEN_CODE_HOME", root.path().join("engine"));
        claurst_plugins::set_global_registry(Default::default());
        claurst_plugins::set_global_hooks(Default::default());
        Self {
            root,
            previous_engine,
            _lock: lock,
        }
    }

    fn context(&self, project: &str) -> CommandContext {
        let working_dir = self.root.path().join(project);
        std::fs::create_dir_all(&working_dir).unwrap();
        CommandContext {
            config: Config {
                project_dir: Some(working_dir.clone()),
                ..Default::default()
            },
            cost_tracker: CostTracker::new(),
            messages: vec![],
            working_dir,
            session_id: "plugin-mcp-ownership".into(),
            session_title: None,
            remote_session_url: None,
            mcp_manager: None,
            mcp_auth_runner: None,
        }
    }

    fn write_plugin(&self, project: &Path, capabilities: &[&str], with_server: bool) -> PathBuf {
        let plugin = project.join(".coven-code/plugins/fixture");
        std::fs::create_dir_all(&plugin).unwrap();
        let servers = if with_server {
            vec![serde_json::json!({
                "name": "marker-server", "type": "stdio", "command": "sh",
                "args": ["-c", "printf x >> \"$PLUGIN_TEST_MARKER\"; exit 0"],
                "env": { "PLUGIN_TEST_MARKER": self.root.path().join("launched") }
            })]
        } else {
            vec![]
        };
        std::fs::write(
            plugin.join("plugin.json"),
            serde_json::json!({
                "name": "fixture", "capabilities": capabilities, "mcpServers": servers,
            })
            .to_string(),
        )
        .unwrap();
        plugin
    }

    async fn activate(&self, ctx: &mut CommandContext) {
        self.write_plugin(&ctx.working_dir, &["mcp", "shell"], true);
        message(PluginCommand.execute("enable fixture", ctx).await);
        message(PluginCommand.execute("reload", ctx).await);
        assert!(message(McpCommand.execute("list", ctx).await).contains("marker-server"));
    }

    fn launches(&self) -> usize {
        std::fs::read(self.root.path().join("launched")).map_or(0, |bytes| bytes.len())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        claurst_plugins::set_global_registry(Default::default());
        claurst_plugins::set_global_hooks(Default::default());
        match &self.previous_engine {
            Some(value) => std::env::set_var("COVEN_CODE_HOME", value),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }
    }
}

fn message(result: CommandResult) -> String {
    match result {
        CommandResult::Message(text) => text,
        other => panic!("expected message: {other:?}"),
    }
}

async fn reconnect(ctx: &CommandContext) {
    let configs: Vec<_> = ctx.config.effective_mcp_servers().cloned().collect();
    let manager = tokio::time::timeout(
        Duration::from_secs(3),
        claurst_mcp::McpManager::connect_all(&configs),
    )
    .await
    .expect("marker processes must finish promptly");
    assert_eq!(manager.server_count(), 0);
    assert_eq!(manager.failed_servers().len(), configs.len());
}

#[test]
fn plugin_mcp_never_becomes_saved_user_configuration() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut ctx = fixture.context("a");
        fixture.activate(&mut ctx).await;
        // ConfigChange copies the same Config into Settings; exercise that exact boundary.
        let settings = Settings {
            config: ctx.config.clone(),
            ..Default::default()
        };
        settings.save_sync().unwrap();
        let saved = Settings::load_sync().unwrap();
        assert!(
            saved.config.mcp_servers.is_empty(),
            "runtime plugin servers were promoted to user ownership"
        );
        assert!(
            ctx.config.mcp_servers.is_empty(),
            "user collection must stay independent"
        );
    });
}

#[test]
fn disabling_plugin_withdraws_reconnect_inputs_and_real_launches() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut ctx = fixture.context("a");
        fixture.activate(&mut ctx).await;
        reconnect(&ctx).await;
        assert_eq!(fixture.launches(), 1, "positive launch control");
        message(PluginCommand.execute("disable fixture", &mut ctx).await);
        reconnect(&ctx).await;
        assert_eq!(fixture.launches(), 1, "disabled contribution relaunched");
        assert!(!message(McpCommand.execute("list", &mut ctx).await).contains("marker-server"));
    });
}

#[test]
fn reload_withdraws_removed_capabilities_and_manifest_servers() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut ctx = fixture.context("a");
        for (capabilities, with_server) in [(vec!["mcp"], true), (vec!["mcp", "shell"], false)] {
            fixture.activate(&mut ctx).await;
            let before = fixture.launches();
            reconnect(&ctx).await;
            assert_eq!(fixture.launches(), before + 1);
            fixture.write_plugin(&ctx.working_dir, &capabilities, with_server);
            message(ReloadPluginsCommand.execute("", &mut ctx).await);
            reconnect(&ctx).await;
            assert_eq!(
                fixture.launches(),
                before + 1,
                "withdrawn contribution relaunched"
            );
        }
    });
}

#[test]
fn same_name_user_server_added_then_edited_survives_plugin_disable() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut ctx = fixture.context("a");
        fixture.activate(&mut ctx).await;
        ctx.config.mcp_servers.push(McpServerConfig {
            name: "marker-server".into(),
            command: Some("user-command".into()),
            args: vec![],
            env: Default::default(),
            url: None,
            server_type: "stdio".into(),
        });
        let user = ctx.config.mcp_servers.last_mut().unwrap();
        user.command = Some("edited-user-command".into());
        message(PluginCommand.execute("disable fixture", &mut ctx).await);
        assert_eq!(
            ctx.config.mcp_servers.len(),
            1,
            "plugin origin contaminated user ownership"
        );
        assert_eq!(
            ctx.config.mcp_servers[0].command.as_deref(),
            Some("edited-user-command")
        );
        let listing = message(McpCommand.execute("list", &mut ctx).await);
        assert!(listing.contains("edited-user-command"));
        assert!(!listing.contains("PLUGIN_TEST_MARKER"));
    });
}

#[test]
fn enable_contributes_immediately_and_reenable_restores_reconnect_candidates() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut ctx = fixture.context("a");
        fixture.write_plugin(&ctx.working_dir, &["mcp", "shell"], true);
        message(PluginCommand.execute("enable fixture", &mut ctx).await);
        reconnect(&ctx).await;
        assert_eq!(fixture.launches(), 1);
        message(PluginCommand.execute("disable fixture", &mut ctx).await);
        reconnect(&ctx).await;
        assert_eq!(fixture.launches(), 1);
        message(PluginCommand.execute("enable fixture", &mut ctx).await);
        reconnect(&ctx).await;
        assert_eq!(fixture.launches(), 2);
    });
}

#[test]
fn project_reload_replaces_old_contributions_without_borrowing_consent() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut ctx = fixture.context("a");
        fixture.activate(&mut ctx).await;
        let a = ctx.working_dir.clone();
        reconnect(&ctx).await;
        let b = fixture.context("b").working_dir;
        fixture.write_plugin(&b, &["mcp", "shell"], true);
        ctx.working_dir = b;
        message(PluginCommand.execute("reload", &mut ctx).await);
        reconnect(&ctx).await;
        assert_eq!(fixture.launches(), 1);
        assert!(ctx.config.plugin_mcp_servers.is_empty());
        ctx.working_dir = a;
        message(PluginCommand.execute("reload", &mut ctx).await);
        reconnect(&ctx).await;
        assert_eq!(
            fixture.launches(),
            2,
            "original folder consent remains valid"
        );
    });
}

#[test]
fn deleted_plugin_is_withdrawn_and_explicit_same_name_user_launch_is_preserved() {
    let fixture = Fixture::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut ctx = fixture.context("a");
        fixture.activate(&mut ctx).await;
        let user_marker = fixture.root.path().join("user-starts");
        ctx.config.mcp_servers.push(McpServerConfig {
            name: "marker-server".into(),
            command: Some("sh".into()),
            args: vec![
                "-c".into(),
                "printf user >> \"$USER_TEST_MARKER\"; exit 0".into(),
            ],
            env: [("USER_TEST_MARKER".into(), user_marker.display().to_string())]
                .into_iter()
                .collect(),
            url: None,
            server_type: "stdio".into(),
        });
        reconnect(&ctx).await;
        assert_eq!(
            fixture.launches(),
            0,
            "user configuration must win the name collision"
        );
        assert_eq!(std::fs::read_to_string(&user_marker).unwrap(), "user");
        std::fs::remove_file(
            ctx.working_dir
                .join(".coven-code/plugins/fixture/plugin.json"),
        )
        .unwrap();
        message(PluginCommand.execute("reload", &mut ctx).await);
        assert!(ctx.config.plugin_mcp_servers.is_empty());
        reconnect(&ctx).await;
        assert_eq!(fixture.launches(), 0);
        assert_eq!(std::fs::read_to_string(&user_marker).unwrap(), "useruser");
        let saved = Settings {
            config: ctx.config.clone(),
            ..Default::default()
        };
        saved.save_sync().unwrap();
        let loaded = Settings::load_sync().unwrap();
        assert_eq!(loaded.config.mcp_servers.len(), 1);
        assert!(loaded.config.plugin_mcp_servers.is_empty());
    });
}
