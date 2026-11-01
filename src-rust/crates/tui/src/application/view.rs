//! Renderer-independent projection of the presentation state.
//!
//! Both renderers derive what they show from the same [`App`] state. This
//! projection names the semantic content a renderer must be able to show:
//! transcript entries, live output, tool activity, an outstanding permission
//! request and the composer draft. It holds copies for display only; it is
//! never a source of authority for permissions, tools or persistence.
//!
//! Every string that came from a model, tool or user passes through
//! [`terminal_safe`], so no projection can carry a terminal control sequence
//! (clipboard writes, hyperlinks, screen clears, bidi overrides) to a
//! renderer.

use crate::app::{App, ToolStatus};
use claurst_core::types::{ContentBlock, MessageContent, Role};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub role: EntryRole,
    pub text: String,
    /// Text stand-ins for images and documents, since no renderer may
    /// depend on a terminal image protocol.
    pub attachments: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolActivityState {
    Running,
    Done,
    Error,
    /// Running when the user cancelled; the runtime has not settled yet.
    CancelRequested,
    /// Interrupted by a cancel the runtime has settled. Whether the tool's
    /// own effects stopped is not known here (see #373).
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolActivity {
    pub id: String,
    pub name: String,
    pub state: ToolActivityState,
    /// One-line, terminal-safe summary of what the call targets.
    pub target: Option<String>,
    /// Terminal-safe arguments as sent by the model.
    pub input: String,
    /// Terminal-safe output preview, once the tool has produced one.
    pub output: Option<String>,
    /// The user's permission decision for this call, when one was asked.
    pub allowed: Option<bool>,
}

/// Longest target shown on a tool row.
const TARGET_MAX_CHARS: usize = 60;

/// Replace control characters with visible stand-ins: C0 controls and DEL
/// in caret notation (`^[` for Escape), C1 controls and bidirectional
/// overrides as U+FFFD. Newlines are kept and tabs become spaces.
pub fn terminal_safe(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => safe.push('\n'),
            '\t' => safe.push_str("    "),
            '\u{7f}' => safe.push_str("^?"),
            '\u{0}'..='\u{1f}' => {
                safe.push('^');
                safe.push(char::from(b'@' + ch as u8));
            }
            '\u{80}'..='\u{9f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {
                safe.push('\u{fffd}')
            }
            _ => safe.push(ch),
        }
    }
    safe
}

/// The one-line target of a tool call: the first line of a command, or a
/// path, pattern, URL or query, shortened to [`TARGET_MAX_CHARS`].
fn tool_target(input_json: &str, project_root: Option<&std::path::Path>) -> Option<String> {
    let input: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(input_json).ok()?;
    let raw = [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ]
    .iter()
    .find_map(|key| input.get(*key).and_then(serde_json::Value::as_str))?;
    let first_line = raw.lines().next().unwrap_or_default();
    let relative = project_root
        .and_then(|root| std::path::Path::new(first_line).strip_prefix(root).ok())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| first_line.to_string());
    let safe = terminal_safe(&relative);
    if safe.chars().count() > TARGET_MAX_CHARS {
        let short: String = safe.chars().take(TARGET_MAX_CHARS - 1).collect();
        Some(format!("{short}…"))
    } else {
        Some(safe)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPermission {
    pub tool_use_id: String,
    pub tool_name: String,
    pub description: String,
    /// Why the request is risky, when the producer explained it.
    pub danger: String,
    pub input_preview: Option<String>,
    /// Option labels in display order, each with its key.
    pub options: Vec<(char, String)>,
    pub selected_option: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptView {
    pub entries: Vec<TranscriptEntry>,
    pub live_text: String,
    pub is_streaming: bool,
    pub tools: Vec<ToolActivity>,
    pub permission: Option<PendingPermission>,
    pub status: Option<String>,
    pub draft: String,
    /// Composer byte offset of the cursor in `draft`.
    pub draft_cursor: usize,
    /// Text stand-ins for attachments waiting to be sent with the draft.
    pub draft_attachments: Vec<String>,
}

/// Text stand-in for one attachment block, or `None` for other blocks.
fn attachment_label(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Image { source } => Some(format!(
            "[image {}]",
            source.media_type.as_deref().unwrap_or("of unknown type")
        )),
        ContentBlock::Document { source, title, .. } => Some(format!(
            "[document {}]",
            title
                .as_deref()
                .or(source.media_type.as_deref())
                .unwrap_or("of unknown type")
        )),
        _ => None,
    }
}

impl TranscriptView {
    pub fn project(app: &App) -> Self {
        let entries = app
            .messages
            .iter()
            .filter_map(|message| {
                let text = terminal_safe(&message.get_all_text());
                let attachments: Vec<String> = match &message.content {
                    MessageContent::Blocks(blocks) => {
                        blocks.iter().filter_map(attachment_label).collect()
                    }
                    MessageContent::Text(_) => Vec::new(),
                };
                if text.trim().is_empty() && attachments.is_empty() {
                    return None;
                }
                let role = match message.role {
                    Role::User => EntryRole::User,
                    Role::Assistant => EntryRole::Assistant,
                };
                Some(TranscriptEntry {
                    role,
                    text,
                    attachments,
                })
            })
            .collect();
        let project_root = app.config.project_dir.as_deref();
        let cancelling = app.cancellation != crate::application::TurnCancellation::Idle;
        let interrupted = app.interrupted_tools.iter().map(|block| {
            let state = if cancelling {
                ToolActivityState::CancelRequested
            } else {
                ToolActivityState::Cancelled
            };
            (block, state)
        });
        let tools = app
            .tool_use_blocks
            .iter()
            .map(|block| {
                let state = match block.status {
                    ToolStatus::Running => ToolActivityState::Running,
                    ToolStatus::Done => ToolActivityState::Done,
                    ToolStatus::Error => ToolActivityState::Error,
                };
                (block, state)
            })
            .chain(interrupted)
            .map(|(block, state)| ToolActivity {
                id: block.id.clone(),
                name: terminal_safe(&block.name),
                state,
                target: tool_target(&block.input_json, project_root),
                input: terminal_safe(&block.input_json),
                output: block.output_preview.as_deref().map(terminal_safe),
                allowed: app.permission_decisions.get(&block.id).copied(),
            })
            .collect();
        let permission = app
            .permission_request
            .as_ref()
            .map(|request| PendingPermission {
                tool_use_id: request.tool_use_id.clone(),
                tool_name: terminal_safe(&request.tool_name),
                description: terminal_safe(&request.description),
                danger: terminal_safe(&request.danger_explanation),
                input_preview: request.input_preview.as_deref().map(terminal_safe),
                options: request
                    .options
                    .iter()
                    .map(|option| (option.key, terminal_safe(&option.label)))
                    .collect(),
                selected_option: request.selected_option,
            });
        Self {
            entries,
            live_text: terminal_safe(&app.streaming_text),
            is_streaming: app.is_streaming,
            tools,
            permission,
            status: app.status_message.as_deref().map(terminal_safe),
            draft: terminal_safe(&app.prompt_input.text),
            // The cursor moves with the stand-ins drawn before it.
            draft_cursor: app
                .prompt_input
                .text
                .get(..app.prompt_input.cursor)
                .map_or(0, |before| terminal_safe(before).len()),
            draft_attachments: app
                .prompt_input
                .pending_images
                .iter()
                .map(|image| match image.dimensions {
                    Some((width, height)) => format!("[image {} {width}x{height}]", image.label),
                    None => format!("[image {}]", image.label),
                })
                .collect(),
        }
    }
}
