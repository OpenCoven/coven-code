use claurst_plugins::{
    check_plugin_capability, hooks, CommandRunAction, LoadedPlugin, PluginCommandDef,
    PluginManifest, PluginRegistry, PluginSource, RegisteredHook,
};
use std::path::Path;

fn plugin(root: &Path, capabilities: Option<Vec<&str>>) -> LoadedPlugin {
    LoadedPlugin {
        name: "fixture".into(),
        path: root.to_path_buf(),
        source: PluginSource::User,
        source_id: "fixture@user".into(),
        manifest: PluginManifest {
            name: "fixture".into(),
            capabilities: capabilities.map(|caps| caps.into_iter().map(String::from).collect()),
            ..Default::default()
        },
        enabled: true,
        commands_path: None,
        agents_path: None,
        skills_path: None,
        output_styles_path: None,
        hooks_config: Some(
            hooks::parse_hooks_value(&serde_json::json!({
                "PreToolUse": [{"hooks": [{"command": "exit 0"}]}]
            }))
            .unwrap(),
        ),
    }
}

fn hook(root: &Path, capabilities: Option<Vec<&str>>) -> RegisteredHook {
    RegisteredHook {
        command: if cfg!(windows) {
            "echo ran>\"%CLAUDE_PLUGIN_ROOT%\\marker\""
        } else {
            "printf ran > \"$CLAUDE_PLUGIN_ROOT/marker\""
        }
        .into(),
        matcher: None,
        blocking: false,
        plugin_root: root.to_string_lossy().into_owned(),
        plugin_name: "fixture".into(),
        plugin_source: "fixture@user".into(),
        plugin_capabilities: capabilities.map(|caps| caps.into_iter().map(String::from).collect()),
        scope: hooks::HookScope::Global,
    }
}

#[test]
fn command_effects_require_explicit_capabilities() {
    for (action, required) in [
        (
            CommandRunAction::ShellCommand {
                command: "exit 0".into(),
                plugin_root: ".".into(),
            },
            Some("shell"),
        ),
        (
            CommandRunAction::MarkdownPrompt {
                file_path: "test.md".into(),
                plugin_root: ".".into(),
            },
            Some("read_files"),
        ),
        (CommandRunAction::StaticResponse("hello".into()), None),
    ] {
        for capabilities in [
            None,
            Some(vec![]),
            Some(vec!["network"]),
            Some(vec!["shell"]),
            Some(vec!["read_files"]),
            Some(vec!["shell", "read_files"]),
        ] {
            let expected = required.is_none_or(|required| {
                capabilities
                    .as_ref()
                    .is_some_and(|caps| caps.contains(&required))
            });
            let def = PluginCommandDef {
                name: "fixture:run".into(),
                description: String::new(),
                plugin_name: "fixture".into(),
                plugin_source_id: "fixture@user".into(),
                run_action: action.clone(),
                plugin_capabilities: capabilities
                    .map(|caps| caps.into_iter().map(String::from).collect()),
            };
            assert_eq!(check_plugin_capability(&def).is_ok(), expected, "{def:?}");
        }
    }
}

#[test]
fn hook_extraction_requires_shell() {
    let root = tempfile::tempdir().unwrap();
    for capabilities in [
        None,
        Some(vec![]),
        Some(vec!["network"]),
        Some(vec!["shell"]),
    ] {
        let allowed = capabilities
            .as_ref()
            .is_some_and(|caps| caps.contains(&"shell"));
        let mut registry = PluginRegistry::new();
        registry.insert(plugin(root.path(), capabilities));
        assert_eq!(!registry.build_hook_registry().is_empty(), allowed);
    }
}

#[test]
fn direct_hook_execution_requires_shell() {
    let root = tempfile::tempdir().unwrap();
    for capabilities in [Some(vec!["shell"]), None, Some(vec![]), Some(vec!["mcp"])] {
        let allowed = capabilities
            .as_ref()
            .is_some_and(|caps| caps.contains(&"shell"));
        hooks::run_hook_sync(&hook(root.path(), capabilities), "{}", root.path());
        assert_eq!(root.path().join("marker").exists(), allowed);
        if allowed {
            std::fs::remove_file(root.path().join("marker")).unwrap();
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn separate_post_dispatch_requires_shell_and_preserves_tool_environment() {
    let root = tempfile::tempdir().unwrap();
    for capabilities in [
        Some(vec!["shell"]),
        None,
        Some(vec![]),
        Some(vec!["network"]),
    ] {
        let allowed = capabilities
            .as_ref()
            .is_some_and(|caps| caps.contains(&"shell"));
        let mut registered = hook(root.path(), capabilities);
        // The separate post runner intentionally exposes the tool environment.
        registered.command = format!("printf '%s|%s|%s' \"$CLAUDE_TOOL_NAME\" \"$CLAUDE_TOOL_INPUT\" \"$CLAUDE_TOOL_RESULT\" > '{}'; printf done", root.path().join("marker").display());
        let registry = [("PostToolUse".into(), vec![registered])]
            .into_iter()
            .collect();
        let outputs =
            hooks::dispatch_post_tool_hooks(&registry, "Read", "input", "result", root.path())
                .await;
        assert_eq!(root.path().join("marker").exists(), allowed);
        if allowed {
            assert_eq!(
                std::fs::read_to_string(root.path().join("marker")).unwrap(),
                "Read|input|result"
            );
            assert_eq!(outputs, ["done"]);
            std::fs::remove_file(root.path().join("marker")).unwrap();
        } else {
            assert!(outputs.is_empty());
        }
    }
}

#[test]
fn mcp_transport_requires_mcp_and_its_effect_capability() {
    let root = tempfile::tempdir().unwrap();
    for transport in ["stdio", "sse", "http", "unknown", ""] {
        for capabilities in [
            None,
            Some(vec![]),
            Some(vec!["mcp"]),
            Some(vec!["shell"]),
            Some(vec!["network"]),
            Some(vec!["mcp", "shell"]),
            Some(vec!["mcp", "network"]),
            Some(vec!["mcp", "shell", "network"]),
        ] {
            let caps = capabilities.as_deref().unwrap_or_default();
            let allowed = caps.contains(&"mcp")
                && match transport {
                    "stdio" => caps.contains(&"shell"),
                    "sse" | "http" => caps.contains(&"network"),
                    _ => false,
                };
            let mut loaded = plugin(root.path(), capabilities);
            loaded
                .manifest
                .mcp_servers
                .push(claurst_plugins::PluginMcpServer {
                    name: "server".into(),
                    server_type: transport.into(),
                    ..Default::default()
                });
            let mut registry = PluginRegistry::new();
            registry.insert(loaded);
            assert_eq!(
                !registry.all_mcp_servers().is_empty(),
                allowed,
                "{transport}"
            );
        }
    }
}

#[test]
fn lsp_extraction_requires_shell() {
    let root = tempfile::tempdir().unwrap();
    for capabilities in [None, Some(vec![]), Some(vec!["mcp"]), Some(vec!["shell"])] {
        let allowed = capabilities
            .as_ref()
            .is_some_and(|caps| caps.contains(&"shell"));
        let mut loaded = plugin(root.path(), capabilities);
        loaded
            .manifest
            .lsp_servers
            .push(claurst_plugins::PluginLspServer {
                name: "server".into(),
                command: "unused".into(),
                ..Default::default()
            });
        let mut registry = PluginRegistry::new();
        registry.insert(loaded);
        assert_eq!(!registry.all_lsp_servers().is_empty(), allowed);
    }
}

#[test]
fn unresolved_project_registration_never_becomes_global() {
    let root = tempfile::tempdir().unwrap();
    let mut loaded = plugin(root.path(), Some(vec!["shell"]));
    loaded.source = PluginSource::Project;
    // Deliberately misleading display string must have no authority.
    loaded.source_id = "fixture@user".into();
    for project_root in [None, Some(root.path().join("missing"))] {
        let mut registry = hooks::HookRegistry::new();
        hooks::register_plugin_hooks(&loaded, project_root.as_deref(), &mut registry);
        assert!(registry.is_empty());
    }
    let mut registry = PluginRegistry::new();
    registry.insert(loaded);
    assert!(registry.build_hook_registry().is_empty());
}

#[test]
fn missing_and_unknown_serialized_scope_cannot_authorize_direct_hooks() {
    let root = tempfile::tempdir().unwrap();
    let mut value = serde_json::to_value(hook(root.path(), Some(vec!["shell"]))).unwrap();
    value.as_object_mut().unwrap().remove("scope");
    let missing_scope: RegisteredHook = serde_json::from_value(value.clone()).unwrap();
    assert!(matches!(
        hooks::run_hook_sync(&missing_scope, "{}", root.path()),
        hooks::HookOutcome::Error(_)
    ));
    assert!(!root.path().join("marker").exists());
    value["scope"] = serde_json::json!("Unknown");
    assert!(serde_json::from_value::<RegisteredHook>(value).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn authorized_hooks_preserve_blocking_and_post_matchers() {
    let root = tempfile::tempdir().unwrap();
    let mut registered = hook(root.path(), Some(vec!["shell"]));
    registered.command = "printf reason >&2; exit 7".into();
    registered.blocking = true;
    assert!(
        matches!(hooks::run_hook_sync(&registered, "{}", root.path()), hooks::HookOutcome::Deny(reason) if reason.contains("reason") && reason.contains("7"))
    );
    registered.blocking = false;
    assert!(matches!(
        hooks::run_hook_sync(&registered, "{}", root.path()),
        hooks::HookOutcome::Allow
    ));
    registered.command = "printf matched".into();
    registered.matcher = Some("Read*".into());
    let registry = [("PostToolUse".into(), vec![registered])]
        .into_iter()
        .collect();
    assert!(
        hooks::dispatch_post_tool_hooks(&registry, "Write", "{}", "{}", root.path())
            .await
            .is_empty()
    );
    assert_eq!(
        hooks::dispatch_post_tool_hooks(&registry, "ReadFile", "{}", "{}", root.path()).await,
        ["matched"]
    );
}

#[cfg(unix)]
#[test]
fn global_hooks_enforce_grants_and_preserve_blocking_and_exact_matchers() {
    // This integration binary has just one global-registry writer.
    struct ResetHooks;
    impl Drop for ResetHooks {
        fn drop(&mut self) {
            claurst_plugins::set_global_hooks(Default::default());
        }
    }
    let _reset = ResetHooks;
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("marker");
    for capabilities in [Some(vec!["shell"]), None, Some(vec![])] {
        let allowed = capabilities
            .as_ref()
            .is_some_and(|caps| caps.contains(&"shell"));
        let mut registered = hook(root.path(), capabilities);
        registered.matcher = Some("Read".into());
        registered.blocking = true;
        registered.command.push_str("; printf blocked >&2; exit 7");
        claurst_plugins::set_global_hooks(
            [
                ("PreToolUse".into(), vec![registered.clone()]),
                ("PostToolUse".into(), vec![registered]),
            ]
            .into_iter()
            .collect(),
        );
        assert!(matches!(
            claurst_plugins::run_global_pre_tool_hook("Write", &serde_json::json!({}), root.path()),
            hooks::HookOutcome::Allow
        ));
        assert!(!marker.exists());
        let outcome =
            claurst_plugins::run_global_pre_tool_hook("Read", &serde_json::json!({}), root.path());
        assert_eq!(matches!(outcome, hooks::HookOutcome::Deny(_)), allowed);
        assert_eq!(marker.exists(), allowed);
        if allowed {
            std::fs::remove_file(&marker).unwrap();
        }
        claurst_plugins::run_global_post_tool_hook(
            "Write",
            &serde_json::json!({}),
            "output",
            false,
            root.path(),
        );
        assert!(!marker.exists());
        claurst_plugins::run_global_post_tool_hook(
            "Read",
            &serde_json::json!({}),
            "output",
            false,
            root.path(),
        );
        assert_eq!(marker.exists(), allowed);
        if allowed {
            std::fs::remove_file(&marker).unwrap();
        }
    }
}
