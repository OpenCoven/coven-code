// Session Memory Extraction for cc-query.
//
// Runs a background task after a session to extract key facts worth
// remembering and persist them to AGENTS.md.
//
// This mirrors TypeScript services/SessionMemory/sessionMemory.ts and
// services/extractMemories/extractMemories.ts.
//
// Strategy:
//   1. After sessions with 20+ messages (or on compact), call the API with a
//      structured extraction prompt.
//   2. Parse the response into typed ExtractedMemory entries.
//   3. Append entries under "## Auto-extracted memories" in AGENTS.md
//      (creating the file if it doesn't exist).
//   4. Track state so we don't re-extract from already-processed messages.

use claurst_api::{
    AnthropicStreamEvent, ApiMessage, CreateMessageRequest, StreamAccumulator, StreamHandler,
    SystemPrompt,
};
use claurst_core::hosted_review::{HostedReviewConfig, MemorySourceTrust, RuntimeMode};
use claurst_core::team_memory_sync::scan_for_secrets;
use claurst_core::types::{Message, Role};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Threshold constants (mirrors TypeScript sessionMemoryUtils.ts defaults)
// ---------------------------------------------------------------------------

/// Minimum messages before extraction is attempted.
const MIN_MESSAGES_TO_EXTRACT: usize = 20;

/// Minimum tool calls since last extraction before we run again.
const MIN_TOOL_CALLS_BETWEEN_EXTRACTIONS: usize = 3;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Category of an extracted memory entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryCategory {
    UserPreference,
    ProjectFact,
    CodePattern,
    Decision,
    Constraint,
}

impl MemoryCategory {
    /// Parse from the string tag produced by the model.
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "user_preference" | "userpreference" | "preference" => Self::UserPreference,
            "project_fact" | "projectfact" | "fact" => Self::ProjectFact,
            "code_pattern" | "codepattern" | "pattern" => Self::CodePattern,
            "decision" => Self::Decision,
            "constraint" => Self::Constraint,
            _ => Self::ProjectFact, // default
        }
    }

    /// Display label used in the persisted markdown.
    fn label(&self) -> &'static str {
        match self {
            Self::UserPreference => "user-preference",
            Self::ProjectFact => "project-fact",
            Self::CodePattern => "code-pattern",
            Self::Decision => "decision",
            Self::Constraint => "constraint",
        }
    }
}

/// A single fact extracted from the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedMemory {
    /// The fact to remember, as a markdown bullet point or sentence.
    pub content: String,
    /// Semantic category for the fact.
    pub category: MemoryCategory,
    /// Model confidence, 0.0–1.0.
    pub confidence: f32,
    #[serde(default)]
    pub source_trust: MemorySourceTrust,
}

impl ExtractedMemory {
    pub fn with_source_trust(mut self, source_trust: MemorySourceTrust) -> Self {
        self.source_trust = source_trust;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryCandidateStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

/// A reviewable memory candidate written by hosted review mode instead of
/// directly mutating durable AGENTS.md memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCandidate {
    pub id: String,
    pub content: String,
    pub category: MemoryCategory,
    pub confidence: f32,
    pub provenance: String,
    pub source_trust: MemorySourceTrust,
    pub proposed_scope: String,
    pub proposed_visibility: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_expiry: Option<String>,
    pub status: MemoryCandidateStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
    pub created_at: String,
}

impl MemoryCandidate {
    fn from_memory(
        memory: &ExtractedMemory,
        provenance: &str,
        proposed_scope: &str,
        proposed_visibility: &str,
        rejection_reason: Option<&str>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content: memory.content.clone(),
            category: memory.category.clone(),
            confidence: memory.confidence,
            provenance: provenance.to_string(),
            source_trust: memory.source_trust,
            proposed_scope: proposed_scope.to_string(),
            proposed_visibility: proposed_visibility.to_string(),
            suggested_expiry: None,
            status: MemoryCandidateStatus::Pending,
            rejection_reason: rejection_reason.map(str::to_string),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn to_approved_memory(&self) -> ExtractedMemory {
        ExtractedMemory {
            content: self.content.clone(),
            category: self.category.clone(),
            confidence: self.confidence,
            source_trust: MemorySourceTrust::MaintainerApproved,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryCandidateStore {
    root: PathBuf,
}

impl MemoryCandidateStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn for_working_dir(working_dir: &Path) -> Self {
        Self::new(working_dir.join(".coven-code").join("memory-candidates"))
    }

    pub async fn write_pending(
        &self,
        memories: &[ExtractedMemory],
        provenance: &str,
        proposed_scope: &str,
        proposed_visibility: &str,
        rejection_reason: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryCandidate>> {
        if memories.is_empty() {
            return Ok(Vec::new());
        }

        fs::create_dir_all(&self.root).await?;
        let mut candidates = Vec::with_capacity(memories.len());
        for memory in memories {
            let candidate = MemoryCandidate::from_memory(
                memory,
                provenance,
                proposed_scope,
                proposed_visibility,
                rejection_reason,
            );
            self.write_candidate(&candidate).await?;
            candidates.push(candidate);
        }
        Ok(candidates)
    }

    pub async fn approve(
        &self,
        candidate_id: &str,
        target_path: &Path,
    ) -> anyhow::Result<MemoryCandidate> {
        let mut candidate = self.read_candidate(candidate_id).await?;
        candidate.status = MemoryCandidateStatus::Approved;
        candidate.source_trust = MemorySourceTrust::MaintainerApproved;
        candidate.rejection_reason = None;
        SessionMemoryExtractor::persist(&[candidate.to_approved_memory()], target_path).await?;
        self.write_candidate(&candidate).await?;
        Ok(candidate)
    }

    pub async fn reject(
        &self,
        candidate_id: &str,
        reason: impl Into<String>,
    ) -> anyhow::Result<MemoryCandidate> {
        let mut candidate = self.read_candidate(candidate_id).await?;
        candidate.status = MemoryCandidateStatus::Rejected;
        candidate.rejection_reason = Some(reason.into());
        self.write_candidate(&candidate).await?;
        Ok(candidate)
    }

    pub async fn read_candidate(&self, candidate_id: &str) -> anyhow::Result<MemoryCandidate> {
        let path = self.candidate_path(candidate_id);
        let content = fs::read_to_string(path).await?;
        Ok(serde_json::from_str(&content)?)
    }

    async fn write_candidate(&self, candidate: &MemoryCandidate) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root).await?;
        let path = self.candidate_path(&candidate.id);
        fs::write(path, serde_json::to_string_pretty(candidate)?).await?;
        Ok(())
    }

    fn candidate_path(&self, candidate_id: &str) -> PathBuf {
        self.root.join(format!("{candidate_id}.json"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryPersistenceOutcome {
    DurableWritten { count: usize },
    CandidatesWritten { count: usize, reason: String },
    Skipped,
}

// ---------------------------------------------------------------------------
// Session state (tracks what has already been extracted)
// ---------------------------------------------------------------------------

/// Mutable per-session state for the memory extractor.
#[derive(Debug, Default)]
pub struct SessionMemoryState {
    /// UUID of the last message that was fully extracted.
    pub last_extracted_message_uuid: Option<String>,
    /// Token count at the time of the last extraction.
    pub tokens_at_last_extraction: u64,
    /// Whether session memory has been initialised for this session.
    pub initialized: bool,
}

impl SessionMemoryState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return `true` if the last extracted message UUID is still present in
    /// `messages`, which tells us new messages have been added since then.
    pub fn has_new_messages_since_last_extraction(&self, messages: &[Message]) -> bool {
        match &self.last_extracted_message_uuid {
            None => true, // Nothing extracted yet → treat all messages as new
            Some(uuid) => {
                messages
                    .iter()
                    .any(|m| m.uuid.as_deref() == Some(uuid.as_str()))
                    && messages.last().and_then(|m| m.uuid.as_deref()) != Some(uuid.as_str())
            }
        }
    }

    /// Advance the cursor to the last message in `messages`.
    pub fn advance_cursor(&mut self, messages: &[Message]) {
        self.last_extracted_message_uuid = messages.last().and_then(|m| m.uuid.clone());
    }
}

// ---------------------------------------------------------------------------
// SessionMemoryExtractor
// ---------------------------------------------------------------------------

/// Extracts and persists key memories from a conversation.
pub struct SessionMemoryExtractor {
    pub model: String,
    pub max_tokens: u32,
}

impl SessionMemoryExtractor {
    /// Create a new extractor using the given model.
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            max_tokens: 4096,
        }
    }

    /// Return `true` if we have enough messages to warrant extraction.
    ///
    /// Mirrors `shouldExtractMemory` from TypeScript sessionMemory.ts:
    /// - At least `MIN_MESSAGES_TO_EXTRACT` messages total
    /// - The last assistant turn must not have pending tool calls (safe extraction point)
    pub fn should_extract(messages: &[Message]) -> bool {
        let model_visible = messages
            .iter()
            .filter(|m| m.role == Role::User || m.role == Role::Assistant)
            .count();

        if model_visible < MIN_MESSAGES_TO_EXTRACT {
            return false;
        }

        // Don't extract mid-tool-call chain — wait for a clean end-turn
        let last_assistant = messages.iter().rev().find(|m| m.role == Role::Assistant);
        if let Some(last) = last_assistant {
            if last.has_tool_use() {
                return false; // still in a tool chain
            }
        }

        true
    }

    /// Count tool calls since the last extracted message UUID.
    fn count_tool_calls_since(messages: &[Message], since_uuid: Option<&str>) -> usize {
        let mut found_start = since_uuid.is_none();
        let mut count = 0usize;

        for msg in messages {
            if !found_start {
                if msg.uuid.as_deref() == since_uuid {
                    found_start = true;
                }
                continue;
            }
            if msg.role == Role::Assistant {
                count += msg.get_tool_use_blocks().len();
            }
        }
        count
    }

    /// Check whether extraction should run given the current session state.
    pub fn should_extract_with_state(messages: &[Message], state: &SessionMemoryState) -> bool {
        if !Self::should_extract(messages) {
            return false;
        }

        // Require minimum tool calls between updates (mirrors TS toolCallsBetweenUpdates)
        let tool_calls_since =
            Self::count_tool_calls_since(messages, state.last_extracted_message_uuid.as_deref());

        tool_calls_since >= MIN_TOOL_CALLS_BETWEEN_EXTRACTIONS
            || !state.has_new_messages_since_last_extraction(messages)
    }

    /// Extract key memories from a conversation.
    ///
    /// Calls the API with a structured extraction prompt and parses the
    /// response into `ExtractedMemory` entries.
    pub async fn extract(
        &self,
        messages: &[Message],
        working_dir: &Path,
        api_client: &claurst_api::AnthropicClient,
    ) -> anyhow::Result<Vec<ExtractedMemory>> {
        let model_visible: Vec<&Message> = messages
            .iter()
            .filter(|m| m.role == Role::User || m.role == Role::Assistant)
            .collect();

        if model_visible.is_empty() {
            return Ok(vec![]);
        }

        // Build a compact transcript for the extraction prompt
        let mut transcript = String::new();
        for msg in &model_visible {
            let role_label = match msg.role {
                Role::User => "Human",
                Role::Assistant => "Assistant",
            };
            let text = msg.get_all_text();
            if !text.is_empty() {
                transcript.push_str(&format!("{}: {}\n\n", role_label, text));
            }
        }

        let working_dir_str = working_dir.display().to_string();
        let prompt = build_extraction_prompt(&transcript, &working_dir_str);

        let api_msgs = vec![ApiMessage {
            role: "user".to_string(),
            content: Value::String(prompt),
        }];

        let request = CreateMessageRequest::builder(&self.model, self.max_tokens)
            .messages(api_msgs)
            .system(SystemPrompt::Text(EXTRACTION_SYSTEM_PROMPT.to_string()))
            .build();

        let handler: Arc<dyn StreamHandler> = Arc::new(claurst_api::streaming::NullStreamHandler);
        let mut rx = api_client
            .create_message_stream(request, handler)
            .await
            .map_err(|e| anyhow::anyhow!("API error: {}", e))?;

        let mut acc = StreamAccumulator::new();
        while let Some(evt) = rx.recv().await {
            acc.on_event(&evt);
            if matches!(evt, AnthropicStreamEvent::MessageStop) {
                break;
            }
        }

        let (response_msg, _usage, _stop) = acc.finish();
        let response_text = response_msg.get_all_text();

        if response_text.is_empty() {
            debug!("Session memory extraction produced empty response");
            return Ok(vec![]);
        }

        let memories = parse_extraction_response(&response_text);
        info!(count = memories.len(), "Session memory extraction complete");

        Ok(memories)
    }

    /// Persist extracted memories to `target_path` (creates directories and
    /// the file if they don't exist).  Appends under `## Auto-extracted memories`.
    pub async fn persist(memories: &[ExtractedMemory], target_path: &Path) -> anyhow::Result<()> {
        if memories.is_empty() {
            return Ok(());
        }

        // Ensure parent directory exists
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Read existing content (or start fresh)
        let existing = match fs::read_to_string(target_path).await {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };

        // Build the new entries block
        let date_str = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut new_block = format!("\n### Session memories — {}\n\n", date_str);
        for memory in memories {
            let trust_label = if memory.source_trust.is_unknown() {
                String::new()
            } else {
                format!(", trust: {}", source_trust_label(memory.source_trust))
            };
            new_block.push_str(&format!(
                "- **[{}]** {} *(confidence: {:.0}%{})*\n",
                memory.category.label(),
                memory.content,
                memory.confidence * 100.0,
                trust_label
            ));
        }

        // Insert under the auto-extracted memories section header (or append it)
        const SECTION_HEADER: &str = "## Auto-extracted memories";

        let updated = if existing.contains(SECTION_HEADER) {
            // Find the section and append to it
            if let Some(section_pos) = existing.find(SECTION_HEADER) {
                // Find the end of the section (next ## or end of file)
                let after_header = &existing[section_pos + SECTION_HEADER.len()..];
                let section_end = after_header
                    .find("\n## ")
                    .map(|p| p + section_pos + SECTION_HEADER.len())
                    .unwrap_or(existing.len());

                let mut result = existing[..section_end].to_string();
                result.push_str(&new_block);
                result.push_str(&existing[section_end..]);
                result
            } else {
                existing
            }
        } else {
            // Append the section at the end
            let mut result = existing;
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&format!("\n{}\n", SECTION_HEADER));
            result.push_str(&new_block);
            result
        };

        fs::write(target_path, updated).await?;
        info!(path = %target_path.display(), count = memories.len(), "Memories persisted");
        Ok(())
    }

    pub async fn persist_with_policy(
        memories: &[ExtractedMemory],
        target_path: &Path,
        candidate_store: &MemoryCandidateStore,
        mode: RuntimeMode,
        hosted_config: &HostedReviewConfig,
        provenance: &str,
    ) -> anyhow::Result<MemoryPersistenceOutcome> {
        if memories.is_empty() {
            return Ok(MemoryPersistenceOutcome::Skipped);
        }

        if !mode.is_hosted_review() {
            Self::persist(memories, target_path).await?;
            return Ok(MemoryPersistenceOutcome::DurableWritten {
                count: memories.len(),
            });
        }

        let source_trust = hosted_config.memory_source_trust();
        let trusted_memories: Vec<ExtractedMemory> = memories
            .iter()
            .cloned()
            .map(|memory| memory.with_source_trust(source_trust))
            .collect();

        let secret_labels = memory_secret_labels(&trusted_memories);
        if !secret_labels.is_empty() {
            let label_text = secret_labels.join(", ");
            let redaction_required = ExtractedMemory {
                content: format!("Redaction required before memory review. Detected secret patterns: {label_text}."),
                category: MemoryCategory::Constraint,
                confidence: 1.0,
                source_trust,
            };
            let reason = format!("redaction-required:{}", label_text);
            let candidates = candidate_store
                .write_pending(
                    &[redaction_required],
                    provenance,
                    "hosted-review",
                    "durable-memory",
                    Some(&reason),
                )
                .await?;
            return Ok(MemoryPersistenceOutcome::CandidatesWritten {
                count: candidates.len(),
                reason,
            });
        }

        if hosted_config.allows_auto_memory_persistence() {
            Self::persist(&trusted_memories, target_path).await?;
            return Ok(MemoryPersistenceOutcome::DurableWritten {
                count: trusted_memories.len(),
            });
        }

        let reason = hosted_memory_candidate_reason(hosted_config);
        let candidates = candidate_store
            .write_pending(
                &trusted_memories,
                provenance,
                "hosted-review",
                "durable-memory",
                Some(&reason),
            )
            .await?;
        Ok(MemoryPersistenceOutcome::CandidatesWritten {
            count: candidates.len(),
            reason,
        })
    }
}

fn hosted_memory_candidate_reason(config: &HostedReviewConfig) -> String {
    if !config.allow_auto_memory_persistence {
        return "hosted-approval-required".to_string();
    }

    format!(
        "source-trust-below-threshold:{}<{}",
        source_trust_label(config.memory_source_trust()),
        source_trust_label(config.memory_trust_threshold())
    )
}

fn memory_secret_labels(memories: &[ExtractedMemory]) -> Vec<String> {
    let mut labels = Vec::new();
    for memory in memories {
        for finding in scan_for_secrets(&memory.content) {
            if !labels.iter().any(|label| label == &finding.label) {
                labels.push(finding.label);
            }
        }
    }
    labels
}

fn source_trust_label(trust: MemorySourceTrust) -> &'static str {
    match trust {
        MemorySourceTrust::SystemPolicy => "system-policy",
        MemorySourceTrust::MaintainerApproved => "maintainer-approved",
        MemorySourceTrust::DefaultBranchCode => "default-branch-code",
        MemorySourceTrust::ContributorInput => "contributor-input",
        MemorySourceTrust::ForkInput => "fork-input",
        MemorySourceTrust::ModelInferred => "model-inferred",
        MemorySourceTrust::Unknown => "unknown",
    }
}

// ---------------------------------------------------------------------------
// Extraction prompt
// ---------------------------------------------------------------------------

const EXTRACTION_SYSTEM_PROMPT: &str = "You are a memory extraction assistant. Your job is to \
identify key facts, preferences, patterns, and decisions from a conversation that would be \
useful to remember for future interactions. Be precise, concise, and only extract genuinely \
useful information. Do not extract trivial or transient details.";

fn build_extraction_prompt(transcript: &str, working_dir: &str) -> String {
    format!(
        "Please analyze the following conversation transcript from a coding session in \
directory `{}` and extract key memories that would be useful to remember for future \
interactions.\n\
\n\
For each memory, output a line in this exact format:\n\
MEMORY: <category> | <confidence 0-10> | <concise fact>\n\
\n\
Where <category> is one of:\n\
- user_preference: how the user likes to work, communication style, tool preferences\n\
- project_fact: facts about the codebase, architecture, languages, frameworks\n\
- code_pattern: coding patterns, idioms, or styles used in this project\n\
- decision: key decisions made during the session\n\
- constraint: constraints, requirements, or limitations discovered\n\
\n\
Only output MEMORY: lines — no other text.  If there are no useful memories, output nothing.\n\
\n\
<conversation>\n\
{}\n\
</conversation>",
        working_dir, transcript
    )
}

/// Parse the model's response into `ExtractedMemory` entries.
/// Expected line format: `MEMORY: <category> | <confidence 0-10> | <fact>`
fn parse_extraction_response(response: &str) -> Vec<ExtractedMemory> {
    let mut memories = Vec::new();

    for line in response.lines() {
        let line = line.trim();
        if !line.starts_with("MEMORY:") {
            continue;
        }

        let rest = line["MEMORY:".len()..].trim();
        let parts: Vec<&str> = rest.splitn(3, '|').collect();
        if parts.len() < 3 {
            warn!("Skipping malformed memory line: {}", line);
            continue;
        }

        let category = MemoryCategory::from_str(parts[0].trim());
        let confidence_raw: f32 = parts[1].trim().parse().unwrap_or(5.0);
        // Normalize confidence from 0-10 scale to 0.0-1.0
        let confidence = (confidence_raw / 10.0).clamp(0.0, 1.0);
        let content = parts[2].trim().to_string();

        if content.is_empty() {
            continue;
        }

        memories.push(ExtractedMemory {
            content,
            category,
            confidence,
            source_trust: MemorySourceTrust::Unknown,
        });
    }

    memories
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_core::types::Message;

    fn make_user(text: &str) -> Message {
        Message::user(text)
    }

    fn make_assistant(text: &str) -> Message {
        Message::assistant(text)
    }

    fn make_messages(n: usize) -> Vec<Message> {
        (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    make_user(&format!("user message {}", i))
                } else {
                    make_assistant(&format!("assistant reply {}", i))
                }
            })
            .collect()
    }

    // ---- should_extract ------------------------------------------------

    #[test]
    fn test_should_not_extract_too_few_messages() {
        let msgs = make_messages(5);
        assert!(!SessionMemoryExtractor::should_extract(&msgs));
    }

    #[test]
    fn test_should_extract_enough_messages() {
        let msgs = make_messages(MIN_MESSAGES_TO_EXTRACT);
        // All messages are simple text (no tool use), so the last assistant
        // doesn't have pending tool calls — should be ok to extract.
        assert!(SessionMemoryExtractor::should_extract(&msgs));
    }

    #[test]
    fn test_should_not_extract_mid_tool_chain() {
        use claurst_core::types::ContentBlock;
        let mut msgs = make_messages(MIN_MESSAGES_TO_EXTRACT);
        // Replace the last assistant message with one that has a tool_use block
        let last = msgs.last_mut().unwrap();
        *last = Message::assistant_blocks(vec![ContentBlock::ToolUse {
            id: "tool-123".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "ls"}),
        }]);
        // Last assistant has tool_use → extraction should be deferred
        assert!(!SessionMemoryExtractor::should_extract(&msgs));
    }

    // ---- parse_extraction_response -------------------------------------

    #[test]
    fn test_parse_empty_response() {
        let memories = parse_extraction_response("");
        assert!(memories.is_empty());
    }

    #[test]
    fn test_parse_single_memory() {
        let response = "MEMORY: project_fact | 8 | The project uses Rust 2021 edition";
        let memories = parse_extraction_response(response);
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].category, MemoryCategory::ProjectFact);
        assert!((memories[0].confidence - 0.8).abs() < 0.01);
        assert!(memories[0].content.contains("Rust 2021"));
    }

    #[test]
    fn test_parse_multiple_memories() {
        let response = "\
MEMORY: user_preference | 9 | User prefers verbose error messages\n\
MEMORY: decision | 7 | Decided to use tokio async runtime\n\
MEMORY: constraint | 6 | Must support Windows paths";
        let memories = parse_extraction_response(response);
        assert_eq!(memories.len(), 3);
        assert_eq!(memories[0].category, MemoryCategory::UserPreference);
        assert_eq!(memories[1].category, MemoryCategory::Decision);
        assert_eq!(memories[2].category, MemoryCategory::Constraint);
    }

    #[test]
    fn test_parse_ignores_non_memory_lines() {
        let response = "Here are the memories:\n\
MEMORY: project_fact | 8 | Uses serde for JSON\n\
This is some extra text.\n\
MEMORY: code_pattern | 7 | Uses builder pattern";
        let memories = parse_extraction_response(response);
        assert_eq!(memories.len(), 2);
    }

    #[test]
    fn test_parse_malformed_line_skipped() {
        let response = "MEMORY: only_two_parts | no_confidence";
        let memories = parse_extraction_response(response);
        assert!(memories.is_empty());
    }

    #[test]
    fn test_parse_confidence_normalization() {
        let response = "MEMORY: decision | 10 | High confidence fact";
        let memories = parse_extraction_response(response);
        assert!((memories[0].confidence - 1.0).abs() < 0.01);
    }

    // ---- MemoryCategory parsing ----------------------------------------

    #[test]
    fn test_category_from_str_variants() {
        assert_eq!(
            MemoryCategory::from_str("user_preference"),
            MemoryCategory::UserPreference
        );
        assert_eq!(
            MemoryCategory::from_str("project_fact"),
            MemoryCategory::ProjectFact
        );
        assert_eq!(
            MemoryCategory::from_str("code_pattern"),
            MemoryCategory::CodePattern
        );
        assert_eq!(
            MemoryCategory::from_str("decision"),
            MemoryCategory::Decision
        );
        assert_eq!(
            MemoryCategory::from_str("constraint"),
            MemoryCategory::Constraint
        );
    }

    #[test]
    fn test_category_unknown_defaults_to_project_fact() {
        assert_eq!(
            MemoryCategory::from_str("totally_unknown"),
            MemoryCategory::ProjectFact
        );
    }

    // ---- persist (integration-ish with tempfile) -----------------------

    #[tokio::test]
    async fn test_persist_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".coven-code").join("AGENTS.md");

        let memories = vec![ExtractedMemory {
            content: "Uses async Rust".to_string(),
            category: MemoryCategory::ProjectFact,
            confidence: 0.9,
            source_trust: MemorySourceTrust::Unknown,
        }];

        SessionMemoryExtractor::persist(&memories, &target)
            .await
            .unwrap();

        let content = fs::read_to_string(&target).await.unwrap();
        assert!(content.contains("Auto-extracted memories"));
        assert!(content.contains("Uses async Rust"));
        assert!(content.contains("project-fact"));
    }

    #[tokio::test]
    async fn test_persist_appends_to_existing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("AGENTS.md");

        // Write initial content
        fs::write(&target, "# My Project\n\nExisting content.\n")
            .await
            .unwrap();

        let memories = vec![ExtractedMemory {
            content: "Prefers explicit error handling".to_string(),
            category: MemoryCategory::UserPreference,
            confidence: 0.8,
            source_trust: MemorySourceTrust::Unknown,
        }];

        SessionMemoryExtractor::persist(&memories, &target)
            .await
            .unwrap();

        let content = fs::read_to_string(&target).await.unwrap();
        assert!(content.contains("Existing content."));
        assert!(content.contains("Auto-extracted memories"));
        assert!(content.contains("Prefers explicit error handling"));
    }

    #[tokio::test]
    async fn test_persist_appends_under_existing_section() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("AGENTS.md");

        // Pre-populate the auto-extracted section
        let initial = "# Notes\n\n## Auto-extracted memories\n\n### Old memories\n- old fact\n";
        fs::write(&target, initial).await.unwrap();

        let memories = vec![ExtractedMemory {
            content: "New fact discovered".to_string(),
            category: MemoryCategory::ProjectFact,
            confidence: 0.7,
            source_trust: MemorySourceTrust::Unknown,
        }];

        SessionMemoryExtractor::persist(&memories, &target)
            .await
            .unwrap();

        let content = fs::read_to_string(&target).await.unwrap();
        // Should have both old and new facts
        assert!(content.contains("old fact"));
        assert!(content.contains("New fact discovered"));
        // Section header should appear only once
        assert_eq!(content.matches("## Auto-extracted memories").count(), 1);
    }

    #[tokio::test]
    async fn test_persist_no_op_for_empty_memories() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("AGENTS.md");

        SessionMemoryExtractor::persist(&[], &target).await.unwrap();

        // File should NOT be created when there are no memories to persist
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn hosted_fork_session_writes_candidate_not_durable_memory() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".coven-code").join("AGENTS.md");
        let candidate_dir = dir.path().join(".coven-code").join("memory-candidates");
        let candidate_store = MemoryCandidateStore::new(&candidate_dir);
        let memories = vec![ExtractedMemory {
            content: "This fork claims auth checks are intentionally skipped".to_string(),
            category: MemoryCategory::Constraint,
            confidence: 0.9,
            source_trust: MemorySourceTrust::Unknown,
        }];
        let config = HostedReviewConfig {
            enabled: true,
            memory_source_trust: MemorySourceTrust::ForkInput,
            ..Default::default()
        };

        let outcome = SessionMemoryExtractor::persist_with_policy(
            &memories,
            &target,
            &candidate_store,
            RuntimeMode::HostedReview,
            &config,
            "session:test-session;source:session-memory-extraction",
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            MemoryPersistenceOutcome::CandidatesWritten {
                count: 1,
                reason: "hosted-approval-required".to_string()
            }
        );
        assert!(!target.exists());

        let mut entries = fs::read_dir(&candidate_dir).await.unwrap();
        let entry = entries.next_entry().await.unwrap().unwrap();
        assert!(entries.next_entry().await.unwrap().is_none());
        let candidate: MemoryCandidate =
            serde_json::from_str(&fs::read_to_string(entry.path()).await.unwrap()).unwrap();
        assert_eq!(candidate.status, MemoryCandidateStatus::Pending);
        assert_eq!(candidate.source_trust, MemorySourceTrust::ForkInput);
        assert_eq!(
            candidate.provenance,
            "session:test-session;source:session-memory-extraction"
        );
        assert_eq!(
            candidate.rejection_reason.as_deref(),
            Some("hosted-approval-required")
        );
    }

    #[tokio::test]
    async fn hosted_secret_memory_writes_redaction_candidate_without_secret_value() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".coven-code").join("AGENTS.md");
        let candidate_store = MemoryCandidateStore::for_working_dir(dir.path());
        let secret = format!("ghp_{}", "A".repeat(36));
        let memories = vec![ExtractedMemory {
            content: format!("Store token {secret} for later"),
            category: MemoryCategory::Constraint,
            confidence: 0.9,
            source_trust: MemorySourceTrust::Unknown,
        }];
        let config = HostedReviewConfig {
            enabled: true,
            allow_auto_memory_persistence: true,
            memory_source_trust: MemorySourceTrust::MaintainerApproved,
            ..Default::default()
        };

        let outcome = SessionMemoryExtractor::persist_with_policy(
            &memories,
            &target,
            &candidate_store,
            RuntimeMode::HostedReview,
            &config,
            "session:test-session;source:session-memory-extraction",
        )
        .await
        .unwrap();

        assert!(matches!(
            outcome,
            MemoryPersistenceOutcome::CandidatesWritten { .. }
        ));
        assert!(!target.exists());
        let candidate_dir = dir.path().join(".coven-code").join("memory-candidates");
        let mut entries = fs::read_dir(&candidate_dir).await.unwrap();
        let entry = entries.next_entry().await.unwrap().unwrap();
        let candidate_text = fs::read_to_string(entry.path()).await.unwrap();
        assert!(candidate_text.contains("redaction-required"));
        assert!(candidate_text.contains("GitHub personal access token"));
        assert!(!candidate_text.contains(&secret));
    }

    #[tokio::test]
    async fn hosted_maintainer_session_writes_durable_only_when_policy_allows() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".coven-code").join("AGENTS.md");
        let candidate_store = MemoryCandidateStore::for_working_dir(dir.path());
        let memories = vec![ExtractedMemory {
            content: "Maintainers require explicit error handling".to_string(),
            category: MemoryCategory::CodePattern,
            confidence: 0.8,
            source_trust: MemorySourceTrust::Unknown,
        }];
        let config = HostedReviewConfig {
            enabled: true,
            allow_auto_memory_persistence: true,
            memory_source_trust: MemorySourceTrust::MaintainerApproved,
            ..Default::default()
        };

        let outcome = SessionMemoryExtractor::persist_with_policy(
            &memories,
            &target,
            &candidate_store,
            RuntimeMode::HostedReview,
            &config,
            "session:test-session;source:session-memory-extraction",
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            MemoryPersistenceOutcome::DurableWritten { count: 1 }
        );
        let content = fs::read_to_string(&target).await.unwrap();
        assert!(content.contains("Maintainers require explicit error handling"));
        assert!(content.contains("trust: maintainer-approved"));
        assert!(!dir
            .path()
            .join(".coven-code")
            .join("memory-candidates")
            .exists());
    }

    #[tokio::test]
    async fn candidate_approval_promotes_to_durable_memory() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".coven-code").join("AGENTS.md");
        let store = MemoryCandidateStore::for_working_dir(dir.path());
        let memories = vec![ExtractedMemory {
            content: "Approved candidate fact".to_string(),
            category: MemoryCategory::ProjectFact,
            confidence: 0.7,
            source_trust: MemorySourceTrust::ContributorInput,
        }];
        let candidates = store
            .write_pending(
                &memories,
                "session-memory-extraction",
                "hosted-review",
                "durable-memory",
                Some("hosted-approval-required"),
            )
            .await
            .unwrap();

        let approved = store.approve(&candidates[0].id, &target).await.unwrap();

        assert_eq!(approved.status, MemoryCandidateStatus::Approved);
        assert_eq!(approved.source_trust, MemorySourceTrust::MaintainerApproved);
        let content = fs::read_to_string(&target).await.unwrap();
        assert!(content.contains("Approved candidate fact"));
        assert!(content.contains("trust: maintainer-approved"));
    }

    #[tokio::test]
    async fn candidate_rejection_records_reason_without_durable_write() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".coven-code").join("AGENTS.md");
        let store = MemoryCandidateStore::for_working_dir(dir.path());
        let memories = vec![ExtractedMemory {
            content: "Rejected candidate fact".to_string(),
            category: MemoryCategory::ProjectFact,
            confidence: 0.7,
            source_trust: MemorySourceTrust::ContributorInput,
        }];
        let candidates = store
            .write_pending(
                &memories,
                "session-memory-extraction",
                "hosted-review",
                "durable-memory",
                None,
            )
            .await
            .unwrap();

        let rejected = store
            .reject(&candidates[0].id, "not-repo-policy")
            .await
            .unwrap();

        assert_eq!(rejected.status, MemoryCandidateStatus::Rejected);
        assert_eq!(
            rejected.rejection_reason.as_deref(),
            Some("not-repo-policy")
        );
        assert!(!target.exists());
        let stored = store.read_candidate(&candidates[0].id).await.unwrap();
        assert_eq!(stored.status, MemoryCandidateStatus::Rejected);
        assert_eq!(stored.rejection_reason.as_deref(), Some("not-repo-policy"));
    }

    // ---- SessionMemoryState --------------------------------------------

    #[test]
    fn test_state_has_new_messages_no_cursor() {
        let state = SessionMemoryState::new();
        let msgs = vec![make_user("Hello")];
        // No cursor → always has new messages
        assert!(state.has_new_messages_since_last_extraction(&msgs));
    }

    #[test]
    fn test_state_advance_cursor() {
        let mut state = SessionMemoryState::new();
        let mut msg = make_user("hello");
        msg.uuid = Some("uuid-1".to_string());
        let msgs = vec![msg];
        state.advance_cursor(&msgs);
        assert_eq!(state.last_extracted_message_uuid.as_deref(), Some("uuid-1"));
    }
}
