//! `session/prompt` handler — drives the Coven Code query loop and forwards
//! every meaningful event back to the ACP client as a `session/update`
//! notification.

use std::sync::Arc;

use agent_client_protocol_schema as acp;
use claurst_api::streaming::{AnthropicStreamEvent, ContentDelta};
use claurst_core::types::Message;
use claurst_query::{QueryEvent, QueryOutcome};
use claurst_tools::ToolContext;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use crate::connection::Connection;
use crate::permission::AcpPermissionHandler;
use crate::runtime::AgentRuntime;
use crate::sessions::SessionState;

/// Handle one `session/prompt` JSON-RPC call.
///
/// Drives the full Coven Code query loop with the runtime's tools, MCP servers,
/// and provider registry, while streaming every text delta, thinking delta,
/// and tool invocation back as `session/update` notifications. Returns the
/// final `PromptResponse` with the appropriate `StopReason`.
pub async fn handle(
    runtime: Arc<AgentRuntime>,
    connection: Arc<Connection>,
    session: Arc<SessionState>,
    params: acp::PromptRequest,
) -> Result<acp::PromptResponse, acp::Error> {
    // Convert prompt content blocks → a single user message in Coven Code's
    // internal format.
    let user_text = render_prompt_blocks(&params.prompt);
    if user_text.trim().is_empty() {
        return Err(acp::Error::invalid_params());
    }

    // Append the user turn to the session transcript.
    let mut messages: Vec<Message> = {
        let guard = session.messages.lock();
        guard.clone()
    };
    messages.push(Message::user(user_text));

    // Reset the session's cancellation token for this new turn.
    let cancel = session.cancel_token.clone();

    // Build per-session ToolContext.
    let permission_handler: Arc<dyn claurst_core::PermissionHandler> =
        Arc::new(AcpPermissionHandler);
    let tool_ctx = ToolContext {
        working_dir: session.cwd.clone(),
        permission_mode: runtime.config.permission_mode.clone(),
        permission_handler,
        cost_tracker: runtime.cost_tracker.clone(),
        session_id: session.session_id.0.to_string(),
        file_history: session.file_history.clone(),
        current_turn: session.current_turn.clone(),
        non_interactive: false, // ACP routes permissions via the bridge
        mcp_manager: runtime.mcp_manager.clone(),
        config: runtime.config.clone(),
        managed_agent_config: runtime.config.managed_agents.clone(),
        completion_notifier: None,
        pending_permissions: Some(session.pending_permissions.clone()),
        permission_manager: Some(runtime.permission_manager.clone()),
        user_question_tx: None,
    };

    // Spawn the permission drainer for this turn.
    let drainer_cancel = CancellationToken::new();
    let drainer = crate::permission::spawn_drainer(
        connection.clone(),
        session.session_id.clone(),
        session.pending_permissions.clone(),
        drainer_cancel.clone(),
    );

    // Event channel + forwarder.
    let (ev_tx, ev_rx) = mpsc::unbounded_channel::<QueryEvent>();
    let forwarder = tokio::spawn(forward_events(
        connection.clone(),
        session.session_id.clone(),
        ev_rx,
    ));

    // Run the query loop.
    let outcome = claurst_query::run_query_loop(
        runtime.api_client.as_ref(),
        &mut messages,
        runtime.tools.as_slice(),
        &tool_ctx,
        &runtime.query_config,
        runtime.cost_tracker.clone(),
        Some(ev_tx),
        cancel,
        None,
    )
    .await;

    // Tear down forwarder + drainer.
    drainer_cancel.cancel();
    let _ = drainer.await;
    // Forwarder finishes when ev_tx is dropped at end of run_query_loop.
    let _ = forwarder.await;

    // Persist the updated transcript.
    {
        let mut guard = session.messages.lock();
        *guard = messages;
    }

    let stop_reason = match outcome {
        QueryOutcome::EndTurn { .. } => acp::StopReason::EndTurn,
        QueryOutcome::MaxTokens { .. } => acp::StopReason::MaxTokens,
        QueryOutcome::Cancelled => acp::StopReason::Cancelled,
        QueryOutcome::BudgetExceeded { .. } => acp::StopReason::MaxTurnRequests,
        QueryOutcome::Error(e) => {
            error!(error = ?e, "ACP: query loop errored");
            acp::StopReason::Refusal
        }
    };

    Ok(acp::PromptResponse::new(stop_reason))
}

/// Concatenate text from prompt content blocks. Image / Audio / embedded
/// resources are dropped here (they require additional prompt capabilities
/// which v1 does not advertise) but are tracked for telemetry.
fn render_prompt_blocks(blocks: &[acp::ContentBlock]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in blocks {
        match block {
            acp::ContentBlock::Text(t) => parts.push(t.text.clone()),
            acp::ContentBlock::ResourceLink(link) => {
                parts.push(format!("[resource link: {}]", link.uri));
            }
            acp::ContentBlock::Resource(res) => {
                // Best-effort: emit any embedded text.
                let json = serde_json::to_value(res).unwrap_or_default();
                if let Some(text) = json
                    .get("resource")
                    .and_then(|r| r.get("text"))
                    .and_then(|t| t.as_str())
                {
                    parts.push(text.to_string());
                }
            }
            acp::ContentBlock::Image(_) | acp::ContentBlock::Audio(_) => {
                warn!("ACP: ignoring multimedia content block (capability not advertised)");
            }
            _ => {
                warn!("ACP: ignoring unknown content block variant");
            }
        }
    }
    parts.join("\n\n")
}

/// Pump QueryEvents → `session/update` SessionNotifications.
async fn forward_events(
    connection: Arc<Connection>,
    session_id: acp::SessionId,
    mut rx: mpsc::UnboundedReceiver<QueryEvent>,
) {
    while let Some(event) = rx.recv().await {
        match event {
            QueryEvent::Stream(AnthropicStreamEvent::ContentBlockDelta { delta, .. }) => {
                match delta {
                    ContentDelta::TextDelta { text } => {
                        send_text_chunk(&connection, &session_id, &text, false).await;
                    }
                    ContentDelta::ThinkingDelta { thinking } => {
                        send_text_chunk(&connection, &session_id, &thinking, true).await;
                    }
                    _ => {}
                }
            }
            QueryEvent::ToolStart {
                tool_name,
                tool_id,
                input_json,
            } => {
                let kind = classify_tool_kind(&tool_name);
                let raw_input = serde_json::from_str::<serde_json::Value>(&input_json).ok();
                let title = tool_title(&tool_name, raw_input.as_ref());
                let mut tool_call =
                    acp::ToolCall::new(acp::ToolCallId::new(tool_id.as_str()), title)
                        .kind(kind)
                        .status(acp::ToolCallStatus::InProgress);
                if let Some(input) = raw_input {
                    tool_call = tool_call.raw_input(Some(input));
                }
                send_session_update(
                    &connection,
                    &session_id,
                    acp::SessionUpdate::ToolCall(tool_call),
                )
                .await;
            }
            QueryEvent::ToolEnd {
                tool_name: _,
                tool_id,
                result,
                is_error,
            } => {
                let status = if is_error {
                    acp::ToolCallStatus::Failed
                } else {
                    acp::ToolCallStatus::Completed
                };
                let content = vec![acp::ToolCallContent::Content(acp::Content::new(
                    acp::ContentBlock::Text(acp::TextContent::new(result.clone())),
                ))];
                let raw_output = serde_json::from_str::<serde_json::Value>(&result)
                    .ok()
                    .or_else(|| Some(serde_json::Value::String(result.clone())));
                let mut fields = acp::ToolCallUpdateFields::new()
                    .status(status)
                    .content(content);
                if let Some(out) = raw_output {
                    fields = fields.raw_output(Some(out));
                }
                let update =
                    acp::ToolCallUpdate::new(acp::ToolCallId::new(tool_id.as_str()), fields);
                send_session_update(
                    &connection,
                    &session_id,
                    acp::SessionUpdate::ToolCallUpdate(update),
                )
                .await;
            }
            QueryEvent::Error(msg) => {
                send_text_chunk(
                    &connection,
                    &session_id,
                    &format!("\n[error: {}]", msg),
                    false,
                )
                .await;
            }
            _ => {}
        }
    }
}

async fn send_text_chunk(
    connection: &Arc<Connection>,
    session_id: &acp::SessionId,
    text: &str,
    is_thought: bool,
) {
    let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(text)));
    let update = if is_thought {
        acp::SessionUpdate::AgentThoughtChunk(chunk)
    } else {
        acp::SessionUpdate::AgentMessageChunk(chunk)
    };
    send_session_update(connection, session_id, update).await;
}

async fn send_session_update(
    connection: &Arc<Connection>,
    session_id: &acp::SessionId,
    update: acp::SessionUpdate,
) {
    let notif = acp::SessionNotification::new(session_id.clone(), update);
    if let Err(e) = connection.send_notification("session/update", notif).await {
        warn!(?e, "ACP: failed to send session/update");
    } else {
        debug!("ACP: sent session/update");
    }
}

fn classify_tool_kind(tool_name: &str) -> acp::ToolKind {
    match tool_name {
        "Read" | "FileRead" => acp::ToolKind::Read,
        "Edit" | "FileEdit" | "Write" | "FileWrite" | "BatchEdit" | "ApplyPatch" => {
            acp::ToolKind::Edit
        }
        "Bash" | "Shell" | "Execute" => acp::ToolKind::Execute,
        "WebFetch" | "WebSearch" => acp::ToolKind::Fetch,
        "Glob" | "Grep" | "GlobTool" => acp::ToolKind::Search,
        "Delete" | "Rm" => acp::ToolKind::Delete,
        "Move" | "Rename" => acp::ToolKind::Move,
        "Think" | "Sequential" => acp::ToolKind::Think,
        _ => acp::ToolKind::Other,
    }
}

/// Compose a short, human-readable title for a tool call. Falls back to the
/// tool's bare name if no descriptive field is present.
fn tool_title(tool_name: &str, raw_input: Option<&serde_json::Value>) -> String {
    if let Some(input) = raw_input {
        // Prefer path-like fields for file tools.
        for key in &["file_path", "path", "filename", "url", "pattern", "command"] {
            if let Some(v) = input.get(*key).and_then(|x| x.as_str()) {
                return format!("{}: {}", tool_name, v);
            }
        }
    }
    tool_name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn interleaved_tool_events_preserve_notification_payloads() {
        let (mut reader, writer) = tokio::io::duplex(8192);
        let connection = Connection::new(writer);
        let (tx, rx) = mpsc::unbounded_channel();
        for event in [
            QueryEvent::ToolStart {
                tool_name: "Read".into(),
                tool_id: "read-1".into(),
                input_json: r#"{"file_path":"/workspace/file"}"#.into(),
            },
            QueryEvent::ToolStart {
                tool_name: "Bash".into(),
                tool_id: "shell-2".into(),
                input_json: r#"{"command":"pwd"}"#.into(),
            },
            QueryEvent::ToolEnd {
                tool_name: "Bash".into(),
                tool_id: "shell-2".into(),
                result: "{\"exit_code\":0}".into(),
                is_error: false,
            },
            QueryEvent::ToolEnd {
                tool_name: "Read".into(),
                tool_id: "read-1".into(),
                result: "Permission denied".into(),
                is_error: true,
            },
        ] {
            tx.send(event).unwrap();
        }
        drop(tx);
        let forwarder = forward_events(connection, acp::SessionId::new("session-1"), rx);
        let mut output = String::new();
        let (_, read_result) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(forwarder, reader.read_to_string(&mut output))
        })
        .await
        .expect("forwarder must drain and close output");
        read_result.unwrap();
        let actual: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let expected = [
            json!({"sessionUpdate":"tool_call", "toolCallId":"read-1",
                "title":"Read: /workspace/file", "kind":"read", "status":"in_progress",
                "rawInput":{"file_path":"/workspace/file"}}),
            json!({"sessionUpdate":"tool_call", "toolCallId":"shell-2",
                "title":"Bash: pwd", "kind":"execute", "status":"in_progress",
                "rawInput":{"command":"pwd"}}),
            json!({"sessionUpdate":"tool_call_update", "toolCallId":"shell-2",
                "status":"completed", "rawOutput":{"exit_code":0},
                "content":[{"type":"content", "content":{"type":"text", "text":"{\"exit_code\":0}"}}]}),
            json!({"sessionUpdate":"tool_call_update", "toolCallId":"read-1",
                "status":"failed", "rawOutput":"Permission denied",
                "content":[{"type":"content", "content":{"type":"text", "text":"Permission denied"}}]}),
        ]
        .map(|update| json!({"jsonrpc":"2.0", "method":"session/update",
            "params":{"sessionId":"session-1", "update":update}}));
        assert_eq!(actual, expected);
    }
}
