// providers/anthropic.rs — AnthropicProvider: wraps AnthropicClient in the
// unified LlmProvider trait.
//
// Phase 2A: create_message and create_message_stream are fully implemented by
// mapping ProviderRequest → CreateMessageRequest and mapping
// AnthropicStreamEvent → provider_types::StreamEvent.

use std::pin::Pin;
use std::sync::Arc;

use async_stream::stream;
use async_trait::async_trait;
use claurst_core::provider_id::{ModelId, ProviderId};
use futures::Stream;

use crate::client::{AnthropicClient, ClientConfig};
use crate::provider::{LlmProvider, ModelInfo};
use crate::provider_error::ProviderError;
use crate::provider_types::{
    ProviderCapabilities, ProviderRequest, ProviderResponse, ProviderStatus, StopReason,
    StreamEvent, SystemPromptStyle,
};
use crate::streaming::{AnthropicStreamEvent, ContentDelta, NullStreamHandler};
use crate::types::{ApiMessage, ApiToolDefinition, CreateMessageRequest, ThinkingConfig};
use crate::StreamAccumulator;

use super::message_normalization::normalize_anthropic_messages;

/// The static Claude model catalog, shared by the HTTP provider and the
/// claude-CLI provider (the Anthropic API exposes no /models route here).
pub(crate) fn claude_model_catalog() -> Vec<ModelInfo> {
    let anthropic_id = ProviderId::new(ProviderId::ANTHROPIC);
    vec![
        ModelInfo {
            id: ModelId::new("claude-fable-5"),
            provider_id: anthropic_id.clone(),
            name: "Claude Fable 5".to_string(),
            context_window: 1_000_000,
            max_output_tokens: 128_000,
        },
        ModelInfo {
            id: ModelId::new("claude-opus-4-8"),
            provider_id: anthropic_id.clone(),
            name: "Claude Opus 4.8".to_string(),
            context_window: 1_000_000,
            max_output_tokens: 128_000,
        },
        ModelInfo {
            id: ModelId::new("claude-opus-4-6"),
            provider_id: anthropic_id.clone(),
            name: "Claude Opus 4.6".to_string(),
            context_window: 200_000,
            max_output_tokens: 32_000,
        },
        ModelInfo {
            id: ModelId::new("claude-sonnet-4-6"),
            provider_id: anthropic_id.clone(),
            name: "Claude Sonnet 4.6".to_string(),
            context_window: 200_000,
            max_output_tokens: 16_000,
        },
        ModelInfo {
            id: ModelId::new("claude-haiku-4-5-20251001"),
            provider_id: anthropic_id,
            name: "Claude Haiku 4.5".to_string(),
            context_window: 200_000,
            max_output_tokens: 8_096,
        },
    ]
}

// ---------------------------------------------------------------------------
// AnthropicProvider
// ---------------------------------------------------------------------------

/// Wraps [`AnthropicClient`] so it can be held in a [`ProviderRegistry`] behind
/// `Arc<dyn LlmProvider>`.
pub struct AnthropicProvider {
    client: Arc<AnthropicClient>,
    id: ProviderId,
}

impl AnthropicProvider {
    /// Wrap an already-constructed (and Arc-wrapped) [`AnthropicClient`].
    pub fn new(client: Arc<AnthropicClient>) -> Self {
        Self {
            client,
            id: ProviderId::new(ProviderId::ANTHROPIC),
        }
    }

    /// Construct directly from a [`ClientConfig`], creating the inner client.
    /// Returns an error when the underlying HTTP client cannot be built (e.g.
    /// TLS init failure); callers should treat that as "provider unavailable"
    /// rather than letting the process crash.
    pub fn from_config(config: ClientConfig) -> Result<Self, ProviderError> {
        let client = AnthropicClient::new(config).map_err(|e| ProviderError::Other {
            provider: ProviderId::new(ProviderId::ANTHROPIC),
            message: format!("failed to create AnthropicClient: {e}"),
            status: None,
            body: None,
        })?;
        Ok(Self {
            client: Arc::new(client),
            id: ProviderId::new(ProviderId::ANTHROPIC),
        })
    }

    /// Build a [`CreateMessageRequest`] from a [`ProviderRequest`].
    fn build_request(request: &ProviderRequest) -> CreateMessageRequest {
        let normalized_messages = normalize_anthropic_messages(&request.messages);
        let api_messages: Vec<ApiMessage> =
            normalized_messages.iter().map(ApiMessage::from).collect();

        let api_tools: Option<Vec<ApiToolDefinition>> = if request.tools.is_empty() {
            None
        } else {
            Some(request.tools.iter().map(ApiToolDefinition::from).collect())
        };

        let system = request.system_prompt.clone();

        let mut builder = CreateMessageRequest::builder(&request.model, request.max_tokens)
            .messages(api_messages);

        if let Some(sys) = system {
            builder = builder.system(sys);
        }
        if let Some(tools) = api_tools {
            builder = builder.tools(tools);
        }
        if !request.stop_sequences.is_empty() {
            builder = builder.stop_sequences(request.stop_sequences.clone());
        }

        // Opus 4.7+, Opus 4.8, and Fable 5 reject sampling params and manual
        // `budget_tokens` with a 400. Send adaptive thinking instead and drop
        // temperature / top_p / top_k.
        if claurst_core::effort::model_uses_adaptive_thinking(&request.model) {
            if request.thinking.is_some() {
                builder = builder.thinking(ThinkingConfig::adaptive());
            }
        } else {
            if let Some(t) = request.temperature {
                builder = builder.temperature(t as f32);
            }
            if let Some(p) = request.top_p {
                builder = builder.top_p(p as f32);
            }
            if let Some(k) = request.top_k {
                builder = builder.top_k(k);
            }
            if let Some(tc) = request.thinking.clone() {
                builder = builder.thinking(tc);
            }
        }

        builder.build()
    }

    /// Map a string stop_reason from Anthropic wire format to [`StopReason`].
    fn map_stop_reason(s: &str) -> StopReason {
        match s {
            "end_turn" => StopReason::EndTurn,
            "stop_sequence" => StopReason::StopSequence,
            "max_tokens" => StopReason::MaxTokens,
            "tool_use" => StopReason::ToolUse,
            other => StopReason::Other(other.to_string()),
        }
    }

    /// Map an [`AnthropicStreamEvent`] to the provider-agnostic [`StreamEvent`].
    fn map_stream_event(evt: AnthropicStreamEvent) -> Option<StreamEvent> {
        match evt {
            AnthropicStreamEvent::MessageStart { id, model, usage } => {
                Some(StreamEvent::MessageStart { id, model, usage })
            }
            AnthropicStreamEvent::ContentBlockStart {
                index,
                content_block,
            } => Some(StreamEvent::ContentBlockStart {
                index,
                content_block,
            }),
            AnthropicStreamEvent::ContentBlockDelta { index, delta } => match delta {
                ContentDelta::TextDelta { text } => Some(StreamEvent::TextDelta { index, text }),
                ContentDelta::ThinkingDelta { thinking } => {
                    Some(StreamEvent::ThinkingDelta { index, thinking })
                }
                ContentDelta::SignatureDelta { signature } => {
                    Some(StreamEvent::SignatureDelta { index, signature })
                }
                ContentDelta::InputJsonDelta { partial_json } => {
                    Some(StreamEvent::InputJsonDelta {
                        index,
                        partial_json,
                    })
                }
            },
            AnthropicStreamEvent::ContentBlockStop { index } => {
                Some(StreamEvent::ContentBlockStop { index })
            }
            AnthropicStreamEvent::MessageDelta { stop_reason, usage } => {
                let mapped_stop = stop_reason.as_deref().map(Self::map_stop_reason);
                Some(StreamEvent::MessageDelta {
                    stop_reason: mapped_stop,
                    usage,
                })
            }
            AnthropicStreamEvent::MessageStop => Some(StreamEvent::MessageStop),
            AnthropicStreamEvent::Error {
                error_type,
                message,
            } => Some(StreamEvent::Error {
                error_type,
                message,
            }),
            AnthropicStreamEvent::Ping => None,
        }
    }

    async fn collect_stream_response(
        provider_id: &ProviderId,
        mut stream: Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>,
    ) -> Result<ProviderResponse, ProviderError> {
        use futures::StreamExt;

        let mut accumulator = StreamAccumulator::new();
        while let Some(event) = stream.next().await {
            let event = event?;
            if let StreamEvent::Error {
                error_type,
                message,
            } = &event
            {
                let (partial, _, _) = accumulator.finish();
                let partial_response = partial.get_all_text();
                return Err(ProviderError::StreamError {
                    provider: provider_id.clone(),
                    message: format!("[{error_type}] {message}"),
                    partial_response: (!partial_response.is_empty()).then_some(partial_response),
                });
            }

            accumulator.on_provider_event(&event);
            if matches!(event, StreamEvent::MessageStop) {
                break;
            }
        }

        if !accumulator.received_message_stop() {
            let (partial, _, _) = accumulator.finish();
            let partial_response = partial.get_all_text();
            return Err(ProviderError::StreamError {
                provider: provider_id.clone(),
                message: "stream ended before MessageStop".to_string(),
                partial_response: (!partial_response.is_empty()).then_some(partial_response),
            });
        }

        let id = accumulator.message_id().unwrap_or("unknown").to_string();
        let model = accumulator.model().unwrap_or_default().to_string();
        let (message, usage, stop_reason) = accumulator.finish();

        Ok(ProviderResponse {
            id,
            content: message.content_blocks(),
            stop_reason: stop_reason
                .as_deref()
                .map(Self::map_stop_reason)
                .unwrap_or(StopReason::EndTurn),
            usage,
            model,
        })
    }
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn name(&self) -> &str {
        "Anthropic"
    }

    async fn create_message(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let stream = self.create_message_stream(request).await?;
        Self::collect_stream_response(&self.id, stream).await
    }

    async fn create_message_stream(
        &self,
        request: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        let api_request = Self::build_request(&request);
        let handler = Arc::new(NullStreamHandler);

        let provider_id = self.id.clone();

        let mut rx = self
            .client
            .create_message_stream(api_request, handler)
            .await
            .map_err(|e| ProviderError::Other {
                provider: provider_id.clone(),
                message: e.to_string(),
                status: None,
                body: None,
            })?;

        let s = stream! {
            while let Some(anthropic_evt) = rx.recv().await {
                if let Some(unified_evt) = AnthropicProvider::map_stream_event(anthropic_evt) {
                    yield Ok(unified_evt);
                }
            }
        };

        Ok(Box::pin(s))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(claude_model_catalog())
    }

    async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
        // Client was successfully constructed with a non-empty API key.
        Ok(ProviderStatus::Healthy)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            thinking: true,
            image_input: true,
            pdf_input: true,
            audio_input: false,
            video_input: false,
            caching: true,
            structured_output: true,
            system_prompt_style: SystemPromptStyle::TopLevel,
        }
    }
}

#[cfg(test)]
mod tests {
    use claurst_core::types::{ContentBlock, UsageInfo};
    use futures::stream;

    use super::*;

    fn response_stream(
        events: Vec<StreamEvent>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>> {
        Box::pin(stream::iter(events.into_iter().map(Ok)))
    }

    #[tokio::test]
    async fn collected_response_rejects_eof_without_message_stop() {
        let provider_id = ProviderId::new(ProviderId::ANTHROPIC);
        let error = AnthropicProvider::collect_stream_response(
            &provider_id,
            response_stream(vec![
                StreamEvent::MessageStart {
                    id: "premature-eof".to_string(),
                    model: "claude-test".to_string(),
                    usage: UsageInfo::default(),
                },
                StreamEvent::ContentBlockStart {
                    index: 0,
                    content_block: ContentBlock::Text {
                        text: String::new(),
                    },
                },
                StreamEvent::TextDelta {
                    index: 0,
                    text: "partial".to_string(),
                },
            ]),
        )
        .await
        .expect_err("EOF before MessageStop must not be treated as a complete response");

        assert!(matches!(
            error,
            ProviderError::StreamError {
                message,
                partial_response: Some(partial_response),
                ..
            } if message.contains("MessageStop") && partial_response == "partial"
        ));
    }

    #[tokio::test]
    async fn collected_response_preserves_interleaved_and_open_indexed_blocks() {
        let provider_id = ProviderId::new(ProviderId::ANTHROPIC);
        let response = AnthropicProvider::collect_stream_response(
            &provider_id,
            response_stream(vec![
                StreamEvent::MessageStart {
                    id: "indexed-response".to_string(),
                    model: "claude-test".to_string(),
                    usage: UsageInfo {
                        input_tokens: 10,
                        cache_creation_input_tokens: 2,
                        cache_read_input_tokens: 3,
                        ..UsageInfo::default()
                    },
                },
                StreamEvent::ContentBlockStart {
                    index: 2,
                    content_block: ContentBlock::Text {
                        text: "partial ".to_string(),
                    },
                },
                StreamEvent::ContentBlockStart {
                    index: 0,
                    content_block: ContentBlock::Thinking {
                        thinking: "thought ".to_string(),
                        signature: "sig-".to_string(),
                    },
                },
                StreamEvent::ThinkingDelta {
                    index: 0,
                    thinking: "one".to_string(),
                },
                StreamEvent::SignatureDelta {
                    index: 0,
                    signature: "one".to_string(),
                },
                StreamEvent::ContentBlockStart {
                    index: 1,
                    content_block: ContentBlock::ToolUse {
                        id: "open-tool".to_string(),
                        name: "Write".to_string(),
                        input: serde_json::json!({}),
                    },
                },
                StreamEvent::InputJsonDelta {
                    index: 1,
                    partial_json: r#"{"path":"unfinished"#.to_string(),
                },
                StreamEvent::TextDelta {
                    index: 2,
                    text: "text".to_string(),
                },
                StreamEvent::ContentBlockStart {
                    index: 3,
                    content_block: ContentBlock::Text {
                        text: "complete".to_string(),
                    },
                },
                StreamEvent::ContentBlockStop { index: 3 },
                StreamEvent::MessageDelta {
                    stop_reason: Some(StopReason::EndTurn),
                    usage: Some(UsageInfo {
                        output_tokens: 4,
                        ..UsageInfo::default()
                    }),
                },
                StreamEvent::MessageStop,
            ]),
        )
        .await
        .expect("an explicit MessageStop completes the response");

        assert_eq!(response.id, "indexed-response");
        assert_eq!(response.model, "claude-test");
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.cache_creation_input_tokens, 2);
        assert_eq!(response.usage.cache_read_input_tokens, 3);
        assert_eq!(response.usage.output_tokens, 4);
        assert!(matches!(
            response.content.as_slice(),
            [
                ContentBlock::Thinking { thinking, signature },
                ContentBlock::ToolUse { id, name, input },
                ContentBlock::Text { text: partial_text },
                ContentBlock::Text { text: completed_text },
            ] if thinking == "thought one"
                && signature == "sig-one"
                && id == "open-tool"
                && name == "Write"
                && input == &serde_json::Value::String(r#"{"path":"unfinished"#.to_string())
                && partial_text == "partial text"
                && completed_text == "complete"
        ));
    }
}
