//! Long-lived stream-json mode: the Coven runtime stream protocol.
//!
//! Engaged by `--print --input-format stream-json`. The process stays alive
//! across chat turns: stdin is newline-delimited JSON user frames, stdout is
//! newline-delimited JSON events, and the process exits when stdin closes.
//! This is the contract the Coven daemon drives for a runtime that declares
//! `capabilities.stream` in its coven-runtimes manifest (see
//! `spec/runtime-manifest/coven-code.json`), and it mirrors Claude Code's
//! `-p --input-format stream-json --output-format stream-json` mode so the
//! daemon's existing frame parser (`coven`'s `stream_json.rs`) accepts the
//! output unchanged.
//!
//! Frames emitted (one JSON object per line, flushed immediately):
//! - `{"type":"system","subtype":"init",...}` once at startup.
//! - `{"type":"assistant","message":{...}}` for tool-use and final text.
//! - `{"type":"tool_result",...}` after each tool execution.
//! - `{"type":"result","subtype":...}` closing each turn.
//!
//! Frames accepted on stdin:
//! - `{"type":"user","message":{"role":"user","content":...}}` where content
//!   is a string or an array of `{"type":"text","text":...}` blocks — the
//!   Claude/Coven shape. Triggers a turn.
//! - `{"role":"user"|"assistant","content":"..."}` — the legacy coven-code
//!   shape. `assistant` frames append as prefill without running a turn.
//!
//! The positional `<prompt>` argument is ignored in this mode (matching
//! Claude Code); the first user message must arrive as a stdin frame.

use std::sync::Arc;
use std::time::Instant;

use claurst_core::cost::CostTracker;
use claurst_core::types::Message;
use claurst_query::{QueryEvent, QueryOutcome};
use claurst_tools::{Tool, ToolContext};
use tokio::io::{AsyncBufReadExt, BufReader};

/// Everything a stream-loop run needs from `main`. Bundled because the loop
/// re-runs the query pipeline once per user frame.
pub struct StreamLoopParams {
    pub client: Arc<claurst_api::AnthropicClient>,
    pub tools: Arc<Vec<Box<dyn Tool>>>,
    pub tool_ctx: ToolContext,
    pub query_config: claurst_query::QueryConfig,
    pub cost_tracker: Arc<CostTracker>,
    /// Effective model id, echoed in the `system.init` frame.
    pub model: String,
    /// `--resume <id>`: load this session's transcript before the first turn.
    pub resume_id: Option<String>,
}

/// A user/assistant frame parsed off stdin.
enum InputFrame {
    User(String),
    AssistantPrefill(String),
    Ignored,
}

fn parse_input_frame(line: &str) -> Result<InputFrame, String> {
    let v: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("malformed JSON frame: {e}"))?;

    // Claude/Coven shape: {"type":"user","message":{"role":...,"content":...}}
    if let Some(kind) = v.get("type").and_then(|t| t.as_str()) {
        return match kind {
            "user" | "assistant" => {
                let msg = v
                    .get("message")
                    .ok_or_else(|| format!("`{kind}` frame missing `message`"))?;
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or(kind);
                let text = content_text(msg.get("content"));
                if role == "assistant" {
                    Ok(InputFrame::AssistantPrefill(text))
                } else {
                    Ok(InputFrame::User(text))
                }
            }
            // Control/other frames are not part of the turn protocol.
            _ => Ok(InputFrame::Ignored),
        };
    }

    // Legacy coven-code shape: {"role":"user"|"assistant","content":"..."}
    let role = v.get("role").and_then(|r| r.as_str()).unwrap_or("user");
    let text = content_text(v.get("content"));
    if role == "assistant" {
        Ok(InputFrame::AssistantPrefill(text))
    } else {
        Ok(InputFrame::User(text))
    }
}

/// Extract prompt text from a `content` value that is either a plain string
/// or an array of `{"type":"text","text":...}` blocks.
fn content_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                    b.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn emit(frame: &serde_json::Value) {
    use std::io::Write;
    println!("{frame}");
    std::io::stdout().flush().ok();
}

fn assistant_frame(session_id: &str, content: serde_json::Value, stop_reason: Option<&str>) {
    emit(&serde_json::json!({
        "type": "assistant",
        "message": { "role": "assistant", "content": content },
        "session_id": session_id,
        "stop_reason": stop_reason,
    }));
}

/// Run the persistent stream-json loop. Returns when stdin reaches EOF.
/// Per-turn model errors are reported as `result` frames with
/// `is_error: true` and the loop keeps serving; only I/O-level failures
/// (broken stdin) abort.
pub async fn run_stream_loop(params: StreamLoopParams) -> anyhow::Result<()> {
    let StreamLoopParams {
        client,
        tools,
        mut tool_ctx,
        query_config,
        cost_tracker,
        model,
        resume_id,
    } = params;

    // Resolve the session: --resume loads an existing transcript and keeps
    // its id; otherwise start fresh under the pre-assigned id
    // (--session-id, already threaded into tool_ctx.session_id by main).
    let resume_id = if resume_id.as_deref() == Some("__last__") {
        claurst_core::history::list_sessions()
            .await
            .first()
            .map(|s| s.id.clone())
    } else {
        resume_id
    };
    let mut session = if let Some(ref id) = resume_id {
        match claurst_core::history::load_session(id).await {
            Ok(s) => {
                if let Some(dir) = s.working_dir.as_ref() {
                    let saved = std::path::PathBuf::from(dir);
                    if saved.exists() {
                        tool_ctx.working_dir = saved;
                    }
                }
                tool_ctx.session_id = s.id.clone();
                s
            }
            Err(e) => {
                eprintln!("Warning: could not load session {id}: {e}. Starting a new session.");
                new_session(&tool_ctx, &model)
            }
        }
    } else {
        new_session(&tool_ctx, &model)
    };
    let session_id = session.id.clone();

    emit(&serde_json::json!({
        "type": "system",
        "subtype": "init",
        "cwd": tool_ctx.working_dir.display().to_string(),
        "session_id": session_id,
        "tools": tools.iter().map(|t| t.name()).collect::<Vec<_>>(),
        "agent_mode": serde_json::Value::Null,
        "model": model,
        "permission": serde_json::Value::Null,
    }));

    let mut messages: Vec<Message> = session.messages.clone();
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break; // stdin closed: the daemon ended the chat.
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let prompt = match parse_input_frame(trimmed) {
            Ok(InputFrame::User(text)) if !text.trim().is_empty() => text,
            Ok(InputFrame::User(_)) => continue,
            Ok(InputFrame::AssistantPrefill(text)) => {
                if !text.is_empty() {
                    messages.push(Message::assistant(text));
                }
                continue;
            }
            Ok(InputFrame::Ignored) => continue,
            Err(e) => {
                eprintln!("Warning: skipping frame: {e}");
                continue;
            }
        };

        messages.push(Message::user(prompt));
        messages = run_turn(
            messages,
            &client,
            &tools,
            &tool_ctx,
            &query_config,
            &cost_tracker,
            &session_id,
        )
        .await;

        // Persist so a later relaunch with --resume <id> continues this chat.
        session.messages = messages.clone();
        session.updated_at = chrono::Utc::now();
        if session.title.is_none() {
            session.title = messages
                .iter()
                .find(|m| m.role == claurst_core::types::Role::User)
                .map(|m| m.get_all_text().chars().take(80).collect());
        }
        if let Err(e) = claurst_core::history::save_session(&session).await {
            eprintln!("Warning: failed to persist session {session_id}: {e}");
        }
    }

    Ok(())
}

fn new_session(tool_ctx: &ToolContext, model: &str) -> claurst_core::history::ConversationSession {
    let mut session = claurst_core::history::ConversationSession::new(model.to_string());
    session.id = tool_ctx.session_id.clone();
    session.working_dir = Some(tool_ctx.working_dir.display().to_string());
    session
}

/// Run one chat turn: feed `messages` through the query loop, translating
/// query events into stream-json frames. Returns the updated transcript
/// (the query loop appends assistant/tool messages in place).
async fn run_turn(
    messages: Vec<Message>,
    client: &Arc<claurst_api::AnthropicClient>,
    tools: &Arc<Vec<Box<dyn Tool>>>,
    tool_ctx: &ToolContext,
    query_config: &claurst_query::QueryConfig,
    cost_tracker: &Arc<CostTracker>,
    session_id: &str,
) -> Vec<Message> {
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    let started = Instant::now();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<QueryEvent>();
    let cancel = CancellationToken::new();

    let client_clone = client.clone();
    let tools_clone = tools.clone();
    let tool_ctx_clone = tool_ctx.clone();
    let qcfg = query_config.clone();
    let tracker = cost_tracker.clone();

    let handle = tokio::spawn(async move {
        let mut msgs = messages;
        let outcome = claurst_query::run_query_loop(
            client_clone.as_ref(),
            &mut msgs,
            tools_clone.as_slice(),
            &tool_ctx_clone,
            &qcfg,
            tracker,
            Some(event_tx),
            cancel,
            None,
        )
        .await;
        (outcome, msgs)
    });

    let mut full_text = String::new();
    let mut num_turns: u32 = 0;
    while let Some(event) = event_rx.recv().await {
        match &event {
            QueryEvent::Stream(claurst_api::AnthropicStreamEvent::ContentBlockDelta {
                delta: claurst_api::streaming::ContentDelta::TextDelta { text },
                ..
            }) => full_text.push_str(text),
            QueryEvent::ToolStart {
                tool_name,
                tool_id,
                input_json,
            } => {
                let input: serde_json::Value =
                    serde_json::from_str(input_json).unwrap_or(serde_json::Value::Null);
                assistant_frame(
                    session_id,
                    serde_json::json!([{
                        "type": "tool_use",
                        "id": tool_id,
                        "name": tool_name,
                        "input": input,
                    }]),
                    None,
                );
            }
            QueryEvent::ToolEnd {
                tool_id,
                result,
                is_error,
                ..
            } => {
                emit(&serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_id,
                    "content": [{ "type": "text", "text": result }],
                    "is_error": is_error,
                    "session_id": session_id,
                }));
            }
            QueryEvent::TurnComplete { turn, .. } => num_turns = (*turn).max(num_turns),
            QueryEvent::Error(msg) => {
                eprintln!("Error: {msg}");
            }
            _ => {}
        }
    }

    let (outcome, msgs) = match handle.await {
        Ok(pair) => pair,
        Err(e) => (
            QueryOutcome::Error(claurst_core::error::ClaudeError::Other(format!(
                "query task panicked: {e}"
            ))),
            Vec::new(),
        ),
    };

    let (subtype, stop_reason, is_error, error): (&str, &str, bool, Option<String>) = match &outcome
    {
        QueryOutcome::EndTurn { .. } => ("success", "end_turn", false, None),
        QueryOutcome::MaxTokens { .. } => ("success", "max_tokens", false, None),
        QueryOutcome::Cancelled => (
            "error_cancelled",
            "end_turn",
            true,
            Some("cancelled".to_string()),
        ),
        QueryOutcome::BudgetExceeded {
            cost_usd,
            limit_usd,
        } => (
            "error_budget_exceeded",
            "end_turn",
            true,
            Some(format!(
                "budget limit ${limit_usd:.4} reached (spent ${cost_usd:.4})"
            )),
        ),
        QueryOutcome::Error(e) => (
            "error_during_execution",
            "end_turn",
            true,
            Some(e.to_string()),
        ),
    };

    let final_text = if full_text.is_empty() {
        match &outcome {
            QueryOutcome::EndTurn { message, .. } => message.get_all_text(),
            QueryOutcome::MaxTokens {
                partial_message, ..
            } => partial_message.get_all_text(),
            _ => String::new(),
        }
    } else {
        full_text
    };
    if !final_text.is_empty() {
        assistant_frame(
            session_id,
            serde_json::json!([{ "type": "text", "text": final_text }]),
            Some(stop_reason),
        );
    }

    emit(&serde_json::json!({
        "type": "result",
        "subtype": subtype,
        "duration_ms": started.elapsed().as_millis() as u64,
        "is_error": is_error,
        "num_turns": num_turns,
        "session_id": session_id,
        "error": error,
    }));

    msgs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_shape_user_frame_with_block_content() {
        let frame = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hello"},{"type":"text","text":"world"}]},"session_id":"s1"}"#;
        match parse_input_frame(frame) {
            Ok(InputFrame::User(text)) => assert_eq!(text, "hello\nworld"),
            other => panic!("expected user frame, got {:?}", frame_kind(&other)),
        }
    }

    #[test]
    fn parses_claude_shape_user_frame_with_string_content() {
        let frame = r#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        match parse_input_frame(frame) {
            Ok(InputFrame::User(text)) => assert_eq!(text, "hi"),
            other => panic!("expected user frame, got {:?}", frame_kind(&other)),
        }
    }

    #[test]
    fn parses_legacy_shape_frames() {
        match parse_input_frame(r#"{"role":"user","content":"hi"}"#) {
            Ok(InputFrame::User(text)) => assert_eq!(text, "hi"),
            other => panic!("expected user frame, got {:?}", frame_kind(&other)),
        }
        match parse_input_frame(r#"{"role":"assistant","content":"pre"}"#) {
            Ok(InputFrame::AssistantPrefill(text)) => assert_eq!(text, "pre"),
            other => panic!("expected prefill frame, got {:?}", frame_kind(&other)),
        }
    }

    #[test]
    fn ignores_unknown_typed_frames() {
        match parse_input_frame(r#"{"type":"control_request","request":{}}"#) {
            Ok(InputFrame::Ignored) => {}
            other => panic!("expected ignored frame, got {:?}", frame_kind(&other)),
        }
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_input_frame("{not json").is_err());
    }

    fn frame_kind(frame: &Result<InputFrame, String>) -> &'static str {
        match frame {
            Ok(InputFrame::User(_)) => "user",
            Ok(InputFrame::AssistantPrefill(_)) => "assistant_prefill",
            Ok(InputFrame::Ignored) => "ignored",
            Err(_) => "error",
        }
    }
}
