#![cfg(unix)]

use claurst_plugins::{
    LoadedPlugin, PluginManifest, PluginMcpServer, PluginRegistry, PluginSource,
};

#[tokio::test]
async fn plugin_stdio_capabilities_gate_real_manager_process_launch() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("launched");
    for capabilities in [
        Some(vec!["mcp", "shell"]),
        None,
        Some(vec![]),
        Some(vec!["mcp"]),
        Some(vec!["shell"]),
        Some(vec!["mcp", "network"]),
    ] {
        let allowed = capabilities
            .as_ref()
            .is_some_and(|caps| caps.contains(&"mcp") && caps.contains(&"shell"));
        let mut registry = PluginRegistry::new();
        registry.insert(LoadedPlugin {
            name: "fixture".into(),
            path: root.path().to_path_buf(),
            source: PluginSource::User,
            source_id: "fixture@user".into(),
            enabled: true,
            commands_path: None,
            agents_path: None,
            skills_path: None,
            output_styles_path: None,
            hooks_config: None,
            manifest: PluginManifest {
                name: "fixture".into(),
                capabilities: capabilities.map(|caps| caps.into_iter().map(String::from).collect()),
                mcp_servers: vec![PluginMcpServer {
                    name: "marker-server".into(),
                    command: Some("sh".into()),
                    // Exits immediately: proves launch without a server or credentials.
                    args: vec![
                        "-c".into(),
                        "printf started > \"$PLUGIN_TEST_MARKER\"; exit 0".into(),
                    ],
                    env: [(
                        "PLUGIN_TEST_MARKER".into(),
                        marker.to_string_lossy().into_owned(),
                    )]
                    .into_iter()
                    .collect(),
                    server_type: "stdio".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        });
        let configs = registry.all_mcp_servers();
        let manager = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            claurst_mcp::McpManager::connect_all(&configs),
        )
        .await
        .expect("marker child must exit promptly");
        assert_eq!(
            marker.exists(),
            allowed,
            "unexpected process launch for {configs:?}"
        );
        assert_eq!(manager.failed_servers().len(), usize::from(allowed));
        assert_eq!(manager.server_count(), 0);
        drop(manager);
        if allowed {
            std::fs::remove_file(&marker).unwrap();
        }
    }
}

#[tokio::test]
async fn slash_command_adapter_denies_effects_without_grants_and_allows_static_output() {
    use claurst_commands::{
        CommandContext, CommandResult, PluginSlashCommandAdapter, SlashCommand,
    };
    use claurst_plugins::{CommandRunAction, PluginCommandDef};
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("marker");
    let prompt = root.path().join("prompt.md");
    std::fs::write(&prompt, "fixture prompt").unwrap();
    let mut ctx = CommandContext {
        config: Default::default(),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        messages: vec![],
        working_dir: root.path().to_path_buf(),
        session_id: "capabilities".into(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    for capabilities in [Some(vec!["shell", "read_files"]), None, Some(vec![])] {
        let allowed = capabilities.as_ref().is_some_and(|caps| !caps.is_empty());
        for action in [
            CommandRunAction::ShellCommand {
                command: "printf ran > \"$CLAUDE_PLUGIN_ROOT/marker\"; printf ran".into(),
                plugin_root: root.path().to_string_lossy().into_owned(),
            },
            CommandRunAction::MarkdownPrompt {
                file_path: prompt.to_string_lossy().into_owned(),
                plugin_root: root.path().to_string_lossy().into_owned(),
            },
            CommandRunAction::StaticResponse("static".into()),
        ] {
            let is_static = matches!(action, CommandRunAction::StaticResponse(_));
            let is_shell = matches!(action, CommandRunAction::ShellCommand { .. });
            let command = PluginSlashCommandAdapter {
                def: PluginCommandDef {
                    name: "fixture:command".into(),
                    description: String::new(),
                    plugin_name: "fixture".into(),
                    plugin_source_id: "fixture@user".into(),
                    run_action: action,
                    plugin_capabilities: capabilities
                        .as_ref()
                        .map(|caps| caps.iter().map(|cap| String::from(*cap)).collect()),
                },
            };
            let result = command.execute("", &mut ctx).await;
            assert_eq!(
                matches!(result, CommandResult::Error(_)),
                !(allowed || is_static),
                "{result:?}"
            );
            assert_eq!(marker.exists(), allowed && is_shell);
            if marker.exists() {
                std::fs::remove_file(&marker).unwrap();
            }
        }
    }
}
