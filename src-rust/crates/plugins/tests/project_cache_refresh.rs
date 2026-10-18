use claurst_plugins::{
    global_plugin_registry, load_plugins, refresh_cached_plugins, run_global_pre_tool_hook,
    save_plugin_enabled, set_global_hooks, set_global_registry,
};
use std::ffi::OsString;
use std::path::Path;
use std::sync::Mutex;

static TEST_STATE: Mutex<()> = Mutex::new(());

struct ConfigHome(Option<OsString>);

impl Drop for ConfigHome {
    fn drop(&mut self) {
        set_global_hooks(Default::default());
        set_global_registry(Default::default());
        match &self.0 {
            Some(value) => std::env::set_var("COVEN_CODE_HOME", value),
            None => std::env::remove_var("COVEN_CODE_HOME"),
        }
    }
}

fn fixture(path: &Path, name: &str) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::write(
        path.join("plugin.json"),
        serde_json::json!({
            "name": name,
            "capabilities": ["shell"],
            "hooks": { "PreToolUse": [{ "hooks": [{
                "command": if cfg!(windows) {
                    "echo ran>\"%CLAUDE_PLUGIN_ROOT%\\marker\""
                } else {
                    "printf ran > \"$CLAUDE_PLUGIN_ROOT/marker\""
                }
            }] }] }
        })
        .to_string(),
    )
    .unwrap();
}

// This integration binary has one test so its initial global registry is genuinely None.
#[test]
fn project_transition_refresh_preserves_startup_and_same_project_but_replaces_stale_hooks() {
    let _lock = TEST_STATE.lock().unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let _home = ConfigHome(std::env::var_os("COVEN_CODE_HOME"));
    std::env::set_var("COVEN_CODE_HOME", root.path().join("engine"));
    let first = root.path().join("repo-a");
    let second = root.path().join("repo-b");
    let first_plugin = first.join(".coven-code/plugins/fixture");
    let second_plugin = second.join(".coven-code/plugins/fixture");
    fixture(&first_plugin, "fixture");
    fixture(&second_plugin, "fixture");
    let user_plugin = claurst_plugins::default_user_plugins_dir().join("user-fixture");
    fixture(&user_plugin, "user-fixture");

    tokio::runtime::Runtime::new().unwrap().block_on(async {
        assert!(global_plugin_registry().is_none());
        refresh_cached_plugins(&first).await;
        assert!(
            global_plugin_registry().is_none(),
            "resume must not activate startup discovery"
        );
        run_global_pre_tool_hook("Read", &serde_json::json!({}), &first);
        assert!(!user_plugin.join("marker").exists());

        let extra = root.path().join("extra");
        fixture(&extra, "extra-fixture");
        let mut registry = load_plugins(&first, &[extra]).await;
        save_plugin_enabled(&first, registry.get("fixture").unwrap(), true).unwrap();
        registry.enable("fixture");
        set_global_hooks(registry.build_hook_registry());
        set_global_registry(registry);
        let original = global_plugin_registry().unwrap();
        refresh_cached_plugins(&first.join(".")).await;
        let preserved = global_plugin_registry().unwrap();
        assert!(std::sync::Arc::ptr_eq(&original, &preserved));
        assert!(preserved.get("extra-fixture").is_some());
        run_global_pre_tool_hook("Read", &serde_json::json!({}), &first);
        assert!(first_plugin.join("marker").exists());
        std::fs::remove_file(first_plugin.join("marker")).unwrap();

        // The CLI calls this immediately after session resume changes its working directory.
        // No /plugin or dynamic plugin command runs before hook dispatch here.
        refresh_cached_plugins(&second).await;
        run_global_pre_tool_hook("Read", &serde_json::json!({}), &second);
        assert!(
            !first_plugin.join("marker").exists(),
            "old project hook survived resume"
        );
        assert!(
            !second_plugin.join("marker").exists(),
            "new project inherited consent"
        );
        let refreshed = global_plugin_registry().unwrap();
        assert!(refreshed.matches_project(&second));
        assert!(!refreshed.is_enabled("fixture"));
        assert!(refreshed.is_enabled("user-fixture"));

        refresh_cached_plugins(&first).await;
        run_global_pre_tool_hook("Read", &serde_json::json!({}), &first);
        assert!(first_plugin.join("marker").exists());
        std::fs::remove_file(first_plugin.join("marker")).unwrap();
        refresh_cached_plugins(&root.path().join("missing-project")).await;
        run_global_pre_tool_hook(
            "Read",
            &serde_json::json!({}),
            &root.path().join("missing-project"),
        );
        assert!(!first_plugin.join("marker").exists());
    });
}
