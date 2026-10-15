// providers/codex.rs — OpenAI Codex provider (OAuth-authenticated).
//
// Codex uses OpenAI's Responses API at:
//   https://chatgpt.com/backend-api/codex/responses
//
// Auth: Bearer token obtained via the Codex OAuth flow stored in
//   ~/.coven-code/codex_tokens.json (`CodexTokens` struct).
//
// Token refresh: if `expires_at` is in the past we POST to the OpenAI token
//   endpoint with `grant_type=refresh_token` before making the request.
//
// Model list: static — the Codex endpoint does not expose a /models route,
//   so we use the `CODEX_MODELS` constant from `claurst-core`.

use parking_lot::Mutex;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_stream::stream;
use async_trait::async_trait;
use claurst_core::codex_oauth::{
    CODEX_API_ENDPOINT, CODEX_MODELS, CODEX_TOKEN_URL, DEFAULT_CODEX_MODEL,
};
use claurst_core::goal::{GoalTurn, GoalTurnPermission};
use claurst_core::oauth_config::{get_codex_tokens, save_codex_tokens, CodexTokens};
use claurst_core::provider_id::{ModelId, ProviderId};
use claurst_core::types::UsageInfo;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::error_handling::{parse_error_response_with_retry_after, parse_retry_after_secs};
use crate::provider::{LlmProvider, ModelInfo};
use crate::provider_error::ProviderError;
use crate::provider_types::{
    ProviderCapabilities, ProviderRequest, ProviderResponse, ProviderStatus, StopReason,
    StreamEvent, SystemPromptStyle,
};

use crate::providers::responses_input::to_responses_input;

/// Extract a `Retry-After` delay (in seconds) from response headers, if the
/// server sent one in the delay-seconds form.
fn retry_after_from_headers(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after_secs)
}

// ---------------------------------------------------------------------------
// CodexProvider
// ---------------------------------------------------------------------------

struct RefreshedToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

pub struct CodexProvider {
    id: ProviderId,
    http_client: reqwest::Client,
    /// Mutable token cache: updated in-place when a refresh succeeds.
    tokens: Arc<Mutex<CodexTokens>>,
    goal_authority: Option<(GoalTurn, GoalTurnPermission)>,
    #[cfg(test)]
    test_endpoints: Option<(String, String)>,
}

impl CodexProvider {
    pub fn new(tokens: CodexTokens) -> Result<Self, ProviderError> {
        let id = ProviderId::new(ProviderId::CODEX);
        let http_client = crate::providers::http_util::build_default_http_client(&id)?;
        Ok(Self {
            id,
            http_client,
            tokens: Arc::new(Mutex::new(tokens)),
            goal_authority: None,
            #[cfg(test)]
            test_endpoints: None,
        })
    }

    fn for_goal(&self, turn: &GoalTurn, permission: GoalTurnPermission) -> Self {
        Self {
            id: self.id.clone(),
            http_client: self.http_client.clone(),
            tokens: self.tokens.clone(),
            goal_authority: Some((turn.clone(), permission)),
            #[cfg(test)]
            test_endpoints: self.test_endpoints.clone(),
        }
    }

    fn validate_goal_request(&self) -> Result<(), ProviderError> {
        if let Some((turn, permission)) = &self.goal_authority {
            let actual = turn.validate().map_err(|error| ProviderError::Other {
                provider: self.id.clone(),
                message: format!("Goal authority revoked: {error}"),
                status: None,
                body: None,
            })?;
            if actual != *permission {
                return Err(ProviderError::Other {
                    provider: self.id.clone(),
                    message: "Goal permits only its foreground final response".into(),
                    status: None,
                    body: None,
                });
            }
        }
        Ok(())
    }

    /// Construct from stored tokens; returns `None` if no tokens are saved or
    /// if the HTTP client could not be built.
    pub fn from_stored() -> Option<Self> {
        let tokens = get_codex_tokens()?;
        if tokens.access_token.is_empty() {
            return None;
        }
        Self::new(tokens).ok()
    }

    // -----------------------------------------------------------------------
    // Token management
    // -----------------------------------------------------------------------

    fn is_expired(tokens: &CodexTokens) -> bool {
        let Some(expires_at) = tokens.expires_at else {
            return false; // No expiry info — assume still valid.
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Treat as expired 60 s early to avoid races.
        now + 60 >= expires_at
    }

    /// Return the current access token, refreshing first if it is expired.
    async fn access_token(&self) -> Result<String, ProviderError> {
        // Check expiry under the lock; clone what we need; release.
        let (token, needs_refresh, refresh_token) = {
            let guard = self.tokens.lock();
            let expired = Self::is_expired(&guard);
            (
                guard.access_token.clone(),
                expired,
                guard.refresh_token.clone(),
            )
        };

        if !needs_refresh {
            return Ok(token);
        }

        let Some(refresh) = refresh_token else {
            // No refresh token — return what we have and hope for the best.
            warn!("Codex access token is expired and no refresh token is available");
            return Ok(token);
        };

        debug!("Codex access token expired — refreshing");
        self.refresh_token(&refresh).await
    }

    fn decode_refresh_response(
        &self,
        status: u16,
        text: &str,
    ) -> Result<RefreshedToken, ProviderError> {
        if !(200..300).contains(&(status as usize)) {
            return Err(ProviderError::Other {
                provider: self.id.clone(),
                message: format!("Token refresh failed (HTTP {})", status),
                status: Some(status),
                body: None,
            });
        }

        let json_val: Value = serde_json::from_str(text).map_err(|_| ProviderError::Other {
            provider: self.id.clone(),
            message: "Failed to parse token refresh response".to_string(),
            status: Some(status),
            body: None,
        })?;

        let new_access = json_val
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if new_access.is_empty() {
            return Err(ProviderError::Other {
                provider: self.id.clone(),
                message: "Token refresh response missing access_token".to_string(),
                status: Some(status),
                body: None,
            });
        }

        let new_refresh = json_val
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let expires_in = json_val.get("expires_in").and_then(|v| v.as_u64());

        Ok(RefreshedToken {
            access_token: new_access,
            refresh_token: new_refresh,
            expires_in,
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<String, ProviderError> {
        let body = json!({
            "grant_type": "refresh_token",
            "client_id": claurst_core::codex_oauth::CODEX_CLIENT_ID,
            "refresh_token": refresh_token,
        });

        #[cfg(not(test))]
        let endpoint = CODEX_TOKEN_URL;
        #[cfg(test)]
        let endpoint = self
            .test_endpoints
            .as_ref()
            .map_or(CODEX_TOKEN_URL, |urls| urls.0.as_str());
        let resp = self
            .http_client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Other {
                provider: self.id.clone(),
                message: format!("Token refresh request failed: {}", e.without_url()),
                status: None,
                body: None,
            })?;

        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Failed to read token refresh response: {}", e.without_url()),
            status: Some(status),
            body: None,
        })?;

        let RefreshedToken {
            access_token: new_access,
            refresh_token: new_refresh,
            expires_in,
        } = self.decode_refresh_response(status, &text)?;

        let new_expires_at = expires_in.map(|secs| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                + secs
        });

        // Persist and cache the refreshed tokens.
        let mut updated = {
            let guard = self.tokens.lock();
            guard.clone()
        };
        updated.access_token = new_access.clone();
        if let Some(r) = new_refresh {
            updated.refresh_token = Some(r);
        }
        updated.expires_at = new_expires_at;

        if let Err(e) = save_codex_tokens(&updated) {
            warn!("Failed to persist refreshed Codex tokens: {}", e);
        }

        {
            let mut guard = self.tokens.lock();
            *guard = updated;
        }

        Ok(new_access)
    }

    // -----------------------------------------------------------------------
    // Request helpers
    // -----------------------------------------------------------------------

    fn codex_headers(
        &self,
        builder: reqwest::RequestBuilder,
        token: &str,
        account_id: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let builder = builder.bearer_auth(token).header(
            "User-Agent",
            concat!("coven-code/", env!("CARGO_PKG_VERSION")),
        );

        if let Some(id) = account_id {
            builder.header("ChatGPT-Account-Id", id)
        } else {
            builder
        }
    }

    fn account_id(&self) -> Option<String> {
        self.tokens.lock().account_id.clone()
    }

    fn system_prompt_to_text(request: &ProviderRequest) -> String {
        match request.system_prompt.as_ref() {
            Some(crate::provider_types::SystemPrompt::Text(text)) => text.clone(),
            Some(crate::provider_types::SystemPrompt::Blocks(blocks)) => blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            None => String::new(),
        }
    }

    /// Build the Responses-API request body for Codex.
    fn build_responses_body(request: &ProviderRequest) -> Value {
        let input = to_responses_input(request);
        let instructions = Self::system_prompt_to_text(request);

        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                    "strict": false,
                })
            })
            .collect();

        let mut body = json!({
            "model": request.model,
            "input": input,
            "instructions": instructions,
            "store": false,
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        body
    }

    fn extract_stream_error_message(json_val: &Value) -> String {
        json_val
            .pointer("/error/message")
            .and_then(|value| value.as_str())
            .or_else(|| {
                json_val
                    .pointer("/response/error/message")
                    .and_then(|value| value.as_str())
            })
            .or_else(|| {
                json_val
                    .pointer("/message")
                    .and_then(|value| value.as_str())
            })
            .map(|value| value.to_string())
            .unwrap_or_else(|| json_val.to_string())
    }

    // -----------------------------------------------------------------------
    // HTTP call
    // -----------------------------------------------------------------------

    async fn send_responses_request(
        &self,
        request: &ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        self.validate_goal_request()?;
        let token = self.access_token().await?;
        let account_id = self.account_id();

        let body = Self::build_responses_body(request);

        #[cfg(not(test))]
        let endpoint = CODEX_API_ENDPOINT;
        #[cfg(test)]
        let endpoint = self
            .test_endpoints
            .as_ref()
            .map_or(CODEX_API_ENDPOINT, |urls| urls.1.as_str());
        let builder = self
            .http_client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");
        let builder = self.codex_headers(builder, &token, account_id.as_deref());

        let builder = builder.json(&body);
        // OAuth preparation may await refresh HTTP/body reads. Recheck at the
        // actual model-send boundary after that preparation has completed.
        self.validate_goal_request()?;
        let resp = builder.send().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("HTTP request failed: {}", e),
            status: None,
            body: None,
        })?;

        let status = resp.status().as_u16();
        let retry_after = retry_after_from_headers(resp.headers());
        let text = resp.text().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Failed to read response body: {}", e),
            status: Some(status),
            body: None,
        })?;

        if !(200..300).contains(&(status as usize)) {
            return Err(parse_error_response_with_retry_after(
                status,
                &text,
                &self.id,
                retry_after,
            ));
        }

        let json_val: Value = serde_json::from_str(&text).map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Failed to parse response JSON: {}", e),
            status: Some(status),
            body: Some(text.clone()),
        })?;

        Self::parse_responses_response(&self.id, &json_val)
    }

    async fn send_responses_streaming_request(
        &self,
        request: &ProviderRequest,
    ) -> Result<reqwest::Response, ProviderError> {
        self.validate_goal_request()?;
        let token = self.access_token().await?;
        let account_id = self.account_id();

        let mut body = Self::build_responses_body(request);
        body["stream"] = json!(true);

        #[cfg(not(test))]
        let endpoint = CODEX_API_ENDPOINT;
        #[cfg(test)]
        let endpoint = self
            .test_endpoints
            .as_ref()
            .map_or(CODEX_API_ENDPOINT, |urls| urls.1.as_str());
        let builder = self
            .http_client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");
        let builder = self.codex_headers(builder, &token, account_id.as_deref());

        let builder = builder.json(&body);
        // OAuth preparation may await refresh HTTP/body reads. Recheck at the
        // actual model-send boundary after that preparation has completed.
        self.validate_goal_request()?;
        let resp = builder.send().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("HTTP request failed: {}", e),
            status: None,
            body: None,
        })?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&(status as usize)) {
            let retry_after = retry_after_from_headers(resp.headers());
            let text = resp.text().await.map_err(|e| ProviderError::Other {
                provider: self.id.clone(),
                message: format!("Failed to read response body: {}", e),
                status: Some(status),
                body: None,
            })?;
            return Err(parse_error_response_with_retry_after(
                status,
                &text,
                &self.id,
                retry_after,
            ));
        }

        Ok(resp)
    }

    // -----------------------------------------------------------------------
    // Response parsing for the OpenAI Responses API
    // -----------------------------------------------------------------------

    fn parse_responses_response(
        provider_id: &ProviderId,
        json_val: &Value,
    ) -> Result<ProviderResponse, ProviderError> {
        use claurst_core::types::ContentBlock;

        let id = json_val
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let model = json_val
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_CODEX_MODEL)
            .to_string();

        let output = json_val
            .get("output")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ProviderError::Other {
                provider: provider_id.clone(),
                message: "No output in Codex Responses API response".to_string(),
                status: None,
                body: Some(json_val.to_string()),
            })?;

        let mut content: Vec<ContentBlock> = Vec::new();
        let mut has_tool_call = false;

        for item in output {
            match item.get("type").and_then(|v| v.as_str()) {
                Some("message") => {
                    if let Some(parts) = item.get("content").and_then(|v| v.as_array()) {
                        for part in parts {
                            match part.get("type").and_then(|v| v.as_str()) {
                                Some("output_text") | Some("text") => {
                                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                        if !text.is_empty() {
                                            content.push(ContentBlock::Text {
                                                text: text.to_string(),
                                            });
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Some("reasoning") => {
                    if let Some(summaries) = item.get("summary").and_then(|v| v.as_array()) {
                        let reasoning: String = summaries
                            .iter()
                            .filter_map(|s| s.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join("");
                        if !reasoning.is_empty() {
                            content.push(ContentBlock::Thinking {
                                thinking: reasoning,
                                signature: String::new(),
                            });
                        }
                    }
                }
                Some("function_call") => {
                    has_tool_call = true;
                    let id = item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    let input = serde_json::from_str(args).unwrap_or_else(|_| json!({}));
                    content.push(ContentBlock::ToolUse { id, name, input });
                }
                _ => {}
            }
        }

        let stop_reason = if has_tool_call {
            StopReason::ToolUse
        } else {
            match json_val
                .get("incomplete_details")
                .and_then(|v| v.get("reason"))
                .and_then(|v| v.as_str())
            {
                Some("max_output_tokens") => StopReason::MaxTokens,
                Some("content_filter") => StopReason::ContentFiltered,
                Some(other) if !other.is_empty() => StopReason::Other(other.to_string()),
                _ => StopReason::EndTurn,
            }
        };

        let usage = {
            let u = json_val.get("usage");
            UsageInfo {
                input_tokens: u
                    .and_then(|v| v.get("input_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                output_tokens: u
                    .and_then(|v| v.get("output_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            }
        };

        Ok(ProviderResponse {
            id,
            content,
            stop_reason,
            usage,
            model,
        })
    }

    fn parse_stream(
        mut byte_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
        provider_id: ProviderId,
    ) -> impl Stream<Item = Result<StreamEvent, ProviderError>> {
        stream! {
            let mut lines = crate::sse_lines::SseLines::default();
            let mut current_event = String::new();
            let mut current_data: Vec<String> = Vec::new();
            let mut message_started = false;
            let mut message_id = String::from("unknown");
            let mut model_name = String::from(DEFAULT_CODEX_MODEL);
            let mut saw_tool_call = false;
            let mut open_blocks: std::collections::HashSet<usize> = std::collections::HashSet::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        yield Err(ProviderError::StreamError {
                            provider: provider_id.clone(),
                            message: format!("Stream read error: {}", e),
                            partial_response: None,
                        });
                        return;
                    }
                };

                for raw_line in lines.feed(&chunk) {
                    let line = raw_line.trim_end_matches('\r');

                    if line.is_empty() {
                        if current_data.is_empty() {
                            current_event.clear();
                            continue;
                        }

                        let data = current_data.join("\n");
                        current_data.clear();
                        let trimmed = data.trim();
                        if trimmed.is_empty() || trimmed == "[DONE]" {
                            current_event.clear();
                            continue;
                        }

                        let json_val: Value = match serde_json::from_str(trimmed) {
                            Ok(value) => value,
                            Err(e) => {
                                yield Err(ProviderError::StreamError {
                                    provider: provider_id.clone(),
                                    message: format!("Failed to parse Codex stream JSON: {}", e),
                                    partial_response: Some(trimmed.to_string()),
                                });
                                return;
                            }
                        };

                        let event_name = if current_event.is_empty() {
                            json_val
                                .get("type")
                                .and_then(|value| value.as_str())
                                .unwrap_or("")
                                .to_string()
                        } else {
                            current_event.clone()
                        };

                        match event_name.as_str() {
                            "response.created" | "response.in_progress" => {
                                if let Some(response) = json_val.get("response") {
                                    if let Some(id) = response.get("id").and_then(|value| value.as_str()) {
                                        message_id = id.to_string();
                                    }
                                    if let Some(model) = response.get("model").and_then(|value| value.as_str()) {
                                        model_name = model.to_string();
                                    }
                                    if !message_started {
                                        yield Ok(StreamEvent::MessageStart {
                                            id: message_id.clone(),
                                            model: model_name.clone(),
                                            usage: UsageInfo::default(),
                                        });
                                        message_started = true;
                                    }
                                }
                            }
                            "response.output_item.added" => {
                                let output_index = json_val
                                    .get("output_index")
                                    .and_then(|value| value.as_u64())
                                    .unwrap_or(0) as usize;
                                if let Some(item) = json_val.get("item") {
                                    match item.get("type").and_then(|value| value.as_str()) {
                                        Some("message") => {
                                            if let Some(id) = item.get("id").and_then(|value| value.as_str()) {
                                                message_id = id.to_string();
                                            }
                                            if !message_started {
                                                yield Ok(StreamEvent::MessageStart {
                                                    id: message_id.clone(),
                                                    model: model_name.clone(),
                                                    usage: UsageInfo::default(),
                                                });
                                                message_started = true;
                                            }
                                        }
                                        Some("function_call") => {
                                            saw_tool_call = true;
                                            let call_id = item
                                                .get("call_id")
                                                .or_else(|| item.get("id"))
                                                .and_then(|value| value.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let name = item
                                                .get("name")
                                                .and_then(|value| value.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            if open_blocks.insert(output_index) {
                                                yield Ok(StreamEvent::ContentBlockStart {
                                                    index: output_index,
                                                    content_block: claurst_core::types::ContentBlock::ToolUse {
                                                        id: call_id,
                                                        name,
                                                        input: json!({}),
                                                    },
                                                });
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            "response.content_part.added" => {
                                let output_index = json_val
                                    .get("output_index")
                                    .and_then(|value| value.as_u64())
                                    .unwrap_or(0) as usize;
                                if let Some(part) = json_val.get("part") {
                                    if matches!(part.get("type").and_then(|value| value.as_str()), Some("output_text") | Some("text")) {
                                        if !message_started {
                                            yield Ok(StreamEvent::MessageStart {
                                                id: message_id.clone(),
                                                model: model_name.clone(),
                                                usage: UsageInfo::default(),
                                            });
                                            message_started = true;
                                        }
                                        if open_blocks.insert(output_index) {
                                            yield Ok(StreamEvent::ContentBlockStart {
                                                index: output_index,
                                                content_block: claurst_core::types::ContentBlock::Text {
                                                    text: String::new(),
                                                },
                                            });
                                        }
                                    }
                                }
                            }
                            "response.output_text.delta" => {
                                let output_index = json_val
                                    .get("output_index")
                                    .and_then(|value| value.as_u64())
                                    .unwrap_or(0) as usize;
                                let delta = json_val
                                    .get("delta")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("");
                                if !message_started {
                                    yield Ok(StreamEvent::MessageStart {
                                        id: message_id.clone(),
                                        model: model_name.clone(),
                                        usage: UsageInfo::default(),
                                    });
                                    message_started = true;
                                }
                                if open_blocks.insert(output_index) {
                                    yield Ok(StreamEvent::ContentBlockStart {
                                        index: output_index,
                                        content_block: claurst_core::types::ContentBlock::Text {
                                            text: String::new(),
                                        },
                                    });
                                }
                                if !delta.is_empty() {
                                    yield Ok(StreamEvent::TextDelta {
                                        index: output_index,
                                        text: delta.to_string(),
                                    });
                                }
                            }
                            "response.function_call_arguments.delta" => {
                                let output_index = json_val
                                    .get("output_index")
                                    .and_then(|value| value.as_u64())
                                    .unwrap_or(0) as usize;
                                let delta = json_val
                                    .get("delta")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("");
                                if !delta.is_empty() {
                                    yield Ok(StreamEvent::InputJsonDelta {
                                        index: output_index,
                                        partial_json: delta.to_string(),
                                    });
                                }
                            }
                            "response.output_item.done" => {
                                let output_index = json_val
                                    .get("output_index")
                                    .and_then(|value| value.as_u64())
                                    .unwrap_or(0) as usize;
                                if open_blocks.remove(&output_index) {
                                    yield Ok(StreamEvent::ContentBlockStop { index: output_index });
                                }
                            }
                            "response.completed" => {
                                if let Some(response) = json_val.get("response") {
                                    if let Some(id) = response.get("id").and_then(|value| value.as_str()) {
                                        message_id = id.to_string();
                                    }
                                    if let Some(model) = response.get("model").and_then(|value| value.as_str()) {
                                        model_name = model.to_string();
                                    }

                                    if !message_started {
                                        yield Ok(StreamEvent::MessageStart {
                                            id: message_id.clone(),
                                            model: model_name.clone(),
                                            usage: UsageInfo::default(),
                                        });
                                        // No reassignment: this branch is the
                                        // stream-completion handler — execution
                                        // never re-enters the guard below.
                                    }

                                    let mut remaining: Vec<usize> = open_blocks.drain().collect();
                                    remaining.sort_unstable();
                                    for index in remaining {
                                        yield Ok(StreamEvent::ContentBlockStop { index });
                                    }

                                    let usage_json = response.get("usage");
                                    let usage = UsageInfo {
                                        input_tokens: usage_json
                                            .and_then(|value| value.get("input_tokens"))
                                            .and_then(|value| value.as_u64())
                                            .unwrap_or(0),
                                        output_tokens: usage_json
                                            .and_then(|value| value.get("output_tokens"))
                                            .and_then(|value| value.as_u64())
                                            .unwrap_or(0),
                                        cache_creation_input_tokens: 0,
                                        cache_read_input_tokens: usage_json
                                            .and_then(|value| value.get("input_tokens_details"))
                                            .and_then(|value| value.get("cached_tokens"))
                                            .and_then(|value| value.as_u64())
                                            .unwrap_or(0),
                                    };

                                    let stop_reason = if saw_tool_call {
                                        StopReason::ToolUse
                                    } else {
                                        match response
                                            .get("incomplete_details")
                                            .and_then(|value| value.get("reason"))
                                            .and_then(|value| value.as_str())
                                        {
                                            Some("max_output_tokens") => StopReason::MaxTokens,
                                            Some("content_filter") => StopReason::ContentFiltered,
                                            Some(other) if !other.is_empty() => StopReason::Other(other.to_string()),
                                            _ => StopReason::EndTurn,
                                        }
                                    };

                                    yield Ok(StreamEvent::MessageDelta {
                                        stop_reason: Some(stop_reason),
                                        usage: Some(usage),
                                    });
                                    yield Ok(StreamEvent::MessageStop);
                                    return;
                                }
                            }
                            "response.failed" | "response.error" | "error" => {
                                yield Err(ProviderError::StreamError {
                                    provider: provider_id.clone(),
                                    message: Self::extract_stream_error_message(&json_val),
                                    partial_response: Some(trimmed.to_string()),
                                });
                                return;
                            }
                            _ => {}
                        }

                        current_event.clear();
                        continue;
                    }

                    if let Some(rest) = line.strip_prefix("event:") {
                        current_event = rest.trim().to_string();
                        continue;
                    }

                    if let Some(rest) = line.strip_prefix("data:") {
                        current_data.push(rest.trim_start().to_string());
                    }
                }
            }

            yield Err(ProviderError::StreamError {
                provider: provider_id,
                message: "Codex stream ended before response.completed".to_string(),
                partial_response: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for CodexProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn name(&self) -> &str {
        "OpenAI Codex"
    }

    async fn create_message(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        self.send_responses_request(&request).await
    }

    async fn create_message_stream_for_goal(
        &self,
        request: ProviderRequest,
        turn: Option<&GoalTurn>,
        permission: GoalTurnPermission,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        match turn {
            Some(turn) => {
                self.for_goal(turn, permission)
                    .create_message_stream(request)
                    .await
            }
            None => self.create_message_stream(request).await,
        }
    }

    async fn create_message_stream(
        &self,
        request: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        let resp = self.send_responses_streaming_request(&request).await?;
        Ok(Box::pin(Self::parse_stream(
            resp.bytes_stream(),
            self.id.clone(),
        )))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let models = CODEX_MODELS
            .iter()
            .map(|(id, name)| ModelInfo {
                id: ModelId::new(*id),
                provider_id: self.id.clone(),
                name: name.to_string(),
                context_window: match *id {
                    // Codex backend window for the GPT-5.6 family, per the
                    // upstream models.json catalog (openai/codex models-manager).
                    "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" => 372_000,
                    "gpt-5.5" | "gpt-5.4" => 1_050_000,
                    "gpt-5.4-mini" | "gpt-5.3-codex" | "gpt-5.2-codex" | "gpt-5.1-codex"
                    | "gpt-5.1-codex-mini" | "gpt-5.1-codex-max" | "gpt-5.2" | "gpt-5-codex"
                    | "gpt-5-mini" => 400_000,
                    "gpt-5.3-codex-spark" => 128_000,
                    "gpt-4.1" => 128_000,
                    "o4-mini" => 200_000,
                    _ => 128_000,
                },
                max_output_tokens: match *id {
                    "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" => 128_000,
                    "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" | "gpt-5.3-codex" | "gpt-5.2-codex"
                    | "gpt-5.1-codex" | "gpt-5.1-codex-mini" | "gpt-5.1-codex-max" | "gpt-5.2"
                    | "gpt-5-codex" | "gpt-5-mini" => 128_000,
                    _ => 32_768,
                },
            })
            .collect();
        Ok(models)
    }

    async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
        // Validate that a non-expired token exists without making a network call.
        let guard = self.tokens.lock();
        if guard.access_token.is_empty() {
            return Ok(ProviderStatus::Unavailable {
                reason: "no Codex access token — run /connect to authenticate".to_string(),
            });
        }
        if Self::is_expired(&guard) && guard.refresh_token.is_none() {
            return Ok(ProviderStatus::Unavailable {
                reason: "Codex access token expired and no refresh token — re-run /connect"
                    .to_string(),
            });
        }
        Ok(ProviderStatus::Healthy)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            thinking: false,
            image_input: false,
            pdf_input: false,
            audio_input: false,
            video_input: false,
            caching: false,
            structured_output: false,
            system_prompt_style: SystemPromptStyle::SystemMessage,
        }
    }
}

#[cfg(test)]
mod refresh_error_tests {
    use super::*;

    fn provider() -> CodexProvider {
        CodexProvider::new(CodexTokens {
            access_token: "unused-fixture".into(),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn refresh_failures_do_not_retain_token_response_bodies() {
        let provider = provider();
        let cases = [
            (400, r#"{"error":"refresh-secret-canary"}"#),
            (200, r#"{"refresh_token":"refresh-secret-canary", invalid}"#),
            (200, r#"{"refresh_token":"refresh-secret-canary"}"#),
            (
                200,
                r#"{"access_token":"","refresh_token":"refresh-secret-canary"}"#,
            ),
        ];
        let errors: Vec<_> = cases
            .into_iter()
            .map(|(status, body)| {
                let error = match provider.decode_refresh_response(status, body) {
                    Ok(_) => panic!("invalid refresh fixture accepted"),
                    Err(error) => error,
                };
                (status, error)
            })
            .collect();
        for (expected_status, error) in errors {
            let rendered = format!("{error} {error:?} {error:#} {error:#?}");
            assert!(
                !rendered.contains("refresh-secret-canary"),
                "credential body escaped through error representation"
            );
            match error {
                ProviderError::Other { status, body, .. } => {
                    assert_eq!(status, Some(expected_status));
                    assert!(body.is_none());
                }
                _ => panic!("unexpected error category"),
            }
        }
    }

    #[test]
    fn refresh_success_preserves_optional_fields() {
        let provider = provider();
        let token = provider
            .decode_refresh_response(
                200,
                r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}"#,
            )
            .unwrap();
        assert_eq!(token.access_token, "new-access");
        assert_eq!(token.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(token.expires_in, Some(3600));
        let token = provider
            .decode_refresh_response(200, r#"{"access_token":"new-access"}"#)
            .unwrap();
        assert!(token.refresh_token.is_none());
        assert!(token.expires_in.is_none());
    }
}

#[cfg(test)]
mod byte_boundary_tests {
    use super::*;
    use futures::FutureExt;

    async fn events(chunks: Vec<bytes::Bytes>) -> Vec<String> {
        CodexProvider::parse_stream(
            futures::stream::iter(chunks.into_iter().map(Ok)),
            ProviderId::new("codex"),
        )
        .map(|event| format!("{event:?}"))
        .collect()
        .await
    }

    #[tokio::test]
    async fn codex_sse_keeps_lossy_decoding_and_reports_incomplete_stream() {
        let malformed = b"event: response.output_text.delta\ndata: {\"delta\":\"\xff\"}\n\n";
        let actual = events(
            malformed
                .iter()
                .map(|byte| bytes::Bytes::from(vec![*byte]))
                .collect(),
        )
        .await;
        assert!(actual
            .iter()
            .any(|event| event.contains("TextDelta") && event.contains('\u{fffd}')));
        assert!(actual
            .last()
            .unwrap()
            .contains("ended before response.completed"));
        for wire in [
            b"event: response.output_text.delta\ndata: {\"delta\":\"not dispatched\"}".as_slice(),
            b"event: response.output_text.delta\ndata: {\"delta\":\"not dispatched\"}\n",
        ] {
            let actual = events(vec![bytes::Bytes::copy_from_slice(wire)]).await;
            assert_eq!(actual.len(), 1);
            assert!(actual[0].contains("ended before response.completed"));
        }
    }

    #[tokio::test]
    async fn codex_sse_preserves_unicode_at_every_byte_split() {
        let text = "é中\u{10000}";
        let wire = format!(
            "event: response.output_text.delta\r\ndata: {}\r\n\r\nevent: response.function_call_arguments.delta\r\ndata: {}\r\n\r\nevent: response.completed\r\ndata: {}\r\n\r\n",
            json!({"output_index":0,"delta":text}),
            json!({"output_index":1,"delta":format!("{{\"name\":\"{text}\"}}")}),
            json!({"response":{"id":"fixture","model":"codex","usage":{"input_tokens":2,"output_tokens":3}}}),
        );
        let expected = events(vec![bytes::Bytes::copy_from_slice(wire.as_bytes())]).await;
        assert!(expected
            .iter()
            .any(|event| event.contains("TextDelta") && event.contains(text)));
        assert!(expected
            .iter()
            .any(|event| event.contains("InputJsonDelta") && event.contains(text)));
        assert_eq!(expected.last().unwrap(), "Ok(MessageStop)");
        for split in std::iter::once(wire.find('é').unwrap() + 1).chain(0..=wire.len()) {
            assert_eq!(
                events(vec![
                    bytes::Bytes::copy_from_slice(&wire.as_bytes()[..split]),
                    bytes::Bytes::copy_from_slice(&wire.as_bytes()[split..])
                ])
                .await,
                expected,
                "split at byte {split}"
            );
        }
        assert_eq!(
            events(
                wire.bytes()
                    .map(|byte| bytes::Bytes::from(vec![byte]))
                    .collect()
            )
            .await,
            expected
        );
    }

    #[tokio::test]
    async fn codex_sse_waits_for_real_blank_line_and_preserves_multiline_data() {
        let (input, stream) = futures::channel::mpsc::unbounded();
        let parsed = CodexProvider::parse_stream(stream, ProviderId::new("codex"));
        tokio::pin!(parsed);
        for chunk in [
            "event: response.output_text.delta\n",
            "data: {\"output_index\":0,\n",
            "data: \"delta\":\"kept\"}\n",
        ] {
            input
                .unbounded_send(Ok(bytes::Bytes::from_static(chunk.as_bytes())))
                .unwrap();
            assert!(
                parsed.next().now_or_never().is_none(),
                "frame dispatched before blank line"
            );
        }
        input
            .unbounded_send(Ok(bytes::Bytes::from_static(b"\n")))
            .unwrap();
        drop(input);
        let events: Vec<_> = parsed.map(|event| format!("{event:?}")).collect().await;
        assert!(events
            .iter()
            .any(|event| event.contains("TextDelta") && event.contains("kept")));
        assert!(events
            .last()
            .unwrap()
            .contains("ended before response.completed"));
    }
}

#[cfg(test)]
mod goal_tests {
    use super::*;
    use claurst_core::goal::{GoalStatus, GoalStore, GoalTurnPermission};

    #[test]
    fn goal_r1_codex_refresh_rechecks_before_model_send() {
        let name = "providers::codex::goal_tests::goal_r1_codex_refresh_rechecks_before_model_send";
        if std::env::var("COVEN_CODE_CODEX_GOAL_TEST").as_deref() != Ok(name) {
            let dir =
                std::env::temp_dir().join(format!("coven-codex-goal-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).unwrap();
            let home = dir.join("code");
            std::fs::create_dir(&home).unwrap();
            std::fs::write(home.join("auth.json"), r#"{"credentials":{}}"#).unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            for (key, _) in std::env::vars_os() {
                let name = key.to_string_lossy();
                if name.contains("API_KEY")
                    || name.starts_with("COVEN")
                        && (name.contains("TEST") || name.contains("FIXTURE"))
                {
                    command.env_remove(key);
                }
            }
            let mut child = command
                .args(["--exact", name, "--nocapture", "--test-threads=1"])
                .env("COVEN_CODE_CODEX_GOAL_TEST", name)
                .env("COVEN_CODE_HOME", &home)
                .env("COVEN_HOME", dir.join("coven"))
                .env("COVEN_CODE_DISABLE_MODELS_FETCH", "1")
                .env("COVEN_DAEMON_SOCKET", dir.join("absent.sock"))
                .env("COVEN_CODE_CLAUDE_BIN", dir.join("absent-claude"))
                .env("CLAUDE_CONFIG_DIR", dir.join("claude"))
                .stdout(std::fs::File::create(dir.join("out")).unwrap())
                .stderr(std::fs::File::create(dir.join("err")).unwrap())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if std::time::Instant::now() > deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("Codex fixture timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            let out = std::fs::read_to_string(dir.join("out")).unwrap();
            let err = std::fs::read_to_string(dir.join("err")).unwrap();
            assert!(
                status.success() && out.contains("1 passed; 0 failed"),
                "{out}\n{err}"
            );
            return;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            for (revoke, streaming) in [(false, true), (true, true), (false, false), (true, false)] {
                let dir = std::env::temp_dir().join(format!("coven-codex-goal-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).unwrap();
                let path = dir.join("goals.sqlite");
                let store = GoalStore::open(&path).unwrap();
                store.set_goal("s", "A", None).unwrap();
                let turn = store.capture_active_goal("s").unwrap().unwrap();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let base = format!("http://{}", listener.local_addr().unwrap());
                let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let seen = count.clone();
                let (stop, mut done) = tokio::sync::oneshot::channel::<()>();
                let server = tokio::spawn(async move {
                    loop {
                        let (mut socket, _) = tokio::select! { _ = &mut done => break, result = listener.accept() => result.unwrap() };
                        let mut bytes = Vec::new();
                        loop {
                            let mut buffer = [0u8; 4096];
                            let n = socket.read(&mut buffer).await.unwrap();
                            assert!(n > 0);
                            bytes.extend_from_slice(&buffer[..n]);
                            if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                                let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                                let len = headers.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|n| n.trim().parse::<usize>().unwrap())).unwrap();
                                if bytes.len() >= end + 4 + len { break; }
                            }
                        }
                        let refresh = bytes.starts_with(b"POST /refresh ");
                        let body = if refresh {
                            if revoke { GoalStore::open(&path).unwrap().set_status("s", GoalStatus::Paused).unwrap(); }
                            r#"{"access_token":"fake-refreshed","refresh_token":"fake-refresh","expires_in":3600}"#
                        } else { seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst); if streaming { "data: [DONE]\n\n" } else { r#"{"id":"local","model":"local-codex","output":[]}"# } };
                        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                    }
                });
                let mut provider = CodexProvider::new(CodexTokens { access_token: "fake-expired".into(), refresh_token: Some("fake-refresh".into()), expires_at: Some(1), account_id: None }).unwrap();
                provider.test_endpoints = Some((format!("{base}/refresh"), format!("{base}/responses")));
                let request = ProviderRequest { model: "local-codex".into(), messages: vec![], system_prompt: None, tools: vec![], max_tokens: 10, temperature: None, top_p: None, top_k: None, stop_sequences: vec![], thinking: None, provider_options: json!({}) };
                let result = if streaming {
                    provider.create_message_stream_for_goal(request, Some(&turn), GoalTurnPermission::Work).await.map(|_| ())
                } else {
                    provider.for_goal(&turn, GoalTurnPermission::Work).create_message(request).await.map(|_| ())
                };
                let _ = stop.send(()); server.await.unwrap();
                assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), usize::from(!revoke));
                assert_eq!(result.is_err(), revoke);
            }
        });
    }
}
