use claurst_plugins::{
    format_plugin_list, load_plugins, reload_plugins, run_global_pre_tool_hook, set_global_hooks,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
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
        set_global_hooks(Default::default());
        Self {
            root,
            previous_home,
            _lock: lock,
        }
    }

    fn project(&self, name: &str) -> PathBuf {
        let path = self.root.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        set_global_hooks(Default::default());
        match &self.previous_home {
            Some(value) => std::env::set_var("COVEN_CODE_HOME", value),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }
    }
}

fn write_plugin(root: &Path, capabilities: Option<Vec<&str>>) -> PathBuf {
    let plugin = root.join(".coven-code/plugins/fixture");
    std::fs::create_dir_all(&plugin).unwrap();
    let mut manifest = serde_json::json!({
        "name": "fixture",
        "hooks": { "PreToolUse": [{ "hooks": [{
            "command": if cfg!(windows) {
                "echo ran>\"%CLAUDE_PLUGIN_ROOT%\\marker\""
            } else {
                "printf ran > \"$CLAUDE_PLUGIN_ROOT/marker\""
            }
        }] }] }
    });
    if let Some(capabilities) = capabilities {
        manifest["capabilities"] = serde_json::json!(capabilities);
    }
    std::fs::write(plugin.join("plugin.json"), manifest.to_string()).unwrap();
    plugin
}

#[test]
fn project_discovery_list_and_reload_do_not_authorize_hooks() {
    let env = TestEnv::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for (name, capabilities) in [
            ("omitted", None),
            ("empty", Some(vec![])),
            ("declared", Some(vec!["shell"])),
        ] {
            let project = env.project(name);
            let plugin = write_plugin(&project, capabilities);
            let registry = load_plugins(&project, &[]).await;
            assert!(format_plugin_list(&registry).contains("fixture"));
            set_global_hooks(registry.build_hook_registry());
            run_global_pre_tool_hook("Read", &serde_json::json!({}), &project);
            assert!(
                !plugin.join("marker").exists(),
                "discovery activated {name} plugin"
            );
            assert!(!registry.is_enabled("fixture"));
            let (reloaded, _) = reload_plugins(&registry, &project, &[]).await;
            set_global_hooks(reloaded.build_hook_registry());
            run_global_pre_tool_hook("Read", &serde_json::json!({}), &project);
            assert!(!plugin.join("marker").exists());
            assert!(!reloaded.is_enabled("fixture"));
        }
    });
}

#[test]
fn legacy_name_consent_never_authorizes_a_project_plugin() {
    let env = TestEnv::new();
    let project = env.project("project");
    let plugin = write_plugin(&project, None);
    let mut settings = claurst_core::config::Settings::default();
    settings.enabled_plugins.insert("fixture".to_string());
    settings.save_sync().unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let registry = load_plugins(&project, &[]).await;
        assert!(!registry.is_enabled("fixture"));
        assert!(registry.build_hook_registry().is_empty());
        set_global_hooks(registry.build_hook_registry());
        run_global_pre_tool_hook("Read", &serde_json::json!({}), &project);
        assert!(!plugin.join("marker").exists());
    });
}

#[test]
fn user_plugin_precedence_and_saved_disable_survive_reload() {
    let env = TestEnv::new();
    let project = env.project("project");
    let project_plugin = write_plugin(&project, Some(vec!["shell"]));
    let user_plugin = claurst_plugins::default_user_plugins_dir().join("fixture");
    std::fs::create_dir_all(&user_plugin).unwrap();
    std::fs::copy(
        project_plugin.join("plugin.json"),
        user_plugin.join("plugin.json"),
    )
    .unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let registry = load_plugins(&project, &[]).await;
        assert_eq!(
            registry.get("fixture").unwrap().source,
            claurst_plugins::PluginSource::User
        );
        assert!(registry.is_enabled("fixture"));
        assert_eq!(registry.error_count(), 1);
        set_global_hooks(registry.build_hook_registry());
        run_global_pre_tool_hook("Read", &serde_json::json!({}), &project);
        assert!(user_plugin.join("marker").exists());
        std::fs::remove_file(user_plugin.join("marker")).unwrap();
        let mut settings = claurst_core::config::Settings::default();
        settings.disabled_plugins.insert("fixture".to_string());
        settings.save_sync().unwrap();
        let (reloaded, _) = reload_plugins(&registry, &project, &[]).await;
        assert!(!reloaded.is_enabled("fixture"));
        assert!(reloaded.build_hook_registry().is_empty());
        set_global_hooks(reloaded.build_hook_registry());
        run_global_pre_tool_hook("Read", &serde_json::json!({}), &project);
        assert!(!user_plugin.join("marker").exists());
        assert!(!project_plugin.join("marker").exists());
    });
}

#[cfg(unix)]
#[test]
fn user_plugin_keeps_precedence_when_project_aliases_same_folder() {
    let env = TestEnv::new();
    let engine = env.root.path().join("engine");
    std::fs::create_dir_all(&engine).unwrap();
    // Use the canonical engine path so /var versus /private/var on macOS cannot
    // hide the same-path replacement case behind different path spellings.
    std::env::set_var("COVEN_CODE_HOME", engine.canonicalize().unwrap());
    let project = env.project("project");
    let project_plugin = write_plugin(&project, Some(vec!["shell"]));
    let user_plugin = claurst_plugins::default_user_plugins_dir().join("fixture");
    std::fs::create_dir_all(user_plugin.parent().unwrap()).unwrap();
    std::fs::rename(&project_plugin, &user_plugin).unwrap();
    std::os::unix::fs::symlink(&user_plugin, &project_plugin).unwrap();

    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let registry = load_plugins(&project, &[]).await;
        set_global_hooks(registry.build_hook_registry());
        run_global_pre_tool_hook("Read", &serde_json::json!({}), &project);
        assert!(
            user_plugin.join("marker").exists(),
            "project alias disabled the trusted user hook"
        );
        assert_eq!(
            registry.get("fixture").unwrap().source,
            claurst_plugins::PluginSource::User
        );
        assert!(registry.get("fixture").unwrap().enabled);
        assert!(registry.is_enabled("fixture"));
        assert!(matches!(
            registry.errors.as_slice(),
            [claurst_plugins::PluginError::DuplicateName { .. }]
        ));
    });
}

#[test]
fn cached_project_hooks_are_scoped_to_each_caller_at_every_runner() {
    let env = TestEnv::new();
    let project = env.project("owner");
    let foreign = env.project("foreign");
    let missing = env.root.path().join("missing");
    let plugin = write_plugin(&project, Some(vec!["shell"]));
    let user_plugin = claurst_plugins::default_user_plugins_dir().join("user-fixture");
    std::fs::create_dir_all(&user_plugin).unwrap();
    let mut user_manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(plugin.join("plugin.json")).unwrap()).unwrap();
    user_manifest["name"] = serde_json::json!("user-fixture");
    std::fs::write(user_plugin.join("plugin.json"), user_manifest.to_string()).unwrap();
    let mut callers = vec![
        (foreign.clone(), false),
        (missing, false),
        (project.clone(), true),
    ];
    #[cfg(unix)]
    {
        let alias = env.root.path().join("owner-alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        callers.push((alias, true));
    }
    callers.push((foreign, false));
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut registry = load_plugins(&project, &[]).await;
        claurst_plugins::save_plugin_enabled(&project, registry.get("fixture").unwrap(), true)
            .unwrap();
        registry.enable("fixture");
        let mut hooks = registry.build_hook_registry();
        let mut post_hooks = hooks["PreToolUse"].clone();
        for hook in &mut post_hooks {
            // The separate post runner exposes tool env, not plugin-root env.
            hook.command = if cfg!(windows) {
                format!("echo ran>\"{}\\marker\"", hook.plugin_root)
            } else {
                format!(
                    "printf ran > '{}'/marker",
                    hook.plugin_root.replace('\'', "'\"'\"'")
                )
            };
        }
        hooks.insert("PostToolUse".into(), post_hooks);
        set_global_hooks(hooks.clone());
        for (caller, allowed) in callers {
            for runner in ["direct", "global-pre", "global-post", "separate-post"] {
                match runner {
                    "direct" => {
                        for hook in &hooks["PreToolUse"] {
                            claurst_plugins::hooks::run_hook_sync(hook, "{}", &caller);
                        }
                    }
                    "global-pre" => {
                        run_global_pre_tool_hook("Read", &serde_json::json!({}), &caller);
                    }
                    "global-post" => {
                        claurst_plugins::run_global_post_tool_hook(
                            "Read",
                            &serde_json::json!({}),
                            "output",
                            false,
                            &caller,
                        );
                    }
                    _ => {
                        claurst_plugins::hooks::dispatch_post_tool_hooks(
                            &hooks, "Read", "{}", "output", &caller,
                        )
                        .await;
                    }
                }
                assert_eq!(
                    plugin.join("marker").exists(),
                    allowed,
                    "{runner}: {}",
                    caller.display()
                );
                assert!(
                    user_plugin.join("marker").exists(),
                    "user hook unavailable via {runner}"
                );
                std::fs::remove_file(user_plugin.join("marker")).unwrap();
                if allowed {
                    std::fs::remove_file(plugin.join("marker")).unwrap();
                }
            }
        }
    });
}
