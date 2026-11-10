// AgentTool: spawn a sub-agent to handle a complex sub-task.
//
// Lives in cc-query (not cc-tools) to avoid a circular dependency:
//   cc-tools would need cc-query, but cc-query already needs cc-tools.
//
// The AgentTool creates a nested query loop with its own context, enabling
// the model to delegate complex work to specialized sub-agents. Each sub-agent:
//   - Runs its own agentic loop
//   - Has access to all tools (except AgentTool itself, preventing infinite recursion)
//   - Returns its final output as the tool result
//
// New capabilities (TS parity):
//   - `isolation: "worktree"` — run the agent in a dedicated git worktree so
//     file edits don't conflict with the parent checkout or sibling agents.
//   - `run_in_background: true` — fire-and-forget; returns agent_id immediately.
//     Use the `monitor` tool to check completion status/output.

use async_trait::async_trait;
use claurst_api::client::ClientConfig;
use claurst_api::{AnthropicClient, ModelRegistry, ProviderRegistry};
use claurst_core::types::Message;
use claurst_tools::{PermissionLevel, Tool, ToolContext, ToolResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::{run_query_loop, QueryConfig, QueryOutcome};

// ---------------------------------------------------------------------------
// Worktree isolation helpers
// ---------------------------------------------------------------------------

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

async fn create_worktree(git_root: &Path, agent_id: &str) -> Option<PathBuf> {
    let worktree_dir = std::env::temp_dir().join(format!("claude-agent-{}", agent_id));
    let output = tokio::process::Command::new("git")
        .args([
            "worktree",
            "add",
            "--detach",
            worktree_dir.to_str().unwrap_or_default(),
            "HEAD",
        ])
        .current_dir(git_root)
        .output()
        .await
        .ok()?;
    if output.status.success() {
        Some(worktree_dir)
    } else {
        warn!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        None
    }
}

async fn remove_worktree(git_root: &Path, worktree_dir: &Path) {
    let _ = tokio::process::Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            worktree_dir.to_str().unwrap_or_default(),
        ])
        .current_dir(git_root)
        .output()
        .await;
}

// ---------------------------------------------------------------------------
// AgentTool
// ---------------------------------------------------------------------------

pub struct AgentTool;

/// Execution acknowledgement, independent of Monitor's presentation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentCompletion {
    Succeeded,
    Partial,
    Cancelled,
    Failed,
}

struct BackgroundObservation {
    lease: Arc<std::fs::File>,
    cancel: CancellationToken,
    completed: tokio::sync::oneshot::Sender<AgentCompletion>,
}

/// Internal Tool adapter preserves the boxed async Tool boundary that breaks
/// recursive query futures, while transporting typed lifecycle ownership.
pub(crate) struct ObservedAgentTool {
    observation: std::sync::Mutex<Option<BackgroundObservation>>,
}

impl ObservedAgentTool {
    pub(crate) fn new(
        lease: Arc<std::fs::File>,
        cancel: CancellationToken,
    ) -> (Self, tokio::sync::oneshot::Receiver<AgentCompletion>) {
        let (completed, receiver) = tokio::sync::oneshot::channel();
        (
            Self {
                observation: std::sync::Mutex::new(Some(BackgroundObservation {
                    lease,
                    cancel,
                    completed,
                })),
            },
            receiver,
        )
    }
}

#[async_trait]
impl Tool for ObservedAgentTool {
    fn name(&self) -> &str {
        AgentTool.name()
    }
    fn description(&self) -> &str {
        AgentTool.description()
    }
    fn permission_level(&self) -> PermissionLevel {
        AgentTool.permission_level()
    }
    fn input_schema(&self) -> Value {
        AgentTool.input_schema()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let observation = match self.observation.lock() {
            Ok(mut slot) => slot.take(),
            Err(_) => return ToolResult::error("Agent lifecycle ownership is poisoned"),
        };
        let Some(observation) = observation else {
            return ToolResult::error("Observed agent can only be launched once");
        };
        AgentTool::execute_agent(input, ctx, Some(observation)).await
    }
}

async fn supervise_background<Q, C>(
    query: Q,
    cleanup: C,
    agent_id: String,
    observation: Option<BackgroundObservation>,
) where
    Q: std::future::Future<Output = QueryOutcome> + Send + 'static,
    C: std::future::Future<Output = ()> + Send,
{
    // Joining a separate query task makes its panic observable and still lets
    // this owner attempt cleanup. No abort handle is exposed to the waiter.
    let outcome = match tokio::spawn(query).await {
        Ok(outcome) => outcome,
        Err(error) => QueryOutcome::Error(claurst_core::error::ClaudeError::Api(format!(
            "Agent execution task failed: {error}"
        ))),
    };
    cleanup.await;
    let completion = match &outcome {
        QueryOutcome::EndTurn { .. } => AgentCompletion::Succeeded,
        QueryOutcome::MaxTokens { .. } => AgentCompletion::Partial,
        QueryOutcome::Cancelled => AgentCompletion::Cancelled,
        QueryOutcome::Error(_) | QueryOutcome::BudgetExceeded { .. } => AgentCompletion::Failed,
    };
    let result_text = format_outcome(&outcome);
    claurst_core::tasks::global_registry().append_output(&agent_id, &result_text);
    claurst_core::tasks::global_registry()
        .finish_if_running(&agent_id, background_status(&outcome, &result_text));
    if let Some(observer) = observation {
        // The supervisor retains its lease through the cleanup attempt and
        // bookkeeping. The caller retains another through state persistence.
        let _ = observer.completed.send(completion);
    }
}

fn build_model_registry() -> ModelRegistry {
    let mut registry = ModelRegistry::new();
    if let Some(cache_dir) = dirs::cache_dir() {
        let cache_path = cache_dir.join("coven-code").join("models_dev.json");
        registry.load_cache(&cache_path);
    }
    registry
}

fn resolve_subagent_model(params: &AgentInput, ctx: &ToolContext) -> String {
    let base_model = params
        .model
        .clone()
        .filter(|m| !m.is_empty())
        .or_else(|| {
            ctx.managed_agent_config
                .as_ref()
                .map(|c| c.executor_model.clone())
                .filter(|m| !m.is_empty())
        })
        .unwrap_or_else(|| ctx.config.effective_model().to_string());

    if base_model.contains('/') {
        base_model
    } else {
        let provider_id = ctx.config.selected_provider_id();
        if provider_id != "anthropic" {
            format!("{}/{}", provider_id, base_model)
        } else {
            base_model
        }
    }
}

#[derive(Debug, Deserialize)]
struct AgentInput {
    /// Short description of the agent's task (used for logging).
    description: String,
    /// The complete task prompt to send as the first user message.
    prompt: String,
    /// Optional: which tools to make available (defaults to all minus AgentTool).
    #[serde(default)]
    tools: Option<Vec<String>>,
    /// Optional: system prompt override for the sub-agent.
    #[serde(default)]
    system_prompt: Option<String>,
    /// Optional: max turns for the sub-agent (default 10).
    #[serde(default)]
    max_turns: Option<u32>,
    /// Optional: model override for this sub-agent.
    #[serde(default)]
    model: Option<String>,
    /// Set to "worktree" to run the agent in an isolated git worktree.
    /// Omit (or set to null) for shared working directory.
    #[serde(default)]
    isolation: Option<String>,
    /// If true, start the agent in the background and return agent_id immediately.
    /// Default: false (wait for completion).
    #[serde(default)]
    run_in_background: bool,
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_AGENT
    }

    fn description(&self) -> &str {
        "Launch a new agent to handle complex, multi-step tasks autonomously. \
         The agent runs its own agentic loop with access to tools and returns \
         its final result. Use this to delegate sub-tasks, run parallel \
         workstreams, or handle tasks that require many tool calls."
    }

    fn permission_level(&self) -> PermissionLevel {
        // The agent inherits parent permissions; no extra level required.
        PermissionLevel::None
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Short description of the agent's task (3-5 words)"
                },
                "prompt": {
                    "type": "string",
                    "description": "The complete task for the agent to perform"
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of tool names to make available. Defaults to all tools."
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Optional system prompt override for the sub-agent"
                },
                "max_turns": {
                    "type": "number",
                    "description": "Maximum number of turns for the sub-agent (default 10)"
                },
                "model": {
                    "type": "string",
                    "description": "Optional model to use for this agent"
                },
                "isolation": {
                    "type": "string",
                    "enum": ["worktree"],
                    "description": "Set to \"worktree\" to run the agent in an isolated git worktree. \
                                    Prevents file-edit conflicts when multiple agents run in parallel."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "If true, the agent starts immediately and this call returns an \
                                    agent_id without waiting for completion. Use the monitor tool \
                                    with action=status/output and task_id=agent_id. Default: false."
                }
            },
            "required": ["description", "prompt"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        AgentTool::execute_agent(input, ctx, None).await
    }
}

impl AgentTool {
    async fn execute_agent(
        input: Value,
        ctx: &ToolContext,
        observation: Option<BackgroundObservation>,
    ) -> ToolResult {
        let params: AgentInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        if observation.is_some() && !params.run_in_background {
            return ToolResult::error("Observed agent requires background execution");
        }

        let mut child_ctx = ctx.clone();
        child_ctx.goal_turn = ctx.goal_turn.for_child_work();
        let ctx = &child_ctx;
        info!(description = %params.description, "Spawning sub-agent");

        let anthropic_key = ctx.config.resolve_anthropic_api_key().unwrap_or_default();
        let anthropic_base = ctx.config.resolve_anthropic_api_base();
        let client = match AnthropicClient::new(ClientConfig {
            api_key: anthropic_key.clone(),
            api_base: anthropic_base,
            ..Default::default()
        }) {
            Ok(c) => Arc::new(c),
            Err(e) => return ToolResult::error(format!("Failed to create client: {}", e)),
        };

        let provider_registry = ProviderRegistry::from_config(
            &ctx.config,
            ClientConfig {
                api_key: anthropic_key,
                api_base: ctx.config.resolve_anthropic_api_base(),
                ..Default::default()
            },
        );
        let model_registry = Arc::new(build_model_registry());

        // Build the tool list for the sub-agent.
        // Always exclude AgentTool itself to prevent unbounded recursion.
        let all = claurst_tools::all_tools();
        let agent_tools: Vec<Box<dyn Tool>> = if let Some(ref allowed) = params.tools {
            all.into_iter()
                .filter(|t| allowed.contains(&t.name().to_string()))
                .collect()
        } else {
            all.into_iter()
                .filter(|t| t.name() != claurst_core::constants::TOOL_NAME_AGENT)
                .collect()
        };

        // Resolve model: explicit override > managed config executor model > provider default.
        let model = resolve_subagent_model(&params, ctx);

        let system_prompt = params.system_prompt.unwrap_or_else(|| {
            let mut prompt = "You are a specialized AI agent helping with a specific sub-task. \
                 Complete the task thoroughly and return your findings."
                .to_string();

            // Append plugin-contributed agent definitions so the sub-agent
            // is aware of any specialised agents declared by plugins.
            if let Some(registry) = claurst_plugins::global_plugin_registry() {
                let mut agent_defs = String::new();
                for agent_dir in registry.all_agent_paths() {
                    if let Ok(entries) = std::fs::read_dir(&agent_dir) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.extension().is_some_and(|e| e == "md") {
                                if let Ok(content) = std::fs::read_to_string(&p) {
                                    let name =
                                        p.file_stem().and_then(|s| s.to_str()).unwrap_or("agent");
                                    agent_defs.push_str(&format!(
                                        "\n\n## Agent: {}\n{}",
                                        name,
                                        content.trim()
                                    ));
                                }
                            }
                        }
                    }
                }
                if !agent_defs.is_empty() {
                    prompt.push_str("\n\nThe following specialized agents are available:");
                    prompt.push_str(&agent_defs);
                }
            }

            prompt
        });

        // Resolve max_turns: explicit > managed config executor_max_turns > default.
        let resolved_max_turns = params.max_turns.unwrap_or_else(|| {
            ctx.managed_agent_config
                .as_ref()
                .map(|c| c.executor_max_turns)
                .unwrap_or(10)
        });

        // Resolve isolation: explicit param > managed config executor_isolation.
        let resolved_isolation = params.isolation.clone().or_else(|| {
            if ctx
                .managed_agent_config
                .as_ref()
                .map(|c| c.executor_isolation)
                .unwrap_or(false)
            {
                Some("worktree".to_string())
            } else {
                None
            }
        });

        // -----------------------------------------------------------------------
        // Determine working directory - optionally isolate in a git worktree.
        // -----------------------------------------------------------------------
        let use_isolation = resolved_isolation.as_deref() == Some("worktree");
        let agent_id = uuid::Uuid::new_v4().to_string();

        let (working_dir_str, worktree_path, git_root): (String, Option<PathBuf>, Option<PathBuf>) =
            if use_isolation {
                let git_root = find_git_root(&ctx.working_dir);
                if let Some(ref root) = git_root {
                    if let Some(wt) = create_worktree(root, &agent_id).await {
                        let wd = wt.display().to_string();
                        (wd, Some(wt), git_root)
                    } else {
                        warn!(
                            agent_id = %agent_id,
                            "Worktree creation failed; running agent in shared working directory"
                        );
                        (ctx.working_dir.display().to_string(), None, None)
                    }
                } else {
                    warn!(
                        agent_id = %agent_id,
                        "No git root found; isolation=worktree ignored"
                    );
                    (ctx.working_dir.display().to_string(), None, None)
                }
            } else {
                (ctx.working_dir.display().to_string(), None, None)
            };

        let query_config = QueryConfig {
            model,
            max_tokens: claurst_core::constants::DEFAULT_MAX_TOKENS,
            max_turns: resolved_max_turns,
            system_prompt: Some(system_prompt),
            append_system_prompt: None,
            output_style: ctx.config.effective_output_style(),
            output_style_prompt: ctx.config.resolve_output_style_prompt(),
            working_directory: Some(working_dir_str),
            thinking_budget: None,
            temperature: None,
            tool_result_budget: 50_000,
            effort_level: None,
            command_queue: None,
            skill_index: None,
            max_budget_usd: None,
            fallback_model: None,
            provider_registry: Some(Arc::new(provider_registry)),
            agent_name: None,
            agent_definition: None,
            model_registry: Some(model_registry),
            managed_agents: None,
            preserve_selected_model: false,
        };
        // -----------------------------------------------------------------------
        // Background mode: spawn and return agent_id immediately.
        // -----------------------------------------------------------------------
        if params.run_in_background {
            let cancel = observation
                .as_ref()
                .map(|observer| observer.cancel.clone())
                .unwrap_or_default();
            let mut task = claurst_core::tasks::BackgroundTask::new(format!(
                "subagent: {}",
                params.description
            ))
            .with_cancel_token(cancel.clone());
            task.id = agent_id.clone();
            let _ = claurst_core::tasks::global_registry().register(task);

            let agent_tools_bg = agent_tools;

            let client_bg = client.clone();
            let ctx_bg = ctx.clone();
            let config_bg = query_config.clone();
            let cost_tracker_bg = ctx.cost_tracker.clone();
            let description_bg = params.description.clone();
            let prompt_bg = params.prompt.clone();
            let agent_id_bg = agent_id.clone();

            let execution_lease = observation
                .as_ref()
                .map(|observer| Arc::clone(&observer.lease));
            let query = async move {
                let _execution_lease = execution_lease;
                let mut messages = vec![Message::user(prompt_bg)];
                run_query_loop(
                    client_bg.as_ref(),
                    &mut messages,
                    &agent_tools_bg,
                    &ctx_bg,
                    &config_bg,
                    cost_tracker_bg,
                    None,
                    cancel,
                    None,
                )
                .await
            };
            let cleanup = async move {
                if let (Some(root), Some(wt)) = (git_root, worktree_path) {
                    remove_worktree(&root, &wt).await;
                }
            };
            tokio::spawn(supervise_background(
                query,
                cleanup,
                agent_id_bg,
                observation,
            ));
            debug!(description = %description_bg, "Background agent launched");

            return ToolResult::success(
                    serde_json::json!({
                        "agent_id": agent_id,
                        "status": "running",
                        "message": format!(
                            "Agent '{}' started in background. Use monitor with action=status/output and task_id='{}'.",
                            params.description, agent_id
                        )
                    })
                    .to_string(),
                );
        }

        // -----------------------------------------------------------------------
        // Synchronous mode: run the sub-agent loop and wait for completion.
        // -----------------------------------------------------------------------
        let mut messages = vec![Message::user(params.prompt)];
        let cancel = CancellationToken::new();

        let outcome = run_query_loop(
            client.as_ref(),
            &mut messages,
            &agent_tools,
            ctx,
            &query_config,
            ctx.cost_tracker.clone(),
            None, // no event forwarding for sub-agents
            cancel,
            None, // no pending message queue for sub-agents
        )
        .await;

        // Cleanup worktree if one was created.
        if let (Some(root), Some(wt)) = (git_root, worktree_path) {
            remove_worktree(&root, &wt).await;
        }

        match outcome {
            QueryOutcome::EndTurn { message, usage } => {
                let text = message.get_all_text();
                debug!(
                    description = %params.description,
                    output_tokens = usage.output_tokens,
                    "Sub-agent completed"
                );
                ToolResult::success(text)
            }
            QueryOutcome::MaxTokens {
                partial_message, ..
            } => {
                let text = partial_message.get_all_text();
                ToolResult::success(format!("{}\n\n[Note: Agent hit max_tokens limit]", text))
            }
            QueryOutcome::Cancelled => ToolResult::error("Sub-agent was cancelled".to_string()),
            QueryOutcome::Error(e) => ToolResult::error(format!("Sub-agent error: {}", e)),
            QueryOutcome::BudgetExceeded {
                cost_usd,
                limit_usd,
            } => ToolResult::error(format!(
                "Sub-agent stopped: budget ${:.4} exceeded (limit ${:.4})",
                cost_usd, limit_usd
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: convert a QueryOutcome into a result string for background agents
// ---------------------------------------------------------------------------

fn background_status(outcome: &QueryOutcome, result_text: &str) -> claurst_core::tasks::TaskStatus {
    use claurst_core::tasks::TaskStatus;
    match outcome {
        QueryOutcome::EndTurn { .. } | QueryOutcome::MaxTokens { .. } => TaskStatus::Completed,
        QueryOutcome::Cancelled => TaskStatus::Cancelled,
        QueryOutcome::Error(_) | QueryOutcome::BudgetExceeded { .. } => {
            TaskStatus::Failed(result_text.to_owned())
        }
    }
}

fn format_outcome(outcome: &QueryOutcome) -> String {
    match outcome {
        QueryOutcome::EndTurn { message, .. } => message.get_all_text(),
        QueryOutcome::MaxTokens {
            partial_message, ..
        } => format!(
            "{}\n\n[Note: Agent hit max_tokens limit]",
            partial_message.get_all_text()
        ),
        QueryOutcome::Cancelled => "[Agent was cancelled]".to_string(),
        QueryOutcome::Error(e) => format!("[Agent error: {}]", e),
        QueryOutcome::BudgetExceeded {
            cost_usd,
            limit_usd,
        } => format!(
            "[Agent stopped: budget ${:.4} exceeded (limit ${:.4})]",
            cost_usd, limit_usd
        ),
    }
}

// ---------------------------------------------------------------------------
// Team swarm runner injection
// ---------------------------------------------------------------------------
//
// Called once at process startup (e.g. from main.rs) to inject a real agent
// runner into cc-tools so that TeamCreateTool can spawn sub-agents via
// run_query_loop without creating a circular crate dependency.

/// Register the cc-query-backed agent runner with cc-tools.
///
/// After this call, `TeamCreateTool` will actually invoke `run_query_loop` for
/// each agent instead of returning stub output.
///
/// # Panics
/// Panics if the runner was already registered.
pub fn init_team_swarm_runner() {
    let runner: claurst_tools::AgentRunFn = Arc::new(
        |description: String,
         prompt: String,
         tools: Option<Vec<String>>,
         system: Option<String>,
         max_turns: Option<u32>,
         ctx: Arc<claurst_tools::ToolContext>| {
            // We must return a Pin<Box<dyn Future<...> + Send>>.
            Box::pin(async move {
                let mut child_ctx = (*ctx).clone();
                child_ctx.goal_turn = ctx.goal_turn.for_child_work();
                let ctx = Arc::new(child_ctx);
                let anthropic_key = ctx.config.resolve_anthropic_api_key().unwrap_or_default();
                let anthropic_base = ctx.config.resolve_anthropic_api_base();
                let client =
                    match claurst_api::AnthropicClient::new(claurst_api::client::ClientConfig {
                        api_key: anthropic_key.clone(),
                        api_base: anthropic_base,
                        ..Default::default()
                    }) {
                        Ok(c) => Arc::new(c),
                        Err(e) => {
                            return format!(
                                "[Agent '{}' failed to create client: {}]",
                                description, e
                            )
                        }
                    };

                let provider_registry = ProviderRegistry::from_config(
                    &ctx.config,
                    claurst_api::client::ClientConfig {
                        api_key: anthropic_key,
                        api_base: ctx.config.resolve_anthropic_api_base(),
                        ..Default::default()
                    },
                );
                let model_registry = Arc::new(build_model_registry());

                // Build the tool list, filtering to the allowlist if provided.
                let all = claurst_tools::all_tools();
                let agent_tools: Vec<Box<dyn claurst_tools::Tool>> =
                    if let Some(ref allowed) = tools {
                        all.into_iter()
                            .filter(|t| allowed.contains(&t.name().to_string()))
                            .collect()
                    } else {
                        all.into_iter()
                            .filter(|t| t.name() != claurst_core::constants::TOOL_NAME_AGENT)
                            .collect()
                    };

                let model = resolve_subagent_model(
                    &AgentInput {
                        description: description.clone(),
                        prompt: prompt.clone(),
                        tools: tools.clone(),
                        system_prompt: system.clone(),
                        max_turns,
                        model: None,
                        isolation: None,
                        run_in_background: false,
                    },
                    &ctx,
                );

                let system_prompt = system.unwrap_or_else(|| {
                    "You are a specialized AI agent helping with a specific sub-task. \
                     Complete the task thoroughly and return your findings."
                        .to_string()
                });

                let query_config = crate::QueryConfig {
                    model,
                    max_tokens: claurst_core::constants::DEFAULT_MAX_TOKENS,
                    max_turns: max_turns.unwrap_or(10),
                    system_prompt: Some(system_prompt),
                    working_directory: Some(ctx.working_dir.display().to_string()),
                    output_style: ctx.config.effective_output_style(),
                    output_style_prompt: ctx.config.resolve_output_style_prompt(),
                    provider_registry: Some(Arc::new(provider_registry)),
                    model_registry: Some(model_registry),
                    ..Default::default()
                };

                let cancel = tokio_util::sync::CancellationToken::new();
                let mut messages = vec![claurst_core::types::Message::user(prompt)];
                let outcome = crate::run_query_loop(
                    client.as_ref(),
                    &mut messages,
                    &agent_tools,
                    &ctx,
                    &query_config,
                    ctx.cost_tracker.clone(),
                    None,
                    cancel,
                    None,
                )
                .await;

                format_outcome(&outcome)
            }) as Pin<Box<dyn std::future::Future<Output = String> + Send>>
        },
    );

    claurst_tools::register_agent_runner(runner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_core::{error::ClaudeError, tasks::TaskStatus, types::UsageInfo};

    #[tokio::test]
    async fn completion_receipt_follows_cleanup_and_distinguishes_partial_output() {
        for partial in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let dream = crate::auto_dream::AutoDream::new(
                home.path().join("memory"),
                home.path().join("conversations"),
            );
            let lease = dream.acquire_lock().await.unwrap();
            let cancel = CancellationToken::new();
            let task = claurst_core::tasks::BackgroundTask::new("receipt fixture")
                .with_cancel_token(cancel.clone());
            let id = task.id.clone();
            claurst_core::tasks::global_registry().register(task);
            let (tx, mut rx) = tokio::sync::oneshot::channel();
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let outcome = if partial {
                QueryOutcome::MaxTokens {
                    partial_message: Message::assistant("partial"),
                    usage: UsageInfo::default(),
                }
            } else {
                QueryOutcome::EndTurn {
                    message: Message::assistant("done"),
                    usage: UsageInfo::default(),
                }
            };
            let supervisor = tokio::spawn(supervise_background(
                async move { outcome },
                async move {
                    entered_tx.send(()).unwrap();
                    release_rx.await.unwrap();
                },
                id.clone(),
                Some(BackgroundObservation {
                    lease: Arc::clone(&lease),
                    cancel,
                    completed: tx,
                }),
            ));
            entered_rx.await.unwrap();
            assert!(matches!(
                rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ));
            drop(lease);
            assert!(
                dream.acquire_lock().await.is_err(),
                "cleanup still owns exclusion"
            );
            release_tx.send(()).unwrap();
            supervisor.await.unwrap();
            assert_eq!(
                rx.await.unwrap(),
                if partial {
                    AgentCompletion::Partial
                } else {
                    AgentCompletion::Succeeded
                }
            );
            let task = claurst_core::tasks::global_registry().get(&id).unwrap();
            assert_eq!(task.status, TaskStatus::Completed);
            assert!(dream.acquire_lock().await.is_ok());
        }
    }

    #[tokio::test]
    async fn cancelled_monitor_and_dropped_waiter_do_not_release_execution_lock() {
        let home = tempfile::tempdir().unwrap();
        let dream = crate::auto_dream::AutoDream::new(
            home.path().join("memory"),
            home.path().join("conversations"),
        );
        let lease = dream.acquire_lock().await.unwrap();
        let cancel = CancellationToken::new();
        let task = claurst_core::tasks::BackgroundTask::new("cancel receipt fixture")
            .with_cancel_token(cancel.clone());
        let id = task.id.clone();
        claurst_core::tasks::global_registry().register(task);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let waiter_task = crate::auto_dream::ConsolidationTask {
            prompt: String::new(),
            memory_dir: home.path().join("memory"),
            state_file: home.path().join("memory/.consolidation_state.json"),
            lock_file: home.path().join("memory/.consolidation_lock"),
            lock: Arc::clone(&lease),
        };
        let waiter = tokio::spawn(crate::auto_dream::AutoDream::finish_after_execution(
            waiter_task,
            rx,
            cancel.clone(),
            Default::default(),
            "s".into(),
        ));
        let (query_started_tx, query_started_rx) = tokio::sync::oneshot::channel();
        let worker_cancel = cancel.clone();
        let worker_lease = Arc::clone(&lease);
        let supervisor = tokio::spawn(supervise_background(
            async move {
                let _lease = worker_lease;
                query_started_tx.send(()).unwrap();
                worker_cancel.cancelled().await;
                QueryOutcome::Cancelled
            },
            async move {
                entered_tx.send(()).unwrap();
                release_rx.await.unwrap();
            },
            id.clone(),
            Some(BackgroundObservation {
                lease: Arc::clone(&lease),
                cancel,
                completed: tx,
            }),
        ));
        query_started_rx.await.unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        drop(lease);
        assert!(
            dream.acquire_lock().await.is_err(),
            "aborted waiter cannot unlock a live query"
        );
        claurst_core::tasks::global_registry().cancel(&id);
        entered_rx.await.unwrap();
        assert_eq!(
            claurst_core::tasks::global_registry()
                .get(&id)
                .unwrap()
                .status,
            TaskStatus::Cancelled
        );
        assert!(dream.acquire_lock().await.is_err());
        release_tx.send(()).unwrap();
        supervisor.await.unwrap();
        assert!(dream.acquire_lock().await.is_ok());
    }

    #[tokio::test]
    async fn query_panic_still_attempts_cleanup_before_failed_receipt() {
        let home = tempfile::tempdir().unwrap();
        let dream = crate::auto_dream::AutoDream::new(
            home.path().join("memory"),
            home.path().join("conversations"),
        );
        let lease = dream.acquire_lock().await.unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let cleaned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cleanup_flag = Arc::clone(&cleaned);
        supervise_background(
            async { panic!("query panic fixture") },
            async move {
                cleanup_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            },
            uuid::Uuid::new_v4().to_string(),
            Some(BackgroundObservation {
                lease,
                cancel: CancellationToken::new(),
                completed: tx,
            }),
        )
        .await;
        assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(rx.await.unwrap(), AgentCompletion::Failed);
        assert!(dream.acquire_lock().await.is_ok());
    }

    #[test]
    fn monitor_cancel_stops_background_agent_request() {
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "agent_tool::tests::background_cancel_worker",
                "--ignored",
            ])
            .env("COVEN_HOME", home.path())
            .env("COVEN_CAVE_HOME", home.path())
            .env("COVEN_CODE_HOME", home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    #[ignore = "isolated home worker invoked by parent test"]
    async fn background_cancel_worker() {
        use claurst_core::config::{Config, PermissionMode, ProviderConfig};
        use claurst_core::permissions::AutoPermissionHandler;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n")
                .await
                .unwrap();
            let _ = started_tx.send(());
            let mut byte = [0_u8; 1];
            loop {
                match stream.read(&mut byte).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = closed_tx.send(());
        });

        let directory = tempfile::tempdir().unwrap();
        let mut config = Config {
            api_key: Some("test-key".to_string()),
            permission_mode: PermissionMode::BypassPermissions,
            ..Config::default()
        };
        config.provider_configs.insert(
            "anthropic".to_string(),
            ProviderConfig {
                api_base: Some(format!("http://{addr}")),
                ..ProviderConfig::default()
            },
        );
        let ctx = ToolContext {
            working_dir: directory.path().to_path_buf(),
            permission_mode: PermissionMode::BypassPermissions,
            permission_handler: Arc::new(AutoPermissionHandler {
                mode: PermissionMode::BypassPermissions,
            }),
            cost_tracker: claurst_core::cost::CostTracker::new(),
            session_id: uuid::Uuid::new_v4().to_string(),
            goal_turn: Default::default(),
            file_history: Default::default(),
            current_turn: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: None,
            config,
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
        };
        let result = AgentTool
            .execute(
                json!({"description": "cancellation fixture", "prompt": "wait", "run_in_background": true}),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.content);
        let response: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        let id = response["agent_id"].as_str().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), started_rx)
            .await
            .expect("background agent starts request")
            .unwrap();

        let cancelled = claurst_tools::MonitorTool
            .execute(json!({"action": "cancel", "task_id": id}), &ctx)
            .await;
        assert!(!cancelled.is_error, "{}", cancelled.content);
        tokio::time::timeout(std::time::Duration::from_secs(2), closed_rx)
            .await
            .expect("monitor cancellation closes background agent request")
            .unwrap();
    }

    #[test]
    fn background_success_text_does_not_determine_status() {
        for text in [
            "done",
            "[Agent error: quoted example]",
            "[Agent stopped: quoted example]",
        ] {
            for outcome in [
                QueryOutcome::EndTurn {
                    message: Message::assistant(text),
                    usage: UsageInfo::default(),
                },
                QueryOutcome::MaxTokens {
                    partial_message: Message::assistant(text),
                    usage: UsageInfo::default(),
                },
            ] {
                let expected = match &outcome {
                    QueryOutcome::MaxTokens { .. } => {
                        format!("{text}\n\n[Note: Agent hit max_tokens limit]")
                    }
                    _ => text.to_owned(),
                };
                let output = format_outcome(&outcome);
                assert_eq!(output, expected);
                assert_eq!(background_status(&outcome, &output), TaskStatus::Completed);
            }
        }
    }

    #[test]
    fn background_cancelled_outcome_is_cancelled() {
        let outcome = QueryOutcome::Cancelled;
        let output = format_outcome(&outcome);
        assert_eq!(output, "[Agent was cancelled]");
        assert_eq!(background_status(&outcome, &output), TaskStatus::Cancelled);
    }

    #[test]
    fn background_failures_retain_formatted_payload() {
        for (outcome, expected) in [
            (
                QueryOutcome::Error(ClaudeError::Api("fixture".into())),
                "[Agent error: API error: fixture]",
            ),
            (
                QueryOutcome::BudgetExceeded {
                    cost_usd: 2.5,
                    limit_usd: 1.0,
                },
                "[Agent stopped: budget $2.5000 exceeded (limit $1.0000)]",
            ),
        ] {
            let output = format_outcome(&outcome);
            assert_eq!(output, expected);
            assert_eq!(
                background_status(&outcome, &output),
                TaskStatus::Failed(expected.to_owned())
            );
        }
    }
}
