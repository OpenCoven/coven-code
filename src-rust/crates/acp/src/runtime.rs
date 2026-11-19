//! Shared agent runtime owned by the ACP server.
//!
//! Built once on startup and reused for every session. Per-session state
//! (cwd, transcript, cancellation token, permission ID counter) is layered on
//! top via `sessions::SessionState`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use claurst_core::config::{Config, Settings};
use claurst_core::permissions::PermissionManager;
use claurst_core::{CostTracker, PermissionMode};
use claurst_query::QueryConfig;
use claurst_tools::Tool;
use tracing::warn;

/// Snapshot of the global agent runtime — built at server startup, cloned
/// (cheaply, via Arc) into each session.
#[derive(Clone)]
pub struct AgentRuntime {
    pub config: Config,
    pub settings: Settings,
    pub api_client: Arc<claurst_api::AnthropicClient>,
    pub provider_registry: Arc<claurst_api::ProviderRegistry>,
    pub tools: Arc<Vec<Box<dyn Tool>>>,
    pub cost_tracker: Arc<CostTracker>,
    pub query_config: QueryConfig,
    pub mcp_manager: Option<Arc<claurst_mcp::McpManager>>,
    pub permission_manager: Arc<std::sync::Mutex<PermissionManager>>,
    pub working_dir: PathBuf,
}

impl AgentRuntime {
    /// Build the runtime from on-disk settings, env vars, and a working
    /// directory. Mirrors the headless startup path but with ACP-specific
    /// defaults (non-interactive, permission decisions routed back to the
    /// connected client).
    pub async fn build(working_dir: PathBuf) -> anyhow::Result<Self> {
        let settings = Settings::load_sync().with_context(|| {
            format!(
                "Failed to load ACP settings {}",
                Settings::global_settings_path().display()
            )
        })?;
        let mut config = settings.effective_config();
        config.permission_mode = acp_permission_mode(&config.permission_mode);
        config.project_dir = Some(working_dir.clone());

        let active_provider = config.selected_provider_id().to_string();
        let (api_key, use_bearer_auth) = if active_provider == "anthropic" {
            config
                .resolve_anthropic_auth_async()
                .await
                .unwrap_or_default()
        } else {
            (String::new(), false)
        };

        let client_config = claurst_api::client::ClientConfig {
            api_key: api_key.clone(),
            api_base: config.resolve_anthropic_api_base(),
            use_bearer_auth,
            ..Default::default()
        };
        let api_client = Arc::new(claurst_api::AnthropicClient::new(client_config.clone())?);
        let provider_registry = Arc::new(claurst_api::ProviderRegistry::from_config(
            &config,
            client_config,
        ));

        let permission_manager = Arc::new(std::sync::Mutex::new(PermissionManager::new(
            config.permission_mode.clone(),
            &settings,
        )));

        let cost_tracker = CostTracker::new();

        // MCP servers from settings — connect upfront so their tools are
        // visible to every session. Per-session MCP servers supplied via
        // `session/new` params are additive on top of this (v1: ignored,
        // tracked in plan/migration-todo).
        let mcp_manager = build_mcp_manager(&config).await;

        // Build tools: built-ins + AgentTool. MCP tool wrappers are NOT
        // attached here — the wrapper type lives in the CLI crate today and
        // adding it would create a circular dep. Built-in tools (Bash, Read,
        // Edit, Glob, Grep, WebFetch, …) cover the common ACP-editor flows.
        let mut tools: Vec<Box<dyn Tool>> = claurst_tools::all_tools();
        tools.push(Box::new(claurst_query::AgentTool));
        let tools = Arc::new(tools);

        let mut query_config = QueryConfig::from_config(&config);
        query_config.working_directory = Some(working_dir.display().to_string());
        query_config.provider_registry = Some(provider_registry.clone());

        Ok(Self {
            config,
            settings,
            api_client,
            provider_registry,
            tools,
            cost_tracker,
            query_config,
            mcp_manager,
            permission_manager,
            working_dir,
        })
    }
}

/// The mode an ACP server runs in for the mode saved in settings.
///
/// `coven-code acp` takes no flag and reads no variable that names a mode, so
/// nothing can opt in to a saved `bypassPermissions`: under the rule every
/// start shares, it runs in `default`, and the server says so on stderr, never
/// on the protocol stream. Plan mode needs the interactive plan approval ACP
/// does not have, so a saved `plan` also runs in `default`, where the
/// permission bridge routes each decision to the client.
fn acp_permission_mode(saved: &PermissionMode) -> PermissionMode {
    let restored = saved.restored_from_settings();
    if restored.mode != *saved {
        if let Some(notice) = restored.notice {
            warn!("ACP: {notice}; running in default mode, as ACP has no opt-in for bypass");
        }
    }
    match restored.mode {
        PermissionMode::Plan => PermissionMode::Default,
        mode => mode,
    }
}

async fn build_mcp_manager(config: &Config) -> Option<Arc<claurst_mcp::McpManager>> {
    if config.mcp_servers.is_empty() {
        return None;
    }
    let mgr = Arc::new(claurst_mcp::McpManager::connect_all(&config.mcp_servers).await);
    mgr.clone().spawn_notification_poll_loop(None);
    Some(mgr)
}

#[cfg(test)]
mod tests {
    use super::AgentRuntime;
    use claurst_core::PermissionMode;
    use std::path::PathBuf;
    use std::process::Command;

    const ROOT_ENV: &str = "ACP_SAVED_PERMISSION_MODE_FIXTURE_ROOT";
    const COMPLETE: &str = "acp-saved-permission-mode-fixture-complete";

    /// Builds the ACP runtime over each saved mode. Only the parent below may
    /// run this: it rewrites the settings file of the engine home the
    /// environment names.
    #[test]
    #[ignore = "run by isolated parent with temporary engine home"]
    fn saved_permission_mode_child() {
        let root = PathBuf::from(std::env::var_os(ROOT_ENV).expect("run through the parent test"));
        assert_eq!(
            std::env::var_os("COVEN_CODE_HOME").as_deref(),
            Some(root.as_os_str())
        );
        let tokio = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut failures = Vec::new();
        for (saved, expected) in [
            ("default", PermissionMode::Default),
            ("acceptEdits", PermissionMode::AcceptEdits),
            // Plan needs the interactive plan approval ACP does not have.
            ("plan", PermissionMode::Default),
            // ACP has no explicit opt-in, so a saved bypass never applies.
            ("bypassPermissions", PermissionMode::Default),
        ] {
            // A provider other than Anthropic keeps the build off credentials.
            std::fs::write(
                root.join("settings.json"),
                format!(r#"{{"config":{{"provider":"codex","permission_mode":"{saved}"}}}}"#),
            )
            .unwrap();
            let runtime = tokio
                .block_on(AgentRuntime::build(root.clone()))
                .unwrap_or_else(|error| panic!("saved {saved}: {error:#}"));
            // The mode each turn and the keyless child get, and the mode
            // tool approvals are checked in.
            let turn = runtime.config.permission_mode.clone();
            let approvals = runtime.permission_manager.lock().unwrap().mode.clone();
            if turn != expected || approvals != expected {
                failures.push(format!(
                    "saved {saved}: turn {turn:?}, approvals {approvals:?}, expected {expected:?}"
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        println!("{COMPLETE}");
    }

    #[test]
    fn acp_never_restores_a_saved_bypass_mode() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::saved_permission_mode_child",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("HOME", &root_path)
            .env("USERPROFILE", &root_path)
            .env("COVEN_HOME", root_path.join("coven"))
            .env("COVEN_CAVE_HOME", root_path.join("cave"))
            .env("COVEN_CODE_HOME", &root_path)
            .env("XDG_CONFIG_HOME", &root_path)
            .env("XDG_CACHE_HOME", &root_path)
            .env(ROOT_ENV, &root_path)
            .current_dir(&root_path)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains(COMPLETE),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
