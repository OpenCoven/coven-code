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
            run_global_pre_tool_hook("Read", &serde_json::json!({}));
            assert!(
                !plugin.join("marker").exists(),
                "discovery activated {name} plugin"
            );
            assert!(!registry.is_enabled("fixture"));
            let (reloaded, _) = reload_plugins(&registry, &project, &[]).await;
            set_global_hooks(reloaded.build_hook_registry());
            run_global_pre_tool_hook("Read", &serde_json::json!({}));
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
        run_global_pre_tool_hook("Read", &serde_json::json!({}));
        assert!(!plugin.join("marker").exists());
    });
}

#[test]
fn user_plugin_precedence_and_saved_disable_survive_reload() {
    let env = TestEnv::new();
    let project = env.project("project");
    let project_plugin = write_plugin(&project, None);
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
        run_global_pre_tool_hook("Read", &serde_json::json!({}));
        assert!(user_plugin.join("marker").exists());
        std::fs::remove_file(user_plugin.join("marker")).unwrap();
        let mut settings = claurst_core::config::Settings::default();
        settings.disabled_plugins.insert("fixture".to_string());
        settings.save_sync().unwrap();
        let (reloaded, _) = reload_plugins(&registry, &project, &[]).await;
        assert!(!reloaded.is_enabled("fixture"));
        assert!(reloaded.build_hook_registry().is_empty());
        set_global_hooks(reloaded.build_hook_registry());
        run_global_pre_tool_hook("Read", &serde_json::json!({}));
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
        run_global_pre_tool_hook("Read", &serde_json::json!({}));
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
