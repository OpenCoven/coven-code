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

/// An error followed by each cause under it, joined with ": ".
///
/// `reqwest::Error`'s `Display` stops at "error sending request for url
/// (...)" and keeps the reason in `source()`. Without the chain, a failed
/// send never says whether DNS, a refused or reset connection, TLS or the
/// timeout was at fault. A cause whose text is already present is skipped,
/// since some errors repeat their source in their own `Display`.
pub fn error_with_causes(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !cause_text.is_empty() && !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Layer(&'static str, Option<Box<Layer>>);

    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref().map(|layer| layer as _)
        }
    }

    #[test]
    fn error_with_causes_names_each_cause_once() {
        let chain = Layer(
            "error sending request for url (https://example.invalid/)",
            Some(Box::new(Layer(
                "client error (Connect)",
                Some(Box::new(Layer(
                    "client error (Connect)",
                    Some(Box::new(Layer("Connection refused (os error 61)", None))),
                ))),
            ))),
        );
        assert_eq!(
            error_with_causes(&chain),
            "error sending request for url (https://example.invalid/): client error (Connect): Connection refused (os error 61)"
        );
        assert_eq!(error_with_causes(&Layer("alone", None)), "alone");
    }

    #[tokio::test]
    async fn a_failed_send_says_why() {
        // Bind and release a loopback port, so nothing listens on it and the
        // connection is refused without leaving the machine.
        let addr = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("loopback port");
        let client = build_default_http_client(&ProviderId::new("test")).expect("client");
        let error = client
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect_err("nothing listens on the port");
        let top = error.to_string();
        let text = error_with_causes(&error);
        assert!(text.starts_with(&top), "{text}");
        assert!(text.len() > top.len(), "the cause is missing: {text}");
    }

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
