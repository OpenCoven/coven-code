// providers/http_util.rs — shared HTTP-client construction helpers for
// provider adapters.
//
// Every provider needs a `reqwest::Client` with a long timeout (LLM calls
// routinely run >60s). Before this module each constructor inlined
// `Client::builder().timeout(...).build().expect("...")` which panicked
// at startup on TLS-init failures. AGENTS.md forbids `.expect()` on
// fallible production paths, so we centralize the build here and return
// a structured `ProviderError` instead.

use std::time::Duration;

use claurst_core::provider_id::ProviderId;

use crate::provider_error::ProviderError;

/// Default timeout applied to every provider HTTP client. LLM streaming
/// responses can take well over a minute; ten minutes is the standard
/// upper bound across the adapters in this crate.
pub const DEFAULT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(600);

/// Build a default `reqwest::Client` for a provider adapter, propagating
/// any builder failure as a structured [`ProviderError::Other`] so the
/// registry can skip the provider instead of crashing the process.
pub fn build_default_http_client(provider: &ProviderId) -> Result<reqwest::Client, ProviderError> {
    reqwest::Client::builder()
        .timeout(DEFAULT_PROVIDER_TIMEOUT)
        .build()
        .map_err(|e| ProviderError::Other {
            provider: provider.clone(),
            message: format!("failed to build HTTP client: {e}"),
            status: None,
            body: None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_default_http_client_succeeds_for_well_known_provider() {
        // Under normal conditions the reqwest builder succeeds; the test
        // primarily guards against a regression in the timeout arg or the
        // ProviderError mapping.
        let pid = ProviderId::new("test");
        let client = build_default_http_client(&pid).expect("client must build");
        // Smoke-check: the client is usable.
        let _ = client.get("http://127.0.0.1:1").build();
    }
}
