// OAuth 2.0 PKCE login flow for the Coven Code CLI.
//
// Anthropic OAuth login requires a Coven Code OAuth client identity. Reusing
// another application's client ID would misidentify this client during consent
// and token exchange, so the flow stays disabled unless a first-party client ID
// is supplied through configuration.

use anyhow::{bail, Context};
use claurst_core::oauth::{self, OAuthTokens};
use serde::Deserialize;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

#[derive(Debug, Deserialize)]
struct TokenExchangeResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: u64,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    account: Option<serde_json::Value>,
    #[serde(default)]
    organization: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct CreateApiKeyResponse {
    raw_key: Option<String>,
}

// ---- Public entry point -----------------------------------------------------

/// Outcome of a completed login flow.
#[derive(Debug, Clone)]
pub struct LoginResult {
    /// The credential to use: either an API key or Bearer token.
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
    login_with_claude_ai: bool,
    label: Option<&str>,
) -> anyhow::Result<LoginResult> {
    let client_id = configured_anthropic_oauth_client_id()?;

    let code_verifier = oauth::generate_code_verifier();
    let code_challenge = oauth::generate_code_challenge(&code_verifier);
    let state = oauth::generate_state();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("Failed to bind OAuth callback server")?;
    let port = listener.local_addr()?.port();

    let authorize_base = if login_with_claude_ai {
        oauth::CLAUDE_AI_AUTHORIZE_URL
    } else {
        oauth::CONSOLE_AUTHORIZE_URL
    };
    let manual_url = oauth::build_auth_url_with_client_id(
        authorize_base,
        &client_id,
        &code_challenge,
        &state,
        port,
        true,
    );
    let automatic_url = oauth::build_auth_url_with_client_id(
        authorize_base,
        &client_id,
        &code_challenge,
        &state,
        port,
        false,
    );

    println!("\nOpening browser for authentication...");
    println!("If the browser did not open, visit:\n\n  {}\n", manual_url);
    try_open_browser(&automatic_url);

    let auth_code = wait_for_auth_code_impl(listener, &state)
        .await
        .context("OAuth callback failed")?;
    debug!("OAuth auth code received");

    let token_resp =
        exchange_code_for_tokens(&client_id, &auth_code, &state, &code_verifier, port, false)
            .await
            .context("Token exchange failed")?;

    let expires_at_ms =
        chrono::Utc::now().timestamp_millis() + (token_resp.expires_in as i64 * 1000);

    let scopes: Vec<String> = token_resp
        .scope
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .map(String::from)
        .collect();

    let account_uuid = token_resp
        .account
        .as_ref()
        .and_then(|a| a.get("uuid").and_then(|v| v.as_str()).map(String::from));
    let email = token_resp.account.as_ref().and_then(|a| {
        a.get("email_address")
            .and_then(|v| v.as_str())
            .map(String::from)
    });
    let organization_uuid = token_resp
        .organization
        .as_ref()
        .and_then(|o| o.get("uuid").and_then(|v| v.as_str()).map(String::from));

    let uses_bearer = scopes.iter().any(|s| s == oauth::CLAUDE_AI_INFERENCE_SCOPE);

    let api_key = if !uses_bearer {
        match create_api_key(&token_resp.access_token).await {
            Ok(key) => {
                info!("OAuth API key created successfully");
                Some(key)
            }
            Err(e) => {
                warn!("Failed to create API key from OAuth token: {}", e);
                None
            }
        }
    } else {
        None
    };

    let tokens = OAuthTokens {
        access_token: token_resp.access_token.clone(),
        refresh_token: token_resp.refresh_token.clone(),
        expires_at_ms: Some(expires_at_ms),
        scopes: scopes.clone(),
        account_uuid,
        email,
        organization_uuid,
        subscription_type: None,
        oauth_client_id: Some(client_id.clone()),
        api_key: api_key.clone(),
    };
    tokens
        .save_and_register(label)
        .await
        .context("Failed to save OAuth tokens")?;

    let (credential, use_bearer_auth) = if uses_bearer {
        (token_resp.access_token.clone(), true)
    } else if let Some(key) = api_key {
        (key, false)
    } else {
        bail!("Login succeeded but could not obtain a usable credential")
    };

    Ok(LoginResult {
        credential,
        use_bearer_auth,
        tokens,
    })
}

fn configured_anthropic_oauth_client_id() -> anyhow::Result<String> {
    std::env::var(oauth::CLIENT_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Anthropic OAuth requires a Coven OAuth client ID. Set {} after registering a first-party OAuth client, or use ANTHROPIC_API_KEY for now.",
                oauth::CLIENT_ID_ENV
            )
        })
}

fn try_open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let ps_cmd = format!("Start-Process '{}'", url.replace('\'', "''"));
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

async fn run_callback_server(
    listener: TcpListener,
    expected_state: &str,
) -> anyhow::Result<String> {
    debug!(
        "OAuth callback server listening on port {}",
        listener.local_addr()?.port()
    );

    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(120), listener.accept())
        .await
        .context("Timeout waiting for browser redirect")?
        .context("Accept failed")?;

    let (reader, mut writer) = socket.split();
    let mut reader = BufReader::new(reader);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;

    loop {
        let mut header = String::new();
        reader.read_line(&mut header).await?;
        if header.trim().is_empty() {
            break;
        }
    }

    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();

    let parsed_url = url::Url::parse(&format!("http://localhost{}", path))
        .context("Failed to parse callback URL")?;

    let code = parsed_url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.to_string());

    let received_state = parsed_url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string());

    let response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        oauth::CLAUDEAI_SUCCESS_URL
    );
    writer.write_all(response.as_bytes()).await?;

    if received_state.as_deref() != Some(expected_state) {
        bail!("OAuth state mismatch; possible CSRF attack");
    }
    code.context("No authorization code in callback")
}

async fn read_line_from_stdin() -> anyhow::Result<String> {
    print!("  Or paste authorization code here: ");
    use std::io::Write;
    std::io::stdout().flush().ok();

    let mut line = String::new();
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    reader.read_line(&mut line).await?;
    Ok(line)
}

async fn exchange_code_for_tokens(
    client_id: &str,
    code: &str,
    state: &str,
    code_verifier: &str,
    port: u16,
    use_manual_redirect: bool,
) -> anyhow::Result<TokenExchangeResponse> {
    let redirect_uri = if use_manual_redirect {
        oauth::MANUAL_REDIRECT_URL.to_string()
    } else {
        format!("http://localhost:{}/callback", port)
    };

    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": redirect_uri,
        "client_id": client_id,
        "code_verifier": code_verifier,
        "state": state,
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let resp = client
        .post(oauth::TOKEN_URL)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Token exchange HTTP request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("Token exchange failed ({}): {}", status, text);
    }

    resp.json::<TokenExchangeResponse>()
        .await
        .context("Failed to parse token exchange response")
}

async fn create_api_key(access_token: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let resp = client
        .post(oauth::API_KEY_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .context("API key creation request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("API key creation failed ({}): {}", status, text);
    }

    let data: CreateApiKeyResponse = resp
        .json()
        .await
        .context("Failed to parse API key response")?;
    data.raw_key.context("Server returned no API key")
}

#[allow(dead_code)]
pub async fn refresh_oauth_token(tokens: &OAuthTokens) -> anyhow::Result<OAuthTokens> {
    let client_id = configured_anthropic_oauth_client_id()?;
    let refresh_token = tokens
        .refresh_token
        .as_deref()
        .context("No refresh token available")?;

    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": client_id,
        "scope": oauth::ALL_SCOPES.join(" "),
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let resp = client
        .post(oauth::TOKEN_URL)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Token refresh HTTP request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("Token refresh failed ({}): {}", status, text);
    }

    let token_resp: TokenExchangeResponse = resp.json().await?;
    let expires_at_ms =
        chrono::Utc::now().timestamp_millis() + (token_resp.expires_in as i64 * 1000);

    let scopes: Vec<String> = token_resp
        .scope
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .map(String::from)
        .collect();

    let mut updated = tokens.clone();
    updated.access_token = token_resp.access_token;
    if let Some(new_rt) = token_resp.refresh_token {
        updated.refresh_token = Some(new_rt);
    }
    updated.expires_at_ms = Some(expires_at_ms);
    updated.scopes = scopes;

    updated.save().await?;
    Ok(updated)
}

async fn wait_for_auth_code_impl(
    listener: TcpListener,
    expected_state: &str,
) -> anyhow::Result<String> {
    let expected_state_clone = expected_state.to_string();
    let (cb_tx, cb_rx) = tokio::sync::oneshot::channel::<anyhow::Result<String>>();

    tokio::spawn(async move {
        let result = run_callback_server(listener, &expected_state_clone).await;
        let _ = cb_tx.send(result);
    });

    let (paste_tx, paste_rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        if let Ok(line) = read_line_from_stdin().await {
            let trimmed = line.trim().to_string();
            if !trimmed.is_empty() {
                let _ = paste_tx.send(trimmed);
            }
        }
    });

    tokio::select! {
        result = cb_rx => {
            result.unwrap_or_else(|_| Err(anyhow::anyhow!("Callback server dropped")))
        }
        code = paste_rx => {
            code.map_err(|_| anyhow::anyhow!("Stdin closed unexpectedly"))
        }
        _ = tokio::time::sleep(Duration::from_secs(120)) => {
            bail!("Authentication timed out after 120 seconds")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn anthropic_oauth_login_requires_first_party_client_id() {
        std::env::remove_var(oauth::CLIENT_ID_ENV);
        let err = run_oauth_login_flow(true)
            .await
            .expect_err("Anthropic OAuth login should require a first-party client ID");
        assert!(err.to_string().contains(oauth::CLIENT_ID_ENV));
    }
}
