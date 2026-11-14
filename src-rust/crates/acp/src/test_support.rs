//! Shared ACP fixture; construction does not start authentication or provider turns.
use crate::{AgentRuntime, AgentServer, Connection};
use claurst_core::{Config, Settings};
use std::sync::{Arc, Mutex as StdMutex};

pub(crate) fn server(connection: Arc<Connection>) -> Arc<AgentServer> {
    let settings = Settings::default();
    let config = Config::default();
    let permission_manager = Arc::new(StdMutex::new(
        claurst_core::permissions::PermissionManager::new(
            config.permission_mode.clone(),
            &settings,
        ),
    ));
    let client = claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
        api_key: "unused-fixture-key".into(),
        api_base: "http://127.0.0.1:9".into(),
        ..Default::default()
    })
    .unwrap();
    AgentServer::new(
        connection,
        Arc::new(AgentRuntime {
            config,
            settings,
            api_client: Arc::new(client),
            provider_registry: Arc::new(claurst_api::ProviderRegistry::new()),
            tools: Arc::new(Vec::new()),
            cost_tracker: claurst_core::CostTracker::new(),
            query_config: claurst_query::QueryConfig::default(),
            mcp_manager: None,
            permission_manager,
            working_dir: std::path::PathBuf::from("/unused-fixture"),
        }),
    )
}
