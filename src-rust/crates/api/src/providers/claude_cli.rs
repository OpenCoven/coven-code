// providers/claude_cli.rs — Claude provider backed by the local `claude` CLI.
//
// Subscription access to Claude goes through the Claude Code binary
// (`claude -p --input-format stream-json --output-format stream-json`), the
// same adapter contract coven and cave use to drive harness CLIs. Coven Code
// never reads or imports Claude Code's OAuth credentials: tokens minted for
// the Claude Code client get rate limited when replayed by third-party
// clients, so auth stays inside the CLI that owns it.
//
// Delegation model: the CLI runs its own agent loop (tools, permissions,
// context) in the current working directory. This provider forwards the
// user's prompt (with any pasted images as standard Anthropic image blocks),
// streams Claude's text back, and renders the CLI's tool activity as
// one-line notices. It never emits `ToolUse` blocks, so the
// query loop treats every turn as self-contained (`end_turn`).
//
// Session continuity: the CLI's `session_id` (from the `init` envelope) is
// cached per process and replayed via `--resume` on follow-up turns. A fresh
// conversation (no assistant turns in the request) resets the cache. When no
// session can be resumed (e.g. Coven Code restarted mid-conversation), the
// prior transcript is flattened into the prompt instead.

use parking_lot::Mutex;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use async_stream::stream;
use async_trait::async_trait;
use claurst_core::provider_id::ProviderId;
use claurst_core::types::{ContentBlock, Message, MessageContent, Role, UsageInfo};
use futures::Stream;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tracing::{debug, warn};

use crate::provider::{LlmProvider, ModelInfo};
use crate::provider_error::ProviderError;
use crate::provider_types::{
    ProviderCapabilities, ProviderRequest, ProviderResponse, ProviderStatus, StopReason,
    StreamEvent, SystemPromptStyle,
};

/// Env var overriding the `claude` binary location.
pub const CLAUDE_BIN_ENV: &str = "COVEN_CODE_CLAUDE_BIN";

const INSTALL_HINT: &str = "claude CLI not found — install with `npm install -g @anthropic-ai/claude-code`, sign in by running `claude`, and ensure it is on PATH (or set COVEN_CODE_CLAUDE_BIN)";

/// The `claude` session id carried across turns within this process.
static CLI_SESSION: Mutex<Option<String>> = Mutex::new(None);

/// Locate the `claude` binary: explicit env override first, then PATH.
pub fn resolve_claude_binary() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var(CLAUDE_BIN_ENV) {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    let path = std::env::var_os("PATH")?;
    let names: &[&str] = if cfg!(windows) {
        &["claude.cmd", "claude.exe", "claude"]
    } else {
        &["claude"]
    };
    for dir in std::env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Prompt assembly
// ---------------------------------------------------------------------------

/// Plain-text rendering of a message's content (text blocks only).
fn message_text(message: &Message) -> String {
    match &message.content {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The latest user message's text — the prompt for a resumed session.
fn last_user_text(request: &ProviderRequest) -> String {
    request
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role, Role::User))
        .map(message_text)
        .unwrap_or_default()
}

/// Image blocks from the latest user message, forwarded to the CLI alongside
/// the text prompt. Images from earlier turns are not re-sent: on a resumed
/// session the CLI already has them, and on a flattened restart they are gone
/// with the rest of the non-text content.
fn last_user_images(request: &ProviderRequest) -> Vec<ContentBlock> {
    request
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role, Role::User))
        .map(|message| match &message.content {
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter(|block| matches!(block, ContentBlock::Image { .. }))
                .cloned()
                .collect(),
            MessageContent::Text(_) => Vec::new(),
        })
        .unwrap_or_default()
}

/// Flatten the whole transcript into one prompt for sessions that cannot be
/// resumed (the CLI has no matching session to `--resume`).
fn flattened_transcript(request: &ProviderRequest) -> String {
    let mut earlier: Vec<String> = Vec::new();
    let mut last_user = String::new();
    for message in &request.messages {
        let text = message_text(message);
        if text.is_empty() {
            continue;
        }
        match message.role {
            Role::User => {
                if !last_user.is_empty() {
                    earlier.push(format!("user: {}", last_user));
                }
                last_user = text;
            }
            Role::Assistant => {
                if !last_user.is_empty() {
                    earlier.push(format!("user: {}", last_user));
                    last_user = String::new();
                }
                earlier.push(format!("assistant: {}", text));
            }
        }
    }
    if earlier.is_empty() {
        return last_user;
    }
    format!(
        "<conversation-so-far>\nThe session was restarted; this is the conversation before the current message.\n{}\n</conversation-so-far>\n\n{}",
        earlier.join("\n"),
        last_user
    )
}

// ---------------------------------------------------------------------------
// CLI invocation
// ---------------------------------------------------------------------------

/// Argv after the binary, mirroring the coven-runtimes adapter manifest
/// contract for Claude Code.
fn build_args(model: &str, resume: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
    ];
    if !model.is_empty() {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    if let Some(session_id) = resume {
        args.push("--resume".to_string());
        args.push(session_id.to_string());
    }
    args
}

/// The single stream-json stdin line carrying the user prompt. Image blocks
/// ride along in the same Anthropic wire shape the API takes, placed before
/// the text to match the direct `anthropic.rs` path.
fn stdin_line(prompt: &str, images: &[ContentBlock]) -> String {
    let mut content: Vec<Value> = images
        .iter()
        .filter_map(|block| serde_json::to_value(block).ok())
        .collect();
    if !prompt.is_empty() || content.is_empty() {
        content.push(json!({"type": "text", "text": prompt}));
    }
    let envelope = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": content,
        },
    });
    format!("{}\n", envelope)
}

/// One-line notice describing a tool call the CLI made, shown in place of the
/// tool_use block (which this provider intentionally does not forward).
fn tool_notice(name: &str, input: &Value) -> String {
    const ARG_KEYS: &[&str] = &[
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ];
    let arg = ARG_KEYS
        .iter()
        .find_map(|key| input.get(key).and_then(|value| value.as_str()))
        .map(|value| {
            let first_line = value.lines().next().unwrap_or("");
            let mut short: String = first_line.chars().take(80).collect();
            if first_line.chars().count() > 80 {
                short.push('…');
            }
            short
        })
        .filter(|value| !value.is_empty());
    match arg {
        Some(arg) => format!("⚒ {}({})", name, arg),
        None => format!("⚒ {}", name),
    }
}

// ---------------------------------------------------------------------------
// Envelope mapping
// ---------------------------------------------------------------------------

/// Maps `claude --output-format stream-json` stdout lines onto provider
/// [`StreamEvent`]s. Pure state machine — the process plumbing lives in the
/// stream generator so this part stays unit-testable.
struct EnvelopeMapper {
    fallback_model: String,
    message_started: bool,
    block_index: usize,
    session_id: Option<String>,
}

/// What a single stdout line produced.
struct LineOutcome {
    events: Vec<StreamEvent>,
    /// The `result` envelope arrived — the stream is complete.
    done: bool,
}

impl EnvelopeMapper {
    fn new(fallback_model: &str) -> Self {
        Self {
            fallback_model: fallback_model.to_string(),
            message_started: false,
            block_index: 0,
            session_id: None,
        }
    }

    fn start_message_if_needed(&mut self, events: &mut Vec<StreamEvent>, model: Option<&str>) {
        if self.message_started {
            return;
        }
        events.push(StreamEvent::MessageStart {
            id: self
                .session_id
                .clone()
                .unwrap_or_else(|| "claude-cli".to_string()),
            model: model.unwrap_or(&self.fallback_model).to_string(),
            usage: UsageInfo::default(),
        });
        self.message_started = true;
    }

    fn push_text_block(&mut self, events: &mut Vec<StreamEvent>, text: &str) {
        if text.is_empty() {
            return;
        }
        events.push(StreamEvent::ContentBlockStart {
            index: self.block_index,
            content_block: ContentBlock::Text {
                text: String::new(),
            },
        });
        events.push(StreamEvent::TextDelta {
            index: self.block_index,
            text: text.to_string(),
        });
        events.push(StreamEvent::ContentBlockStop {
            index: self.block_index,
        });
        self.block_index += 1;
    }

    fn map_line(&mut self, line: &str) -> LineOutcome {
        let mut events = Vec::new();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return LineOutcome {
                events,
                done: false,
            };
        }
        let Ok(envelope) = serde_json::from_str::<Value>(trimmed) else {
            debug!(line = %trimmed, "claude CLI emitted a non-JSON line; ignoring");
            return LineOutcome {
                events,
                done: false,
            };
        };

        match envelope.get("type").and_then(|value| value.as_str()) {
            Some("system") => {
                if let Some(session_id) = envelope.get("session_id").and_then(|v| v.as_str()) {
                    self.session_id = Some(session_id.to_string());
                }
                let model = envelope.get("model").and_then(|v| v.as_str());
                self.start_message_if_needed(&mut events, model);
            }
            Some("assistant") => {
                let message = envelope.get("message");
                let model = message
                    .and_then(|m| m.get("model"))
                    .and_then(|v| v.as_str());
                self.start_message_if_needed(&mut events, model);
                let blocks = message
                    .and_then(|m| m.get("content"))
                    .and_then(|v| v.as_array());
                if let Some(blocks) = blocks {
                    for block in blocks {
                        match block.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                self.push_text_block(&mut events, text);
                            }
                            Some("thinking") => {
                                let thinking =
                                    block.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
                                if !thinking.is_empty() {
                                    events.push(StreamEvent::ContentBlockStart {
                                        index: self.block_index,
                                        content_block: ContentBlock::Thinking {
                                            thinking: String::new(),
                                            signature: String::new(),
                                        },
                                    });
                                    events.push(StreamEvent::ThinkingDelta {
                                        index: self.block_index,
                                        thinking: thinking.to_string(),
                                    });
                                    events.push(StreamEvent::ContentBlockStop {
                                        index: self.block_index,
                                    });
                                    self.block_index += 1;
                                }
                            }
                            Some("tool_use") => {
                                let name =
                                    block.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                                let default_input = json!({});
                                let input = block.get("input").unwrap_or(&default_input);
                                let notice = tool_notice(name, input);
                                self.push_text_block(&mut events, &notice);
                            }
                            _ => {}
                        }
                    }
                }
            }
            // The CLI's own tool results — internal to its agent loop.
            Some("user") => {}
            Some("result") => {
                self.start_message_if_needed(&mut events, None);
                let is_error = envelope
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if is_error {
                    let detail = envelope
                        .get("result")
                        .and_then(|v| v.as_str())
                        .unwrap_or("claude CLI reported an error");
                    events.push(StreamEvent::Error {
                        error_type: envelope
                            .get("subtype")
                            .and_then(|v| v.as_str())
                            .unwrap_or("error")
                            .to_string(),
                        message: detail.to_string(),
                    });
                }
                let usage_json = envelope.get("usage");
                let read_count = |key: &str| {
                    usage_json
                        .and_then(|u| u.get(key))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                };
                events.push(StreamEvent::MessageDelta {
                    stop_reason: Some(StopReason::EndTurn),
                    usage: Some(UsageInfo {
                        input_tokens: read_count("input_tokens"),
                        output_tokens: read_count("output_tokens"),
                        cache_creation_input_tokens: read_count("cache_creation_input_tokens"),
                        cache_read_input_tokens: read_count("cache_read_input_tokens"),
                    }),
                });
                events.push(StreamEvent::MessageStop);
                return LineOutcome { events, done: true };
            }
            _ => {}
        }

        LineOutcome {
            events,
            done: false,
        }
    }
}

// ---------------------------------------------------------------------------
// ClaudeCliProvider
// ---------------------------------------------------------------------------

/// A spawned CLI turn: the child process, its buffered stdout, the first
/// stdout line (already consumed during the init handshake), and the shared
/// stderr accumulator.
struct CliTurn {
    child: Child,
    stdout: BufReader<ChildStdout>,
    first_line: String,
    stderr: Arc<Mutex<String>>,
}

/// Result of launching one CLI turn. `CliTurn` is boxed to keep the variant
/// sizes comparable (clippy: large_enum_variant).
enum SpawnOutcome {
    Turn(Box<CliTurn>),
    /// The process exited before emitting any stdout — a stale `--resume`
    /// session or a startup failure; stderr explains which.
    ExitedEarly {
        stderr: Arc<Mutex<String>>,
    },
}

/// Give the background stderr drain a beat to finish after the child exits,
/// then take whatever it collected.
async fn collect_stderr(stderr: &Arc<Mutex<String>>) -> String {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    stderr.lock().trim().to_string()
}

pub struct ClaudeCliProvider {
    id: ProviderId,
    /// Resolved lazily so construction never fails; a missing binary
    /// surfaces the install hint when a turn is actually attempted.
    binary: Option<PathBuf>,
}

impl Default for ClaudeCliProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeCliProvider {
    pub fn new() -> Self {
        Self {
            id: ProviderId::new(ProviderId::ANTHROPIC),
            binary: resolve_claude_binary(),
        }
    }

    fn binary_or_default(&self) -> PathBuf {
        self.binary
            .clone()
            .or_else(resolve_claude_binary)
            .unwrap_or_else(|| PathBuf::from("claude"))
    }

    fn spawn_error(&self, error: std::io::Error) -> ProviderError {
        let message = if error.kind() == std::io::ErrorKind::NotFound {
            INSTALL_HINT.to_string()
        } else {
            format!("failed to launch claude CLI: {}", error)
        };
        ProviderError::Other {
            provider: self.id.clone(),
            message,
            status: None,
            body: None,
        }
    }

    /// Spawn one CLI turn, write the prompt, and wait for the first stdout
    /// line to confirm the process came up.
    async fn spawn_turn(
        &self,
        model: &str,
        prompt: &str,
        images: &[ContentBlock],
        resume: Option<&str>,
    ) -> Result<SpawnOutcome, ProviderError> {
        let mut command = Command::new(self.binary_or_default());
        command
            .args(build_args(model, resume))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| self.spawn_error(e))?;

        let mut stdin = child.stdin.take().ok_or_else(|| ProviderError::Other {
            provider: self.id.clone(),
            message: "claude CLI stdin unavailable".to_string(),
            status: None,
            body: None,
        })?;
        let payload = stdin_line(prompt, images);
        if let Err(e) = stdin.write_all(payload.as_bytes()).await {
            warn!(error = %e, "failed writing prompt to claude CLI stdin");
        }
        drop(stdin); // EOF tells the CLI no more input is coming.

        let stderr_buf = Arc::new(Mutex::new(String::new()));
        if let Some(mut stderr) = child.stderr.take() {
            let sink = Arc::clone(&stderr_buf);
            tokio::spawn(async move {
                let mut collected = String::new();
                if stderr.read_to_string(&mut collected).await.is_ok() {
                    *sink.lock() = collected;
                }
            });
        }

        let stdout = child.stdout.take().ok_or_else(|| ProviderError::Other {
            provider: self.id.clone(),
            message: "claude CLI stdout unavailable".to_string(),
            status: None,
            body: None,
        })?;
        let mut reader = BufReader::new(stdout);
        let mut first_line = String::new();
        let read = reader
            .read_line(&mut first_line)
            .await
            .map_err(|e| ProviderError::Other {
                provider: self.id.clone(),
                message: format!("failed reading claude CLI output: {}", e),
                status: None,
                body: None,
            })?;
        if read == 0 {
            // Exited without output. Reap the child so it doesn't linger.
            let status = child.wait().await.ok();
            debug!(
                ?status,
                resume = resume.is_some(),
                "claude CLI exited before emitting output"
            );
            return Ok(SpawnOutcome::ExitedEarly { stderr: stderr_buf });
        }

        Ok(SpawnOutcome::Turn(Box::new(CliTurn {
            child,
            stdout: reader,
            first_line,
            stderr: stderr_buf,
        })))
    }

    /// Spawn with resume + stale-session fallback, per the request's shape.
    async fn start_turn(&self, request: &ProviderRequest) -> Result<CliTurn, ProviderError> {
        let has_assistant_turns = request
            .messages
            .iter()
            .any(|message| matches!(message.role, Role::Assistant));
        let resume_session = if has_assistant_turns {
            CLI_SESSION.lock().clone()
        } else {
            // Fresh conversation (e.g. /clear): drop any cached session.
            *CLI_SESSION.lock() = None;
            None
        };

        let images = last_user_images(request);
        if let Some(session_id) = resume_session {
            match self
                .spawn_turn(
                    &request.model,
                    &last_user_text(request),
                    &images,
                    Some(&session_id),
                )
                .await?
            {
                SpawnOutcome::Turn(turn) => return Ok(*turn),
                SpawnOutcome::ExitedEarly { .. } => {
                    // Stale session — start over with the transcript inlined.
                    warn!(session_id = %session_id, "claude CLI session could not be resumed; starting fresh");
                    *CLI_SESSION.lock() = None;
                }
            }
        }

        let prompt = if has_assistant_turns {
            flattened_transcript(request)
        } else {
            last_user_text(request)
        };
        match self
            .spawn_turn(&request.model, &prompt, &images, None)
            .await?
        {
            SpawnOutcome::Turn(turn) => Ok(*turn),
            SpawnOutcome::ExitedEarly { stderr } => {
                let detail = collect_stderr(&stderr).await;
                Err(ProviderError::Other {
                    provider: self.id.clone(),
                    message: format!(
                        "claude CLI exited without producing output{}",
                        if detail.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", detail)
                        }
                    ),
                    status: None,
                    body: None,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn name(&self) -> &str {
        "Claude CLI"
    }

    async fn create_message(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        use futures::StreamExt;
        let mut stream = self.create_message_stream(request).await?;

        let mut id = String::from("claude-cli");
        let mut model = String::new();
        let mut texts: Vec<String> = Vec::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut usage = UsageInfo::default();

        while let Some(result) = stream.next().await {
            match result? {
                StreamEvent::MessageStart {
                    id: message_id,
                    model: message_model,
                    ..
                } => {
                    id = message_id;
                    model = message_model;
                }
                StreamEvent::TextDelta { text, .. } => texts.push(text),
                StreamEvent::MessageDelta {
                    stop_reason: reason,
                    usage: delta_usage,
                } => {
                    if let Some(reason) = reason {
                        stop_reason = reason;
                    }
                    if let Some(delta_usage) = delta_usage {
                        usage = delta_usage;
                    }
                }
                StreamEvent::Error {
                    error_type,
                    message,
                } => {
                    return Err(ProviderError::StreamError {
                        provider: self.id.clone(),
                        message: format!("[{}] {}", error_type, message),
                        partial_response: None,
                    });
                }
                StreamEvent::MessageStop => break,
                _ => {}
            }
        }

        Ok(ProviderResponse {
            id,
            content: vec![ContentBlock::Text {
                text: texts.join("\n"),
            }],
            stop_reason,
            usage,
            model,
        })
    }

    async fn create_message_stream(
        &self,
        request: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        let mut turn = self.start_turn(&request).await?;
        let provider_id = self.id.clone();
        let fallback_model = request.model.clone();

        let s = stream! {
            let mut mapper = EnvelopeMapper::new(&fallback_model);
            let mut line = std::mem::take(&mut turn.first_line);
            loop {
                let outcome = mapper.map_line(&line);
                if let Some(session_id) = mapper.session_id.as_deref() {
                    let mut cached = CLI_SESSION.lock();
                    if cached.as_deref() != Some(session_id) {
                        *cached = Some(session_id.to_string());
                    }
                }
                for event in outcome.events {
                    yield Ok(event);
                }
                if outcome.done {
                    // Let the child exit cleanly; kill_on_drop covers hangs.
                    let _ = turn.child.wait().await;
                    return;
                }

                line.clear();
                match turn.stdout.read_line(&mut line).await {
                    Ok(0) => {
                        let stderr = collect_stderr(&turn.stderr).await;
                        let detail = if stderr.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", stderr)
                        };
                        yield Err(ProviderError::StreamError {
                            provider: provider_id.clone(),
                            message: format!("claude CLI exited before emitting a result{}", detail),
                            partial_response: None,
                        });
                        return;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        yield Err(ProviderError::StreamError {
                            provider: provider_id.clone(),
                            message: format!("failed reading claude CLI output: {}", e),
                            partial_response: None,
                        });
                        return;
                    }
                }
            }
        };

        Ok(Box::pin(s))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(super::anthropic::claude_model_catalog())
    }

    async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
        if self.binary.is_some() || resolve_claude_binary().is_some() {
            Ok(ProviderStatus::Healthy)
        } else {
            Ok(ProviderStatus::Unavailable {
                reason: INSTALL_HINT.to_string(),
            })
        }
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            // The CLI runs its own tools; Coven Code's tool loop stays out.
            tool_calling: false,
            thinking: false,
            // Pasted images are forwarded on stdin as standard Anthropic
            // image blocks; the CLI accepts them like the API does.
            image_input: true,
            pdf_input: false,
            audio_input: false,
            video_input: false,
            caching: false,
            structured_output: false,
            system_prompt_style: SystemPromptStyle::TopLevel,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with(messages: Vec<Message>) -> ProviderRequest {
        ProviderRequest {
            model: "claude-opus-4-8".to_string(),
            messages,
            system_prompt: None,
            tools: Vec::new(),
            max_tokens: 4096,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: Vec::new(),
            thinking: None,
            provider_options: json!({}),
        }
    }

    #[test]
    fn build_args_matches_adapter_manifest_contract() {
        let args = build_args("claude-opus-4-8", None);
        assert_eq!(
            args,
            vec![
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "claude-opus-4-8",
            ]
        );

        let resumed = build_args("claude-opus-4-8", Some("sess-1"));
        assert!(resumed.ends_with(&["--resume".to_string(), "sess-1".to_string()]));
    }

    #[test]
    fn stdin_line_is_a_stream_json_user_envelope() {
        let line = stdin_line("hello", &[]);
        let parsed: Value = serde_json::from_str(line.trim()).expect("valid JSON");
        assert_eq!(parsed["type"], "user");
        assert_eq!(parsed["message"]["content"][0]["text"], "hello");
        assert!(line.ends_with('\n'));
    }

    fn png_block(data: &str) -> ContentBlock {
        ContentBlock::Image {
            source: claurst_core::types::ImageSource {
                source_type: "base64".to_string(),
                media_type: Some("image/png".to_string()),
                data: Some(data.to_string()),
                url: None,
            },
        }
    }

    #[test]
    fn stdin_line_places_image_blocks_before_the_text() {
        let line = stdin_line("what is this?", &[png_block("aGk=")]);
        let parsed: Value = serde_json::from_str(line.trim()).expect("valid JSON");
        let content = parsed["message"]["content"]
            .as_array()
            .expect("content array");
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["type"], "base64");
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        assert_eq!(content[0]["source"]["data"], "aGk=");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "what is this?");
    }

    #[test]
    fn stdin_line_omits_the_text_block_for_image_only_prompts() {
        let line = stdin_line("", &[png_block("aGk=")]);
        let parsed: Value = serde_json::from_str(line.trim()).expect("valid JSON");
        let content = parsed["message"]["content"]
            .as_array()
            .expect("content array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "image");
    }

    #[test]
    fn last_user_images_reads_only_the_latest_user_message() {
        let request = request_with(vec![
            Message::user_blocks(vec![
                png_block("b2xk"),
                ContentBlock::Text {
                    text: "earlier".to_string(),
                },
            ]),
            Message {
                role: Role::Assistant,
                content: MessageContent::Text("reply".to_string()),
                uuid: None,
                cost: None,
                snapshot_patch: None,
            },
            Message::user_blocks(vec![
                png_block("bmV3"),
                ContentBlock::Text {
                    text: "latest".to_string(),
                },
            ]),
        ]);
        let images = last_user_images(&request);
        assert_eq!(images.len(), 1);
        assert!(matches!(
            &images[0],
            ContentBlock::Image { source } if source.data.as_deref() == Some("bmV3")
        ));

        let text_only = request_with(vec![Message::user("no images")]);
        assert!(last_user_images(&text_only).is_empty());
    }

    #[test]
    fn last_user_text_reads_text_and_blocks() {
        let request = request_with(vec![
            Message::user("first"),
            Message {
                role: Role::Assistant,
                content: MessageContent::Text("reply".to_string()),
                uuid: None,
                cost: None,
                snapshot_patch: None,
            },
            Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Text {
                        text: "second".to_string(),
                    },
                    ContentBlock::ToolResult {
                        tool_use_id: "x".to_string(),
                        content: claurst_core::types::ToolResultContent::Text(
                            "ignored".to_string(),
                        ),
                        is_error: None,
                    },
                ]),
                uuid: None,
                cost: None,
                snapshot_patch: None,
            },
        ]);
        assert_eq!(last_user_text(&request), "second");
    }

    #[test]
    fn flattened_transcript_inlines_prior_turns() {
        let request = request_with(vec![
            Message::user("first question"),
            Message {
                role: Role::Assistant,
                content: MessageContent::Text("first answer".to_string()),
                uuid: None,
                cost: None,
                snapshot_patch: None,
            },
            Message::user("follow-up"),
        ]);
        let prompt = flattened_transcript(&request);
        assert!(prompt.contains("<conversation-so-far>"));
        assert!(prompt.contains("user: first question"));
        assert!(prompt.contains("assistant: first answer"));
        assert!(prompt.trim_end().ends_with("follow-up"));
    }

    #[test]
    fn flattened_transcript_of_single_message_is_bare() {
        let request = request_with(vec![Message::user("only")]);
        assert_eq!(flattened_transcript(&request), "only");
    }

    #[test]
    fn tool_notice_picks_primary_arg_and_truncates() {
        assert_eq!(
            tool_notice("Bash", &json!({"command": "git status"})),
            "⚒ Bash(git status)"
        );
        assert_eq!(tool_notice("TodoWrite", &json!({})), "⚒ TodoWrite");
        let long = "x".repeat(100);
        let notice = tool_notice("Read", &json!({ "file_path": long }));
        assert!(notice.chars().count() < 100);
        assert!(notice.ends_with("…)"));
    }

    #[test]
    fn mapper_translates_init_assistant_and_result() {
        let mut mapper = EnvelopeMapper::new("claude-opus-4-8");

        let init = mapper.map_line(
            r#"{"type":"system","subtype":"init","session_id":"sess-42","model":"claude-opus-4-8"}"#,
        );
        assert!(!init.done);
        assert!(matches!(
            init.events.first(),
            Some(StreamEvent::MessageStart { id, .. }) if id == "sess-42"
        ));
        assert_eq!(mapper.session_id.as_deref(), Some("sess-42"));

        let assistant = mapper.map_line(
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"text","text":"hello"},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
        );
        assert!(!assistant.done);
        let texts: Vec<String> = assistant
            .events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["hello".to_string(), "⚒ Bash(ls)".to_string()]);

        let result = mapper.map_line(
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":10,"output_tokens":5}}"#,
        );
        assert!(result.done);
        assert!(matches!(
            result.events.first(),
            Some(StreamEvent::MessageDelta {
                stop_reason: Some(StopReason::EndTurn),
                usage: Some(usage),
            }) if usage.input_tokens == 10 && usage.output_tokens == 5
        ));
        assert!(matches!(
            result.events.last(),
            Some(StreamEvent::MessageStop)
        ));
    }

    #[test]
    fn mapper_surfaces_result_errors() {
        let mut mapper = EnvelopeMapper::new("claude-opus-4-8");
        let outcome = mapper.map_line(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"limit reached"}"#,
        );
        assert!(outcome.done);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            StreamEvent::Error { message, .. } if message == "limit reached"
        )));
    }

    #[test]
    fn mapper_ignores_noise_lines() {
        let mut mapper = EnvelopeMapper::new("claude-opus-4-8");
        assert!(mapper.map_line("").events.is_empty());
        assert!(mapper.map_line("not json").events.is_empty());
        assert!(mapper
            .map_line(r#"{"type":"user","message":{"content":[]}}"#)
            .events
            .is_empty());
    }
}
