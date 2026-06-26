// registry.rs — Registry of the available LLM providers.
//
// Coven Code supports exactly two providers: **Anthropic** (Claude) and
// **Codex** (OpenAI Codex via OAuth). The registry holds an
// `Arc<dyn LlmProvider>` for each and exposes lookup, health-check, and
// default-provider helpers.

use std::collections::HashMap;
use std::sync::Arc;

use claurst_core::ProviderId;

use crate::client::ClientConfig;
use crate::provider::LlmProvider;
use crate::provider_types::ProviderStatus;
use crate::providers::{AnthropicProvider, CodexProvider};

/// Resolve the configured API base override for a provider, if any.
pub fn resolve_provider_api_base(
    config: &claurst_core::config::Config,
    provider_id: &str,
) -> Option<String> {
    config.resolve_provider_api_base(provider_id)
}

/// Registry of all available LLM providers.
/// Holds `Arc<dyn LlmProvider>` for each registered provider.
pub struct ProviderRegistry {
    providers: HashMap<ProviderId, Arc<dyn LlmProvider>>,
    default_provider_id: ProviderId,
}

/// Build a provider from a stored/explicit API key. Only Anthropic and Codex
/// are supported; everything else returns `None`.
fn provider_from_key(provider_id: &str, key: String) -> Option<Arc<dyn LlmProvider>> {
    match provider_id {
        "anthropic" => match AnthropicProvider::from_config(ClientConfig {
            api_key: key,
            ..Default::default()
        }) {
            Ok(p) => Some(Arc::new(p) as Arc<dyn LlmProvider>),
            Err(e) => {
                tracing::warn!(provider = "anthropic", error = %e, "could not build provider");
                None
            }
        },
        // The Codex provider is OAuth-based; the `key` field is not used.
        // Load from the stored token file instead.
        "codex" | "openai-codex" => {
            CodexProvider::from_stored().map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
        }
        _ => None,
    }
}

pub fn provider_from_config(
    config: &claurst_core::config::Config,
    provider_id: &str,
) -> Option<Arc<dyn LlmProvider>> {
    let provider_cfg = config.provider_configs.get(provider_id);
    if provider_cfg.is_some_and(|provider| !provider.enabled) {
        return None;
    }

    match provider_id {
        // Anthropic is registered via `with_anthropic`, not here.
        "anthropic" => None,
        "codex" | "openai-codex" => {
            CodexProvider::from_stored().map(|provider| Arc::new(provider) as Arc<dyn LlmProvider>)
        }
        _ => None,
    }
}

pub fn runtime_provider_for(provider_id: &str) -> Option<Arc<dyn LlmProvider>> {
    match provider_id {
        "codex" | "openai-codex" => {
            return CodexProvider::from_stored().map(|p| Arc::new(p) as Arc<dyn LlmProvider>);
        }
        _ => {}
    }

    let auth_store = claurst_core::AuthStore::load();
    let key = auth_store.api_key_for(provider_id)?;
    if key.is_empty() {
        return None;
    }
    provider_from_key(provider_id, key)
}

impl ProviderRegistry {
    /// Create an empty registry with Anthropic as the default provider ID.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            default_provider_id: ProviderId::new(ProviderId::ANTHROPIC),
        }
    }

    /// Register a provider. Returns `&mut self` for builder chaining.
    pub fn register(&mut self, provider: Arc<dyn LlmProvider>) -> &mut Self {
        let id = provider.id().clone();
        self.providers.insert(id, provider);
        self
    }

    /// Set the default provider by ID.
    ///
    /// # Panics
    /// Panics if no provider with that ID has been registered.
    pub fn set_default(&mut self, id: ProviderId) -> &mut Self {
        assert!(
            self.providers.contains_key(&id),
            "set_default: provider '{}' is not registered",
            id,
        );
        self.default_provider_id = id;
        self
    }

    /// Get a provider by ID.
    pub fn get(&self, id: &ProviderId) -> Option<&Arc<dyn LlmProvider>> {
        self.providers.get(id)
    }

    /// Get the default provider.
    pub fn default_provider(&self) -> Option<&Arc<dyn LlmProvider>> {
        self.providers.get(&self.default_provider_id)
    }

    /// Get the default provider ID.
    pub fn default_provider_id(&self) -> &ProviderId {
        &self.default_provider_id
    }

    /// List all registered provider IDs.
    pub fn provider_ids(&self) -> Vec<&ProviderId> {
        self.providers.keys().collect()
    }

    /// Check health of all providers sequentially.
    /// Returns `(provider_id, status)` pairs.
    pub async fn check_all_health(&self) -> Vec<(ProviderId, ProviderStatus)> {
        let mut results = Vec::new();
        for (id, provider) in &self.providers {
            let status = provider
                .health_check()
                .await
                .unwrap_or(ProviderStatus::Unavailable {
                    reason: "health check failed".to_string(),
                });
            results.push((id.clone(), status));
        }
        results
    }

    /// Convenience: build a registry with just Anthropic registered as the
    /// default provider.  Takes the same [`ClientConfig`] that
    /// [`AnthropicClient`] takes.
    ///
    /// If the Anthropic HTTP client cannot be built (rare TLS init failure),
    /// the registry is returned empty and a warning is logged. Callers
    /// downstream already handle "no providers registered".
    ///
    /// [`AnthropicClient`]: crate::client::AnthropicClient
    pub fn with_anthropic(config: ClientConfig) -> Self {
        let mut registry = Self::new();
        match AnthropicProvider::from_config(config) {
            Ok(p) => {
                registry.register(Arc::new(p));
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not register Anthropic provider in registry");
            }
        }
        registry
    }

    pub fn from_config(
        config: &claurst_core::config::Config,
        anthropic_config: ClientConfig,
    ) -> Self {
        let mut registry = Self::from_environment_with_auth_store(anthropic_config);
        let active_provider = config.selected_provider_id();

        let mut configured_provider_ids: Vec<String> =
            config.provider_configs.keys().cloned().collect();
        if configured_provider_ids
            .iter()
            .all(|id| id != active_provider)
        {
            configured_provider_ids.push(active_provider.to_string());
        }

        for provider_id in configured_provider_ids {
            if let Some(provider) = provider_from_config(config, &provider_id) {
                registry.register(provider);
            }
        }

        let default_provider_id = ProviderId::new(active_provider);
        if registry.get(&default_provider_id).is_some() {
            registry.set_default(default_provider_id);
        }

        registry
    }

    /// Register [`CodexProvider`] if stored Codex OAuth tokens are available in
    /// `~/.coven-code/codex_tokens.json`.  Returns `&mut self` for builder chaining.
    pub fn with_codex_if_configured(&mut self) -> &mut Self {
        if let Some(p) = CodexProvider::from_stored() {
            self.register(Arc::new(p));
        }
        self
    }

    /// Build a registry with Anthropic plus Codex (when its OAuth tokens are
    /// present).  Anthropic is always the default provider.
    ///
    /// This is the recommended constructor for production use.
    pub fn from_environment(anthropic_config: ClientConfig) -> Self {
        let mut registry = Self::with_anthropic(anthropic_config);
        registry.with_codex_if_configured();
        registry
    }

    /// Build a registry that checks **both** environment variables and the
    /// persistent [`AuthStore`] (`~/.coven-code/auth.json`) for credentials.
    ///
    /// This ensures that credentials stored via `/connect` or `coven-code auth`
    /// are picked up at startup, not just env vars.
    ///
    /// [`AuthStore`]: claurst_core::AuthStore
    pub fn from_environment_with_auth_store(anthropic_config: ClientConfig) -> Self {
        let mut registry = Self::from_environment(anthropic_config);

        let auth_store = claurst_core::AuthStore::load();

        for provider_id in auth_store.credentials.keys() {
            let pid = claurst_core::ProviderId::new(provider_id.as_str());
            if registry.get(&pid).is_some() {
                continue;
            }
            if let Some(key) = auth_store.api_key_for(provider_id) {
                if key.is_empty() {
                    continue;
                }
                if let Some(p) = provider_from_key(provider_id, key) {
                    registry.register(p);
                }
            }
        }

        registry
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
