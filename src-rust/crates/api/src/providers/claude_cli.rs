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
// context) in the current working directory. When Coven starts the
// engine it supplies the permission policy for that loop
// (`claude_cli_policy.rs`). This provider forwards the
// user's prompt (with any pasted images as standard Anthropic image blocks),
// streams Claude's text back, and renders the CLI's tool activity as
// one-line notices. It never emits `ToolUse` blocks, so the
// query loop treats every turn as self-contained (`end_turn`).
//
// Liveness: the CLI prints nothing while one of its tools runs or while it
// waits on a long reply, and those silences routinely outlast the query
// loop's stream stall timeout. A stalled HTTP stream is retried; a "stalled"
// CLI turn would be killed mid-tool and its prompt run again. So while the
// CLI process is alive and quiet, the stream sends `StreamEvent::Ping`, the
// same keepalive the HTTP transport forwards, and the stall timeout is left
// to mean what it says: nothing is there to answer.
//
// Background work: the CLI prints a `result` at the end of each of its own
// turns, and one run can hold several. With stdin closed it stays open for the
// background commands, subagents and watches it started, then takes another
// turn with their outcome. Once a run has started background work, the stream
// therefore ends when the CLI exits, not at a `result`: ending at the first
// one kills the CLI and the work the answer was waiting for.
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

use async_stream::stream;
use async_trait::async_trait;
use claurst_core::config::PermissionMode;
use claurst_core::provider_id::ProviderId;
use claurst_core::types::{ContentBlock, Message, MessageContent, Role, UsageInfo};
use futures::Stream;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::provider::{LlmProvider, ModelInfo};
use crate::provider_error::ProviderError;
use crate::provider_types::{
    ProviderCapabilities, ProviderRequest, ProviderResponse, ProviderStatus, StopReason,
    StreamEvent, SystemPromptStyle,
};

mod stderr;
use stderr::StderrDrain;

#[cfg(all(test, unix))]
mod background_work_tests;

#[cfg(all(test, unix))]
mod integration_tests;

/// Env var overriding the `claude` binary location.
pub const CLAUDE_BIN_ENV: &str = "COVEN_CODE_CLAUDE_BIN";

const INSTALL_HINT: &str = "claude CLI not found — install with `npm install -g @anthropic-ai/claude-code`, sign in by running `claude`, and ensure it is on PATH (or set COVEN_CODE_CLAUDE_BIN)";

/// Env var setting, in seconds, how long a running CLI may print nothing
/// before the turn is stopped. `0` waits without limit.
pub const CLAUDE_QUIET_LIMIT_ENV: &str = "COVEN_CODE_CLAUDE_QUIET_LIMIT_SECS";

/// The quiet limit when the env var does not set one. Claude Code stops its
/// own commands sooner: ten minutes at most in the foreground, two hours at
/// most in the background. A CLI silent for longer than that is not waiting
/// on one of them.
const DEFAULT_QUIET_LIMIT: std::time::Duration = std::time::Duration::from_secs(150 * 60);

/// The quiet limit `value` asks for: `None` waits without limit. A value that
/// is not a whole number of seconds is ignored, so a typing mistake cannot
/// turn the limit off.
fn quiet_limit_from(value: Option<&str>) -> Option<std::time::Duration> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Some(DEFAULT_QUIET_LIMIT),
        Some(value) => match value.parse::<u64>() {
            Ok(0) => None,
            Ok(seconds) => Some(std::time::Duration::from_secs(seconds)),
            Err(_) => {
                warn!(
                    value,
                    "{CLAUDE_QUIET_LIMIT_ENV} is not a whole number of seconds; using the default"
                );
                Some(DEFAULT_QUIET_LIMIT)
            }
        },
    }
}

/// The error for a CLI that is still running after `limit` of silence.
fn quiet_limit_error(provider: &ProviderId, limit: std::time::Duration) -> ProviderError {
    let waited = match limit.as_secs() {
        seconds if seconds >= 120 && seconds % 60 == 0 => format!("{} minutes", seconds / 60),
        seconds => format!("{seconds} s"),
    };
    ProviderError::StreamError {
        provider: provider.clone(),
        message: format!(
            "claude CLI printed nothing for {waited} while still running, so the turn was stopped. Send the request again. To wait longer, set {CLAUDE_QUIET_LIMIT_ENV} to a number of seconds (0 waits without limit)"
        ),
        partial_response: None,
    }
}

/// How often a quiet, running CLI turn tells the consumer it is still alive.
/// Well inside the query loop's 45-second stall timeout, so a late tick
/// cannot let it fire.
const HEARTBEAT_PERIOD: std::time::Duration = std::time::Duration::from_secs(15);

/// How many heartbeat periods a CLI that has finished its background work gets
/// to exit, or to start the turn that reports on it, after a `result`. A CLI
/// that does neither is done, and the stream ends without it.
const EXIT_GRACE_BEATS: u32 = 2;

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

/// Preserve the latest turn's text/image order. A resumed CLI already owns
/// earlier images; a restart retains the existing text-only history fallback.
fn current_turn_content(request: &ProviderRequest, include_history: bool) -> Vec<ContentBlock> {
    let Some(index) = request
        .messages
        .iter()
        .rposition(|message| matches!(message.role, Role::User))
    else {
        return vec![ContentBlock::Text {
            text: String::new(),
        }];
    };
    let current = &request.messages[index];
    let blocks = match &current.content {
        MessageContent::Blocks(blocks)
            if blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::Image { .. })) =>
        {
            blocks
        }
        _ => {
            return vec![ContentBlock::Text {
                text: if include_history {
                    flattened_transcript(request)
                } else {
                    last_user_text(request)
                },
            }]
        }
    };
    let mut content = Vec::new();
    if include_history {
        let earlier = request.messages[..index]
            .iter()
            .filter_map(|message| {
                let text = message_text(message);
                if text.is_empty() {
                    return None;
                }
                let role = match message.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                Some(format!("{role}: {text}"))
            })
            .collect::<Vec<_>>();
        if !earlier.is_empty() {
            content.push(ContentBlock::Text { text: format!(
                "<conversation-so-far>\nThe session was restarted; this is the conversation before the current message.\n{}\n</conversation-so-far>\n\n",
                earlier.join("\n")) });
        }
    }
    content.extend(
        blocks
            .iter()
            .filter(|block| {
                matches!(
                    block,
                    ContentBlock::Text { .. } | ContentBlock::Image { .. }
                )
            })
            .cloned(),
    );
    content
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
///
/// The CLI runs its own tools, so the turn's permission mode must reach it as
/// the CLI's own `--permission-mode`; without one, the CLI's settings decide
/// and can widen a read-only turn. A request that names no mode runs as `plan`.
fn build_args(
    model: &str,
    resume: Option<&str>,
    permission: Option<&PermissionMode>,
) -> Vec<String> {
    let permission_mode = cli_permission_mode(permission);
    let mut args = vec![
        "-p".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--permission-mode".to_string(),
        permission_mode.to_string(),
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

/// The CLI's name for a turn's permission mode. A request that names no mode
/// runs as `plan`.
fn cli_permission_mode(permission: Option<&PermissionMode>) -> &'static str {
    match permission {
        Some(PermissionMode::BypassPermissions) => "bypassPermissions",
        Some(PermissionMode::AcceptEdits) => "acceptEdits",
        Some(PermissionMode::Default) => "default",
        Some(PermissionMode::Plan) | None => READ_ONLY_CLI_MODE,
    }
}

/// The CLI mode a read-only turn runs in.
const READ_ONLY_CLI_MODE: &str = "plan";

/// The single stream-json stdin line carrying ordered current-turn blocks.
fn stdin_line(content: &[ContentBlock]) -> String {
    let envelope = json!({
        "type": "user",
        "message": { "role": "user", "content": content },
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
    /// How many background tasks the CLI last reported as running.
    live_background_tasks: usize,
    /// The CLI has started background work in this run. From then on a
    /// `result` closes one of the CLI's turns, not the run.
    ran_background_work: bool,
    /// One of the CLI's turns is under way: false from a `result` until the
    /// next turn's first line.
    turn_open: bool,
    /// Usage summed over the results so far; `None` until the first one.
    usage: Option<UsageInfo>,
}

/// What a single stdout line produced.
struct LineOutcome {
    events: Vec<StreamEvent>,
    /// The run's last `result` envelope arrived — the stream is complete.
    done: bool,
}

impl EnvelopeMapper {
    fn new(fallback_model: &str) -> Self {
        Self {
            fallback_model: fallback_model.to_string(),
            message_started: false,
            block_index: 0,
            session_id: None,
            live_background_tasks: 0,
            ran_background_work: false,
            turn_open: true,
            usage: None,
        }
    }

    /// The events that close the stream, carrying the usage of every result.
    fn finish(&mut self) -> [StreamEvent; 2] {
        [
            StreamEvent::MessageDelta {
                stop_reason: Some(StopReason::EndTurn),
                usage: Some(self.usage.take().unwrap_or_default()),
            },
            StreamEvent::MessageStop,
        ]
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
                match envelope.get("subtype").and_then(|v| v.as_str()) {
                    Some("init") => self.turn_open = true,
                    // The CLI sends the full set of running tasks each time.
                    Some("background_tasks_changed") => {
                        self.live_background_tasks = envelope
                            .get("tasks")
                            .and_then(|v| v.as_array())
                            .map_or(0, Vec::len);
                        self.ran_background_work |= self.live_background_tasks > 0;
                    }
                    _ => {}
                }
            }
            Some("assistant") => {
                self.turn_open = true;
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
            Some("user") => self.turn_open = true,
            Some("result") => {
                self.start_message_if_needed(&mut events, None);
                let is_error = envelope
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let error_detail = if is_error {
                    let detail = envelope
                        .get("result")
                        .and_then(|v| v.as_str())
                        .unwrap_or("claude CLI reported an error");
                    Some(StreamEvent::Error {
                        error_type: envelope
                            .get("subtype")
                            .and_then(|v| v.as_str())
                            .unwrap_or("error")
                            .to_string(),
                        message: detail.to_string(),
                    })
                } else {
                    None
                };
                let usage_json = envelope.get("usage");
                let read_count = |key: &str| {
                    usage_json
                        .and_then(|u| u.get(key))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                };
                // Each result counts its own turn only, so a run's usage is
                // their sum.
                let usage = self.usage.get_or_insert_with(UsageInfo::default);
                usage.input_tokens += read_count("input_tokens");
                usage.output_tokens += read_count("output_tokens");
                usage.cache_creation_input_tokens += read_count("cache_creation_input_tokens");
                usage.cache_read_input_tokens += read_count("cache_read_input_tokens");
                // Publish completed turns before a later failure or cancellation.
                // Consumers merge cumulative snapshots rather than adding them.
                if self.ran_background_work || is_error {
                    events.push(StreamEvent::MessageDelta {
                        stop_reason: None,
                        usage: Some(usage.clone()),
                    });
                }
                if let Some(error) = error_detail {
                    events.push(error);
                }
                self.turn_open = false;
                // The CLI will exit, or take another turn, when its
                // background work is over; the stream ends then.
                if self.ran_background_work && !is_error {
                    return LineOutcome {
                        events,
                        done: false,
                    };
                }
                events.extend(self.finish());
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
/// bounded stderr drain, plus the owned stdin writer.
struct CliTurn {
    child: Child,
    stdin_writer: JoinSet<()>,
    stdout: BufReader<ChildStdout>,
    first_line: String,
    stderr: StderrDrain,
}

/// Result of launching one CLI turn. `CliTurn` is boxed to keep the variant
/// sizes comparable (clippy: large_enum_variant).
enum SpawnOutcome {
    Turn(Box<CliTurn>),
    /// The process exited before emitting any stdout — a stale `--resume`
    /// session or a startup failure; stderr explains which.
    ExitedEarly {
        stderr: String,
    },
}

pub struct ClaudeCliProvider {
    id: ProviderId,
    /// Resolved lazily so construction never fails; a missing binary
    /// surfaces the install hint when a turn is actually attempted.
    binary: Option<PathBuf>,
    /// See [`HEARTBEAT_PERIOD`]. A field so tests need not wait for it.
    heartbeat: std::time::Duration,
    /// How long a running CLI may print nothing before the turn is stopped,
    /// from its start as well as between its lines. The keepalive holds off
    /// the consumer's stall timeout for as long as the process lives, so this
    /// is the only bound on a CLI that hangs.
    quiet_limit: Option<std::time::Duration>,
    goal_authority: Option<(
        claurst_core::goal::GoalTurn,
        claurst_core::goal::GoalTurnPermission,
    )>,
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
            heartbeat: HEARTBEAT_PERIOD,
            quiet_limit: quiet_limit_from(std::env::var(CLAUDE_QUIET_LIMIT_ENV).ok().as_deref()),
            goal_authority: None,
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

    /// Spawn one CLI turn, start writing the prompt, and wait for the first stdout
    /// line to confirm the process came up.
    async fn spawn_turn(
        &self,
        model: &str,
        content: &[ContentBlock],
        resume: Option<&str>,
        permission: Option<&PermissionMode>,
    ) -> Result<SpawnOutcome, ProviderError> {
        // Coven's permission policy, when Coven started this engine. Without
        // it the CLI would run under whatever the operator's and the
        // workspace's own settings files allow. The policy never names a
        // permission mode: `build_args` passes the turn's own. A read-only
        // turn gets the policy without its allow rules, which the CLI would
        // otherwise apply in plan mode as well.
        let read_only = cli_permission_mode(permission) == READ_ONLY_CLI_MODE;
        let policy_args =
            super::claude_cli_policy::args_from_environment(read_only).map_err(|message| {
                ProviderError::Other {
                    provider: self.id.clone(),
                    message,
                    status: None,
                    body: None,
                }
            })?;
        let mut command = Command::new(self.binary_or_default());
        command
            .args(build_args(model, resume, permission))
            .args(policy_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
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
                    message: "Goal permits only its final response".into(),
                    status: None,
                    body: None,
                });
            }
            if *permission == claurst_core::goal::GoalTurnPermission::FinalResponse {
                // The CLI owns its native tools; ProviderRequest.tools cannot
                // disable them. Safe mode skips ordinary customization hooks
                // while preserving auth; managed policy hooks remain CLI-owned.
                // https://code.claude.com/docs/en/cli-reference
                command.args([
                    "--safe-mode",
                    "--tools", "",
                    "--disallowedTools", "*",
                    "--append-system-prompt",
                    "The goal is complete. Provide only the final summary for the user. Do not perform further work or use tools.",
                ]);
            }
        }
        if let Some((turn, _)) = &self.goal_authority {
            turn.mark_request_attempted();
        }
        let mut child = command.spawn().map_err(|e| self.spawn_error(e))?;

        let mut stdin = child.stdin.take().ok_or_else(|| ProviderError::Other {
            provider: self.id.clone(),
            message: "claude CLI stdin unavailable".to_string(),
            status: None,
            body: None,
        })?;
        let payload = stdin_line(content);
        let mut stdin_writer = JoinSet::new();
        stdin_writer.spawn(async move {
            if let Err(e) = stdin.write_all(payload.as_bytes()).await {
                warn!(error = %e, "failed writing prompt to claude CLI stdin");
            }
            drop(stdin); // EOF tells the CLI no more input is coming.
        });

        let mut stderr = StderrDrain::new(child.stderr.take());

        let stdout = child.stdout.take().ok_or_else(|| ProviderError::Other {
            provider: self.id.clone(),
            message: "claude CLI stdout unavailable".to_string(),
            status: None,
            body: None,
        })?;
        let mut reader = BufReader::new(stdout);
        let mut first_line = String::new();
        // No stream exists yet to carry a keepalive or a stall timeout, so
        // the quiet limit is all that bounds a CLI that hangs before it
        // prints anything. Returning the error drops `child`, which stops it.
        let startup_started = tokio::time::Instant::now();
        let first_read = reader.read_line(&mut first_line);
        let read = match self.quiet_limit {
            Some(limit) => tokio::time::timeout(limit, first_read)
                .await
                .map_err(|_| quiet_limit_error(&self.id, limit))?,
            None => first_read.await,
        }
        .map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("failed reading claude CLI output: {}", e),
            status: None,
            body: None,
        })?;
        if read == 0 {
            // Closing stdout does not prove exit. Keep the startup deadline
            // through reaping, so a closed pipe cannot bypass the quiet limit.
            let wait = child.wait();
            let status = match self.quiet_limit {
                Some(limit) => {
                    tokio::time::timeout(limit.saturating_sub(startup_started.elapsed()), wait)
                        .await
                        .map_err(|_| quiet_limit_error(&self.id, limit))?
                }
                None => wait.await,
            }
            .map_err(|error| self.spawn_error(error))?;
            debug!(
                ?status,
                resume = resume.is_some(),
                "claude CLI exited before emitting output"
            );
            return Ok(SpawnOutcome::ExitedEarly {
                stderr: stderr.finish().await,
            });
        }

        Ok(SpawnOutcome::Turn(Box::new(CliTurn {
            child,
            stdin_writer,
            stdout: reader,
            first_line,
            stderr,
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

        let permission = request.permission_mode.as_ref();
        if let Some(session_id) = resume_session {
            match self
                .spawn_turn(
                    &request.model,
                    &current_turn_content(request, false),
                    Some(&session_id),
                    permission,
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

        let content = current_turn_content(request, has_assistant_turns);
        match self
            .spawn_turn(&request.model, &content, None, permission)
            .await?
        {
            SpawnOutcome::Turn(turn) => Ok(*turn),
            SpawnOutcome::ExitedEarly { stderr: detail } => Err(ProviderError::Other {
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
            }),
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
        let heartbeat_period = self.heartbeat;
        let quiet_limit = self.quiet_limit;

        let s = stream! {
            // Retain ownership even before the stream is first polled; dropping
            // the stream aborts a writer still blocked on the child's stdin.
            let _stdin_writer = turn.stdin_writer;
            let mut mapper = EnvelopeMapper::new(&fallback_model);
            let mut line = std::mem::take(&mut turn.first_line);
            let mut heartbeat = tokio::time::interval_at(
                tokio::time::Instant::now() + heartbeat_period,
                heartbeat_period,
            );
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut exit_deadline = None;
            loop {
                let outcome = mapper.map_line(&line);
                if let Some(session_id) = mapper.session_id.as_deref() {
                    let mut cached = CLI_SESSION.lock();
                    if cached.as_deref() != Some(session_id) {
                        *cached = Some(session_id.to_string());
                    }
                }
                if outcome.done {
                    // Complete the owned drain before publishing MessageStop:
                    // callers may drop the stream as soon as they see it.
                    let _ = turn.stderr.finish().await;
                }
                for event in outcome.events {
                    yield Ok(event);
                }
                if outcome.done {
                    // The result is terminal. Dropping the owned child stops a
                    // CLI that keeps running after publishing it.
                    return;
                }

                // Between its turns, with no background work left, the CLI
                // either exits or starts the turn that reports on that work.
                // One that stays silent instead has nothing more to say.
                let exit_is_due = mapper.usage.is_some()
                    && !mapper.turn_open
                    && mapper.live_background_tasks == 0;
                if exit_is_due {
                    exit_deadline.get_or_insert_with(|| {
                        tokio::time::Instant::now() + heartbeat_period * EXIT_GRACE_BEATS
                    });
                } else {
                    exit_deadline = None;
                }
                let exit_grace = async {
                    match exit_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending::<()>().await,
                    }
                };
                tokio::pin!(exit_grace);
                let mut grace_expired = false;

                line.clear();
                let read = {
                    // `read_line` loses what it has read if it is dropped, so
                    // one read stays pinned across the heartbeats.
                    let read_line = turn.stdout.read_line(&mut line);
                    tokio::pin!(read_line);
                    let quiet_deadline = async {
                        match quiet_limit {
                            Some(limit) => tokio::time::sleep(limit).await,
                            None => std::future::pending::<()>().await,
                        }
                    };
                    tokio::pin!(quiet_deadline);
                    let mut quiet_expired = false;
                    loop {
                        let finished = tokio::select! {
                            result = &mut read_line => Some(result),
                            // Handled as the end of the CLI's output.
                            _ = &mut exit_grace => {
                                grace_expired = true;
                                Some(Ok(0))
                            },
                            _ = &mut quiet_deadline, if !quiet_expired => {
                                quiet_expired = true;
                                if matches!(turn.child.try_wait(), Ok(None)) {
                                    if let Some(limit) = quiet_limit {
                                        yield Err(quiet_limit_error(&provider_id, limit));
                                        return;
                                    }
                                }
                                continue;
                            },
                            _ = heartbeat.tick() => None,
                        };
                        match finished {
                            Some(result) => break result,
                            // Quiet, and still running: a tool is at work or
                            // a reply is on its way. A process that has gone
                            // sends no keepalive, so the consumer's stall
                            // timeout still covers a pipe left open behind it.
                            None => {
                                if !matches!(turn.child.try_wait(), Ok(None)) {
                                    continue;
                                }
                                yield Ok(StreamEvent::Ping);
                            }
                        }
                    }
                };
                // Keep the consumer clock independent of stdout: task snapshots
                // and other internal lines may yield no mapped event at all.
                match read {
                    Ok(0) => {
                        // Real EOF and the between-turn grace have different
                        // meanings. Only a clean exit between completed turns,
                        // or a valid grace expiry, can finish an earlier result.
                        let status = if grace_expired {
                            turn.child.try_wait()
                        } else {
                            let grace = heartbeat_period * EXIT_GRACE_BEATS;
                            let limit = quiet_limit.map_or(grace, |limit| limit.min(grace));
                            match tokio::time::timeout(limit, turn.child.wait()).await {
                                Ok(status) => status.map(Some),
                                Err(_) => Ok(None),
                            }
                        };
                        let stderr = turn.stderr.finish().await;
                        let failure = match status {
                            Ok(Some(status)) if !status.success() => {
                                Some(format!("claude CLI exited unsuccessfully ({status})"))
                            }
                            Err(error) => Some(format!("failed waiting for claude CLI: {error}")),
                            _ if mapper.usage.is_none() => {
                                Some("claude CLI exited before emitting a result".to_string())
                            }
                            _ if mapper.turn_open => {
                                Some("claude CLI exited before completing its reporting turn".to_string())
                            }
                            Ok(None) if !grace_expired => {
                                Some("claude CLI closed stdout before exiting".to_string())
                            }
                            _ => None,
                        };
                        if let Some(message) = failure {
                            let detail = if stderr.is_empty() {
                                String::new()
                            } else {
                                format!(": {stderr}")
                            };
                            yield Err(ProviderError::StreamError {
                                provider: provider_id.clone(),
                                message: format!("{message}{detail}"),
                                partial_response: None,
                            });
                        } else {
                            for event in mapper.finish() {
                                yield Ok(event);
                            }
                        }
                        return;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let stderr = turn.stderr.finish().await;
                        let detail = if stderr.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", stderr)
                        };
                        yield Err(ProviderError::StreamError {
                            provider: provider_id.clone(),
                            message: format!("failed reading claude CLI output: {}{}", e, detail),
                            partial_response: None,
                        });
                        return;
                    }
                }
            }
        };

        Ok(Box::pin(s))
    }

    async fn create_message_stream_for_goal(
        &self,
        request: ProviderRequest,
        turn: Option<&claurst_core::goal::GoalTurn>,
        permission: claurst_core::goal::GoalTurnPermission,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        match turn {
            Some(turn) => {
                Self {
                    id: self.id.clone(),
                    binary: self.binary.clone(),
                    heartbeat: self.heartbeat,
                    quiet_limit: self.quiet_limit,
                    goal_authority: Some((turn.clone(), permission)),
                }
                .create_message_stream(request)
                .await
            }
            None => self.create_message_stream(request).await,
        }
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

    /// Keep newly written scenario files out of execve's ETXTBSY checks.
    #[cfg(unix)]
    pub(super) fn fixture_binary(dir: &std::path::Path, script: &str) -> PathBuf {
        std::fs::write(dir.join("claude"), script).unwrap();
        let binary = dir.join("launcher");
        std::os::unix::fs::symlink(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/claude_cli_fixture.sh"
            ),
            &binary,
        )
        .unwrap();
        binary
    }

    pub(super) fn request_with(messages: Vec<Message>) -> ProviderRequest {
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
            permission_mode: None,
        }
    }

    #[tokio::test]
    async fn missing_binary_reports_startup_failure() {
        let provider = ClaudeCliProvider {
            id: ProviderId::new(ProviderId::ANTHROPIC),
            binary: Some(
                std::env::temp_dir().join(format!("missing-claude-{}", uuid::Uuid::new_v4())),
            ),
            heartbeat: HEARTBEAT_PERIOD,
            quiet_limit: None,
            goal_authority: None,
        };
        let request = request_with(vec![Message::user("fixture prompt")]);
        let error = match provider.create_message_stream(request).await {
            Ok(_) => panic!("nonexistent binary unexpectedly started"),
            Err(error) => error,
        };
        assert!(matches!(error, ProviderError::Other { message, .. } if message == INSTALL_HINT));
    }

    /// A stand-in `claude` that starts a turn, stays quiet for `quiet`
    /// seconds the way a long tool call does, then finishes the turn.
    #[cfg(unix)]
    fn quiet_cli(quiet: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("quiet-claude-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let script = format!(
            "#!/bin/sh\ncat >/dev/null\n\
             echo '{{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"fixture\"}}'\n\
             sleep {quiet}\n\
             echo '{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"after the wait\"}}]}}}}'\n\
             echo '{{\"type\":\"result\",\"is_error\":false,\"usage\":{{}}}}'\n"
        );
        let binary = fixture_binary(&dir, &script);
        (dir, binary)
    }

    /// The query loop gives up on a stream that is silent for 45 seconds,
    /// kills the CLI and runs the prompt again. A CLI that is busy must not
    /// look like that: it has to send keepalives for as long as it is quiet.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_quiet_running_cli_sends_keepalives_until_it_answers() {
        use futures::StreamExt;
        let (dir, binary) = quiet_cli("1");
        let provider = ClaudeCliProvider {
            id: ProviderId::new(ProviderId::ANTHROPIC),
            binary: Some(binary),
            heartbeat: std::time::Duration::from_millis(100),
            quiet_limit: None,
            goal_authority: None,
        };
        let request = request_with(vec![Message::user("fixture prompt")]);
        let mut stream = provider.create_message_stream(request).await.unwrap();

        let mut events = Vec::new();
        let mut longest_gap = std::time::Duration::ZERO;
        let mut last = std::time::Instant::now();
        while let Some(event) = stream.next().await {
            longest_gap = longest_gap.max(last.elapsed());
            last = std::time::Instant::now();
            events.push(event.unwrap());
        }
        let _ = std::fs::remove_dir_all(dir);

        let pings = events
            .iter()
            .filter(|event| matches!(event, StreamEvent::Ping))
            .count();
        // One second of silence at a 100 ms heartbeat. Fewer than a handful
        // would mean the keepalive stopped while the CLI was still working.
        assert!(pings >= 5, "{pings} keepalives in one quiet second");
        assert!(
            longest_gap < std::time::Duration::from_millis(700),
            "the stream was silent for {longest_gap:?}"
        );
        // The turn itself is untouched: its text arrives and it completes.
        assert!(events.iter().any(
            |event| matches!(event, StreamEvent::TextDelta { text, .. } if text == "after the wait")
        ));
        assert!(matches!(events.last(), Some(StreamEvent::MessageStop)));
        // Keepalives are for silence only: none trails the answer.
        let last_ping = events
            .iter()
            .rposition(|event| matches!(event, StreamEvent::Ping))
            .unwrap();
        let first_text = events
            .iter()
            .position(|event| matches!(event, StreamEvent::TextDelta { .. }))
            .unwrap();
        assert!(last_ping < first_text, "a keepalive followed the answer");
    }

    /// A CLI that answers at once is not padded with keepalives.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_prompt_cli_sends_no_keepalive() {
        use futures::StreamExt;
        let (dir, binary) = quiet_cli("0");
        let provider = ClaudeCliProvider {
            id: ProviderId::new(ProviderId::ANTHROPIC),
            binary: Some(binary),
            heartbeat: std::time::Duration::from_secs(30),
            quiet_limit: None,
            goal_authority: None,
        };
        let request = request_with(vec![Message::user("fixture prompt")]);
        let mut stream = provider.create_message_stream(request).await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }
        let _ = std::fs::remove_dir_all(dir);
        assert!(!events
            .iter()
            .any(|event| matches!(event, StreamEvent::Ping)));
        assert!(matches!(events.last(), Some(StreamEvent::MessageStop)));
    }

    /// A stand-in `claude` made of `body`, a provider that runs it with a
    /// 50 ms heartbeat and `quiet_limit`, and everything its stream yields.
    #[cfg(unix)]
    async fn run_with_quiet_limit(
        body: &str,
        quiet_limit: Option<std::time::Duration>,
    ) -> (
        std::path::PathBuf,
        Vec<Result<StreamEvent, ProviderError>>,
        std::time::Duration,
    ) {
        use futures::StreamExt;
        let dir = std::env::temp_dir().join(format!("silent-claude-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let script = format!(
            "#!/bin/sh\ncat >/dev/null\n\
             echo '{{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"fixture\"}}'\n{body}"
        );
        let binary = fixture_binary(&dir, &script);
        let provider = ClaudeCliProvider {
            id: ProviderId::new(ProviderId::ANTHROPIC),
            binary: Some(binary),
            heartbeat: std::time::Duration::from_millis(50),
            quiet_limit,
            goal_authority: None,
        };
        let request = request_with(vec![Message::user("fixture prompt")]);
        let started = std::time::Instant::now();
        let mut stream = provider.create_message_stream(request).await.unwrap();
        let mut items = Vec::new();
        while let Some(item) = stream.next().await {
            items.push(item);
        }
        (dir, items, started.elapsed())
    }

    /// Whether process `pid` is still there after up to five seconds, which
    /// is ample for a stopped process to be reaped.
    #[cfg(unix)]
    async fn still_running_after_a_wait(pid: &str) -> bool {
        for _ in 0..100 {
            let running = std::process::Command::new("kill")
                .args(["-0", pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success();
            if !running {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        true
    }

    #[cfg(unix)]
    fn is_quiet_limit_error(item: &Result<StreamEvent, ProviderError>) -> bool {
        matches!(item, Err(ProviderError::StreamError { message, .. })
            if message.contains("printed nothing for"))
    }

    /// The keepalive tells the query loop a silent CLI is busy for as long as
    /// its process lives. A CLI that has hung is alive too, so without a
    /// limit of the provider's own the turn would wait on it for ever.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_running_cli_silent_past_the_limit_ends_the_turn_with_an_error() {
        // The CLI records its pid, then hangs: one process, alive and silent.
        let (dir, items, took) = run_with_quiet_limit(
            "echo $$ > \"$(dirname \"$0\")/pid\"\nexec sleep 60\n",
            Some(std::time::Duration::from_secs(1)),
        )
        .await;
        let pid = std::fs::read_to_string(dir.join("pid")).unwrap();

        // It was kept alive up to the limit, then failed once, and the stream
        // ended there instead of waiting out the minute.
        assert!(
            took >= std::time::Duration::from_secs(1),
            "ended after {took:?}"
        );
        assert!(
            took < std::time::Duration::from_secs(30),
            "ended after {took:?}"
        );
        let (last, before) = items.split_last().unwrap();
        assert!(is_quiet_limit_error(last), "last item was {last:?}");
        assert!(before.iter().all(Result::is_ok));
        assert!(before
            .iter()
            .any(|item| matches!(item, Ok(StreamEvent::Ping))));

        // Ending the stream stopped the CLI: nothing is left running.
        let running = still_running_after_a_wait(pid.trim()).await;
        let _ = std::fs::remove_dir_all(dir);
        assert!(
            !running,
            "the hung CLI (pid {}) is still running",
            pid.trim()
        );
    }

    /// A CLI can hang before it prints anything. No stream exists yet at
    /// that point, so neither the keepalive nor the stall timeout is running:
    /// the quiet limit has to cover the wait for the first line as well.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_cli_silent_from_the_start_fails_the_turn_at_the_limit() {
        let dir = std::env::temp_dir().join(format!("mute-claude-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let script =
            "#!/bin/sh\necho $$ > \"$(dirname \"$0\")/pid\"\ncat >/dev/null\nexec sleep 60\n";
        let binary = fixture_binary(&dir, script);
        let provider = ClaudeCliProvider {
            id: ProviderId::new(ProviderId::ANTHROPIC),
            binary: Some(binary),
            heartbeat: std::time::Duration::from_millis(50),
            quiet_limit: Some(std::time::Duration::from_secs(1)),
            goal_authority: None,
        };
        let request = request_with(vec![Message::user("fixture prompt")]);
        let started = std::time::Instant::now();
        let error = match provider.create_message_stream(request).await {
            Ok(_) => panic!("a CLI that printed nothing produced a stream"),
            Err(error) => error,
        };
        let took = started.elapsed();
        let error_result = Err(error);
        assert!(is_quiet_limit_error(&error_result), "{error_result:?}");
        let pid = std::fs::read_to_string(dir.join("pid")).unwrap();

        assert!(
            took >= std::time::Duration::from_secs(1),
            "failed after {took:?}"
        );
        assert!(
            took < std::time::Duration::from_secs(30),
            "failed after {took:?}"
        );
        let running = still_running_after_a_wait(pid.trim()).await;
        let _ = std::fs::remove_dir_all(dir);
        assert!(
            !running,
            "the hung CLI (pid {}) is still running",
            pid.trim()
        );
    }

    /// The limit is on unbroken silence, not on the length of the turn: each
    /// line the CLI prints starts the count again.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_cli_that_keeps_printing_is_not_stopped_by_the_quiet_limit() {
        let line = "echo '{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"step\"}]}}'\n";
        // Four quiet stretches of 0.4 s: 1.6 s in all, against a 1.2 s limit.
        let body = format!(
            "{}echo '{{\"type\":\"result\",\"is_error\":false,\"usage\":{{}}}}'\n",
            format!("sleep 0.4\n{line}").repeat(4)
        );
        let (dir, items, took) =
            run_with_quiet_limit(&body, Some(std::time::Duration::from_millis(1200))).await;
        let _ = std::fs::remove_dir_all(dir);

        assert!(
            took >= std::time::Duration::from_millis(1600),
            "ended after {took:?}"
        );
        assert!(items.iter().all(Result::is_ok), "{items:?}");
        let steps = items
            .iter()
            .filter(
                |item| matches!(item, Ok(StreamEvent::TextDelta { text, .. }) if text == "step"),
            )
            .count();
        assert_eq!(steps, 4);
        assert!(matches!(items.last(), Some(Ok(StreamEvent::MessageStop))));
    }

    /// The limit is for a CLI that is running. One that has gone while
    /// something else holds its output open gets no keepalive and no quiet
    /// limit error: the consumer's stall timeout covers it, as before.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_cli_that_has_gone_is_not_reported_as_silent() {
        // The CLI exits at once; a command it left behind keeps the pipe open
        // for two seconds, well past the 300 ms limit.
        let (dir, items, _) = run_with_quiet_limit(
            "sleep 2 &\nexit 0\n",
            Some(std::time::Duration::from_millis(300)),
        )
        .await;
        let _ = std::fs::remove_dir_all(dir);

        assert!(!items.iter().any(is_quiet_limit_error), "{items:?}");
        assert!(matches!(
            items.last(),
            Some(Err(ProviderError::StreamError { message, .. }))
                if message.contains("exited before emitting a result")
        ));
    }

    #[test]
    fn the_quiet_limit_comes_from_the_environment_value() {
        use std::time::Duration;
        // Unset or blank: the default.
        assert_eq!(quiet_limit_from(None), Some(DEFAULT_QUIET_LIMIT));
        assert_eq!(quiet_limit_from(Some("  ")), Some(DEFAULT_QUIET_LIMIT));
        // A number of seconds; zero waits without limit.
        assert_eq!(quiet_limit_from(Some("90")), Some(Duration::from_secs(90)));
        assert_eq!(
            quiet_limit_from(Some(" 600\n")),
            Some(Duration::from_secs(600))
        );
        assert_eq!(quiet_limit_from(Some("0")), None);
        // Anything else is a mistake, and a mistake must not turn it off.
        for value in ["ten", "-5", "1.5", "30m"] {
            assert_eq!(
                quiet_limit_from(Some(value)),
                Some(DEFAULT_QUIET_LIMIT),
                "{value}"
            );
        }
    }

    /// The provider the engine builds is the one the limit has to be on.
    #[test]
    fn a_new_provider_takes_its_quiet_limit_from_the_environment() {
        let expected = quiet_limit_from(std::env::var(CLAUDE_QUIET_LIMIT_ENV).ok().as_deref());
        assert_eq!(ClaudeCliProvider::new().quiet_limit, expected);
    }

    #[test]
    fn the_quiet_limit_error_says_how_long_and_how_to_change_it() {
        let provider = ProviderId::new(ProviderId::ANTHROPIC);
        let text =
            |seconds| match quiet_limit_error(&provider, std::time::Duration::from_secs(seconds)) {
                ProviderError::StreamError { message, .. } => message,
                other => panic!("unexpected error: {other:?}"),
            };
        assert!(text(9000).contains("printed nothing for 150 minutes while still running"));
        assert!(text(90).contains("printed nothing for 90 s while still running"));
        assert!(text(9000).contains(CLAUDE_QUIET_LIMIT_ENV));
    }

    #[test]
    fn build_args_matches_adapter_manifest_contract() {
        let args = build_args("claude-opus-4-8", None, Some(&PermissionMode::Plan));
        assert_eq!(
            args,
            vec![
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "plan",
                "--model",
                "claude-opus-4-8",
            ]
        );

        let resumed = build_args(
            "claude-opus-4-8",
            Some("sess-1"),
            Some(&PermissionMode::Plan),
        );
        assert!(resumed.ends_with(&["--resume".to_string(), "sess-1".to_string()]));
    }

    fn permission_modes(args: &[String]) -> Vec<&str> {
        args.windows(2)
            .filter(|pair| pair[0] == "--permission-mode")
            .map(|pair| pair[1].as_str())
            .collect()
    }

    #[test]
    fn build_args_passes_the_turn_mode_as_the_cli_mode() {
        for (mode, cli_mode) in [
            (PermissionMode::Plan, "plan"),
            (PermissionMode::BypassPermissions, "bypassPermissions"),
            (PermissionMode::AcceptEdits, "acceptEdits"),
            (PermissionMode::Default, "default"),
        ] {
            for resume in [None, Some("sess-1")] {
                let args = build_args("claude-opus-4-8", resume, Some(&mode));
                assert_eq!(permission_modes(&args), [cli_mode], "{mode:?}: {args:?}");
            }
        }
    }

    #[test]
    fn build_args_runs_a_request_without_a_mode_read_only() {
        let args = build_args("claude-opus-4-8", None, None);
        assert_eq!(permission_modes(&args), ["plan"], "{args:?}");
    }

    #[test]
    fn stdin_line_is_a_stream_json_user_envelope() {
        let line = stdin_line(&[ContentBlock::Text {
            text: "hello".into(),
        }]);
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
    fn stdin_line_keeps_the_supplied_image_then_text_order() {
        let line = stdin_line(&[
            png_block("aGk="),
            ContentBlock::Text {
                text: "what is this?".into(),
            },
        ]);
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
        let line = stdin_line(&[png_block("aGk=")]);
        let parsed: Value = serde_json::from_str(line.trim()).expect("valid JSON");
        let content = parsed["message"]["content"]
            .as_array()
            .expect("content array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "image");
    }

    #[test]
    fn current_turn_content_reads_only_the_latest_user_message() {
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
        let images = current_turn_content(&request, false);
        assert_eq!(images.len(), 2);
        assert!(matches!(
            &images[0],
            ContentBlock::Image { source } if source.data.as_deref() == Some("bmV3")
        ));

        let text_only = request_with(vec![Message::user("no images")]);
        assert!(
            matches!(current_turn_content(&text_only, false).as_slice(), [ContentBlock::Text { text }] if text == "no images")
        );
    }

    #[test]
    fn mixed_content_keeps_order_on_resume_and_restart() {
        let latest = vec![
            ContentBlock::Text {
                text: "caption A".into(),
            },
            png_block("first"),
            ContentBlock::Text {
                text: "caption B".into(),
            },
            png_block("second"),
        ];
        let request = request_with(vec![
            Message::user("old question"),
            Message::assistant("old answer"),
            Message::user_blocks(latest.clone()),
        ]);
        let expected = serde_json::to_value(&latest).unwrap();
        let resumed: Value =
            serde_json::from_str(&stdin_line(&current_turn_content(&request, false))).unwrap();
        assert_eq!(resumed["message"]["content"], expected);
        let restarted: Value =
            serde_json::from_str(&stdin_line(&current_turn_content(&request, true))).unwrap();
        let blocks = restarted["message"]["content"].as_array().unwrap();
        assert_eq!(serde_json::to_value(&blocks[1..]).unwrap(), expected);
        let prefix = blocks[0]["text"].as_str().unwrap();
        assert!(prefix.contains("user: old question"));
        assert!(prefix.contains("assistant: old answer"));
        assert!(!prefix.contains("caption A"));
        assert!(prefix.ends_with("</conversation-so-far>\n\n"));
    }

    #[test]
    fn image_only_restart_keeps_history_before_the_current_image() {
        let request = request_with(vec![
            Message::user("old question"),
            Message::assistant("old answer"),
            Message::user_blocks(vec![png_block("current")]),
        ]);
        let content = current_turn_content(&request, true);
        assert!(
            matches!(content.as_slice(), [ContentBlock::Text { text }, ContentBlock::Image { source }] if text.contains("old answer") && source.data.as_deref() == Some("current"))
        );
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
