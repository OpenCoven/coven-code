//! Renderer-independent projection of the presentation state.
//!
//! Both renderers derive what they show from the same [`App`] state. This
//! projection names the semantic content a renderer must be able to show:
//! transcript entries, live output, tool activity, an outstanding permission
//! request and the composer draft. It holds copies for display only; it is
//! never a source of authority for permissions, tools or persistence.

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolActivity {
    pub id: String,
    pub name: String,
    pub state: ToolActivityState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPermission {
    pub tool_use_id: String,
    pub tool_name: String,
    pub description: String,
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
                let text = message.get_all_text();
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
        let tools = app
            .tool_use_blocks
            .iter()
            .map(|block| ToolActivity {
                id: block.id.clone(),
                name: block.name.clone(),
                state: match block.status {
                    ToolStatus::Running => ToolActivityState::Running,
                    ToolStatus::Done => ToolActivityState::Done,
                    ToolStatus::Error => ToolActivityState::Error,
                },
            })
            .collect();
        let permission = app
            .permission_request
            .as_ref()
            .map(|request| PendingPermission {
                tool_use_id: request.tool_use_id.clone(),
                tool_name: request.tool_name.clone(),
                description: request.description.clone(),
                input_preview: request.input_preview.clone(),
                options: request
                    .options
                    .iter()
                    .map(|option| (option.key, option.label.clone()))
                    .collect(),
                selected_option: request.selected_option,
            });
        Self {
            entries,
            live_text: app.streaming_text.clone(),
            is_streaming: app.is_streaming,
            tools,
            permission,
            status: app.status_message.clone(),
            draft: app.prompt_input.text.clone(),
            draft_cursor: app.prompt_input.cursor,
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
