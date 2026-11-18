//! Additive run-budget v1 metadata inside the engine contract v1 envelope.
use anyhow::Context;
use claurst_core::history::ConversationSession;
use claurst_core::types::Message;
use claurst_query::{EffectiveTurnBudget, QueryConfig, QueryOutcome, TurnBudgetSource};
use claurst_tools::ToolContext;
use serde::Serialize;

pub fn capabilities() -> serde_json::Value {
    serde_json::json!({
        "engine_contract_version": 1,
        "run_budget_version": 1,
        "flag": "--max-turns",
        "minimum": 1,
        "maximum": u32::MAX,
        "default": claurst_core::constants::MAX_TURNS_DEFAULT,
        "exhausted_outcome": "turn_budget_exhausted",
        "one_shot_exhausted_exit_code": 1,
        "stream_eof_exit_code": 0,
    })
}

#[derive(Serialize)]
pub struct BudgetReceipt {
    max_turns: std::num::NonZeroU32,
    source: TurnBudgetSource,
    consumed: u32,
}

#[derive(Serialize)]
pub struct Continuation {
    session_id: String,
    pub resumable: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalOutcome {
    TurnBudgetExhausted,
}

#[derive(Serialize)]
pub struct BudgetMetadata {
    run_budget_version: u32,
    turn_budget: BudgetReceipt,
    pub continuation: Continuation,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<LocalOutcome>,
}

impl BudgetMetadata {
    pub fn new(
        budget: EffectiveTurnBudget,
        consumed: u32,
        outcome: &QueryOutcome,
        session_id: &str,
        saved: bool,
    ) -> Self {
        Self {
            run_budget_version: 1,
            turn_budget: BudgetReceipt {
                max_turns: budget.max_turns,
                source: budget.source,
                consumed,
            },
            continuation: Continuation {
                session_id: session_id.to_owned(),
                resumable: saved && !claurst_tools::workspace_scope::active(),
            },
            outcome: matches!(outcome, QueryOutcome::TurnBudgetExhausted { .. })
                .then_some(LocalOutcome::TurnBudgetExhausted),
        }
    }
}

/// The stream result remains a typed boundary until final JSON serialization.
#[derive(Serialize)]
pub struct TurnResult {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub subtype: &'static str,
    pub duration_ms: u64,
    pub is_error: bool,
    pub num_turns: u32,
    pub session_id: String,
    pub error: Option<String>,
    #[serde(flatten)]
    pub budget: BudgetMetadata,
    // A panic can leave only the pre-compaction fallback transcript. Saving
    // that fallback must not promise continuation of all unfinished work.
    #[serde(skip)]
    pub transcript_complete: bool,
}

pub fn annotate(
    frame: &mut serde_json::Value,
    budget: EffectiveTurnBudget,
    consumed: u32,
    outcome: &QueryOutcome,
    session_id: &str,
    saved: bool,
) -> anyhow::Result<()> {
    // Existing one-shot JSON output is assembled as a JSON object. Serialize
    // typed metadata here, at that output boundary, rather than transporting
    // an untyped value between the query, persistence and result layers.
    let metadata = serde_json::to_value(BudgetMetadata::new(
        budget, consumed, outcome, session_id, saved,
    ))?;
    let target = frame
        .as_object_mut()
        .context("Result frame must be an object")?;
    let metadata = metadata
        .as_object()
        .context("Budget metadata must be an object")?;
    target.extend(
        metadata
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    Ok(())
}

pub fn exhaustion_message(budget: EffectiveTurnBudget, consumed: u32) -> String {
    format!(
        "Model-round budget exhausted ({consumed}/{}); work is unfinished.",
        budget.max_turns
    )
}

/// An explicit resume must restore exactly that session or fail before work.
pub async fn session(
    tool_ctx: &mut ToolContext,
    config: &QueryConfig,
    resume: Option<&str>,
) -> anyhow::Result<ConversationSession> {
    if let Some(id) = resume {
        anyhow::ensure!(!claurst_tools::workspace_scope::active(), "This workspace-scoped run requires host-owned continuity; engine resume is unavailable.");
        let id = if id == "__last__" {
            claurst_core::history::list_sessions()
                .await
                .first()
                .map(|session| session.id.clone())
                .context("No saved session to resume")?
        } else {
            id.to_owned()
        };
        let session = claurst_core::history::load_session(&id)
            .await
            .with_context(|| format!("Could not resume session {id}"))?;
        anyhow::ensure!(
            session.id == id,
            "Saved session identity disagrees with requested resume id"
        );
        let directory = session
            .working_dir
            .as_deref()
            .context("Saved working directory is missing; engine continuation is unavailable")?;
        anyhow::ensure!(
            std::path::Path::new(directory).is_absolute(),
            "Saved working directory {directory:?} is relative; engine continuation is unavailable"
        );
        let saved_directory = std::fs::canonicalize(directory)
            .with_context(|| format!("Saved working directory {directory:?} is unavailable"))?;
        let current_directory = std::fs::canonicalize(&tool_ctx.working_dir)
            .context("Could not resolve the requested working directory")?;
        anyhow::ensure!(
            saved_directory == current_directory,
            "Resume working directory differs; saved directory is {directory:?}. Launch with --cwd pointing at that directory"
        );
        tool_ctx.session_id = session.id.clone();
        Ok(session)
    } else {
        let mut session = ConversationSession::new(config.model.clone());
        session.id = tool_ctx.session_id.clone();
        session.working_dir = Some(
            std::fs::canonicalize(&tool_ctx.working_dir)
                .context("Could not resolve the session working directory")?
                .to_string_lossy()
                .into_owned(),
        );
        session.hosted_review = tool_ctx.config.hosted_review_enabled();
        Ok(session)
    }
}

/// Save before publishing a resumable receipt; a failed save never promises it.
pub async fn persist(session: &mut ConversationSession, messages: Vec<Message>) -> bool {
    session.messages = messages;
    session.updated_at = chrono::Utc::now();
    if session.title.is_none() {
        session.title = session
            .messages
            .iter()
            .find(|message| message.role == claurst_core::types::Role::User)
            .map(|message| message.get_all_text().chars().take(80).collect());
    }
    match claurst_core::history::save_session(session).await {
        Ok(()) => true,
        Err(error) => {
            eprintln!("Warning: failed to persist session {}: {error}", session.id);
            false
        }
    }
}
