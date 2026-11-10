// WebFetch tool: HTTP GET with HTML-to-text conversion and LLM-powered semantic extraction
// for edge cases (JS-heavy pages, minimal content).

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::{debug, warn};

mod cache;
mod network;

const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 100_000;

pub struct WebFetchTool;

#[derive(Debug, Deserialize)]
struct WebFetchInput {
    url: String,
    #[serde(default)]
    #[allow(dead_code)]
    prompt: Option<String>,
}

/// Compute a simple hash of the URL for cache purposes.
fn url_hash(url: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Retrieval records describe this HTTP response, never a cached retrieval.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Extraction {
    PlainText,
    HtmlToText,
    ClaudeHaiku,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Retrieval {
    pub schema_version: u8,
    pub requested_url: String,
    pub final_url: String,
    pub url_credentials_redacted: bool,
    pub url_fragments_omitted: bool,
    pub http_status: u16,
    pub content_type: Option<String>,
    pub retrieved_at: String,
    pub body_sha256: String,
    pub body_bytes: usize,
    pub utf8_replacements: bool,
    pub extraction: Extraction,
    pub extraction_cached: bool,
    pub text_truncated: bool,
    pub original_text_bytes: usize,
}

const RETRIEVAL_MARKER: &str = "[coven.web_fetch.retrieval.v1]\n";

impl Retrieval {
    /// The dispatcher creates this separate record block; page text is never
    /// searched for a marker. Existing core/session content types carry it.
    pub fn with_body(
        &self,
        body: String,
    ) -> Result<claurst_core::types::ToolResultContent, serde_json::Error> {
        use claurst_core::types::{ContentBlock, ToolResultContent};
        let record = format!("{RETRIEVAL_MARKER}{}", serde_json::to_string(self)?);
        Ok(ToolResultContent::Blocks(vec![
            ContentBlock::Text { text: body },
            ContentBlock::Text { text: record },
        ]))
    }

    /// Read only the producer's two-block envelope. Old text-only results,
    /// page text containing a marker, and unknown schemas remain unknown.
    pub fn from_content(content: &claurst_core::types::ToolResultContent) -> Option<Self> {
        use claurst_core::types::{ContentBlock, ToolResultContent};
        let ToolResultContent::Blocks(blocks) = content else {
            return None;
        };
        let [ContentBlock::Text { .. }, ContentBlock::Text { text }] = blocks.as_slice() else {
            return None;
        };
        let record: Self = serde_json::from_str(text.strip_prefix(RETRIEVAL_MARKER)?).ok()?;
        record.is_supported().then_some(record)
    }

    pub fn is_supported(&self) -> bool {
        self.schema_version == 1
            && (200..300).contains(&self.http_status)
            && self.body_bytes <= MAX_BODY_BYTES
            && self.body_sha256.len() == 64
            && self
                .body_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            && chrono::DateTime::parse_from_rfc3339(&self.retrieved_at).is_ok()
            && [&self.requested_url, &self.final_url].iter().all(|raw| {
                reqwest::Url::parse(raw).is_ok_and(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.fragment().is_none()
                })
            })
    }
}

fn utf8_prefix(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn provenance_url(url: &reqwest::Url) -> Result<String, &'static str> {
    // An owned copy is required to remove authentication without altering the
    // real request target or its Basic-auth behavior.
    let mut redacted = url.clone();
    if redacted.set_username("").is_err() || redacted.set_password(None).is_err() {
        return Err("invalid HTTP provenance URL");
    }
    redacted.set_fragment(None);
    Ok(redacted.into())
}

fn request_error(error: &reqwest::Error) -> String {
    use std::error::Error;
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(policy) = cause.downcast_ref::<network::DestinationError>() {
            return format!("WebFetch: {policy}");
        }
        source = cause.source();
    }
    let category = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connection"
    } else if error.is_redirect() {
        "redirect"
    } else if error.is_body() {
        "response body"
    } else if error.is_decode() {
        "response decoding"
    } else if error.is_builder() {
        "request construction"
    } else {
        "request"
    };
    format!("WebFetch {category} failed")
}

/// Detect if HTML is likely a JS-heavy page with minimal semantic content.
fn is_edge_case_html(html: &str, extracted_text: &str) -> bool {
    // Check word count (rough estimate)
    let word_count = extracted_text.split_whitespace().count();
    if word_count < 100 {
        debug!(word_count, "Edge case: low word count");
        return true;
    }

    // Check for semantic HTML tags
    let lower = html.to_ascii_lowercase();
    let has_semantic =
        lower.contains("<article") || lower.contains("<main") || lower.contains("<body");

    if !has_semantic {
        debug!("Edge case: no semantic HTML tags");
        return true;
    }

    false
}

/// Call Claude Haiku to extract main content from HTML.
async fn semantic_extraction(html: &str, ctx: &ToolContext) -> Option<String> {
    // Try to create an Anthropic client from the config
    let client = match claurst_api::AnthropicClient::from_config(&ctx.config) {
        Ok(c) => c,
        Err(_) => {
            warn!("Failed to create Anthropic client for semantic extraction");
            return None;
        }
    };

    // Truncate HTML to avoid exceeding token limits
    let html_excerpt = if html.len() > 20000 {
        format!("{}...", utf8_prefix(html, 20000))
    } else {
        html.to_string()
    };

    let system = "You are a content extraction expert. Given HTML, extract and return only the main text content. Return just plain text, no markdown or formatting.";
    let user_message = format!(
        "Extract the main content from this HTML and return only the text:\n\n{}",
        html_excerpt
    );

    // Use the builder API to construct the request
    let api_messages = vec![claurst_api::ApiMessage {
        role: "user".to_string(),
        content: serde_json::Value::String(user_message),
    }];

    let request = claurst_api::CreateMessageRequest::builder("claude-haiku-4-5", 2000)
        .messages(api_messages)
        .system(claurst_api::SystemPrompt::Text(system.to_string()))
        .build();

    match client.create_message(request).await {
        Ok(response) => {
            // Extract text from the response content (Vec<Value>)
            // Response content is JSON objects like {"type": "text", "text": "..."}
            let text = response.content.iter().find_map(|block| {
                if block.get("type")?.as_str()? == "text" {
                    block.get("text")?.as_str().map(str::to_owned)
                } else {
                    None
                }
            });

            if let Some(extracted) = text {
                debug!(
                    extracted_len = extracted.len(),
                    "Semantic extraction successful"
                );
                return Some(extracted);
            }

            warn!("No text block in semantic extraction response");
            None
        }
        Err(_) => {
            warn!("Semantic extraction API call failed");
            None
        }
    }
}

/// Naively strip HTML tags and decode common entities.
fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;

    let lower = html.to_ascii_lowercase();
    let chars: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if !in_tag && chars[i] == '<' {
            in_tag = true;
            // Check for script/style open/close
            let rest: String = lower_chars[i..].iter().take(20).collect();
            if rest.starts_with("<script") {
                in_script = true;
            } else if rest.starts_with("</script") {
                in_script = false;
            } else if rest.starts_with("<style") {
                in_style = true;
            } else if rest.starts_with("</style") {
                in_style = false;
            }
            // Block tags => newline
            let block_tags = [
                "<br", "<p ", "<p>", "</p>", "<div", "</div>", "<h1", "<h2", "<h3", "<h4", "<h5",
                "<h6", "</h1", "</h2", "</h3", "</h4", "</h5", "</h6", "<li", "</li", "<tr",
                "</tr", "<hr",
            ];
            for tag in &block_tags {
                if rest.starts_with(tag) {
                    result.push('\n');
                    break;
                }
            }
            i += 1;
            continue;
        }

        if in_tag {
            if chars[i] == '>' {
                in_tag = false;
            }
            i += 1;
            continue;
        }

        if in_script || in_style {
            i += 1;
            continue;
        }

        // Decode basic entities
        if chars[i] == '&' {
            let rest: String = chars[i..].iter().take(10).collect();
            if rest.starts_with("&amp;") {
                result.push('&');
                i += 5;
            } else if rest.starts_with("&lt;") {
                result.push('<');
                i += 4;
            } else if rest.starts_with("&gt;") {
                result.push('>');
                i += 4;
            } else if rest.starts_with("&quot;") {
                result.push('"');
                i += 6;
            } else if rest.starts_with("&#39;") || rest.starts_with("&apos;") {
                result.push('\'');
                i += if rest.starts_with("&#39;") { 5 } else { 6 };
            } else if rest.starts_with("&nbsp;") {
                result.push(' ');
                i += 6;
            } else {
                result.push('&');
                i += 1;
            }
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }

    // Collapse multiple blank lines
    let mut collapsed = String::new();
    let mut blank_count = 0;
    for line in result.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                collapsed.push('\n');
            }
        } else {
            blank_count = 0;
            collapsed.push_str(trimmed);
            collapsed.push('\n');
        }
    }

    collapsed.trim().to_string()
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_WEB_FETCH
    }

    fn description(&self) -> &str {
        "Fetches a web page URL and returns its content as text. HTML is \
         automatically converted to plain text. Use this for reading documentation, \
         APIs, and other web resources."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Network
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "prompt": {
                    "type": "string",
                    "description": "Optional prompt for how to process the content"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: WebFetchInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        // Permission check
        if let Err(e) = ctx.check_permission(
            self.name(),
            &format!("Fetch {}", params.url),
            false, // network authority is required even though this is a GET
        ) {
            return ToolResult::error(e.to_string());
        }

        debug!("Fetching web page");

        let url = match network::parse_url(&params.url) {
            Ok(url) => url,
            Err(error) => return ToolResult::error(format!("WebFetch: {error}")),
        };
        let client = network::client_builder().build();

        let client = match client {
            Ok(c) => c,
            Err(_) => return ToolResult::error("Failed to create WebFetch HTTP client"),
        };

        let cache_dir = claurst_core::config::config_home().join("web_cache");
        fetch_page(&params, ctx, &url, &client, &cache_dir).await
    }
}

// The production caller always supplies the restricted client above. Tests in
// this module can bind a public fixture name to a local HTTP server without
// exposing a private-network option in configuration or the tool API.
async fn fetch_page(
    params: &WebFetchInput,
    ctx: &ToolContext,
    url: &reqwest::Url,
    client: &reqwest::Client,
    cache_dir: &Path,
) -> ToolResult {
    let mut resp = match client
        .get(url.as_str())
        .header("User-Agent", "Coven/1.0")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => return ToolResult::error(request_error(&error)),
    };
    let status = resp.status();
    if !status.is_success() {
        return ToolResult::error(format!("HTTP {status} when fetching public web content"));
    }
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if resp
        .content_length()
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        return ToolResult::error("WebFetch response body exceeds the 2 MiB limit");
    }
    let requested_url = match provenance_url(url) {
        Ok(url) => url,
        Err(error) => return ToolResult::error(error),
    };
    let final_url = match provenance_url(resp.url()) {
        Ok(url) => url,
        Err(error) => return ToolResult::error(error),
    };
    let credentials_redacted = !url.username().is_empty()
        || url.password().is_some()
        || !resp.url().username().is_empty()
        || resp.url().password().is_some();
    let fragments_omitted = url.fragment().is_some() || resp.url().fragment().is_some();
    let mut bytes = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if chunk.len() > MAX_BODY_BYTES.saturating_sub(bytes.len()) {
                    return ToolResult::error("WebFetch response body exceeds the 2 MiB limit");
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => return ToolResult::error(request_error(&error)),
        }
    }
    let retrieved_at = chrono::Utc::now().to_rfc3339();
    let body_sha256 = format!("{:x}", Sha256::digest(&bytes));
    let body = String::from_utf8_lossy(&bytes);
    let utf8_replacements = matches!(&body, std::borrow::Cow::Owned(_));
    let mime = content_type.as_deref().unwrap_or("");
    let cached = cache::load(cache_dir, &params.url, &body_sha256, mime);
    let extraction_cached = cached.is_some();
    let extracted = match cached {
        Some(extracted) => extracted,
        None => {
            let is_html = mime.split(';').next().is_some_and(|mime| {
                mime.trim().eq_ignore_ascii_case("text/html")
                    || mime.trim().eq_ignore_ascii_case("application/xhtml+xml")
            });
            let (mut text, method) = if is_html {
                let stripped = strip_html(&body);
                if is_edge_case_html(&body, &stripped) {
                    match semantic_extraction(&body, ctx).await {
                        Some(semantic) => (semantic, Extraction::ClaudeHaiku),
                        None => (stripped, Extraction::HtmlToText),
                    }
                } else {
                    (stripped, Extraction::HtmlToText)
                }
            } else {
                (body.into_owned(), Extraction::PlainText)
            };
            let original_text_bytes = text.len();
            let text_truncated = original_text_bytes > MAX_OUTPUT_BYTES;
            if text_truncated {
                text.truncate(utf8_prefix(&text, MAX_OUTPUT_BYTES).len());
            }
            let extracted = cache::Extracted {
                text,
                method,
                original_text_bytes,
                text_truncated,
            };
            if cache::save(cache_dir, &params.url, &body_sha256, mime, &extracted).is_err() {
                warn!("Could not persist WebFetch extraction cache");
            }
            extracted
        }
    };
    let retrieval = Retrieval {
        schema_version: 1,
        requested_url,
        final_url,
        url_credentials_redacted: credentials_redacted,
        url_fragments_omitted: fragments_omitted,
        http_status: status.as_u16(),
        content_type,
        retrieved_at,
        body_sha256,
        body_bytes: bytes.len(),
        utf8_replacements,
        extraction: extracted.method,
        extraction_cached,
        text_truncated: extracted.text_truncated,
        original_text_bytes: extracted.original_text_bytes,
    };
    let mut text = extracted.text;
    if retrieval.text_truncated {
        text.push_str(&format!(
            "\n\n... (truncated, {} total UTF-8 bytes)",
            retrieval.original_text_bytes
        ));
    }
    if retrieval.extraction == Extraction::ClaudeHaiku {
        text.insert_str(
            0,
            "[Model-extracted content: Claude Haiku; not verbatim page text]\n\n",
        );
    }
    match serde_json::to_value(&retrieval) {
        Ok(metadata) => ToolResult::success(text).with_metadata(metadata),
        Err(_) => ToolResult::error("Could not serialize WebFetch retrieval provenance"),
    }
}

#[cfg(test)]
mod retrieval_tests;

#[cfg(test)]
mod provenance_tests;
