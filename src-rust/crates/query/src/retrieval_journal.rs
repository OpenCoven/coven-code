//! Keep typed retrieval facts across transient model-context truncation.
//!
//! This is an in-memory projection of existing result blocks, not another
//! session format or a page-text parser. Only the small record is retained;
//! a budgeted page body stays replaced by its context-budget placeholder.

use claurst_core::types::{ContentBlock, Message, MessageContent, Role, ToolResultContent};
use claurst_tools::web_fetch::Retrieval;
use std::collections::HashMap;

const CONTEXT_PLACEHOLDER: &str = "[tool result truncated to save context]";

#[derive(Default)]
pub(super) struct RetrievalJournal {
    // Assistant attempt UUID disambiguates reused provider call ids. An
    // ambiguous historical entry with no UUID is never guessed into a call.
    records: HashMap<(Option<String>, String), Option<Retrieval>>,
}

struct CallOwner {
    assistant_uuid: Option<String>,
    web_fetch: bool,
}

fn register_calls(message: &Message, owners: &mut HashMap<String, CallOwner>) {
    if message.role != Role::Assistant {
        return;
    }
    for block in message.get_tool_use_blocks() {
        if let ContentBlock::ToolUse { id, name, .. } = block {
            owners.insert(
                id.clone(),
                CallOwner {
                    assistant_uuid: message.uuid.clone(),
                    web_fetch: name == claurst_core::constants::TOOL_NAME_WEB_FETCH,
                },
            );
        }
    }
}

impl RetrievalJournal {
    pub(super) fn capture(&mut self, messages: &[Message]) {
        let mut owners = HashMap::new();
        for message in messages {
            register_calls(message, &mut owners);
            if message.role != Role::User {
                continue;
            }
            for block in message.get_tool_result_blocks() {
                let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } = block
                else {
                    continue;
                };
                let Some(owner) = owners.get(tool_use_id).filter(|owner| owner.web_fetch) else {
                    continue;
                };
                if is_error == &Some(true) {
                    continue;
                }
                let Some(record) = Retrieval::from_content(content) else {
                    continue;
                };
                let key = (owner.assistant_uuid.clone(), tool_use_id.clone());
                self.records
                    .entry(key)
                    .and_modify(|existing| {
                        if existing.as_ref() != Some(&record) {
                            *existing = None;
                        }
                    })
                    .or_insert(Some(record));
            }
        }
    }

    pub(super) fn restore(&self, messages: &mut [Message]) {
        let mut owners = HashMap::new();
        for message in messages {
            register_calls(message, &mut owners);
            if message.role != Role::User {
                continue;
            }
            let MessageContent::Blocks(blocks) = &mut message.content else {
                continue;
            };
            for block in blocks {
                let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } = block
                else {
                    continue;
                };
                if *is_error == Some(true)
                    || !matches!(content, ToolResultContent::Text(text) if text == CONTEXT_PLACEHOLDER)
                {
                    continue;
                }
                let Some(owner) = owners.get(tool_use_id).filter(|owner| owner.web_fetch) else {
                    continue;
                };
                let key = (owner.assistant_uuid.clone(), tool_use_id.clone());
                let Some(Some(record)) = self.records.get(&key) else {
                    continue;
                };
                match record.with_body(CONTEXT_PLACEHOLDER.to_owned()) {
                    Ok(restored) => *content = restored,
                    Err(_) => tracing::warn!("Could not restore WebFetch retrieval record"),
                }
            }
        }
    }
}
