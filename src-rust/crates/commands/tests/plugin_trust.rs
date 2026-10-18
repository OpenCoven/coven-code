use claurst_commands::{
    execute_command, CommandContext, CommandResult, PluginCommand, SlashCommand,
};
use claurst_core::{config::Settings, cost::CostTracker};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static TEST_STATE: Mutex<()> = Mutex::new(());

struct TestEnv {
    root: tempfile::TempDir,
    previous_home: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl TestEnv {
    fn new() -> Self {
        let lock = TEST_STATE.lock().unwrap_or_else(|err| err.into_inner());
        let root = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("COVEN_CODE_HOME");
        std::env::set_var("COVEN_CODE_HOME", root.path().join("engine"));
        claurst_plugins::set_global_hooks(Default::default());
        Self {
            root,
            previous_home,
            _lock: lock,
        }
    }

    async fn context(&self, project_name: &str) -> (CommandContext, PathBuf) {
        let project = self.root.path().join(project_name);
        let plugin = project.join(".coven-code/plugins/fixture");
        std::fs::create_dir_all(&plugin).unwrap();
        let manifest = serde_json::json!({
            "name": "fixture",
            "capabilities": ["shell", "read_files"],
            "hooks": { "PreToolUse": [{ "hooks": [{
                "command": if cfg!(windows) {
                    "echo ran>\"%CLAUDE_PLUGIN_ROOT%\\marker\""
                } else {
                    "printf ran > \"$CLAUDE_PLUGIN_ROOT/marker\""
                }
            }] }] }
        });
        std::fs::write(plugin.join("plugin.json"), manifest.to_string()).unwrap();
        let registry = claurst_plugins::load_plugins(&project, &[]).await;
        claurst_plugins::set_global_registry(registry);
        (
            CommandContext {
                config: Default::default(),
                cost_tracker: CostTracker::new(),
                messages: vec![],
                working_dir: project,
                session_id: "plugin-trust-test".to_string(),
                session_title: None,
                remote_session_url: None,
                mcp_manager: None,
                mcp_auth_runner: None,
            },
            plugin,
        )
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        claurst_plugins::set_global_hooks(Default::default());
        claurst_plugins::set_global_registry(Default::default());
        match &self.previous_home {
            Some(value) => std::env::set_var("COVEN_CODE_HOME", value),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }
    }
}

#[test]
fn plugin_enable_reports_settings_read_failure() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, plugin) = env.context("project").await;
        std::fs::create_dir_all(Settings::global_settings_path()).unwrap();
        let result = PluginCommand.execute("enable fixture", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)), "{result:?}");
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(!plugin.join("marker").exists());
    });
}

#[test]
fn plugin_enable_reports_malformed_settings_without_overwriting() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, plugin) = env.context("project").await;
        std::fs::create_dir_all(Settings::config_dir()).unwrap();
        std::fs::write(Settings::global_settings_path(), "{broken").unwrap();
        let result = PluginCommand.execute("enable fixture", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)), "{result:?}");
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(!plugin.join("marker").exists());
        assert_eq!(
            std::fs::read_to_string(Settings::global_settings_path()).unwrap(),
            "{broken"
        );
    });
}

fn message(result: CommandResult) -> String {
    match result {
        CommandResult::Message(text) => text,
        other => panic!("expected success message, got {other:?}"),
    }
}

#[test]
fn enable_and_disable_persist_project_folder_consent() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, plugin) = env.context("project").await;
        message(PluginCommand.execute("enable fixture", &mut ctx).await);
        let settings = Settings::load_sync().unwrap();
        assert!(
            !settings.enabled_plugins.contains("fixture"),
            "project consent must not be name-only"
        );
        assert_eq!(settings.enabled_plugins.len(), 1);
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(
            plugin.join("marker").exists(),
            "explicit enable should activate hooks"
        );
        std::fs::remove_file(plugin.join("marker")).unwrap();
        message(PluginCommand.execute("reload", &mut ctx).await);
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(plugin.join("marker").exists());
        std::fs::remove_file(plugin.join("marker")).unwrap();

        message(PluginCommand.execute("disable fixture", &mut ctx).await);
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(
            !plugin.join("marker").exists(),
            "disable must remove live hooks"
        );
        message(PluginCommand.execute("reload", &mut ctx).await);
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(!plugin.join("marker").exists());
        assert!(!claurst_plugins::global_plugin_registry()
            .unwrap()
            .is_enabled("fixture"));
        assert!(Settings::load_sync().unwrap().enabled_plugins.is_empty());
    });
}

#[test]
fn cached_consent_cannot_follow_a_command_context_into_another_project() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut first, _) = env.context("repo-a").await;
        let (mut second, second_plugin) = env.context("repo-b").await;
        message(PluginCommand.execute("reload", &mut first).await);
        message(PluginCommand.execute("enable fixture", &mut first).await);
        message(PluginCommand.execute("reload", &mut first).await);
        let listing = message(PluginCommand.execute("list", &mut second).await);
        assert!(listing.contains("fixture [disabled]"), "{listing}");
        assert_eq!(
            claurst_plugins::global_plugin_registry()
                .unwrap()
                .get("fixture")
                .unwrap()
                .path,
            second_plugin.canonicalize().unwrap()
        );
        claurst_plugins::run_global_pre_tool_hook(
            "Read",
            &serde_json::json!({}),
            &second.working_dir,
        );
        assert!(!second_plugin.join("marker").exists());
    });
}

#[test]
fn project_settings_cannot_authorize_their_own_plugin() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, plugin) = env.context("project").await;
        message(PluginCommand.execute("enable fixture", &mut ctx).await);
        let consent = std::fs::read(Settings::global_settings_path()).unwrap();
        std::fs::remove_file(Settings::global_settings_path()).unwrap();
        std::fs::write(ctx.working_dir.join(".coven-code/settings.json"), &consent).unwrap();
        std::fs::write(
            ctx.working_dir.join(".coven-code/settings.local.json"),
            &consent,
        )
        .unwrap();
        message(PluginCommand.execute("reload", &mut ctx).await);
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(!plugin.join("marker").exists());
        assert!(!claurst_plugins::global_plugin_registry()
            .unwrap()
            .is_enabled("fixture"));
    });
}

#[cfg(unix)]
#[test]
fn plugin_enable_reports_settings_write_failure() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, plugin) = env.context("project").await;
        std::fs::create_dir_all(Settings::config_dir()).unwrap();
        std::os::unix::fs::symlink(
            env.root.path().join("missing/settings.json"),
            Settings::global_settings_path(),
        )
        .unwrap();
        let result = PluginCommand.execute("enable fixture", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)), "{result:?}");
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(!plugin.join("marker").exists());
    });
}

#[cfg(unix)]
#[test]
fn canonical_project_and_plugin_aliases_share_only_the_same_folder_grant() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, plugin) = env.context("project").await;
        let actual_plugin = env.root.path().join("actual-plugin");
        std::fs::rename(&plugin, &actual_plugin).unwrap();
        std::os::unix::fs::symlink(&actual_plugin, &plugin).unwrap();
        let project_alias = env.root.path().join("project-alias");
        std::os::unix::fs::symlink(&ctx.working_dir, &project_alias).unwrap();
        ctx.working_dir = project_alias;
        message(PluginCommand.execute("reload", &mut ctx).await);
        message(PluginCommand.execute("enable fixture", &mut ctx).await);

        ctx.working_dir = ctx.working_dir.canonicalize().unwrap();
        message(PluginCommand.execute("reload", &mut ctx).await);
        let registry = claurst_plugins::global_plugin_registry().unwrap();
        assert!(registry.is_enabled("fixture"));
        assert_eq!(
            registry.get("fixture").unwrap().path,
            actual_plugin.canonicalize().unwrap()
        );
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(actual_plugin.join("marker").exists());
        std::fs::remove_file(actual_plugin.join("marker")).unwrap();

        let replacement = env.root.path().join("replacement");
        std::fs::create_dir(&replacement).unwrap();
        std::fs::copy(
            actual_plugin.join("plugin.json"),
            replacement.join("plugin.json"),
        )
        .unwrap();
        std::fs::remove_file(&plugin).unwrap();
        std::os::unix::fs::symlink(&replacement, &plugin).unwrap();
        message(PluginCommand.execute("reload", &mut ctx).await);
        assert!(!claurst_plugins::global_plugin_registry()
            .unwrap()
            .is_enabled("fixture"));
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(!actual_plugin.join("marker").exists());
        assert!(!replacement.join("marker").exists());
    });
}

#[test]
fn missing_plugin_folder_cannot_receive_consent() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, plugin) = env.context("project").await;
        std::fs::remove_dir_all(&plugin).unwrap();
        let result = PluginCommand.execute("enable fixture", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)), "{result:?}");
        assert!(!Settings::global_settings_path().exists());
    });
}

#[test]
fn list_info_and_reload_keep_project_hooks_disabled_without_consent() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, plugin) = env.context("project").await;
        // A registry without a matching project exercises the lazy loading path.
        claurst_plugins::set_global_registry(Default::default());
        for args in ["list", "info fixture", "reload"] {
            let result = message(PluginCommand.execute(args, &mut ctx).await);
            assert!(
                result.contains("fixture") || result.contains("0 enabled"),
                "{result}"
            );
            claurst_plugins::run_global_pre_tool_hook(
                "Read",
                &serde_json::json!({}),
                &ctx.working_dir,
            );
            assert!(!plugin.join("marker").exists(), "{args} activated a hook");
            assert!(!claurst_plugins::global_plugin_registry()
                .unwrap()
                .is_enabled("fixture"));
        }
    });
}

#[test]
fn cached_plugin_command_is_not_dispatched_in_another_project() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut first, first_plugin) = env.context("repo-a").await;
        let (mut second, _) = env.context("repo-b").await;
        std::fs::create_dir(first_plugin.join("commands")).unwrap();
        std::fs::write(first_plugin.join("commands/hello.md"), "first project only").unwrap();
        message(PluginCommand.execute("reload", &mut first).await);
        message(PluginCommand.execute("enable fixture", &mut first).await);
        assert!(
            matches!(execute_command("/fixture:hello", &mut first).await,
            Some(CommandResult::UserMessage(text)) if text == "first project only")
        );
        assert!(execute_command("/fixture:hello", &mut second)
            .await
            .is_none());
        assert!(!claurst_plugins::global_plugin_registry()
            .unwrap()
            .is_enabled("fixture"));
    });
}

#[test]
fn failed_disable_does_not_report_saved_consent() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, plugin) = env.context("project").await;
        message(PluginCommand.execute("enable fixture", &mut ctx).await);
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(plugin.join("marker").exists());
        std::fs::remove_file(plugin.join("marker")).unwrap();
        std::fs::write(Settings::global_settings_path(), "{broken").unwrap();
        let result = PluginCommand.execute("disable fixture", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)), "{result:?}");
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(
            plugin.join("marker").exists(),
            "failed save must preserve live enabled state"
        );
        std::fs::remove_file(plugin.join("marker")).unwrap();
        message(PluginCommand.execute("reload", &mut ctx).await);
        assert!(!claurst_plugins::global_plugin_registry()
            .unwrap()
            .is_enabled("fixture"));
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(!plugin.join("marker").exists());
    });
}

#[cfg(unix)]
#[test]
fn folder_grants_round_trip_paths_with_json_delimiters() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut first, _) = env.context("project\"one:two\\three").await;
        let (mut second, _) = env.context("project\"one:two\\three-other").await;
        message(PluginCommand.execute("enable fixture", &mut first).await);
        message(PluginCommand.execute("reload", &mut first).await);
        assert!(claurst_plugins::global_plugin_registry()
            .unwrap()
            .is_enabled("fixture"));
        message(PluginCommand.execute("reload", &mut second).await);
        assert!(!claurst_plugins::global_plugin_registry()
            .unwrap()
            .is_enabled("fixture"));
    });
}

#[test]
fn successful_disable_save_removes_hooks_even_if_mcp_retirement_fails() {
    let env = TestEnv::new();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (mut ctx, folder) = env.context("project").await;
        message(PluginCommand.execute("enable fixture", &mut ctx).await);
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(folder.join("marker").exists());
        std::fs::remove_file(folder.join("marker")).unwrap();
        let contribution = claurst_core::plugin_mcp::PluginMcpContribution::new(
            claurst_core::plugin_mcp::PluginMcpOrigin::new(
                claurst_core::plugin_mcp::PluginMcpSource::Project,
                &folder,
                Some(&ctx.working_dir),
            )
            .unwrap(),
            claurst_core::McpServerConfig {
                name: "fixture".into(),
                command: Some("unused".into()),
                args: vec![],
                env: Default::default(),
                url: None,
                server_type: "stdio".into(),
            },
        );
        contribution
            .spawn_retirement(Some(&ctx.working_dir), |mut revoked| async move {
                let _ = revoked.changed().await;
                anyhow::bail!("fixture transport close failure")
            })
            .unwrap();
        ctx.config.plugin_mcp_servers.push(contribution);
        let result = PluginCommand.execute("disable fixture", &mut ctx).await;
        assert!(
            matches!(result, CommandResult::Error(_)),
            "cleanup failure must remain visible: {result:?}"
        );
        assert!(Settings::load_sync().unwrap().enabled_plugins.is_empty());
        claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), &ctx.working_dir);
        assert!(
            !folder.join("marker").exists(),
            "transport cleanup failure must not leave successfully disabled hooks enabled"
        );
    });
}
