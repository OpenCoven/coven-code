// providers/responses_input.rs — OpenAI "Responses API" input translation.
//
// Builds the `input` array for the OpenAI Responses API from a
// [`ProviderRequest`]. Used by the Codex provider, which speaks the Responses
// API over an OAuth-authenticated ChatGPT/Codex backend.

use claurst_core::types::{ContentBlock, ImageSource, MessageContent, Role, ToolResultContent};
use serde_json::{json, Value};

use crate::provider_types::{ProviderRequest, SystemPrompt};

fn system_prompt_to_text(request: &ProviderRequest) -> Option<String> {
    request.system_prompt.as_ref().map(|prompt| match prompt {
        SystemPrompt::Text(text) => text.clone(),
        SystemPrompt::Blocks(blocks) => blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn image_source_to_url(source: &ImageSource) -> String {
    if let Some(url) = &source.url {
        return url.clone();
    }
    let media_type = source.media_type.as_deref().unwrap_or("image/png");
    let data = source.data.as_deref().unwrap_or("");
    format!("data:{};base64,{}", media_type, data)
}

fn tool_result_to_response_output(content: &ToolResultContent) -> String {
    match content {
        ToolResultContent::Text(text) => text.clone(),
        ToolResultContent::Blocks(blocks) => {
            let text = blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                serde_json::to_string(blocks).unwrap_or_else(|_| "[]".to_string())
            } else {
                text
            }
        }
    }
}

fn user_block_to_responses_part(block: &ContentBlock, index: usize) -> Option<Value> {
    match block {
        ContentBlock::Text { text } => Some(json!({
            "type": "input_text",
            "text": text,
        })),
        ContentBlock::Image { source } => Some(json!({
            "type": "input_image",
            "image_url": image_source_to_url(source),
        })),
        ContentBlock::Document { source, .. }
            if source.media_type.as_deref() == Some("application/pdf") =>
        {
            if let Some(url) = &source.url {
                Some(json!({
                    "type": "input_file",
                    "file_url": url,
                }))
            } else {
                source.data.as_ref().map(|data| {
                    json!({
                        "type": "input_file",
                        "filename": format!("document-{}.pdf", index),
                        "file_data": format!("data:application/pdf;base64,{}", data),
                    })
                })
            }
        }
        _ => None,
    }
}

/// Translate a [`ProviderRequest`] into the OpenAI Responses API `input` array.
pub fn to_responses_input(request: &ProviderRequest) -> Vec<Value> {
    let mut input = Vec::new();

    if let Some(system_text) = system_prompt_to_text(request) {
        input.push(json!({
            "role": "system",
            "content": [{
                "type": "input_text",
                "text": system_text,
            }],
        }));
    }

    for message in &request.messages {
        match &message.content {
            MessageContent::Text(text) => {
                let role = match &message.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                let content_type = if matches!(&message.role, Role::Assistant) {
                    "output_text"
                } else {
                    "input_text"
                };
                input.push(json!({
                    "role": role,
                    "content": [{
                        "type": content_type,
                        "text": text,
                    }],
                }));
            }
            MessageContent::Blocks(blocks) => match &message.role {
                Role::User => {
                    let mut message_parts = Vec::new();
                    let flush_user_content = |input: &mut Vec<Value>, content: &mut Vec<Value>| {
                        if !content.is_empty() {
                            input.push(json!({
                                "role": "user",
                                "content": std::mem::take(content),
                            }));
                        }
                    };
                    for (index, block) in blocks.iter().enumerate() {
                        if let Some(part) = user_block_to_responses_part(block, index) {
                            message_parts.push(part);
                        } else if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } = block
                        {
                            flush_user_content(&mut input, &mut message_parts);
                            input.push(json!({
                                "type": "function_call_output",
                                "call_id": tool_use_id,
                                "output": tool_result_to_response_output(content),
                            }));
                        }
                    }
                    flush_user_content(&mut input, &mut message_parts);
                }
                Role::Assistant => {
                    let mut message_parts = Vec::new();
                    let flush_assistant_content =
                        |input: &mut Vec<Value>, content: &mut Vec<Value>| {
                            if !content.is_empty() {
                                input.push(json!({
                                    "role": "assistant",
                                    "content": std::mem::take(content),
                                }));
                            }
                        };
                    for block in blocks {
                        match block {
                            ContentBlock::Text { text } => message_parts.push(json!({
                                "type": "output_text",
                                "text": text,
                            })),
                            ContentBlock::ToolUse {
                                id,
                                name,
                                input: tool_input,
                            } => {
                                flush_assistant_content(&mut input, &mut message_parts);
                                input.push(json!({
                                    "type": "function_call",
                                    "call_id": id,
                                    "name": name,
                                    "arguments": serde_json::to_string(tool_input)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                }));
                            }
                            ContentBlock::Thinking { thinking, .. } if !thinking.is_empty() => {
                                flush_assistant_content(&mut input, &mut message_parts);
                                input.push(json!({
                                    "type": "reasoning",
                                    "summary": [{
                                        "type": "summary_text",
                                        "text": thinking,
                                    }],
                                }));
                            }
                            ContentBlock::RedactedThinking { data } if !data.is_empty() => {
                                flush_assistant_content(&mut input, &mut message_parts);
                                input.push(json!({
                                    "type": "reasoning",
                                    "encrypted_content": data,
                                    "summary": [],
                                }));
                            }
                            _ => {}
                        }
                    }
                    flush_assistant_content(&mut input, &mut message_parts);
                }
            },
        }
    }

    input
}
