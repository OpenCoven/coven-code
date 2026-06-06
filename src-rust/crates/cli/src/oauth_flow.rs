// OAuth 2.0 PKCE login flow for the Coven Code CLI.
//
// Anthropic OAuth login is disabled until Coven Code has an OAuth client
// identity issued for this application. Reusing another application's client
// ID would misidentify this client during consent and token exchange.

use anyhow::bail;
use claurst_core::oauth::OAuthTokens;

// ---- Public entry point -----------------------------------------------------

/// Outcome of a completed login flow.
#[derive(Debug, Clone)]
pub struct LoginResult {
    /// The credential to use: either an API key or Bearer token.
    #[allow(dead_code)]
    pub credential: String,
    /// When true, present as `Authorization: Bearer <credential>`.
    pub use_bearer_auth: bool,
    /// Cached tokens saved to disk.
    pub tokens: OAuthTokens,
}

/// Run the interactive Anthropic OAuth PKCE login flow.
pub async fn run_oauth_login_flow(login_with_claude_ai: bool) -> anyhow::Result<LoginResult> {
    run_oauth_login_flow_with_label(login_with_claude_ai, None).await
}

/// Same as [`run_oauth_login_flow`] but lets the caller supply a human-friendly
/// label for the new profile.
pub async fn run_oauth_login_flow_with_label(
    _login_with_claude_ai: bool,
    _label: Option<&str>,
) -> anyhow::Result<LoginResult> {
    bail!(
        "Anthropic OAuth login is disabled because Coven Code does not have an application-specific OAuth client. Set ANTHROPIC_API_KEY or store an Anthropic API key instead."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn anthropic_oauth_login_is_disabled() {
        let err = run_oauth_login_flow(true)
            .await
            .expect_err("Anthropic OAuth login should be disabled");
        assert!(err.to_string().contains("application-specific OAuth client"));
    }
}
