//! MCP OAuth / XAA IdP login flow.
//! Mirrors src/services/mcp/xaaIdpLogin.ts and src/services/mcp/auth.ts.

use claurst_core::plugin_mcp::PluginMcpContribution;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const CALLBACK_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const CALLBACK_REQUEST_MAX_BYTES: usize = 16 * 1024;
const CALLBACK_ERROR_CODE_MAX_BYTES: usize = 64;

// ---------------------------------------------------------------------------
// Token storage
// ---------------------------------------------------------------------------

/// An OAuth access token with expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix timestamp (seconds) when the token expires.
    pub expires_at: Option<u64>,
    pub scope: Option<String>,
    pub server_name: String,
    /// Normalized MCP server URL this token was minted for.
    /// `None` means a legacy stored token is unbound and must re-authenticate.
    #[serde(default)]
    pub server_url: Option<String>,
}

impl McpToken {
    /// Returns `true` if the token is expired or will expire within `grace_secs`.
    pub fn is_expired(&self, grace_secs: u64) -> bool {
        let Some(exp) = self.expires_at else {
            return false;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_add(grace_secs) >= exp
    }

    pub fn expiry_datetime(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        token_expiry_datetime(self.expires_at)
    }

    pub fn is_bound_to_server_url(&self, server_url: &str) -> bool {
        self.server_url.as_deref().map(normalized_server_url)
            == Some(normalized_server_url(server_url))
    }
}

/// Path to the token store for a given MCP server.
///
/// Routed through `config_home()` so MCP tokens live in the same engine home
/// as all other state and migrate together under the unified coven CLI. (This
/// previously constructed `~/.coven-code/mcp-tokens` directly, which would have
/// stranded tokens in the legacy dir after relocation.)
fn token_path(server_name: &str) -> std::io::Result<PathBuf> {
    if server_name.is_empty() || server_name.contains(['/', '\\', ':', '\0']) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "MCP server name must be non-empty and contain no path separators, colon, or NUL",
        ));
    }
    Ok(claurst_core::config::config_home()
        .join("mcp-tokens")
        .join(format!("{}.json", server_name)))
}

/// Persist an MCP OAuth token to disk.
pub fn store_mcp_token(token: &McpToken) -> std::io::Result<()> {
    let path = token_path(&token.server_name)?;
    let json = serde_json::to_string_pretty(token).map_err(std::io::Error::other)?;
    claurst_core::secret_file::write_secret_file_sync_in_private_dir(&path, json)
}

/// Read a stored MCP OAuth token (None if not found or invalid).
pub fn get_mcp_token(server_name: &str) -> Option<McpToken> {
    let path = token_path(server_name).ok()?;
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Delete the stored token for a server (effectively logs out).
pub fn remove_mcp_token(server_name: &str) -> std::io::Result<()> {
    let path = token_path(server_name)?;
    if path.exists() {
        std::fs::remove_file(&path)
    } else {
        Ok(())
    }
}

pub fn token_expiry_datetime(expires_at: Option<u64>) -> Option<chrono::DateTime<chrono::Utc>> {
    let timestamp = i64::try_from(expires_at?).ok()?;
    chrono::DateTime::from_timestamp(timestamp, 0)
}

// ---------------------------------------------------------------------------
// OAuth metadata / auth flow helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOAuthMetadata {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
}

#[derive(Debug, Clone)]
pub struct McpAuthSession {
    pub(crate) plugin: Option<claurst_core::plugin_mcp::PluginMcpContribution>,
    pub(crate) caller: Option<PathBuf>,
    pub server_name: String,
    pub server_url: String,
    pub auth_url: String,
    pub redirect_uri: String,
    pub verifier: String,
    pub(crate) state: String,
    pub metadata: McpOAuthMetadata,
}

#[derive(Debug, Clone)]
pub struct McpAuthResult {
    pub server_name: String,
    pub auth_url: String,
    pub redirect_uri: String,
    pub token_path: PathBuf,
}

fn normalized_server_url(server_url: &str) -> &str {
    server_url.trim_end_matches('/')
}

fn normalize_server_url_owned(server_url: &str) -> String {
    normalized_server_url(server_url).to_string()
}

fn fallback_oauth_metadata(server_url: &str) -> McpOAuthMetadata {
    let base_url = normalized_server_url(server_url);
    McpOAuthMetadata {
        authorization_endpoint: format!("{}/oauth/authorize", base_url),
        token_endpoint: format!("{}/oauth/token", base_url),
    }
}

fn build_mcp_auth_url(
    authorization_endpoint: &str,
    redirect_uri: &str,
    verifier: &str,
    state: &str,
) -> anyhow::Result<String> {
    let challenge = pkce_challenge(verifier);
    let mut auth_url = url::Url::parse(authorization_endpoint).map_err(|error| {
        anyhow::anyhow!("Failed to parse OAuth authorization endpoint: {}", error)
    })?;
    const RESERVED_PARAMETERS: &[&str] = &[
        "response_type",
        "client_id",
        "redirect_uri",
        "scope",
        "code_challenge",
        "code_challenge_method",
        "state",
    ];
    anyhow::ensure!(
        !auth_url
            .query_pairs()
            .any(|(key, _)| RESERVED_PARAMETERS.contains(&key.as_ref())),
        "OAuth authorization endpoint contains a reserved request parameter"
    );
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", "coven-code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
    Ok(auth_url.into())
}

pub async fn fetch_oauth_metadata(server_url: &str) -> anyhow::Result<McpOAuthMetadata> {
    let base_url = normalized_server_url(server_url);
    let fallback = fallback_oauth_metadata(base_url);
    let metadata_url = format!("{}/.well-known/oauth-authorization-server", base_url);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

    match client.get(&metadata_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let meta: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("OAuth metadata parse error: {}", e))?;
            Ok(McpOAuthMetadata {
                authorization_endpoint: meta
                    .get("authorization_endpoint")
                    .and_then(|value| value.as_str())
                    .unwrap_or(fallback.authorization_endpoint.as_str())
                    .to_string(),
                token_endpoint: meta
                    .get("token_endpoint")
                    .and_then(|value| value.as_str())
                    .unwrap_or(fallback.token_endpoint.as_str())
                    .to_string(),
            })
        }
        Ok(_) | Err(_) => Ok(fallback),
    }
}

pub async fn begin_mcp_auth(server_name: &str, server_url: &str) -> anyhow::Result<McpAuthSession> {
    let metadata = fetch_oauth_metadata(server_url).await?;
    let redirect_port = oauth_port_alloc()
        .map_err(|e| anyhow::anyhow!("Failed to allocate OAuth redirect port: {}", e))?;
    let redirect_uri = format!("http://127.0.0.1:{}/callback", redirect_port);
    let verifier = pkce_verifier();
    let state = random_urlsafe_token()?;
    let auth_url = build_mcp_auth_url(
        &metadata.authorization_endpoint,
        &redirect_uri,
        &verifier,
        &state,
    )?;

    Ok(McpAuthSession {
        plugin: None,
        caller: None,
        server_name: server_name.to_string(),
        server_url: normalize_server_url_owned(server_url),
        auth_url,
        redirect_uri,
        verifier,
        state,
        metadata,
    })
}

async fn bind_callback_listener(
    redirect_uri: &str,
) -> anyhow::Result<(TcpListener, String, String)> {
    let redirect_url = url::Url::parse(redirect_uri)
        .map_err(|e| anyhow::anyhow!("Failed to parse redirect URI '{}': {}", redirect_uri, e))?;
    let host = redirect_url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Redirect URI '{}' is missing host", redirect_uri))?
        .to_string();
    let port = redirect_url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("Redirect URI '{}' is missing port", redirect_uri))?;
    let callback_path = if redirect_url.path().is_empty() {
        "/callback".to_string()
    } else {
        redirect_url.path().to_string()
    };
    let listener = TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to bind OAuth callback listener on {}:{}: {}",
                host,
                port,
                e
            )
        })?;

    let authority = format!("{}:{}", host, port);
    Ok((listener, authority, callback_path))
}

#[derive(Clone, Copy)]
struct CallbackLimits {
    overall_timeout: Duration,
    request_timeout: Duration,
    max_request_bytes: usize,
}

impl Default for CallbackLimits {
    fn default() -> Self {
        Self {
            overall_timeout: CALLBACK_TIMEOUT,
            request_timeout: CALLBACK_REQUEST_TIMEOUT,
            max_request_bytes: CALLBACK_REQUEST_MAX_BYTES,
        }
    }
}

fn has_valid_query_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            let high = (bytes[index + 1] as char).to_digit(16).unwrap_or_default() as u8;
            let low = (bytes[index + 2] as char).to_digit(16).unwrap_or_default() as u8;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    std::str::from_utf8(&decoded).is_ok()
}

/// A fully validated callback whose `state` matched this session.
enum CallbackOutcome {
    /// An authorization code to exchange once.
    Code(String),
    /// An RFC 6749 §4.1.2.1 error redirect. Holds the `error` code only when
    /// it is a short ASCII token; free text is never kept.
    AuthorizationError(Option<String>),
}

/// RFC 6749 Appendix A: `code` and `state` values are `1*VSCHAR` (%x20-7E),
/// so decoded control bytes and non-ASCII are never accepted.
fn is_oauth_vschar_value(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

fn reportable_authorization_error(error: String) -> Option<String> {
    let reportable = (1..=CALLBACK_ERROR_CODE_MAX_BYTES).contains(&error.len())
        && error
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    reportable.then_some(error)
}

fn parse_callback_request(
    request: &[u8],
    expected_authority: &str,
    callback_path: &str,
    expected_state: &str,
) -> anyhow::Result<CallbackOutcome> {
    let request = std::str::from_utf8(request)
        .map_err(|_| anyhow::anyhow!("OAuth callback request was not valid UTF-8"))?;
    let header_end = request
        .find("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("OAuth callback request headers were incomplete"))?;
    anyhow::ensure!(
        request.len() == header_end + 4,
        "OAuth callback request contained an unexpected body"
    );
    let mut lines = request[..header_end].split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("OAuth callback request line was missing"))?;
    let mut request_parts = request_line.split(' ');
    let method = request_parts.next().unwrap_or_default();
    let target = request_parts.next().unwrap_or_default();
    let version = request_parts.next().unwrap_or_default();
    anyhow::ensure!(
        request_parts.next().is_none() && !method.is_empty() && !target.is_empty(),
        "OAuth callback request line was malformed"
    );
    anyhow::ensure!(method == "GET", "OAuth callback method was not GET");
    anyhow::ensure!(version == "HTTP/1.1", "OAuth callback was not HTTP/1.1");
    // Validate the received bytes themselves: only visible ASCII may appear in
    // a request target, so nothing is stripped or re-encoded before the checks.
    anyhow::ensure!(
        target.starts_with('/')
            && target.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
            && !target.contains('#'),
        "OAuth callback request target was invalid"
    );

    let mut host = None;
    let mut content_length = None;
    let mut transfer_encoding = false;
    for line in lines {
        anyhow::ensure!(
            !line.starts_with([' ', '\t']),
            "OAuth callback header folding is not accepted"
        );
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("OAuth callback header was malformed"))?;
        anyhow::ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
            "OAuth callback header name was malformed"
        );
        // HTTP optional whitespace is only SP and HTAB, never Unicode spaces.
        let value = value.trim_matches([' ', '\t']);
        anyhow::ensure!(
            value
                .bytes()
                .all(|byte| byte == b'\t' || (0x20..=0x7e).contains(&byte)),
            "OAuth callback header value was malformed"
        );
        if name.eq_ignore_ascii_case("host") {
            anyhow::ensure!(
                host.replace(value).is_none(),
                "OAuth callback had duplicate Host headers"
            );
        } else if name.eq_ignore_ascii_case("content-length") {
            anyhow::ensure!(
                content_length.is_none(),
                "OAuth callback had ambiguous Content-Length headers"
            );
            anyhow::ensure!(
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
                "OAuth callback Content-Length was malformed"
            );
            content_length =
                Some(value.parse::<u64>().map_err(|_| {
                    anyhow::anyhow!("OAuth callback Content-Length was out of range")
                })?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            transfer_encoding = true;
        }
    }
    anyhow::ensure!(
        host == Some(expected_authority),
        "OAuth callback Host did not match the loopback listener"
    );
    anyhow::ensure!(
        !transfer_encoding && content_length.unwrap_or(0) == 0,
        "OAuth callback request bodies are not accepted"
    );

    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    anyhow::ensure!(
        raw_path == callback_path,
        "OAuth callback path did not match"
    );
    anyhow::ensure!(
        has_valid_query_encoding(raw_query),
        "OAuth callback query encoding was malformed"
    );

    // Decode the received query directly; re-parsing it as a URL would
    // normalize it first.
    let mut received_state = None;
    let mut code = None;
    let mut error = None;
    for (key, value) in url::form_urlencoded::parse(raw_query.as_bytes()) {
        let slot = match key.as_ref() {
            "state" => &mut received_state,
            "code" => &mut code,
            "error" => &mut error,
            _ => continue,
        };
        anyhow::ensure!(
            slot.replace(value.into_owned()).is_none(),
            "OAuth callback had a duplicate security parameter"
        );
    }
    anyhow::ensure!(
        received_state
            .as_deref()
            .is_some_and(|state| is_oauth_vschar_value(state) && state == expected_state),
        "OAuth callback state did not match"
    );
    match (code, error) {
        (Some(code), None) => {
            anyhow::ensure!(
                is_oauth_vschar_value(&code),
                "OAuth callback code was empty or malformed"
            );
            Ok(CallbackOutcome::Code(code))
        }
        (None, Some(error)) => Ok(CallbackOutcome::AuthorizationError(
            reportable_authorization_error(error),
        )),
        (None, None) => anyhow::bail!("OAuth callback code was missing"),
        (Some(_), Some(_)) => anyhow::bail!("OAuth callback had both a code and an error"),
    }
}

async fn read_callback_request(
    socket: &mut TcpStream,
    max_request_bytes: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut request = Vec::with_capacity(max_request_bytes.min(1024));
    let mut chunk = [0_u8; 1024];
    loop {
        let read = socket
            .read(&mut chunk)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to read OAuth callback request: {}", error))?;
        anyhow::ensure!(
            read > 0,
            "OAuth callback connection closed before request completed"
        );
        anyhow::ensure!(
            request.len().saturating_add(read) <= max_request_bytes,
            "OAuth callback request exceeded the size limit"
        );
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(request);
        }
    }
}

const CALLBACK_SUCCESS_RESPONSE: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nMCP OAuth authentication finished. You can close this tab.\r\n";
const CALLBACK_REJECTED_RESPONSE: &str = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nInvalid OAuth callback request.\r\n";
const CALLBACK_AUTHORIZATION_ERROR_RESPONSE: &str = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nMCP OAuth authorization failed. You can close this tab.\r\n";

/// Writes a fixed response; a peer that never reads cannot outlive `deadline`.
async fn write_callback_response(
    socket: &mut TcpStream,
    response: &'static str,
    deadline: Instant,
) -> anyhow::Result<()> {
    tokio::time::timeout_at(deadline, socket.write_all(response.as_bytes()))
        .await
        .map_err(|_| anyhow::anyhow!("Timeout writing OAuth callback response"))?
        .map_err(|error| anyhow::anyhow!("Failed to write OAuth callback response: {}", error))
}

async fn wait_for_authorization_code_with_limits(
    listener: TcpListener,
    expected_authority: &str,
    callback_path: &str,
    expected_state: &str,
    limits: CallbackLimits,
) -> anyhow::Result<String> {
    // One session deadline bounds accept, read and response write together.
    let deadline = Instant::now() + limits.overall_timeout;
    loop {
        // `timeout_at` polls `accept` before its timer, so a steady stream of
        // queued connections must not keep this loop alive past the deadline.
        anyhow::ensure!(
            Instant::now() < deadline,
            "Timeout waiting for OAuth callback"
        );
        let (mut socket, _) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| anyhow::anyhow!("Timeout waiting for OAuth callback"))?
            .map_err(|error| {
                anyhow::anyhow!("Failed to accept OAuth callback connection: {}", error)
            })?;
        let request_deadline = std::cmp::min(deadline, Instant::now() + limits.request_timeout);
        let request = tokio::time::timeout_at(
            request_deadline,
            read_callback_request(&mut socket, limits.max_request_bytes),
        )
        .await;
        let outcome = match request {
            Ok(Ok(request)) => {
                parse_callback_request(&request, expected_authority, callback_path, expected_state)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err(anyhow::anyhow!("OAuth callback request timed out")),
        };
        match outcome {
            Ok(CallbackOutcome::Code(code)) => {
                write_callback_response(&mut socket, CALLBACK_SUCCESS_RESPONSE, deadline).await?;
                return Ok(code);
            }
            // The state matched, so the authorization server ended this flow:
            // stop now rather than waiting out the session deadline.
            Ok(CallbackOutcome::AuthorizationError(error)) => {
                let _ = write_callback_response(
                    &mut socket,
                    CALLBACK_AUTHORIZATION_ERROR_RESPONSE,
                    deadline,
                )
                .await;
                return Err(match error {
                    Some(error) => {
                        anyhow::anyhow!("OAuth authorization server returned error '{}'", error)
                    }
                    None => anyhow::anyhow!("OAuth authorization server returned an error"),
                });
            }
            Err(_) => {
                let _ = write_callback_response(&mut socket, CALLBACK_REJECTED_RESPONSE, deadline)
                    .await;
            }
        }
    }
}

async fn wait_for_authorization_code(
    listener: TcpListener,
    expected_authority: &str,
    callback_path: &str,
    expected_state: &str,
) -> anyhow::Result<String> {
    wait_for_authorization_code_with_limits(
        listener,
        expected_authority,
        callback_path,
        expected_state,
        CallbackLimits::default(),
    )
    .await
}

pub async fn run_mcp_auth_session(session: McpAuthSession) -> anyhow::Result<McpAuthResult> {
    run_mcp_auth_session_with(session, |url| {
        open::that(url)
            .map_err(|error| anyhow::anyhow!("Failed to open browser for OAuth: {}", error))
    })
    .await
}

async fn run_mcp_auth_session_with(
    session: McpAuthSession,
    open_browser: impl FnOnce(&str) -> anyhow::Result<()>,
) -> anyhow::Result<McpAuthResult> {
    let plugin = session.plugin.clone();
    let caller = session.caller.clone();
    match plugin {
        Some(plugin) => {
            plugin
                .guard(
                    caller.as_deref(),
                    run_mcp_auth_session_inner(session, open_browser),
                )
                .await
        }
        None => run_mcp_auth_session_inner(session, open_browser).await,
    }
}

async fn run_mcp_auth_session_inner(
    session: McpAuthSession,
    open_browser: impl FnOnce(&str) -> anyhow::Result<()>,
) -> anyhow::Result<McpAuthResult> {
    if let Some(plugin) = &session.plugin {
        plugin.check_caller(session.caller.as_deref())?;
    }
    let (listener, authority, callback_path) =
        bind_callback_listener(&session.redirect_uri).await?;
    match &session.plugin {
        Some(plugin) => plugin.authorize(session.caller.as_deref(), || {
            open_browser(&session.auth_url)
        })?,
        None => open_browser(&session.auth_url)?,
    }

    let code =
        wait_for_authorization_code(listener, &authority, &callback_path, &session.state).await?;
    if let Some(plugin) = &session.plugin {
        plugin.check_caller(session.caller.as_deref())?;
    }
    let mut token = exchange_code(
        &session.metadata.token_endpoint,
        &code,
        &session.verifier,
        &session.redirect_uri,
    )
    .await?;
    token.server_name = session.server_name.clone();
    token.server_url = Some(session.server_url.clone());
    let persist = || store_mcp_token(&token).map_err(anyhow::Error::from);
    match &session.plugin {
        Some(plugin) => plugin.authorize(session.caller.as_deref(), persist)?,
        None => persist()?,
    }

    Ok(McpAuthResult {
        server_name: session.server_name,
        auth_url: session.auth_url,
        redirect_uri: session.redirect_uri,
        token_path: token_path(&token.server_name)?,
    })
}

pub async fn run_mcp_auth_flow(
    server_name: &str,
    server_url: &str,
) -> anyhow::Result<McpAuthResult> {
    let session = begin_mcp_auth(server_name, server_url).await?;
    run_mcp_auth_session(session).await
}

pub async fn get_valid_mcp_token(
    server_name: &str,
    server_url: &str,
) -> anyhow::Result<Option<McpToken>> {
    get_valid_mcp_token_for(server_name, server_url, None, None).await
}

pub(crate) async fn get_valid_mcp_token_for(
    server_name: &str,
    server_url: &str,
    plugin: Option<&PluginMcpContribution>,
    caller: Option<&Path>,
) -> anyhow::Result<Option<McpToken>> {
    if let Some(plugin) = plugin {
        plugin.check_caller(caller)?;
    }
    let Some(token) = get_mcp_token(server_name) else {
        return Ok(None);
    };

    if !token.is_bound_to_server_url(server_url) {
        return Ok(None);
    }

    if !token.is_expired(60) {
        return Ok(Some(token));
    }

    if token.refresh_token.is_none() {
        return Ok(None);
    }

    let metadata = fetch_oauth_metadata(server_url).await?;
    refresh_mcp_token_for(
        server_name,
        server_url,
        &metadata.token_endpoint,
        plugin,
        caller,
    )
    .await
    .map(Some)
}

pub async fn get_valid_mcp_access_token(
    server_name: &str,
    server_url: &str,
) -> anyhow::Result<Option<String>> {
    Ok(get_valid_mcp_token(server_name, server_url)
        .await?
        .map(|token| token.access_token))
}

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

/// Generate a PKCE code verifier (43 URL-safe random chars per RFC 7636).
pub fn pkce_verifier() -> String {
    use base64::Engine as _;
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("getrandom failed");
    // base64url-encode → 43 chars (256 bits of entropy, no padding)
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_urlsafe_token() -> anyhow::Result<String> {
    use base64::Engine as _;
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| anyhow::anyhow!("Failed to generate OAuth state: {}", error))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// Derive a PKCE code challenge from a verifier (S256 method).
pub use claurst_core::oauth_config::pkce::code_challenge as pkce_challenge;

// ---------------------------------------------------------------------------
// OAuth port allocation
// ---------------------------------------------------------------------------

/// Bind to an ephemeral localhost port for the OAuth redirect.
/// Returns the allocated port number.
pub fn oauth_port_alloc() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

// ---------------------------------------------------------------------------
// Browser-based login flow
// ---------------------------------------------------------------------------

/// OAuth login state.
#[derive(Debug, Clone)]
pub struct XaaLoginState {
    pub server_name: String,
    pub idp_url: String,
    pub verifier: String,
    pub redirect_port: u16,
}

/// Initiate an XAA (cross-agent authorization) login flow.
///
/// Opens the browser to the IdP authorization URL with PKCE parameters.
/// Returns the login state needed to complete the exchange.
pub fn initiate_xaa_login(server_name: &str, idp_url: &str) -> std::io::Result<XaaLoginState> {
    let port = oauth_port_alloc()?;
    let verifier = pkce_verifier();
    let challenge = pkce_challenge(&verifier);
    let redirect_uri = format!("http://127.0.0.1:{}/callback", port);

    let auth_url = format!(
        "{}?response_type=code&code_challenge={}&code_challenge_method=S256&redirect_uri={}",
        idp_url, challenge, redirect_uri
    );

    // Open the browser (best-effort; ignore errors on headless systems).
    let _ = open_browser(&auth_url);

    Ok(XaaLoginState {
        server_name: server_name.to_string(),
        idp_url: idp_url.to_string(),
        verifier,
        redirect_port: port,
    })
}

/// Open a URL in the system browser.
fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .status()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).status()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).status()?;
    }
    Ok(())
}

/// Convert a relative token lifetime without turning overflow into no expiry.
fn token_expiry_from_lifetime(expires_in: Option<u64>) -> anyhow::Result<Option<u64>> {
    let Some(seconds) = expires_in else {
        return Ok(None);
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.checked_add(seconds)
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("OAuth token expires_in overflows the Unix timestamp range"))
}

/// Exchange an authorization code for an access token.
pub async fn exchange_code(
    token_endpoint: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> anyhow::Result<McpToken> {
    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect_uri),
    ];

    let resp = client
        .post(token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("exchange_code: request failed: {}", e.without_url()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // Token responses can reflect credentials; never retain their error bodies.
        anyhow::bail!("exchange_code: HTTP {}", status);
    }

    #[derive(serde::Deserialize)]
    struct TokenResponse {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
        scope: Option<String>,
    }

    let tr: TokenResponse = resp
        .json()
        .await
        .map_err(|_| anyhow::anyhow!("exchange_code: invalid token response"))?;

    let expires_at = token_expiry_from_lifetime(tr.expires_in)?;

    Ok(McpToken {
        access_token: tr.access_token,
        refresh_token: tr.refresh_token,
        expires_at,
        scope: tr.scope,
        server_name: String::new(), // caller should set this
        server_url: None,           // caller should set this
    })
}

/// Refresh an existing MCP token using the stored refresh token.
pub async fn refresh_mcp_token(
    server_name: &str,
    server_url: &str,
    token_endpoint: &str,
) -> anyhow::Result<McpToken> {
    refresh_mcp_token_for(server_name, server_url, token_endpoint, None, None).await
}

async fn refresh_mcp_token_for(
    server_name: &str,
    server_url: &str,
    token_endpoint: &str,
    plugin: Option<&PluginMcpContribution>,
    caller: Option<&Path>,
) -> anyhow::Result<McpToken> {
    if let Some(plugin) = plugin {
        plugin.check_caller(caller)?;
    }
    let existing = get_mcp_token(server_name)
        .ok_or_else(|| anyhow::anyhow!("No stored token for {}", server_name))?;
    if !existing.is_bound_to_server_url(server_url) {
        let stored_binding = existing
            .server_url
            .as_deref()
            .map(normalized_server_url)
            .unwrap_or("<unbound legacy token>");
        anyhow::bail!(
            "Stored token for '{}' is not bound to requested MCP server URL (expected: {}, stored: {})",
            server_name,
            normalized_server_url(server_url),
            stored_binding
        );
    }

    let refresh = existing
        .refresh_token
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Token for {} has no refresh token", server_name))?
        .to_string();

    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh.as_str()),
    ];

    let resp = client
        .post(token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("refresh: request failed: {}", e.without_url()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // Token responses can reflect credentials; never retain their error bodies.
        anyhow::bail!("refresh: HTTP {}", status);
    }

    #[derive(serde::Deserialize)]
    struct TokenResponse {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    }

    let tr: TokenResponse = resp
        .json()
        .await
        .map_err(|_| anyhow::anyhow!("refresh: invalid token response"))?;
    let expires_at = token_expiry_from_lifetime(tr.expires_in)?;

    let new_token = McpToken {
        access_token: tr.access_token,
        refresh_token: tr.refresh_token.or(existing.refresh_token),
        expires_at,
        scope: existing.scope,
        server_name: server_name.to_string(),
        server_url: Some(normalize_server_url_owned(server_url)),
    };

    let persist = || store_mcp_token(&new_token).map_err(anyhow::Error::from);
    match plugin {
        Some(plugin) => plugin.authorize(caller, persist)?,
        None => persist()?,
    }
    Ok(new_token)
}

/// Test-only helpers shared across this crate's test modules.
#[cfg(test)]
pub(crate) mod test_support {
    /// Serializes tests that write token files (which land under
    /// `config_home()`), pointing `config_home()` at a temp dir so they never
    /// touch the developer's real `~/.coven-code/mcp-tokens/`.
    ///
    /// Uses `COVEN_CODE_HOME` (honored in all builds), not the
    /// `#[cfg(test)]`-gated `COVEN_CODE_TEST_HOME`, which is compiled out when
    /// `claurst_core` is a plain dependency of this crate's test binary.
    static TOKEN_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII guard that redirects `config_home()` at a fresh temp dir for the
    /// duration of a test and restores the previous `COVEN_CODE_HOME` on drop.
    pub(crate) struct TokenHomeGuard {
        tmp: tempfile::TempDir,
        prev: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TokenHomeGuard {
        pub(crate) fn new() -> Self {
            let lock = TOKEN_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let tmp = tempfile::tempdir().expect("tempdir");
            let prev = std::env::var("COVEN_CODE_HOME").ok();
            std::env::set_var("COVEN_CODE_HOME", tmp.path());
            Self {
                tmp,
                prev,
                _lock: lock,
            }
        }

        pub(crate) fn path(&self) -> &std::path::Path {
            self.tmp.path()
        }
    }

    impl Drop for TokenHomeGuard {
        fn drop(&mut self) {
            match self.prev.take() {
                Some(v) => std::env::set_var("COVEN_CODE_HOME", v),
                None => std::env::remove_var("COVEN_CODE_HOME"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::TokenHomeGuard;
    use super::*;

    fn callback_request(authority: &str, target: &str) -> Vec<u8> {
        format!("GET {target} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n")
            .into_bytes()
    }

    async fn send_callback(address: std::net::SocketAddr, request: &[u8]) -> String {
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(request).await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        response
    }

    #[test]
    fn pkce_challenge_known_vectors() {
        // RFC 7636 Appendix B, through the existing public name.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn relative_token_expiry_bounds() {
        assert_eq!(token_expiry_from_lifetime(None).unwrap(), None);
        assert!(token_expiry_from_lifetime(Some(u64::MAX)).is_err());
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let expiry = token_expiry_from_lifetime(Some(0)).unwrap().unwrap();
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!((before..=after).contains(&expiry));
    }

    #[test]
    fn pkce_challenge_length() {
        let v = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
        let c = pkce_challenge(v);
        assert!(!c.is_empty());
        // Base64url of SHA256 is 43 chars (without padding).
        assert_eq!(c.len(), 43);
    }

    #[test]
    fn oauth_states_are_independent_urlsafe_tokens() {
        let first = random_urlsafe_token().unwrap();
        let second = random_urlsafe_token().unwrap();
        assert_eq!(first.len(), 43);
        assert_eq!(second.len(), 43);
        assert_ne!(first, second);
        assert!(first
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')));
        assert!(second
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')));
    }

    #[test]
    fn token_expiry() {
        let t = McpToken {
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: Some(1), // expired long ago
            scope: None,
            server_name: "test".to_string(),
            server_url: Some("https://example.com/mcp".to_string()),
        };
        assert!(t.is_expired(0));
    }

    #[test]
    fn token_expiry_datetime_bounds() {
        assert_eq!(token_expiry_datetime(None), None);
        let maximum = u64::try_from(chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp()).unwrap();
        for timestamp in [0, 1_700_000_000, maximum] {
            assert_eq!(
                token_expiry_datetime(Some(timestamp)).unwrap().timestamp(),
                i64::try_from(timestamp).unwrap()
            );
        }
        for timestamp in [maximum + 1, i64::MAX as u64, u64::MAX] {
            assert_eq!(token_expiry_datetime(Some(timestamp)), None);
        }
    }

    #[test]
    fn token_expiry_grace_overflow() {
        let mut token = McpToken {
            access_token: "test".into(),
            refresh_token: None,
            expires_at: Some(u64::MAX),
            scope: None,
            server_name: "test".into(),
            server_url: None,
        };
        assert!(!token.is_expired(0));
        assert!(token.is_expired(u64::MAX));
        token.expires_at = None;
        assert!(!token.is_expired(u64::MAX));
    }

    #[test]
    fn token_server_url_binding_matches_normalized_url() {
        let t = McpToken {
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scope: None,
            server_name: "test".to_string(),
            server_url: Some("https://example.com/mcp".to_string()),
        };
        assert!(t.is_bound_to_server_url("https://example.com/mcp/"));
        assert!(!t.is_bound_to_server_url("https://attacker.example/mcp"));
    }

    #[test]
    fn token_without_server_url_is_not_bound() {
        let t = McpToken {
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
            scope: None,
            server_name: "test".to_string(),
            server_url: None,
        };
        assert!(!t.is_bound_to_server_url("https://example.com/mcp"));
    }

    #[cfg(unix)]
    #[test]
    fn token_writer_creates_private_file_and_token_directory_only() {
        use std::os::unix::fs::PermissionsExt;

        let home = TokenHomeGuard::new();
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let token = McpToken {
            access_token: "fixture-access".into(),
            refresh_token: Some("fixture-refresh".into()),
            expires_at: Some(123),
            scope: Some("fixture-scope".into()),
            server_name: "fixture".into(),
            server_url: Some("https://example.com/mcp".into()),
        };

        store_mcp_token(&token).unwrap();

        let path = home.path().join("mcp-tokens/fixture.json");
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let home_mode = std::fs::metadata(home.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
        assert_eq!(dir_mode, 0o700);
        assert_eq!(home_mode, 0o755);
        assert_eq!(
            get_mcp_token("fixture").unwrap().access_token,
            "fixture-access"
        );
    }

    #[cfg(unix)]
    #[test]
    fn traversal_names_cannot_read_write_remove_or_chmod_outside_token_directory() {
        use std::os::unix::fs::PermissionsExt;

        let home = TokenHomeGuard::new();
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let sentinel = home.path().join("outside.json");
        let sentinel_body = br#"{"access_token":"outside-sentinel"}"#;
        std::fs::write(&sentinel, sentinel_body).unwrap();
        std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o644)).unwrap();

        for traversal in ["../outside", "..\\outside", "/outside", "C:\\outside"] {
            let token = McpToken {
                access_token: "replacement".into(),
                refresh_token: None,
                expires_at: None,
                scope: None,
                server_name: traversal.into(),
                server_url: Some("https://example.com/mcp".into()),
            };

            assert!(store_mcp_token(&token).is_err());
            assert!(get_mcp_token(traversal).is_none());
            assert!(remove_mcp_token(traversal).is_err());
        }
        assert_eq!(std::fs::read(&sentinel).unwrap(), sentinel_body);
        assert_eq!(
            std::fs::metadata(&sentinel).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(
            std::fs::metadata(home.path()).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[tokio::test]
    async fn refresh_reports_unbound_legacy_token_binding() {
        let _home = TokenHomeGuard::new();
        let server_name = format!("legacy-unbound-{}", std::process::id());
        let _ = remove_mcp_token(&server_name);
        let token = McpToken {
            access_token: "tok".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: None,
            scope: None,
            server_name: server_name.clone(),
            server_url: None,
        };
        store_mcp_token(&token).expect("store legacy token");

        let err = refresh_mcp_token(
            &server_name,
            "https://example.com/mcp/",
            "https://example.com/oauth/token",
        )
        .await
        .expect_err("unbound token should not refresh");
        let message = err.to_string();

        assert!(message.contains("expected: https://example.com/mcp"));
        assert!(message.contains("stored: <unbound legacy token>"));
        let _ = remove_mcp_token(&server_name);
    }

    #[test]
    fn fallback_metadata_uses_default_oauth_paths() {
        let metadata = fallback_oauth_metadata("https://example.com/mcp/");
        assert_eq!(
            metadata.authorization_endpoint,
            "https://example.com/mcp/oauth/authorize"
        );
        assert_eq!(
            metadata.token_endpoint,
            "https://example.com/mcp/oauth/token"
        );
    }

    #[test]
    fn build_auth_url_contains_required_params() {
        let verifier = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
        let redirect_uri = "http://127.0.0.1:9999/callback";
        let url = build_mcp_auth_url(
            "https://example.com/oauth/authorize",
            redirect_uri,
            verifier,
            "fixture-state",
        )
        .unwrap();
        assert!(url.contains("client_id=coven-code"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A9999%2Fcallback"));
        assert!(
            url.contains("state=fixture-state"),
            "authorization URLs must bind callbacks to an OAuth state"
        );
        let parsed = url::Url::parse(&url).unwrap();
        for key in [
            "client_id",
            "redirect_uri",
            "response_type",
            "code_challenge",
            "code_challenge_method",
            "state",
        ] {
            assert_eq!(
                parsed
                    .query_pairs()
                    .filter(|(found, _)| found == key)
                    .count(),
                1,
                "authorization URL must contain exactly one {key}"
            );
        }
        assert_eq!(
            parsed
                .query_pairs()
                .filter(|(key, _)| key == "scope")
                .count(),
            0,
            "the client must not invent an OAuth scope"
        );
    }

    #[test]
    fn build_auth_url_rejects_preexisting_reserved_parameters() {
        for key in [
            "client_id",
            "redirect_uri",
            "response_type",
            "scope",
            "code_challenge",
            "code_challenge_method",
            "state",
        ] {
            let endpoint =
                format!("https://example.com/oauth/authorize?tenant=fixture&{key}=preset");
            assert!(
                build_mcp_auth_url(
                    &endpoint,
                    "http://127.0.0.1:9999/callback",
                    "fixture-verifier",
                    "fixture-state"
                )
                .is_err(),
                "reserved endpoint parameter {key} must be rejected"
            );
        }

        assert!(build_mcp_auth_url(
            "https://example.com/oauth/authorize?%73tate=preset",
            "http://127.0.0.1:9999/callback",
            "fixture-verifier",
            "fixture-state"
        )
        .is_err());

        let url = build_mcp_auth_url(
            "https://example.com/oauth/authorize?tenant=fixture",
            "http://127.0.0.1:9999/callback",
            "fixture-verifier",
            "fixture-state",
        )
        .unwrap();
        assert_eq!(
            url::Url::parse(&url)
                .unwrap()
                .query_pairs()
                .filter(|(key, value)| key == "tenant" && value == "fixture")
                .count(),
            1
        );
    }

    #[test]
    fn callback_parser_accepts_only_bound_header_only_get() {
        let authority = "127.0.0.1:14555";
        let valid = callback_request(authority, "/callback?code=fixture-code&state=fixture-state");
        assert!(matches!(
            parse_callback_request(&valid, authority, "/callback", "fixture-state").unwrap(),
            CallbackOutcome::Code(code) if code == "fixture-code"
        ));

        let invalid = [
            "POST /callback?code=x&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /wrong?code=x&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=x&state=fixture-state HTTP/1.1\r\nHost: localhost:14555\r\n\r\n",
            "GET /callback?code=x&state=fixture-state HTTP/1.1\r\n\r\n",
            "GET /callback?code=x&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=x HTTP/1.1\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=x&state=wrong HTTP/1.1\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=x&code=y&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=x&state=fixture-state&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=%GG&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=%FF&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=x&state=fixture-state HTTP/1.0\r\nHost: 127.0.0.1:14555\r\n\r\n",
            "GET /callback?code=x&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\nContent-Length: 1\r\n\r\nx",
            "GET /callback?code=x&state=fixture-state HTTP/1.1\r\nHost: 127.0.0.1:14555\r\nTransfer-Encoding: chunked\r\n\r\n",
            "GET /callback?code=x&state=fixture-state HTTP/1.1\r\nHost 127.0.0.1:14555\r\n\r\n",
        ];
        for request in invalid {
            assert!(
                parse_callback_request(request.as_bytes(), authority, "/callback", "fixture-state")
                    .is_err(),
                "unexpectedly accepted malformed callback fixture"
            );
        }
    }

    #[tokio::test]
    async fn invalid_callback_never_receives_success_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let authority = address.to_string();
        let callback = tokio::spawn(async move {
            wait_for_authorization_code_with_limits(
                listener,
                &authority,
                "/callback",
                "expected-state",
                CallbackLimits {
                    overall_timeout: Duration::from_millis(100),
                    ..CallbackLimits::default()
                },
            )
            .await
        });
        let response = send_callback(
            address,
            &callback_request(&address.to_string(), "/wrong?code=fixture&state=wrong"),
        )
        .await;

        assert!(!response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(callback.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn rejected_callback_does_not_consume_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let authority = address.to_string();
        let callback_authority = authority.clone();
        let callback = tokio::spawn(async move {
            wait_for_authorization_code_with_limits(
                listener,
                &callback_authority,
                "/callback",
                "expected-state",
                CallbackLimits {
                    overall_timeout: Duration::from_secs(1),
                    request_timeout: Duration::from_millis(100),
                    max_request_bytes: 512,
                },
            )
            .await
        });

        let wrong = send_callback(
            address,
            &callback_request(&authority, "/callback?code=wrong&state=wrong"),
        )
        .await;
        assert!(wrong.starts_with("HTTP/1.1 400"), "{wrong}");
        let valid = send_callback(
            address,
            &callback_request(
                &authority,
                "/callback?code=fixture-code&state=expected-state",
            ),
        )
        .await;
        assert!(valid.starts_with("HTTP/1.1 200"), "{valid}");
        assert_eq!(callback.await.unwrap().unwrap(), "fixture-code");
    }

    #[tokio::test]
    async fn normalized_path_alias_does_not_consume_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let authority = address.to_string();
        let callback_authority = authority.clone();
        let callback = tokio::spawn(async move {
            wait_for_authorization_code_with_limits(
                listener,
                &callback_authority,
                "/callback",
                "expected-state",
                CallbackLimits {
                    overall_timeout: Duration::from_secs(1),
                    ..CallbackLimits::default()
                },
            )
            .await
        });

        for alias in ["/other/../callback", "/other/%2e%2e/callback"] {
            let response = send_callback(
                address,
                &callback_request(
                    &authority,
                    &format!("{alias}?code=alias-code&state=expected-state"),
                ),
            )
            .await;
            assert!(response.starts_with("HTTP/1.1 400"), "{response}");
        }
        let valid = send_callback(
            address,
            &callback_request(
                &authority,
                "/callback?code=fixture-code&state=expected-state",
            ),
        )
        .await;
        assert!(valid.starts_with("HTTP/1.1 200"), "{valid}");
        assert_eq!(callback.await.unwrap().unwrap(), "fixture-code");
    }

    fn spawn_callback_session(
        listener: TcpListener,
        expected_state: &'static str,
        overall_timeout: Duration,
    ) -> tokio::task::JoinHandle<anyhow::Result<String>> {
        let authority = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            wait_for_authorization_code_with_limits(
                listener,
                &authority,
                "/callback",
                expected_state,
                CallbackLimits {
                    overall_timeout,
                    ..CallbackLimits::default()
                },
            )
            .await
        })
    }

    #[test]
    fn callback_parser_rejects_raw_control_bytes_in_request_target() {
        let authority = "127.0.0.1:14555";
        for (index, target) in [
            "/callback?code=fixture-code&st\tate=fixture-state",
            "/callback?code=fixture-code&\nstate=fixture-state",
            "/callback?code=fixture-code&\rstate=fixture-state",
            "/callback?code=fixture-code&state=fixture\t-state",
            "/callback?code=fixture\0code&state=fixture-state",
            "/callback?code=fixture\x0bcode&state=fixture-state",
            "/callback?code=fixture\x7fcode&state=fixture-state",
        ]
        .into_iter()
        .enumerate()
        {
            let request = callback_request(authority, target);
            assert!(
                parse_callback_request(&request, authority, "/callback", "fixture-state").is_err(),
                "accepted raw control byte target fixture #{index}"
            );
        }
    }

    #[test]
    fn callback_parser_rejects_control_bytes_in_decoded_code_and_state() {
        let authority = "127.0.0.1:14555";
        for (index, target) in [
            "/callback?code=fixture%00code&state=fixture-state",
            "/callback?code=fixture%09code&state=fixture-state",
            "/callback?code=fixture%0Acode&state=fixture-state",
            "/callback?code=fixture%0Bcode&state=fixture-state",
            "/callback?code=fixture%7Fcode&state=fixture-state",
            "/callback?code=fixture%C2%85code&state=fixture-state",
            "/callback?code=fixture%C3%A9code&state=fixture-state",
        ]
        .into_iter()
        .enumerate()
        {
            let request = callback_request(authority, target);
            assert!(
                parse_callback_request(&request, authority, "/callback", "fixture-state").is_err(),
                "accepted decoded control byte fixture #{index}"
            );
        }

        // A control byte in `state` is rejected even when the expected value carries it.
        let request = callback_request(
            authority,
            "/callback?code=fixture-code&state=fixture%07state",
        );
        assert!(
            parse_callback_request(&request, authority, "/callback", "fixture\u{7}state").is_err()
        );

        // Printable values, including a form-decoded space, are still accepted.
        let request = callback_request(authority, "/callback?code=a%2Bb+c%7E&state=fixture-state");
        assert!(parse_callback_request(&request, authority, "/callback", "fixture-state").is_ok());
    }

    #[test]
    fn callback_parser_trims_only_http_whitespace_around_host() {
        let authority = "127.0.0.1:14555";
        let request = |host: &str| {
            format!(
                "GET /callback?code=fixture-code&state=fixture-state HTTP/1.1\r\nHost:{host}\r\n\r\n"
            )
            .into_bytes()
        };
        for (index, host) in [
            "\u{a0}127.0.0.1:14555\u{a0}",
            "\u{3000}127.0.0.1:14555",
            "127.0.0.1:14555\u{85}",
            " 127.0.0.1:14555\u{2003}",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                parse_callback_request(&request(host), authority, "/callback", "fixture-state")
                    .is_err(),
                "accepted Unicode whitespace around Host, fixture #{index}"
            );
        }
        for host in [
            "127.0.0.1:14555",
            " 127.0.0.1:14555",
            "\t127.0.0.1:14555 \t",
        ] {
            assert!(parse_callback_request(
                &request(host),
                authority,
                "/callback",
                "fixture-state"
            )
            .is_ok());
        }
    }

    #[test]
    fn callback_parser_classifies_state_bound_error_redirects() {
        let authority = "127.0.0.1:14555";
        let parse = |target: &str| {
            parse_callback_request(
                &callback_request(authority, target),
                authority,
                "/callback",
                "fixture-state",
            )
        };
        assert!(matches!(
            parse("/callback?error=access_denied&error_description=description-canary&state=fixture-state")
                .unwrap(),
            CallbackOutcome::AuthorizationError(Some(error)) if error == "access_denied"
        ));
        let longest = format!("/callback?error={}&state=fixture-state", "e".repeat(64));
        assert!(matches!(
            parse(&longest).unwrap(),
            CallbackOutcome::AuthorizationError(Some(_))
        ));
        // Free text, markup, empty and over-long values are never reported.
        let too_long = format!("/callback?error={}&state=fixture-state", "e".repeat(65));
        for target in [
            "/callback?error=access%20denied&state=fixture-state",
            "/callback?error=denied%22%3E%3Cscript%3E&state=fixture-state",
            "/callback?error=&state=fixture-state",
            too_long.as_str(),
        ] {
            assert!(matches!(
                parse(target).unwrap(),
                CallbackOutcome::AuthorizationError(None)
            ));
        }
        // Without the exact state, or when ambiguous, an error redirect is a rejection.
        for target in [
            "/callback?error=access_denied",
            "/callback?error=access_denied&state=other-state",
            "/callback?error=access_denied&error=server_error&state=fixture-state",
            "/callback?error=access_denied&code=fixture-code&state=fixture-state",
        ] {
            assert!(parse(target).is_err());
        }
    }

    #[tokio::test]
    async fn raw_control_byte_callback_does_not_consume_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let authority = address.to_string();
        let callback = spawn_callback_session(listener, "expected-state", Duration::from_secs(5));

        let smuggled = send_callback(
            address,
            &callback_request(
                &authority,
                "/callback?code=smuggled-code&st\tate=expected-state",
            ),
        )
        .await;
        assert!(smuggled.starts_with("HTTP/1.1 400"), "{smuggled}");
        let valid = send_callback(
            address,
            &callback_request(
                &authority,
                "/callback?code=fixture-code&state=expected-state",
            ),
        )
        .await;
        assert!(valid.starts_with("HTTP/1.1 200"), "{valid}");
        let code = tokio::time::timeout(Duration::from_secs(2), callback)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(code, "fixture-code");
    }

    #[tokio::test]
    async fn authorization_error_redirect_with_matching_state_ends_session() {
        let long_error = "e".repeat(65);
        let long_target = format!("/callback?error={long_error}&state=expected-state");
        for (index, (target, reported)) in [
            (
                "/callback?error=access_denied&state=expected-state",
                Some("access_denied"),
            ),
            (
                "/callback?state=expected-state&error=invalid_scope&error_description=description-canary&error_uri=https%3A%2F%2Furi-canary.example",
                Some("invalid_scope"),
            ),
            (
                "/callback?error=access+denied+description-canary&state=expected-state",
                None,
            ),
            ("/callback?error=&state=expected-state", None),
            (long_target.as_str(), None),
        ]
        .into_iter()
        .enumerate()
        {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let callback =
                spawn_callback_session(listener, "expected-state", Duration::from_secs(30));

            let response =
                send_callback(address, &callback_request(&address.to_string(), target)).await;
            assert!(
                response == CALLBACK_AUTHORIZATION_ERROR_RESPONSE,
                "authorization error fixture #{index} must get the fixed failure page"
            );
            let error = tokio::time::timeout(Duration::from_secs(2), callback)
                .await
                .unwrap_or_else(|_| {
                    panic!("authorization error fixture #{index} must end the session promptly")
                })
                .unwrap()
                .expect_err("an authorization error must not yield a code");
            let rendered = format!("{error} {error:?} {error:#}");
            match reported {
                Some(code) => assert!(
                    rendered.contains(&format!("'{code}'")),
                    "fixture #{index} must name the validated error code"
                ),
                None => assert!(
                    rendered.contains("returned an error"),
                    "fixture #{index} must use the generic authorization error"
                ),
            }
            for leaked in [
                "expected-state",
                "description-canary",
                "uri-canary",
                long_error.as_str(),
                "Timeout",
            ] {
                assert!(
                    !rendered.contains(leaked),
                    "fixture #{index} authorization error leaked request data or timed out"
                );
            }
        }
    }

    #[tokio::test]
    async fn authorization_error_redirect_without_matching_state_keeps_listening() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let authority = address.to_string();
        let callback = spawn_callback_session(listener, "expected-state", Duration::from_secs(5));

        for (index, target) in [
            "/callback?error=access_denied",
            "/callback?error=access_denied&state=wrong-state",
            "/callback?error=access_denied&state=expected-state&state=expected-state",
            "/callback?error=access_denied&error=server_error&state=expected-state",
            "/callback?error=access_denied&code=fixture-code&state=expected-state",
            "/wrong?error=access_denied&state=expected-state",
        ]
        .into_iter()
        .enumerate()
        {
            let response = send_callback(address, &callback_request(&authority, target)).await;
            assert!(
                response == CALLBACK_REJECTED_RESPONSE,
                "error redirect fixture #{index} must get the generic rejection"
            );
            assert!(
                !callback.is_finished(),
                "fixture #{index} ended the session"
            );
        }
        let valid = send_callback(
            address,
            &callback_request(
                &authority,
                "/callback?code=fixture-code&state=expected-state",
            ),
        )
        .await;
        assert!(valid.starts_with("HTTP/1.1 200"), "{valid}");
        let code = tokio::time::timeout(Duration::from_secs(2), callback)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(code, "fixture-code");
    }

    #[tokio::test]
    async fn stalled_and_oversized_clients_do_not_consume_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let authority = address.to_string();
        let callback_authority = authority.clone();
        let callback = tokio::spawn(async move {
            wait_for_authorization_code_with_limits(
                listener,
                &callback_authority,
                "/callback",
                "expected-state",
                CallbackLimits {
                    overall_timeout: Duration::from_secs(1),
                    request_timeout: Duration::from_millis(40),
                    max_request_bytes: 256,
                },
            )
            .await
        });

        let mut stalled = TcpStream::connect(address).await.unwrap();
        stalled.write_all(b"GET /callback").await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        drop(stalled);

        let oversized = vec![b'x'; 300];
        let rejected = send_callback(address, &oversized).await;
        assert!(rejected.starts_with("HTTP/1.1 400"), "{rejected}");
        let valid = send_callback(
            address,
            &callback_request(
                &authority,
                "/callback?code=bounded-code&state=expected-state",
            ),
        )
        .await;
        assert!(valid.starts_with("HTTP/1.1 200"), "{valid}");
        assert_eq!(callback.await.unwrap().unwrap(), "bounded-code");
    }

    #[tokio::test]
    async fn concurrent_sessions_reject_swapped_states() {
        let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let first_address = first.local_addr().unwrap();
        let second_address = second.local_addr().unwrap();
        let first_authority = first_address.to_string();
        let second_authority = second_address.to_string();
        let first_task_authority = first_authority.clone();
        let second_task_authority = second_authority.clone();
        let limits = CallbackLimits {
            overall_timeout: Duration::from_secs(1),
            ..CallbackLimits::default()
        };
        let first_task = tokio::spawn(async move {
            wait_for_authorization_code_with_limits(
                first,
                &first_task_authority,
                "/callback",
                "first-state",
                limits,
            )
            .await
        });
        let second_task = tokio::spawn(async move {
            wait_for_authorization_code_with_limits(
                second,
                &second_task_authority,
                "/callback",
                "second-state",
                limits,
            )
            .await
        });

        let wrong_first = callback_request(
            &first_authority,
            "/callback?code=wrong-first&state=second-state",
        );
        let wrong_second = callback_request(
            &second_authority,
            "/callback?code=wrong-second&state=first-state",
        );
        let swapped = tokio::join!(
            send_callback(first_address, &wrong_first),
            send_callback(second_address, &wrong_second)
        );
        assert!(swapped.0.starts_with("HTTP/1.1 400"));
        assert!(swapped.1.starts_with("HTTP/1.1 400"));

        let correct_first = callback_request(
            &first_authority,
            "/callback?code=first-code&state=first-state",
        );
        let correct_second = callback_request(
            &second_authority,
            "/callback?code=second-code&state=second-state",
        );
        let correct = tokio::join!(
            send_callback(first_address, &correct_first),
            send_callback(second_address, &correct_second)
        );
        assert!(correct.0.starts_with("HTTP/1.1 200"));
        assert!(correct.1.starts_with("HTTP/1.1 200"));
        assert_eq!(first_task.await.unwrap().unwrap(), "first-code");
        assert_eq!(second_task.await.unwrap().unwrap(), "second-code");
    }

    #[tokio::test]
    async fn callback_overall_timeout_releases_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let result = wait_for_authorization_code_with_limits(
            listener,
            &address.to_string(),
            "/callback",
            "fixture-state",
            CallbackLimits {
                overall_timeout: Duration::from_millis(20),
                request_timeout: Duration::from_millis(10),
                max_request_bytes: 256,
            },
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("Timeout"));
        assert!(TcpListener::bind(address).await.is_ok());
    }

    #[tokio::test]
    async fn valid_callback_exchanges_once_persists_and_cannot_replay() {
        let _home = TokenHomeGuard::new();
        let token_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let token_endpoint = format!("http://{}/token", token_listener.local_addr().unwrap());
        let callback_port = oauth_port_alloc().unwrap();
        let callback_address: std::net::SocketAddr =
            format!("127.0.0.1:{callback_port}").parse().unwrap();
        let redirect_uri = format!("http://{callback_address}/callback");
        let state = "fixture-state".to_string();
        let session = McpAuthSession {
            plugin: None,
            caller: None,
            server_name: "callback-exchange-fixture".into(),
            server_url: "https://mcp.example/fixture".into(),
            auth_url: "https://auth.example/unused".into(),
            redirect_uri: redirect_uri.clone(),
            verifier: "fixture-verifier".into(),
            state: state.clone(),
            metadata: McpOAuthMetadata {
                authorization_endpoint: "https://auth.example/unused".into(),
                token_endpoint: token_endpoint.clone(),
            },
        };
        let token_server = tokio::spawn(async move {
            let (mut socket, _) = token_listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let read = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("POST /token HTTP/1.1"));
            let body = r#"{"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let callback_request = callback_request(
            &callback_address.to_string(),
            &format!("/callback?code=fixture-code&state={state}"),
        );
        let flow = tokio::spawn(run_mcp_auth_session_with(session, move |_| {
            tokio::spawn(async move {
                let response = send_callback(callback_address, &callback_request).await;
                assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            });
            Ok(())
        }));

        let result = flow.await.unwrap().unwrap();
        token_server.await.unwrap();
        assert_eq!(result.server_name, "callback-exchange-fixture");
        assert_eq!(
            get_mcp_token("callback-exchange-fixture")
                .unwrap()
                .access_token,
            "fixture-access"
        );
        assert!(TcpStream::connect(callback_address).await.is_err());
    }

    #[tokio::test]
    async fn bind_callback_listener_reuses_redirect_port() {
        let (listener, authority, callback_path) =
            bind_callback_listener("http://127.0.0.1:14555/callback")
                .await
                .expect("listener should bind");
        let port = listener.local_addr().expect("listener addr").port();
        assert_eq!(authority, "127.0.0.1:14555");
        assert_eq!(callback_path, "/callback");
        assert_eq!(port, 14555);
        drop(listener);
    }

    /// #256: a client that connects and then stalls cannot hold the flow past
    /// the session deadline. The per-connection budget is deliberately longer
    /// than the outer guard, so only the session deadline can end the wait.
    async fn assert_callback_timeout(partial_request: Option<&[u8]>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        // Queue the connection and bytes before starting the deadline.
        let mut peer = match partial_request {
            Some(request) => {
                let mut peer = TcpStream::connect(address).await.unwrap();
                peer.write_all(request).await.unwrap();
                Some(peer)
            }
            None => None,
        };
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_authorization_code_with_limits(
                listener,
                &address.to_string(),
                "/callback",
                "fixture-state",
                CallbackLimits {
                    overall_timeout: Duration::from_millis(100),
                    request_timeout: Duration::from_secs(60),
                    ..CallbackLimits::default()
                },
            ),
        )
        .await
        .expect("callback must enforce its own deadline");
        assert_eq!(
            result.unwrap_err().to_string(),
            "Timeout waiting for OAuth callback"
        );
        if let Some(peer) = peer.as_mut() {
            let mut response = Vec::new();
            tokio::time::timeout(Duration::from_secs(2), peer.read_to_end(&mut response))
                .await
                .expect("timed-out callback must close the accepted socket")
                .unwrap();
            assert!(
                !response.starts_with(b"HTTP/1.1 200"),
                "a stalled client must never be acknowledged as a success"
            );
        }
    }

    #[tokio::test]
    async fn callback_timeout_without_peer() {
        assert_callback_timeout(None).await;
    }

    #[tokio::test]
    async fn callback_timeout_after_idle_connection() {
        assert_callback_timeout(Some(b"")).await;
    }

    #[tokio::test]
    async fn callback_timeout_during_partial_headers() {
        assert_callback_timeout(Some(
            b"GET /callback?code=fixture HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Partial:",
        ))
        .await;
    }

    /// #256's handshake fixtures under the bounded contract: the decoded code
    /// is returned with the unchanged success page, and only after validation.
    /// Every invalid callback, including the old `state: None` row now that
    /// state is mandatory, is answered 400 without ending the session.
    #[tokio::test]
    async fn callback_preserves_code_response_and_validation() {
        const SUCCESS: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nMCP OAuth authentication finished. You can close this tab.\r\n";
        for (target, expected) in [
            ("/callback?code=a%2Bb%20c&state=expected", Some("a+b c")),
            ("/callback?code=fixture&state=ignored", None),
            ("/wrong?code=fixture&state=expected", None),
            ("/callback?code=fixture&state=wrong", None),
            ("/callback?code=fixture", None),
            ("/callback?state=expected", None),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let authority = address.to_string();
            let task_authority = authority.clone();
            let callback = tokio::spawn(async move {
                wait_for_authorization_code_with_limits(
                    listener,
                    &task_authority,
                    "/callback",
                    "expected",
                    CallbackLimits {
                        overall_timeout: Duration::from_secs(5),
                        ..CallbackLimits::default()
                    },
                )
                .await
            });
            let response = send_callback(address, &callback_request(&authority, target)).await;
            let expected_code = match expected {
                Some(code) => {
                    assert_eq!(response, SUCCESS);
                    code
                }
                None => {
                    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
                    let valid = send_callback(
                        address,
                        &callback_request(&authority, "/callback?code=fixture&state=expected"),
                    )
                    .await;
                    assert_eq!(valid, SUCCESS);
                    "fixture"
                }
            };
            let code = tokio::time::timeout(Duration::from_secs(2), callback)
                .await
                .expect("a valid callback must end the session")
                .unwrap()
                .unwrap();
            assert_eq!(code, expected_code);
        }
    }
}

#[cfg(test)]
mod token_error_tests {
    use super::test_support::TokenHomeGuard;
    use super::*;
    use tokio::io::AsyncReadExt;

    async fn token_response(status: u16, body: &str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let response = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let worker = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(5), async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 2048];
                loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                    assert!(request.len() < 16384);
                    if let Some(end) = request.windows(4).position(|s| s == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                socket.write_all(response.as_bytes()).await.unwrap();
            })
            .await
            .unwrap();
        });
        (endpoint, worker)
    }

    #[tokio::test]
    async fn token_errors_do_not_disclose_response_canaries() {
        let _home = TokenHomeGuard::new();
        let existing = McpToken {
            access_token: "old-access-fixture".into(),
            refresh_token: Some("old-refresh-fixture".into()),
            expires_at: None,
            scope: None,
            server_name: "error-fixture".into(),
            server_url: Some("http://fixture.invalid/mcp".into()),
        };
        store_mcp_token(&existing).unwrap();
        for (status, body) in [
            (400, r#"{"error":"response-secret-canary"}"#),
            (
                200,
                r#"{"access_token":"fixture","expires_in":"response-secret-canary"}"#,
            ),
        ] {
            for refresh in [false, true] {
                let (endpoint, worker) = token_response(status, body).await;
                let result = if refresh {
                    refresh_mcp_token("error-fixture", "http://fixture.invalid/mcp", &endpoint)
                        .await
                } else {
                    exchange_code(
                        &endpoint,
                        "code-fixture",
                        "verifier-fixture",
                        "http://127.0.0.1/callback",
                    )
                    .await
                };
                worker.await.unwrap();
                let error = result.expect_err("fixture must fail");
                let rendered = format!("{error} {error:?} {error:#} {error:#?}");
                assert!(
                    !rendered.contains("response-secret-canary"),
                    "response body escaped through error formatting"
                );
                assert_eq!(
                    get_mcp_token("error-fixture").unwrap().access_token,
                    existing.access_token
                );
            }
        }
    }

    #[tokio::test]
    async fn token_transport_errors_do_not_disclose_endpoint_queries() {
        let _home = TokenHomeGuard::new();
        let token = McpToken {
            access_token: "old-access-fixture".into(),
            refresh_token: Some("old-refresh-fixture".into()),
            expires_at: None,
            scope: None,
            server_name: "transport-fixture".into(),
            server_url: Some("http://fixture.invalid/mcp".into()),
        };
        store_mcp_token(&token).unwrap();
        // Unsupported scheme fails locally without a network connection.
        let endpoint = "ftp://127.0.0.1/token?secret=url-secret-canary";
        for refresh in [false, true] {
            let result = if refresh {
                refresh_mcp_token("transport-fixture", "http://fixture.invalid/mcp", endpoint).await
            } else {
                exchange_code(
                    endpoint,
                    "code-fixture",
                    "verifier-fixture",
                    "http://127.0.0.1/callback",
                )
                .await
            };
            let error = result.expect_err("unsupported transport must fail");
            assert!(
                !format!("{error} {error:?} {error:#} {error:#?}").contains("url-secret-canary")
            );
        }
    }

    #[tokio::test]
    async fn token_success_still_decodes_and_refreshes() {
        let _home = TokenHomeGuard::new();
        let (endpoint, worker) = token_response(200, r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600,"scope":"read"}"#).await;
        let mut token = exchange_code(
            &endpoint,
            "code-fixture",
            "verifier-fixture",
            "http://127.0.0.1/callback",
        )
        .await
        .unwrap();
        worker.await.unwrap();
        assert_eq!(token.access_token, "new-access");
        assert_eq!(token.scope.as_deref(), Some("read"));
        token.server_name = "success-fixture".into();
        token.server_url = Some("http://fixture.invalid/mcp".into());
        store_mcp_token(&token).unwrap();
        let (endpoint, worker) = token_response(
            200,
            r#"{"access_token":"refreshed-access","expires_in":3600}"#,
        )
        .await;
        let updated = refresh_mcp_token("success-fixture", "http://fixture.invalid/mcp", &endpoint)
            .await
            .unwrap();
        worker.await.unwrap();
        assert_eq!(updated.access_token, "refreshed-access");
        assert_eq!(updated.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(
            get_mcp_token("success-fixture").unwrap().access_token,
            "refreshed-access"
        );
    }
}

#[cfg(test)]
mod plugin_session_tests {
    use super::*;
    use claurst_core::plugin_mcp::{PluginMcpContribution, PluginMcpOrigin, PluginMcpSource};
    use tokio::{io::AsyncReadExt, sync::oneshot, time::timeout};

    fn session(root: &std::path::Path, token_url: &str) -> (PluginMcpContribution, McpAuthSession) {
        let plugin = PluginMcpContribution::new(
            PluginMcpOrigin::new(PluginMcpSource::Project, root, Some(root)).unwrap(),
            claurst_core::McpServerConfig {
                name: "fixture".into(),
                command: None,
                args: vec![],
                env: Default::default(),
                url: Some(token_url.into()),
                server_type: "http".into(),
            },
        );
        let session = McpAuthSession {
            plugin: Some(plugin.clone()),
            caller: Some(root.to_path_buf()),
            server_name: "fixture".into(),
            server_url: token_url.into(),
            auth_url: "http://127.0.0.1:1/unused-browser".into(),
            redirect_uri: format!("http://127.0.0.1:{}/callback", oauth_port_alloc().unwrap()),
            verifier: "fixture-verifier".into(),
            state: "fixture-state".into(),
            metadata: McpOAuthMetadata {
                authorization_endpoint: "unused".into(),
                token_endpoint: token_url.into(),
            },
        };
        (plugin, session)
    }

    #[tokio::test]
    async fn plugin_revocation_releases_pending_oauth_callback_listener() {
        let _home = test_support::TokenHomeGuard::new();
        let root = tempfile::tempdir().unwrap();
        let (plugin, session) = session(root.path(), "http://127.0.0.1:1/token");
        let redirect_uri = session.redirect_uri.clone();
        let (opened, ready) = oneshot::channel();
        let task = tokio::spawn(run_mcp_auth_session_with(session, move |_| {
            let _ = opened.send(());
            Ok(())
        }));
        timeout(Duration::from_secs(2), ready)
            .await
            .unwrap()
            .unwrap();
        plugin.revoke();
        timeout(Duration::from_secs(2), plugin.wait_retired())
            .await
            .unwrap()
            .unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("revoked"));
        assert!(
            bind_callback_listener(&redirect_uri).await.is_ok(),
            "callback socket must be released when retirement completes"
        );
        assert!(get_mcp_token("fixture").is_none());
    }

    #[tokio::test]
    async fn plugin_revocation_during_exchange_prevents_token_persistence() {
        let _home = test_support::TokenHomeGuard::new();
        let root = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let (plugin, session) = session(root.path(), &url);
        let callback = format!(
            "{}?code=fixture-code&state={}",
            session.redirect_uri, session.state
        );
        let (seen, request_seen) = oneshot::channel();
        let (release, resume) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            let _ = seen.send(());
            let _ = resume.await;
            let body = r#"{"access_token":"fixture-token","expires_in":3600}"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = socket.write_all(response.as_bytes()).await;
        });
        let (opened, ready) = oneshot::channel();
        let task = tokio::spawn(run_mcp_auth_session_with(session, move |_| {
            let _ = opened.send(());
            Ok(())
        }));
        timeout(Duration::from_secs(2), ready)
            .await
            .unwrap()
            .unwrap();
        reqwest::get(callback).await.unwrap();
        timeout(Duration::from_secs(2), request_seen)
            .await
            .unwrap()
            .unwrap();
        plugin.revoke();
        timeout(Duration::from_secs(2), plugin.wait_retired())
            .await
            .unwrap()
            .unwrap();
        assert!(task
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("revoked"));
        let _ = release.send(());
        server.await.unwrap();
        assert!(
            get_mcp_token("fixture").is_none(),
            "a revoked callback must never write a token"
        );
    }
    #[tokio::test]
    async fn plugin_refresh_cannot_persist_after_revocation_during_response() {
        let _home = test_support::TokenHomeGuard::new();
        let root = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let (plugin, _) = session(root.path(), &url);
        store_mcp_token(&McpToken {
            access_token: "original-fixture-token".into(),
            refresh_token: Some("fixture-refresh".into()),
            expires_at: Some(0),
            scope: None,
            server_name: "fixture".into(),
            server_url: Some(url.clone()),
        })
        .unwrap();
        let withdrawn = plugin.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            withdrawn.revoke();
            let body = r#"{"access_token":"replacement-fixture-token","expires_in":3600}"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let result =
            refresh_mcp_token_for("fixture", &url, &url, Some(&plugin), Some(root.path())).await;
        server.await.unwrap();
        assert!(
            result.is_err(),
            "refresh continuation must check authority after its network await"
        );
        assert_eq!(
            get_mcp_token("fixture").unwrap().access_token,
            "original-fixture-token"
        );
    }
}
