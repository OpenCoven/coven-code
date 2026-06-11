// claurst-commands: Slash command system for Coven Code.
//
// This crate implements the /command framework that allows users to type
// commands like /help, /compact, /clear, /model, /config, /cost, etc.
// Each command is a struct implementing the `SlashCommand` trait.

use async_trait::async_trait;
use claurst_core::config::{Config, Settings, Theme};
use claurst_core::cost::CostTracker;
use claurst_core::types::{ContentBlock, Message};
use once_cell::sync::Lazy;
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Core trait
// ---------------------------------------------------------------------------

/// Context available to every slash command.
pub struct CommandContext {
    pub config: Config,
    pub cost_tracker: Arc<CostTracker>,
    pub messages: Vec<Message>,
    pub working_dir: std::path::PathBuf,
    pub session_id: String,
    pub session_title: Option<String>,
    /// Remote session URL set when a bridge connection is active.
    pub remote_session_url: Option<String>,
    // Note: config already contains hooks, mcp_servers, etc.
    /// Live MCP manager — present when servers are connected.
    pub mcp_manager: Option<Arc<claurst_mcp::McpManager>>,
    /// Optional callback for starting an MCP OAuth flow in the background.
    pub mcp_auth_runner: Option<Arc<dyn Fn(claurst_mcp::oauth::McpAuthSession) + Send + Sync>>,
}

/// Result of running a slash command.
#[derive(Debug)]
pub enum CommandResult {
    /// Display a message to the user (does NOT go to the model).
    Message(String),
    /// Inject a message into the conversation as though the user typed it.
    UserMessage(String),
    /// Modify the configuration.
    ConfigChange(Config),
    /// Modify the configuration and show a specific status message.
    ConfigChangeMessage(Config, String),
    /// Trigger a background MCP OAuth flow and request runtime reconnect on success.
    McpAuthFlow {
        /// The configured MCP server name.
        server_name: String,
        /// The browser URL shown to the user while the background flow runs.
        auth_url: String,
        /// The local callback URL waiting for the OAuth redirect.
        redirect_uri: String,
    },
    /// Clear the conversation.
    ClearConversation,
    /// Replace the conversation with a specific message list (used by /rewind).
    SetMessages(Vec<Message>),
    /// Load a previously saved session into the live REPL.
    ResumeSession(claurst_core::history::ConversationSession),
    /// Update the current session title.
    RenameSession(String),
    /// Trigger the OAuth login flow (handled by the REPL in main.rs).
    /// The bool indicates whether to use Claude.ai auth (true) or Console auth (false).
    StartOAuthFlow(bool),
    /// Trigger the OAuth login flow for a specific provider with optional
    /// human-friendly label for the new account profile.
    ///
    /// `provider` is one of `claurst_core::accounts::PROVIDER_ANTHROPIC` or
    /// `PROVIDER_CODEX`. `login_with_claude_ai` is only meaningful for
    /// Anthropic.
    StartLoginForProvider {
        provider: String,
        login_with_claude_ai: bool,
        label: Option<String>,
    },
    /// Exit the REPL.
    Exit,
    /// No visible output.
    Silent,
    /// An error.
    Error(String),
    /// Open the rewind/message-selector overlay in the TUI.
    /// The TUI will call SetMessages when the user confirms.
    OpenRewindOverlay,
    /// Open the hooks configuration browser overlay in the TUI.
    /// Falls back to a text listing in non-TUI contexts.
    OpenHooksOverlay,
    /// Open the import-config overlay in the TUI.
    OpenImportConfigOverlay,
    /// Clear saved provider auth, model selection, and model caches, then
    /// rebuild the live runtime state.
    RefreshProviderState,
    /// Activate a speech mode (caveman/rocky) with level, or deactivate (normal).
    /// (mode, level) — mode=None means deactivate.
    SpeechMode { mode: Option<String>, level: String },
}

/// Every slash command implements this trait.
#[async_trait]
pub trait SlashCommand: Send + Sync {
    /// The primary name (without the leading `/`).
    fn name(&self) -> &str;
    /// Alias names (e.g. `["h"]` for `/help`).
    fn aliases(&self) -> Vec<&str> {
        vec![]
    }
    /// One-line description for /help.
    fn description(&self) -> &str;
    /// Detailed help text (shown by `/help <command>`).
    fn help(&self) -> &str {
        self.description()
    }
    /// Whether this command is visible in /help output.
    fn hidden(&self) -> bool {
        false
    }
    /// Execute the command with the given arguments string.
    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult;
}

fn stripped_model_for_provider<'a>(provider_id: &str, model_id: &'a str) -> &'a str {
    model_id
        .strip_prefix(&format!("{provider_id}/"))
        .unwrap_or(model_id)
}

fn canonical_model_for_provider(provider_id: &str, model_id: &str) -> String {
    if provider_id == "anthropic" || model_id.contains('/') {
        model_id.to_string()
    } else {
        format!("{provider_id}/{model_id}")
    }
}

fn provider_lookup_ids(provider_id: &str) -> Vec<&str> {
    match provider_id {
        "togetherai" | "together-ai" => vec!["togetherai", "together-ai"],
        "lmstudio" | "lm-studio" => vec!["lmstudio", "lm-studio"],
        "llamacpp" | "llama-cpp" | "llama-server" => {
            vec!["llamacpp", "llama-cpp", "llama-server"]
        }
        "moonshot" | "moonshotai" => vec!["moonshot", "moonshotai"],
        "zhipu" | "zhipuai" => vec!["zhipu", "zhipuai"],
        "vultr" | "vultr-ai" => vec!["vultr", "vultr-ai"],
        "google" | "google-vertex" => vec!["google", "google-vertex"],
        _ => vec![provider_id],
    }
}

fn resolve_fast_model_id(config: &Config) -> String {
    let provider_id = config.selected_provider_id();
    let registry = claurst_api::ModelRegistry::new();

    provider_lookup_ids(provider_id)
        .into_iter()
        .find_map(|lookup_id| registry.best_small_model_for_provider(lookup_id))
        .unwrap_or_else(|| {
            stripped_model_for_provider(provider_id, config.effective_model()).to_string()
        })
}

async fn provider_for_config(
    config: &Config,
) -> Option<std::sync::Arc<dyn claurst_api::LlmProvider>> {
    let anthropic_auth = config.resolve_anthropic_auth_async().await;
    let registry = claurst_api::ProviderRegistry::from_config(
        config,
        claurst_api::client::ClientConfig {
            api_key: anthropic_auth
                .as_ref()
                .map(|(credential, _)| credential.clone())
                .unwrap_or_default(),
            api_base: config.resolve_anthropic_api_base(),
            use_bearer_auth: anthropic_auth
                .as_ref()
                .is_some_and(|(_, use_bearer)| *use_bearer),
            ..Default::default()
        },
    );

    provider_lookup_ids(config.selected_provider_id())
        .into_iter()
        .find_map(|lookup_id| {
            registry
                .get(&claurst_core::ProviderId::new(lookup_id))
                .cloned()
        })
}

fn text_from_content_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

// ---------------------------------------------------------------------------
// Built-in commands
// ---------------------------------------------------------------------------

pub struct HelpCommand;
pub struct ClearCommand;
pub struct CompactCommand;
pub struct CostCommand;
pub struct ExitCommand;
pub struct ModelCommand;
pub struct ConfigCommand;
pub struct ColorCommand;
pub struct VersionCommand;
pub struct ResumeCommand;
pub struct StatusCommand;
pub struct DiffCommand;
pub struct GoalCommand;
pub struct MemoryCommand;
pub struct BugCommand;
pub struct UsageCommand;
pub struct DoctorCommand;
pub struct LoginCommand;
pub struct LogoutCommand;
pub struct RefreshCommand;
pub struct InitCommand;
pub struct ReviewCommand;
pub struct HooksCommand;
pub struct ImportConfigCommand;
pub struct McpCommand;
pub struct PermissionsCommand;
pub struct PlanCommand;
pub struct TasksCommand;
pub struct SessionCommand;
pub struct ThinkingCommand;
// New commands
pub struct ExportCommand;
pub struct ShareCommand;
pub struct SkillsCommand;
pub struct RewindCommand;
pub struct StatsCommand;
pub struct RenameCommand;
pub struct EffortCommand;
pub struct CommitCommand;
pub struct PluginCommand;
pub struct ReloadPluginsCommand;
pub struct ThemeCommand;
pub struct OutputStyleCommand;
pub struct KeybindingsCommand;
// Batch-1 new commands
pub struct ContextCommand;
pub struct CopyCommand;
pub struct ChromeCommand;
pub struct VimCommand;
pub struct VoiceCommand;
pub struct UpgradeCommand;
pub struct StatuslineCommand;
pub struct SecurityReviewCommand;
pub struct TerminalSetupCommand;
pub struct FastCommand;
pub struct ThinkBackCommand;
pub struct ThinkBackPlayCommand;
// New commands: teleport, btw, ctx-viz, sandbox-toggle
pub struct BtwCommand;
pub struct IncantCommand;
pub struct SandboxToggleCommand;
pub struct UltrareviewCommand;
pub struct AdvisorCommand;
pub struct RevertCommand;
pub struct CheckpointsCommand;
pub struct SnapshotDiffCommand;
pub struct ProvidersCommand;
pub struct ConnectCommand;
pub struct AgentCommand;
pub struct FamiliarCommand;
pub struct SearchCommand;
pub struct ForkCommand;
pub struct ManagedAgentsCommand;
pub struct CovenCommand;
pub struct NamedCommandAdapter {
    pub slash_name: &'static str,
    pub target_name: &'static str,
    pub slash_aliases: &'static [&'static str],
    pub slash_description: &'static str,
    pub slash_help: &'static str,
    /// `true` for one-release compatibility aliases folded into a parent
    /// command — still callable, but absent from /help and autocomplete.
    pub slash_hidden: bool,
}

#[derive(serde::Serialize)]
struct KeybindingTemplateFile {
    #[serde(rename = "$schema")]
    schema: &'static str,
    #[serde(rename = "$docs")]
    docs: &'static str,
    bindings: Vec<KeybindingTemplateBlock>,
}

#[derive(serde::Serialize)]
struct KeybindingTemplateBlock {
    context: String,
    bindings: BTreeMap<String, Option<String>>,
}

fn save_settings_mutation<F>(mutate: F) -> anyhow::Result<()>
where
    F: FnOnce(&mut Settings),
{
    let mut settings = Settings::load_sync()?;
    mutate(&mut settings);
    settings.save_sync()
}

fn open_with_system(target: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        let ps_cmd = format!("Start-Process '{}'", target.replace('\'', "''"));
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        Ok(())
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        Ok(())
    }
}

fn format_keystroke(keystroke: &claurst_core::keybindings::ParsedKeystroke) -> String {
    let mut parts = Vec::new();
    if keystroke.ctrl {
        parts.push("ctrl".to_string());
    }
    if keystroke.alt {
        parts.push("alt".to_string());
    }
    if keystroke.shift {
        parts.push("shift".to_string());
    }
    if keystroke.meta {
        parts.push("meta".to_string());
    }
    parts.push(match keystroke.key.as_str() {
        "space" => "space".to_string(),
        other => other.to_string(),
    });
    parts.join("+")
}

fn format_chord(chord: &[claurst_core::keybindings::ParsedKeystroke]) -> String {
    chord
        .iter()
        .map(format_keystroke)
        .collect::<Vec<_>>()
        .join(" ")
}

fn generate_keybindings_template() -> anyhow::Result<String> {
    let mut grouped: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for binding in claurst_core::keybindings::default_bindings() {
        let chord = format_chord(&binding.chord);
        if claurst_core::keybindings::NON_REBINDABLE.contains(&chord.as_str()) {
            continue;
        }
        grouped
            .entry(format!("{:?}", binding.context))
            .or_default()
            .insert(chord, binding.action.clone());
    }

    let template = KeybindingTemplateFile {
        schema: "https://www.schemastore.org/claude-code-keybindings.json",
        docs: "https://code.claude.com/docs/en/keybindings",
        bindings: grouped
            .into_iter()
            .map(|(context, bindings)| KeybindingTemplateBlock { context, bindings })
            .collect(),
    };

    Ok(format!("{}\n", serde_json::to_string_pretty(&template)?))
}

fn parse_theme(name: &str) -> Option<Theme> {
    match name.trim().to_lowercase().as_str() {
        "default" | "system" => Some(Theme::Default),
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        custom if !custom.is_empty() => Some(Theme::Custom(custom.to_string())),
        _ => None,
    }
}

fn current_output_style_name(config: &Config) -> &str {
    config.output_style.as_deref().unwrap_or("default")
}

fn available_output_style_names() -> Vec<String> {
    claurst_core::output_styles::all_styles(&Settings::config_dir())
        .into_iter()
        .map(|style| style.name)
        .collect()
}

fn split_command_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escape = false;

    for ch in args.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }

        match ch {
            '\\' => escape = true,
            '\'' | '"' if quote == Some(ch) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            ch if ch.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        out.push(current);
    }

    out
}

fn execute_named_command_from_slash(
    target_name: &str,
    args: &str,
    ctx: &CommandContext,
) -> CommandResult {
    let Some(cmd) = named_commands::find_named_command(target_name) else {
        return CommandResult::Error(format!(
            "Named command '{}' is not available in this build.",
            target_name
        ));
    };

    let parsed_args = split_command_args(args);
    let parsed_refs = parsed_args.iter().map(String::as_str).collect::<Vec<_>>();
    cmd.execute_named(&parsed_refs, ctx)
}

// ---- /help ---------------------------------------------------------------

/// Category labels for help grouping.
fn command_category(name: &str) -> &'static str {
    match name {
        "clear" | "compact" | "rewind" | "undo" | "revert" | "export" | "branch" | "fork" => {
            "Conversation"
        }
        "model" | "config" | "theme" | "color" | "vim" | "fast" | "effort" | "voice"
        | "statusline" | "output-style" | "keybindings" | "sandbox" => "Settings",
        "cost" | "usage" | "context" => "Usage & Cost",
        "status" | "doctor" | "terminal-setup" | "version" | "update" | "upgrade" => "System",
        "login" | "logout" | "switch" | "refresh" | "permissions" => "Auth & Permissions",
        "memory" | "diff" | "init" | "commit" | "review" | "import-config" => "Project",
        "mcp" | "hooks" | "chrome" => "Integrations",
        "session" | "resume" | "search" | "share" => "Sessions & Remote",
        "help" | "exit" | "feedback" | "bug" => "General",
        "think-back" | "thinking" | "plan" | "goal" | "tasks" | "advisor" => "AI & Thinking",
        "copy" | "skills" | "agents" | "plugin" | "reload-plugins" | "whisper" | "incant" => {
            "Tools & Extras"
        }
        "coven" | "familiar" | "agent" | "managed-agents" => "Coven",
        _ => "Other",
    }
}

#[async_trait]
impl SlashCommand for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["h", "?"]
    }
    fn description(&self) -> &str {
        "Show available commands and usage information"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        if !args.is_empty() {
            // Show help for a specific command
            if let Some(cmd) = find_command(args) {
                let aliases = cmd.aliases();
                let alias_line = if aliases.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\nAliases: {}",
                        aliases
                            .iter()
                            .map(|a| format!("/{}", a))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                return CommandResult::Message(format!(
                    "/{name}{aliases}\n{desc}\n\n{help}",
                    name = cmd.name(),
                    aliases = alias_line,
                    desc = cmd.description(),
                    help = cmd.help(),
                ));
            }
            return CommandResult::Error(format!("Unknown command: /{}", args));
        }

        // Grouped output
        let commands = all_commands();
        let visible: Vec<_> = commands.iter().filter(|c| !c.hidden()).collect();

        // Collect categories in stable order
        let category_order = [
            "Conversation",
            "Settings",
            "Usage & Cost",
            "System",
            "Auth & Permissions",
            "Project",
            "Integrations",
            "Sessions & Remote",
            "AI & Thinking",
            "Tools & Extras",
            "General",
            "Other",
        ];

        let mut by_cat: std::collections::HashMap<&str, Vec<String>> =
            std::collections::HashMap::new();

        for cmd in &visible {
            let cat = command_category(cmd.name());
            let aliases = cmd.aliases();
            let alias_str = if aliases.is_empty() {
                String::new()
            } else {
                format!(
                    " ({})",
                    aliases
                        .iter()
                        .map(|a| format!("/{}", a))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            by_cat.entry(cat).or_default().push(format!(
                "  /{:<20} {}",
                format!("{}{}", cmd.name(), alias_str),
                cmd.description()
            ));
        }

        let mut output = String::from("Coven Code — Slash Commands\n");
        output.push_str("════════════════════════════\n");

        for cat in &category_order {
            if let Some(entries) = by_cat.get(cat) {
                output.push_str(&format!("\n{}\n", cat));
                for entry in entries {
                    output.push_str(&format!("{}\n", entry));
                }
            }
        }

        output.push_str("\nType /help <command> for detailed help on a specific command.");
        CommandResult::Message(output)
    }
}

// ---- /clear --------------------------------------------------------------

#[async_trait]
impl SlashCommand for ClearCommand {
    fn name(&self) -> &str {
        "clear"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["c", "reset", "new"]
    }
    fn description(&self) -> &str {
        "Clear the conversation history"
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        CommandResult::ClearConversation
    }
}

// ---- /compact ------------------------------------------------------------

#[async_trait]
impl SlashCommand for CompactCommand {
    fn name(&self) -> &str {
        "compact"
    }
    fn description(&self) -> &str {
        "Compact the conversation to reduce token usage"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let msg_count = ctx.messages.len();
        let instruction = if args.is_empty() {
            "Provide a detailed summary of our conversation so far, preserving all \
             key technical details, decisions made, file paths mentioned, and current \
             task status."
                .to_string()
        } else {
            args.to_string()
        };

        CommandResult::UserMessage(format!(
            "[Compact requested ({} messages). Instruction: {}]",
            msg_count, instruction
        ))
    }
}

// ---- /cost ---------------------------------------------------------------

#[async_trait]
impl SlashCommand for CostCommand {
    fn name(&self) -> &str {
        "cost"
    }
    fn hidden(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Show token usage and cost for this session"
    }
    fn help(&self) -> &str {
        "Usage: /cost\n\n\
         Shows per-category token counts and the estimated cost for this session.\n\
         Cache write tokens are priced slightly higher than input; cache read tokens\n\
         are ~10x cheaper — caching reduces cost significantly in long sessions.\n\
         For account quotas use /usage."
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let tracker = &ctx.cost_tracker;
        let model = ctx.config.effective_model();
        let pricing = claurst_core::cost::ModelPricing::for_model(model);

        let input = tracker.input_tokens();
        let output = tracker.output_tokens();
        let cache_create = tracker.cache_creation_tokens();
        let cache_read = tracker.cache_read_tokens();
        let total = tracker.total_tokens();
        let cost = tracker.total_cost_usd();

        // Per-category cost breakdown.
        let input_cost = (input as f64 * pricing.input_per_mtk) / 1_000_000.0;
        let output_cost = (output as f64 * pricing.output_per_mtk) / 1_000_000.0;
        let cc_cost = (cache_create as f64 * pricing.cache_creation_per_mtk) / 1_000_000.0;
        let cr_cost = (cache_read as f64 * pricing.cache_read_per_mtk) / 1_000_000.0;

        // Pricing info line.
        let pricing_line = format!(
            "  Rates ($/MTok): input ${:.2} | output ${:.2} | cache-write ${:.3} | cache-read ${:.3}",
            pricing.input_per_mtk,
            pricing.output_per_mtk,
            pricing.cache_creation_per_mtk,
            pricing.cache_read_per_mtk,
        );

        // Cache savings note: how much input cost was avoided by using cache-read
        // instead of re-sending those tokens as normal input.
        let savings = if cache_read > 0 {
            let saved = (cache_read as f64 * (pricing.input_per_mtk - pricing.cache_read_per_mtk))
                / 1_000_000.0;
            format!(
                "\n  Cache savings:  ${:.4}  ({} tokens served from cache)",
                saved, cache_read
            )
        } else {
            String::new()
        };

        CommandResult::Message(format!(
            "Session Cost — {model}\n\
             ──────────────────────────────\n\
             {pricing_line}\n\n\
               Input tokens:   {input:>10}   ${input_cost:.4}\n\
               Output tokens:  {output:>10}   ${output_cost:.4}\n\
               Cache write:    {cache_create:>10}   ${cc_cost:.4}\n\
               Cache read:     {cache_read:>10}   ${cr_cost:.4}\n\
             ─────────────────────────────\n\
               Total tokens:   {total:>10}\n\
               Total cost:              ${cost:.4}{savings}\n\n\
             Use /usage for quota info",
            model = model,
            pricing_line = pricing_line,
            input = input,
            input_cost = input_cost,
            output = output,
            output_cost = output_cost,
            cache_create = cache_create,
            cc_cost = cc_cost,
            cache_read = cache_read,
            cr_cost = cr_cost,
            total = total,
            cost = cost,
            savings = savings,
        ))
    }
}

// ---- /exit ---------------------------------------------------------------

#[async_trait]
impl SlashCommand for ExitCommand {
    fn name(&self) -> &str {
        "exit"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["quit", "q"]
    }
    fn description(&self) -> &str {
        "Exit Coven Code"
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        CommandResult::Exit
    }
}

// ---- /model --------------------------------------------------------------

#[async_trait]
impl SlashCommand for ModelCommand {
    fn name(&self) -> &str {
        "model"
    }
    fn description(&self) -> &str {
        "Show or change the current model"
    }
    fn help(&self) -> &str {
        "Usage: /model [<model-id>]\n\n\
         Without arguments, shows the current model.\n\n\
         With a model ID, switches to that model.  Accepts both bare model\n\
         names (e.g. claude-sonnet-4-6) and provider-prefixed format\n\
         (e.g. openai/gpt-4o, google/gemini-2.0-flash).\n\n\
         Examples:\n\
           /model                        — show current model\n\
           /model claude-opus-4-6        — switch to Claude Opus 4.6\n\
           /model openai/gpt-4o          — switch to GPT-4o via OpenAI\n\
           /model google/gemini-2.0-flash — switch to Gemini 2.0 Flash"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let args = args.trim();
        if args.is_empty() {
            CommandResult::Message(format!("Current model: {}", ctx.config.effective_model()))
        } else {
            // Accept both "provider/model" and bare model names.
            // The config stores the full string (including provider prefix when present)
            // so that downstream dispatch can route to the correct provider.
            let model_str = args.to_string();
            let confirmation = if let Some((provider, model)) = model_str.split_once('/') {
                if provider == "anthropic" {
                    format!("Switched to {}", model)
                } else {
                    format!("Switched to {}/{}", provider, model)
                }
            } else {
                format!("Switched to {}", model_str)
            };
            let mut new_config = ctx.config.clone();
            new_config.model = Some(model_str.clone());
            if let Some((provider, _)) = model_str.split_once('/') {
                new_config.provider = Some(provider.to_string());
            }
            CommandResult::ConfigChangeMessage(new_config, confirmation)
        }
    }
}

// ---- /config -------------------------------------------------------------

#[async_trait]
impl SlashCommand for ConfigCommand {
    fn name(&self) -> &str {
        "config"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["settings"]
    }
    fn description(&self) -> &str {
        "Show or modify configuration settings"
    }
    fn help(&self) -> &str {
        "Usage: /config [show|get|set|unset|color|vim|voice|statusline|terminal-setup] ...\n\n\
         Shows or modifies configuration settings.\n\n\
         Core settings:\n\
           /config\n\
           /config set theme <default|dark|light>\n\
           /config set output-style <default|concise|explanatory|learning|formal|casual>\n\
           /config set model <model>\n\
           /config set permission-mode <default|accept-edits|bypass-permissions|plan>\n\
           /config unset <model|output-style>\n\n\
         UI settings:\n\
           /config color [<name|#RRGGBB|default>]\n\
           /config vim [on|off]\n\
           /config voice [on|off|status]\n\
           /config statusline [show|hide] [cost|tokens|model|time|all]\n\
           /config terminal-setup"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let args = args.trim();
        if args.is_empty() || matches!(args, "show" | "get") {
            let json = serde_json::to_string_pretty(&ctx.config).unwrap_or_default();
            return CommandResult::Message(format!(
                "Current configuration:\n{}\n\n{}",
                json,
                self.help()
            ));
        }

        let mut subcommand_parts = args.splitn(2, char::is_whitespace);
        let subcommand = subcommand_parts.next().unwrap_or_default();
        let subcommand_args = subcommand_parts.next().unwrap_or_default().trim();
        match subcommand {
            "color" | "prompt-color" | "prompt_color" => {
                return ColorCommand.execute(subcommand_args, ctx).await;
            }
            "vim" | "vi" | "editor" | "editor-mode" | "editor_mode" => {
                return VimCommand.execute(subcommand_args, ctx).await;
            }
            "voice" => {
                return VoiceCommand.execute(subcommand_args, ctx).await;
            }
            "statusline" | "status-line" | "status_line" => {
                return StatuslineCommand.execute(subcommand_args, ctx).await;
            }
            "terminal-setup" | "terminal_setup" | "terminal" => {
                if !subcommand_args.is_empty() {
                    return CommandResult::Error("Usage: /config terminal-setup".to_string());
                }
                return TerminalSetupCommand.execute("", ctx).await;
            }
            _ => {}
        }

        if let Some(key) = args.strip_prefix("get ").map(str::trim) {
            return match key {
                "theme" => CommandResult::Message(format!("theme = {:?}", ctx.config.theme)),
                "output-style" | "output_style" => CommandResult::Message(format!(
                    "output-style = {}",
                    current_output_style_name(&ctx.config)
                )),
                "model" => {
                    CommandResult::Message(format!("model = {}", ctx.config.effective_model()))
                }
                "permission-mode" | "permission_mode" => CommandResult::Message(format!(
                    "permission-mode = {:?}",
                    ctx.config.permission_mode
                )),
                other => CommandResult::Error(format!("Unknown config key '{}'", other)),
            };
        }

        if let Some(key) = args.strip_prefix("unset ").map(str::trim) {
            return match key {
                "model" => {
                    let mut new_config = ctx.config.clone();
                    new_config.model = None;
                    if let Err(err) =
                        save_settings_mutation(|settings| settings.config.model = None)
                    {
                        return CommandResult::Error(format!(
                            "Failed to save configuration: {}",
                            err
                        ));
                    }
                    CommandResult::ConfigChangeMessage(
                        new_config,
                        "Model reset to the default for new sessions.".to_string(),
                    )
                }
                "output-style" | "output_style" => {
                    let mut new_config = ctx.config.clone();
                    new_config.output_style = None;
                    if let Err(err) =
                        save_settings_mutation(|settings| settings.config.output_style = None)
                    {
                        return CommandResult::Error(format!(
                            "Failed to save configuration: {}",
                            err
                        ));
                    }
                    CommandResult::ConfigChangeMessage(
                        new_config,
                        "Output style reset to default.".to_string(),
                    )
                }
                other => CommandResult::Error(format!("Unknown config key '{}'", other)),
            };
        }

        let mut parts = args.splitn(3, ' ');
        let command = parts.next().unwrap_or_default();
        let key = parts.next().unwrap_or_default().trim();
        let value = parts.next().unwrap_or_default().trim();
        if command != "set" || key.is_empty() || value.is_empty() {
            return CommandResult::Error("Usage: /config set <key> <value>".to_string());
        }

        match key {
            "theme" => {
                let Some(theme) = parse_theme(value) else {
                    return CommandResult::Error(
                        "Theme must be one of: default, dark, light".to_string(),
                    );
                };
                let mut new_config = ctx.config.clone();
                new_config.theme = theme.clone();
                if let Err(err) =
                    save_settings_mutation(|settings| settings.config.theme = theme.clone())
                {
                    return CommandResult::Error(format!("Failed to save configuration: {}", err));
                }
                CommandResult::ConfigChangeMessage(
                    new_config,
                    format!("Theme set to {}.", value.trim().to_lowercase()),
                )
            }
            "output-style" | "output_style" => {
                let normalized = value.trim().to_lowercase();
                let valid = available_output_style_names();
                if !valid.iter().any(|name| name == &normalized) {
                    return CommandResult::Error(format!(
                        "Unsupported output style '{}'. Use one of: {}",
                        value,
                        valid.join(", ")
                    ));
                }

                let mut new_config = ctx.config.clone();
                new_config.output_style = (normalized != "default").then(|| normalized.clone());
                if let Err(err) = save_settings_mutation(|settings| {
                    settings.config.output_style =
                        (normalized != "default").then(|| normalized.clone());
                }) {
                    return CommandResult::Error(format!("Failed to save configuration: {}", err));
                }
                CommandResult::ConfigChangeMessage(
                    new_config,
                    format!(
                        "Output style set to {}. Changes take effect on the next request.",
                        normalized
                    ),
                )
            }
            "model" => {
                let mut new_config = ctx.config.clone();
                new_config.model = Some(value.to_string());
                let inferred_provider = value
                    .split_once('/')
                    .map(|(provider, _)| provider.to_string());
                if let Some(ref provider) = inferred_provider {
                    new_config.provider = Some(provider.clone());
                }
                if let Err(err) = save_settings_mutation(|settings| {
                    settings.config.model = Some(value.to_string());
                    if let Some(ref provider) = inferred_provider {
                        settings.provider = Some(provider.clone());
                        settings.config.provider = Some(provider.clone());
                    }
                }) {
                    return CommandResult::Error(format!("Failed to save configuration: {}", err));
                }
                CommandResult::ConfigChangeMessage(new_config, format!("Model set to {}.", value))
            }
            "permission-mode" | "permission_mode" => {
                let mode = match value.trim().to_lowercase().as_str() {
                    "default" => claurst_core::config::PermissionMode::Default,
                    "accept-edits" | "accept_edits" => {
                        claurst_core::config::PermissionMode::AcceptEdits
                    }
                    "bypass-permissions" | "bypass_permissions" => {
                        claurst_core::config::PermissionMode::BypassPermissions
                    }
                    "plan" => claurst_core::config::PermissionMode::Plan,
                    _ => {
                        return CommandResult::Error(
                            "Permission mode must be one of: default, accept-edits, bypass-permissions, plan"
                                .to_string(),
                        )
                    }
                };

                let mut new_config = ctx.config.clone();
                new_config.permission_mode = mode.clone();
                if let Err(err) = save_settings_mutation(|settings| {
                    settings.config.permission_mode = mode.clone();
                }) {
                    return CommandResult::Error(format!("Failed to save configuration: {}", err));
                }
                CommandResult::ConfigChangeMessage(
                    new_config,
                    format!("Permission mode set to {}.", value.trim().to_lowercase()),
                )
            }
            other => CommandResult::Error(format!("Unknown config key '{}'", other)),
        }
    }
}

// ---- /color --------------------------------------------------------------

#[async_trait]
impl SlashCommand for ColorCommand {
    fn name(&self) -> &str {
        "color"
    }
    fn hidden(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Set or show the prompt bar color for this session"
    }
    fn help(&self) -> &str {
        "Usage: /color [<name|#RRGGBB|default>]\n\n\
         Sets the accent color for the prompt bar in this session.\n\
         Named colors: red, green, blue, yellow, cyan, magenta, white, orange, purple\n\
         Hex codes:    #RGB or #RRGGBB\n\
         Reset:        /color default\n\n\
         The color is persisted to ~/.coven-code/ui-settings.json and\n\
         applied on the next REPL startup."
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let color = args.trim();
        if color.is_empty() {
            let current = load_ui_settings();
            return CommandResult::Message(format!(
                "Current prompt color: {}\n\
                 Use /color <name|#RRGGBB|default> to change it.\n\n\
                 Named colors: red, green, blue, yellow, cyan, magenta, white, orange, purple",
                current.prompt_color.as_deref().unwrap_or("default"),
            ));
        }

        let normalized = if color == "default" {
            None
        } else {
            let known_colors = [
                "red", "green", "blue", "yellow", "cyan", "magenta", "white", "orange", "purple",
                "pink", "gray", "grey",
            ];
            let is_hex = color.starts_with('#')
                && (color.len() == 4 || color.len() == 7)
                && color[1..].chars().all(|c| c.is_ascii_hexdigit());
            if !is_hex && !known_colors.contains(&color.to_lowercase().as_str()) {
                return CommandResult::Error(format!(
                    "Unknown color '{}'. Use a color name (red, green, …) or a hex code (#RGB or #RRGGBB).",
                    color
                ));
            }
            Some(color.to_string())
        };

        match mutate_ui_settings(|s| s.prompt_color = normalized.clone()) {
            Ok(_) => CommandResult::Message(format!(
                "Prompt color set to {}.\n\
                 Restart the REPL for the change to take effect.",
                normalized.as_deref().unwrap_or("default")
            )),
            Err(e) => CommandResult::Error(format!("Failed to save color: {}", e)),
        }
    }
}

// ---- /theme --------------------------------------------------------------

#[async_trait]
impl SlashCommand for ThemeCommand {
    fn name(&self) -> &str {
        "theme"
    }
    fn description(&self) -> &str {
        "Show or change the current theme"
    }
    fn help(&self) -> &str {
        "Usage: /theme [default|dark|light]\n\
         Without arguments, shows the active theme. With an argument, updates the theme for this and future sessions."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let args = args.trim();
        if args.is_empty() {
            return CommandResult::Message(format!(
                "Current theme: {:?}\nUse /theme <default|dark|light> to change it.",
                ctx.config.theme
            ));
        }

        let Some(theme) = parse_theme(args) else {
            return CommandResult::Error("Theme must be one of: default, dark, light".to_string());
        };

        let mut new_config = ctx.config.clone();
        new_config.theme = theme.clone();
        if let Err(err) = save_settings_mutation(|settings| settings.config.theme = theme.clone()) {
            return CommandResult::Error(format!("Failed to save theme: {}", err));
        }

        CommandResult::ConfigChangeMessage(
            new_config,
            format!("Theme set to {}.", args.to_lowercase()),
        )
    }
}

// ---- /output-style -------------------------------------------------------

#[async_trait]
impl SlashCommand for OutputStyleCommand {
    fn name(&self) -> &str {
        "output-style"
    }
    fn description(&self) -> &str {
        "Show or switch the current output style"
    }
    fn help(&self) -> &str {
        "Usage: /output-style [style-name]\n\n\
         With no argument: list available styles and show the current one.\n\
         With a style name: switch to that style (persisted to settings).\n\n\
         Built-in styles: default, verbose, concise\n\
         Plugin-defined styles are listed automatically.\n\n\
         Changes take effect on the next request."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let arg = args.trim();
        let valid_styles = available_output_style_names();
        let current = current_output_style_name(&ctx.config);

        if arg.is_empty() {
            // List available styles
            let mut lines = format!("Current output style: {}\n\nAvailable styles:\n", current);
            for style in &valid_styles {
                let marker = if style == current { " *" } else { "" };
                lines.push_str(&format!("  {}{}\n", style, marker));
            }
            lines.push_str("\nUse /output-style <name> to switch.");
            return CommandResult::Message(lines);
        }

        let normalized = arg.to_lowercase();
        if !valid_styles.iter().any(|name| name == &normalized) {
            return CommandResult::Error(format!(
                "Unknown output style '{}'. Available styles: {}",
                arg,
                valid_styles.join(", ")
            ));
        }

        let mut new_config = ctx.config.clone();
        new_config.output_style = (normalized != "default").then(|| normalized.clone());
        if let Err(err) = save_settings_mutation(|settings| {
            settings.config.output_style = (normalized != "default").then(|| normalized.clone());
        }) {
            return CommandResult::Error(format!("Failed to save configuration: {}", err));
        }

        CommandResult::ConfigChangeMessage(
            new_config,
            format!(
                "Output style set to '{}'. Changes take effect on the next request.",
                normalized
            ),
        )
    }
}

// ---- /keybindings --------------------------------------------------------

#[async_trait]
impl SlashCommand for KeybindingsCommand {
    fn name(&self) -> &str {
        "keybindings"
    }
    fn description(&self) -> &str {
        "Create or open ~/.coven-code/keybindings.json"
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let config_dir = Settings::config_dir();
        let path = config_dir.join("keybindings.json");
        let existed = path.exists();

        if !existed {
            if let Err(err) = std::fs::create_dir_all(&config_dir) {
                return CommandResult::Error(format!(
                    "Failed to create {}: {}",
                    config_dir.display(),
                    err
                ));
            }

            let template = match generate_keybindings_template() {
                Ok(template) => template,
                Err(err) => {
                    return CommandResult::Error(format!(
                        "Failed to generate keybindings template: {}",
                        err
                    ))
                }
            };

            if let Err(err) = std::fs::write(&path, template) {
                return CommandResult::Error(format!(
                    "Failed to write {}: {}",
                    path.display(),
                    err
                ));
            }
        }

        match open_with_system(&path.display().to_string()) {
            Ok(_) => CommandResult::Message(if existed {
                format!("Opened {} in your editor.", path.display())
            } else {
                format!(
                    "Created {} with a template and opened it in your editor.",
                    path.display()
                )
            }),
            Err(err) => CommandResult::Message(if existed {
                format!(
                    "Opened {}. Could not launch an editor automatically: {}",
                    path.display(),
                    err
                )
            } else {
                format!(
                    "Created {} with a template. Could not launch an editor automatically: {}",
                    path.display(),
                    err
                )
            }),
        }
    }
}

// ---- /version ------------------------------------------------------------

#[async_trait]
impl SlashCommand for VersionCommand {
    fn name(&self) -> &str {
        "version"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["v"]
    }
    fn description(&self) -> &str {
        "Show version information"
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        CommandResult::Message(format!(
            "Coven Code v{}",
            claurst_core::constants::APP_VERSION
        ))
    }
}

// ---- /resume -------------------------------------------------------------

#[async_trait]
impl SlashCommand for ResumeCommand {
    fn name(&self) -> &str {
        "resume"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["r", "continue"]
    }
    fn description(&self) -> &str {
        "Resume a previous conversation"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        if args.is_empty() {
            let sessions = claurst_core::history::list_sessions().await;
            if sessions.is_empty() {
                return CommandResult::Message("No previous sessions found.".to_string());
            }
            let last = &sessions[0];
            match claurst_core::history::load_session(&last.id).await {
                Ok(session) => CommandResult::ResumeSession(session),
                Err(e) => {
                    CommandResult::Error(format!("Failed to load session {}: {}", last.id, e))
                }
            }
        } else {
            match claurst_core::history::load_session(args.trim()).await {
                Ok(session) => CommandResult::ResumeSession(session),
                Err(e) => {
                    CommandResult::Error(format!("Failed to load session {}: {}", args.trim(), e))
                }
            }
        }
    }
}

// ---- /status -------------------------------------------------------------

#[async_trait]
impl SlashCommand for StatusCommand {
    fn name(&self) -> &str {
        "status"
    }
    fn description(&self) -> &str {
        "Show comprehensive system and session status"
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        // Auth status
        let auth_status = match claurst_core::oauth::OAuthTokens::load().await {
            Some(tokens) => {
                let sub = tokens.subscription_type.as_deref().unwrap_or("oauth");
                format!("Authenticated ({})", sub)
            }
            None => {
                if ctx.config.resolve_api_key().is_some() {
                    "Authenticated (API key)".to_string()
                } else {
                    "Not authenticated".to_string()
                }
            }
        };

        // MCP status
        let mcp_count = ctx.config.mcp_servers.len();
        let mcp_status = if mcp_count == 0 {
            "none configured".to_string()
        } else {
            format!("{} server(s) configured", mcp_count)
        };

        // Hook status
        let hook_count: usize = ctx.config.hooks.values().map(|v| v.len()).sum();

        // UI settings
        let ui = load_ui_settings();
        let editor_mode = ui.editor_mode.as_deref().unwrap_or("normal");
        let fast_mode = ui.fast_mode.unwrap_or(false);

        // Git status
        let git_branch = tokio::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&ctx.working_dir)
            .output()
            .await
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "n/a".to_string());

        CommandResult::Message(format!(
            "Coven Code Status\n\
             ══════════════════\n\
             Auth:           {auth_status}\n\
             Model:          {model}\n\
             Permission mode: {perm:?}\n\
             Fast mode:      {fast}\n\
             Editor mode:    {editor}\n\n\
             Session\n\
             ───────\n\
             Session ID:     {sid}\n\
             Title:          {title}\n\
             Messages:       {msgs}\n\
             Working dir:    {wd}\n\
             Git branch:     {branch}\n\n\
             Integrations\n\
             ────────────\n\
             MCP servers:    {mcp}\n\
             Hooks:          {hooks} configured\n\n\
             Usage\n\
             ─────\n\
             {summary}",
            auth_status = auth_status,
            model = ctx.config.effective_model(),
            perm = ctx.config.permission_mode,
            fast = if fast_mode { "on" } else { "off" },
            editor = editor_mode,
            sid = &ctx.session_id[..ctx.session_id.len().min(12)],
            title = ctx.session_title.as_deref().unwrap_or("(untitled)"),
            msgs = ctx.messages.len(),
            wd = ctx.working_dir.display(),
            branch = git_branch,
            mcp = mcp_status,
            hooks = hook_count,
            summary = ctx.cost_tracker.summary(),
        ))
    }
}

// ---- /diff ---------------------------------------------------------------

#[async_trait]
impl SlashCommand for DiffCommand {
    fn name(&self) -> &str {
        "diff"
    }
    fn description(&self) -> &str {
        "Show git diff of changes in the working directory"
    }
    fn help(&self) -> &str {
        "Usage: /diff [--stat|--staged|<ref>]\n\n\
         Shows git diff output for the current working directory.\n\n\
         Options:\n\
           /diff           — diff of all unstaged changes (git diff)\n\
           /diff --stat    — summary of changed files\n\
           /diff --staged  — diff of staged changes (git diff --cached)\n\
           /diff <ref>     — diff against a branch, tag, or commit"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let args = args.trim();

        let git_args: Vec<&str> = if args == "--stat" {
            vec!["diff", "--stat"]
        } else if args == "--staged" || args == "--cached" {
            vec!["diff", "--cached"]
        } else if args.is_empty() {
            vec!["diff"]
        } else {
            // Treat as a ref
            vec!["diff", args]
        };

        let output = tokio::process::Command::new("git")
            .args(&git_args)
            .current_dir(&ctx.working_dir)
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() || out.status.code() == Some(1) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.trim().is_empty() {
                    CommandResult::Message(
                        "No changes found. Working tree is clean (or not a git repository)."
                            .to_string(),
                    )
                } else {
                    // Truncate very long diffs
                    let text = stdout.as_ref();
                    let display = if text.len() > 8000 {
                        format!(
                            "{}\n… (truncated — {} total bytes; use `git diff` for full output)",
                            &text[..8000],
                            text.len()
                        )
                    } else {
                        text.to_string()
                    };
                    CommandResult::Message(format!("Changes:\n{}", display))
                }
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                CommandResult::Error(format!(
                    "git diff failed (exit {}): {}",
                    out.status.code().unwrap_or(-1),
                    stderr.trim()
                ))
            }
            Err(e) => CommandResult::Error(format!("Failed to run git diff: {}", e)),
        }
    }
}

// ---- /goal ---------------------------------------------------------------

/// Parse a soft token budget from strings like "250K", "1M", "500000".
fn parse_token_budget(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('K').or_else(|| s.strip_suffix('k'))
    {
        (n, 1_000u64)
    } else if let Some(n) = s.strip_suffix('M').or_else(|| s.strip_suffix('m')) {
        (n, 1_000_000u64)
    } else {
        (s, 1u64)
    };
    num_str.trim().parse::<u64>().ok().map(|n| n * multiplier)
}

#[async_trait]
impl SlashCommand for GoalCommand {
    fn name(&self) -> &str {
        "goal"
    }
    fn description(&self) -> &str {
        "Set or manage a durable long-running goal for autonomous work"
    }
    fn help(&self) -> &str {
        "Usage:\n\
         /goal <objective>              — set a new goal and begin working autonomously\n\
         /goal --tokens 250K <text>     — set a goal with a soft token budget\n\
         /goal                          — show current goal status\n\
         /goal status                   — show current goal status\n\
         /goal pause                    — pause the active goal\n\
         /goal resume                   — resume a paused goal\n\
         /goal clear                    — delete the current goal\n\
         /goal complete                 — request a completion audit\n\n\
         Goals let Coven Code work autonomously across turns toward a single\n\
         verifiable objective. Coven Code will keep iterating until the goal is\n\
         complete, you pause it, or the 200-turn runaway guard fires.\n\n\
         Examples:\n\
         /goal Migrate the project from Express to Fastify, keeping all routes passing\n\
         /goal --tokens 500K Fix all TypeScript errors in src/ without breaking tests"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        if !claurst_core::goals_enabled() {
            return CommandResult::Message(
                "Goals are disabled. Unset COVEN_CODE_GOALS=0 (or remove it) to re-enable."
                    .to_string(),
            );
        }

        let args = args.trim();
        let session_id = &ctx.session_id;

        // Parse subcommands with no objective
        match args {
            "" | "status" => return goal_status(session_id),
            "pause" => {
                let store = match open_goal_store() {
                    Some(s) => s,
                    None => return CommandResult::Error("Could not open goal store.".to_string()),
                };
                match store.get_goal(session_id) {
                    None => return CommandResult::Message("No active goal.".to_string()),
                    Some(g) if g.status == claurst_core::GoalStatus::Complete => {
                        return CommandResult::Message("Goal is already complete.".to_string());
                    }
                    Some(g) if g.status == claurst_core::GoalStatus::Paused => {
                        return CommandResult::Message(
                            "Goal is already paused. Use /goal resume to continue.".to_string(),
                        );
                    }
                    _ => {}
                }
                if let Err(e) = store.set_status(session_id, claurst_core::GoalStatus::Paused) {
                    return CommandResult::Error(format!("Failed to pause goal: {}", e));
                }
                return CommandResult::Message(
                    "Goal paused. Use /goal resume to continue.".to_string(),
                );
            }
            "resume" => {
                let store = match open_goal_store() {
                    Some(s) => s,
                    None => return CommandResult::Error("Could not open goal store.".to_string()),
                };
                match store.get_goal(session_id) {
                    None => return CommandResult::Message("No goal to resume.".to_string()),
                    Some(g) if g.status == claurst_core::GoalStatus::Active => {
                        return CommandResult::Message("Goal is already active.".to_string());
                    }
                    Some(g) if g.status == claurst_core::GoalStatus::Complete => {
                        return CommandResult::Message(
                            "Goal is complete. Use /goal <objective> to set a new one.".to_string(),
                        );
                    }
                    _ => {}
                }
                if let Err(e) = store.set_status(session_id, claurst_core::GoalStatus::Active) {
                    return CommandResult::Error(format!("Failed to resume goal: {}", e));
                }
                return CommandResult::Message(
                    "Goal resumed. Coven Code will continue on the next message.".to_string(),
                );
            }
            "clear" => {
                let store = match open_goal_store() {
                    Some(s) => s,
                    None => return CommandResult::Error("Could not open goal store.".to_string()),
                };
                store.clear_goal(session_id).unwrap_or_default();
                return CommandResult::Message("Goal cleared.".to_string());
            }
            "complete" => {
                // Inject a completion-audit user message.
                let store = match open_goal_store() {
                    Some(s) => s,
                    None => return CommandResult::Error("Could not open goal store.".to_string()),
                };
                match store.get_active_goal(session_id) {
                    None => {
                        return CommandResult::Message(
                            "No active goal. Set one with /goal <objective>.".to_string(),
                        );
                    }
                    Some(goal) => {
                        let audit_msg = format!(
                            "[User requested goal completion audit]\n\
                             Please review your active goal:\n\
                             <objective>\n{}\n</objective>\n\n\
                             Run through the completion audit:\n\
                             1. Restate the objective as concrete deliverables.\n\
                             2. Check that all deliverables have been achieved.\n\
                             3. Run any tests or validation commands.\n\
                             4. If fully complete, call GoalComplete with audit_summary and evidence.\n\
                             5. If not complete, describe what remains.",
                            goal.objective
                        );
                        return CommandResult::UserMessage(audit_msg);
                    }
                }
            }
            _ => {} // fall through to parse as objective (possibly with --tokens)
        }

        // Parse optional --tokens flag
        let (token_budget, objective) = if args.starts_with("--tokens") {
            // Expected: --tokens <budget> <objective>
            let rest = args.trim_start_matches("--tokens").trim();
            let mut parts = rest.splitn(2, char::is_whitespace);
            let budget_str = parts.next().unwrap_or("");
            let obj = parts.next().unwrap_or("").trim();
            let budget = parse_token_budget(budget_str);
            (budget, obj)
        } else {
            (None, args)
        };

        if objective.is_empty() {
            return CommandResult::Message(
                "Usage: /goal <objective> [--tokens 250K]\n\
                 Or: /goal status|pause|resume|clear|complete"
                    .to_string(),
            );
        }

        let store = match open_goal_store() {
            Some(s) => s,
            None => return CommandResult::Error("Could not open goal store.".to_string()),
        };

        match store.set_goal(session_id, objective, token_budget) {
            Err(claurst_core::GoalError::ObjectiveTooLong { len, max }) => CommandResult::Error(
                format!("Objective too long ({} chars). Max {} chars.", len, max),
            ),
            Err(e) => CommandResult::Error(format!("Failed to set goal: {}", e)),
            Ok(goal) => {
                // Return UserMessage so the query loop fires immediately and the
                // model begins working toward the goal without user needing to
                // send another message.
                CommandResult::UserMessage(claurst_core::goal_kickoff_message(&goal))
            }
        }
    }
}

fn open_goal_store() -> Option<claurst_core::GoalStore> {
    claurst_core::GoalStore::open_default()
}

fn goal_status(session_id: &str) -> CommandResult {
    let store = match open_goal_store() {
        Some(s) => s,
        None => return CommandResult::Error("Could not open goal store.".to_string()),
    };
    match store.get_goal(session_id) {
        None => {
            CommandResult::Message("No active goal. Set one with:\n  /goal <objective>".to_string())
        }
        Some(g) => {
            let budget_line = g
                .budget_display()
                .map(|b| format!("\nBudget:  {}", b))
                .unwrap_or_default();
            CommandResult::Message(format!(
                "Goal status\n\
                 ───────────\n\
                 Status:  {}\n\
                 Turns:   {}\n\
                 Elapsed: {}{}\n\
                 Objective:\n  {}",
                g.status.as_str(),
                g.turns_used,
                g.elapsed_display(),
                budget_line,
                g.objective,
            ))
        }
    }
}

// ---- /memory -------------------------------------------------------------

#[async_trait]
impl SlashCommand for MemoryCommand {
    fn name(&self) -> &str {
        "memory"
    }
    fn description(&self) -> &str {
        "View, edit, or clear AGENTS.md memory files"
    }
    fn help(&self) -> &str {
        "Usage: /memory [edit|clear] [global]\n\n\
         Shows the content of AGENTS.md files that provide project context to Coven Code.\n\
         Coven Code reads these files automatically at session start.\n\n\
         Subcommands:\n\
           /memory              — show all AGENTS.md files\n\
           /memory edit         — open project AGENTS.md in your editor\n\
           /memory edit global  — open global ~/.coven-code/AGENTS.md in your editor\n\
           /memory clear        — clear the project AGENTS.md\n\
           /memory clear global — clear the global ~/.coven-code/AGENTS.md\n\n\
         Locations checked (in priority order):\n\
           1. <project>/.coven-code/AGENTS.md\n\
           2. <project>/AGENTS.md\n\
           3. ~/.coven-code/AGENTS.md  (global)\n\n\
         Use /init to create a new AGENTS.md from a template."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let project_claude_dir = ctx.working_dir.join(".coven-code").join("AGENTS.md");
        let project_root = ctx.working_dir.join("AGENTS.md");
        let global_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".coven-code")
            .join("AGENTS.md");

        let locations = [
            (
                "project (.coven-code/AGENTS.md)",
                project_claude_dir.clone(),
            ),
            ("project (AGENTS.md)", project_root.clone()),
            ("global (~/.coven-code/AGENTS.md)", global_path.clone()),
        ];

        let cmd = args.trim();

        // ---- /memory edit [global|project] ------------------------------------
        if cmd == "edit" || cmd.starts_with("edit ") {
            let target_hint = cmd
                .strip_prefix("edit")
                .map(|s| s.trim())
                .unwrap_or("project");
            let target = match target_hint {
                "global" => {
                    // Ensure global dir exists
                    if let Some(parent) = global_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    global_path.clone()
                }
                _ => {
                    // Best project AGENTS.md
                    if project_root.exists() {
                        project_root.clone()
                    } else if project_claude_dir.exists() {
                        project_claude_dir.clone()
                    } else {
                        project_root.clone() // will be created by editor
                    }
                }
            };
            // Create file if it doesn't exist yet
            if !target.exists() {
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&target, "");
            }
            let editor = std::env::var("VISUAL")
                .or_else(|_| std::env::var("EDITOR"))
                .unwrap_or_else(|_| {
                    if cfg!(target_os = "windows") {
                        "notepad".to_string()
                    } else {
                        "vi".to_string()
                    }
                });
            let editor_hint = if let Ok(visual) = std::env::var("VISUAL") {
                format!("Using $VISUAL=\"{}\".", visual)
            } else if let Ok(ed) = std::env::var("EDITOR") {
                format!("Using $EDITOR=\"{}\".", ed)
            } else {
                "To use a different editor, set the $EDITOR or $VISUAL environment variable."
                    .to_string()
            };
            let spawn_result = std::process::Command::new(&editor).arg(&target).status();
            return match spawn_result {
                Ok(_) => CommandResult::Message(format!(
                    "Opened {} in your editor.\n{}",
                    target.display(),
                    editor_hint
                )),
                Err(e) => CommandResult::Message(format!(
                    "Could not launch '{}': {}. Edit {} manually.\n{}",
                    editor,
                    e,
                    target.display(),
                    editor_hint
                )),
            };
        }

        // ---- /memory clear [global|project] -----------------------------------
        if cmd == "clear" || cmd.starts_with("clear ") {
            let target_hint = cmd
                .strip_prefix("clear")
                .map(|s| s.trim())
                .unwrap_or("project");
            let (label, target) = match target_hint {
                "global" => ("global (~/.coven-code/AGENTS.md)", global_path.clone()),
                _ => {
                    if project_claude_dir.exists() {
                        (
                            "project (.coven-code/AGENTS.md)",
                            project_claude_dir.clone(),
                        )
                    } else {
                        ("project (AGENTS.md)", project_root.clone())
                    }
                }
            };
            if !target.exists() {
                return CommandResult::Message(format!(
                    "No {} memory file found (nothing to clear).",
                    label
                ));
            }
            return match tokio::fs::write(&target, "").await {
                Ok(_) => CommandResult::Message(format!(
                    "Cleared {} memory file at {}.\n\
                     Coven Code will no longer see this content at session start.",
                    label,
                    target.display()
                )),
                Err(e) => {
                    CommandResult::Error(format!("Failed to clear {}: {}", target.display(), e))
                }
            };
        }

        // ---- /memory (show all) -----------------------------------------------
        let mut output = String::from("AGENTS.md Memory Files\n══════════════════════\n");
        let mut found_any = false;

        for (label, path) in &locations {
            if path.exists() {
                found_any = true;
                match tokio::fs::read_to_string(path).await {
                    Ok(content) => {
                        let lines: usize = content.lines().count();
                        let chars = content.len();
                        output.push_str(&format!(
                            "\n[{label}]\nPath: {path}\nSize: {lines} lines, {chars} chars\n\
                             ─────────────────────────────────\n\
                             {content}\n",
                            label = label,
                            path = path.display(),
                            lines = lines,
                            chars = chars,
                            content = if content.len() > 2000 {
                                format!(
                                    "{}…\n(truncated — file is {} chars)",
                                    &content[..2000],
                                    chars
                                )
                            } else {
                                content.clone()
                            }
                        ));
                    }
                    Err(e) => output.push_str(&format!(
                        "\n[{label}] — Error reading {}: {}\n",
                        path.display(),
                        e,
                        label = label
                    )),
                }
            }
        }

        if !found_any {
            output.push_str(
                "\nNo AGENTS.md files found.\n\
                 Use /init to create one in the current project.\n\
                 Use /memory edit to create and open a memory file.",
            );
        } else {
            output.push_str(
                "\nSubcommands:\n\
                 /memory edit          — edit project AGENTS.md\n\
                 /memory edit global   — edit global ~/.coven-code/AGENTS.md\n\
                 /memory clear         — clear project AGENTS.md\n\
                 /memory clear global  — clear global AGENTS.md",
            );
        }

        CommandResult::Message(output)
    }
}

// ---- /bug ----------------------------------------------------------------

#[async_trait]
impl SlashCommand for BugCommand {
    fn name(&self) -> &str {
        "feedback"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["bug"]
    }
    fn description(&self) -> &str {
        "Submit feedback about Coven Code"
    }
    fn help(&self) -> &str {
        "Usage: /feedback [report]"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let report = args.trim();
        if report.is_empty() {
            CommandResult::Message(
                "To submit feedback or report a bug, visit: https://github.com/OpenCoven/coven-code/issues"
                    .to_string(),
            )
        } else {
            CommandResult::Message(format!(
                "To submit feedback or report a bug, visit: https://github.com/OpenCoven/coven-code/issues\nSuggested report summary: {}",
                report
            ))
        }
    }
}

// ---- /usage --------------------------------------------------------------

#[async_trait]
impl SlashCommand for UsageCommand {
    fn name(&self) -> &str {
        "usage"
    }
    fn description(&self) -> &str {
        "Show API usage, quotas, and rate limit status"
    }
    fn help(&self) -> &str {
        "Usage: /usage [cost|context|stats]\n\n\
         Shows current session API usage and account quota information.\n\
         Use /usage cost for the session cost breakdown, /usage context for\n\
         context-window details, or /usage stats for the full statistics view."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let mut parts = args.trim().splitn(2, char::is_whitespace);
        let subcommand = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or_default().trim();
        match subcommand {
            "" | "summary" | "quota" | "status" => {}
            "cost" | "costs" => return CostCommand.execute(rest, ctx).await,
            "context" | "window" => return ContextCommand.execute(rest, ctx).await,
            "stats" => return StatsCommand.execute(rest, ctx).await,
            other => {
                return CommandResult::Error(format!(
                    "Unknown usage view '{}'. Use: /usage [cost|context|stats]",
                    other
                ))
            }
        }

        let input = ctx.cost_tracker.input_tokens();
        let output = ctx.cost_tracker.output_tokens();
        let cache_creation = ctx.cost_tracker.cache_creation_tokens();
        let cache_read = ctx.cost_tracker.cache_read_tokens();
        let total = ctx.cost_tracker.total_tokens();
        let cost = ctx.cost_tracker.total_cost_usd();

        // Try to get account tier from OAuth tokens
        let account_info = match claurst_core::oauth::OAuthTokens::load().await {
            Some(tokens) => {
                let sub = tokens.subscription_type.as_deref().unwrap_or("unknown");
                format!("Plan: {}", sub)
            }
            None => {
                if ctx.config.resolve_api_key().is_some() {
                    "Plan: API key (Console billing)".to_string()
                } else {
                    "Plan: not authenticated — run /login".to_string()
                }
            }
        };

        CommandResult::Message(format!(
            "API Usage — Current Session\n\
             ────────────────────────────\n\
             {account_info}\n\
             Model:          {model}\n\n\
             Tokens used this session:\n\
               Input:        {input:>10}\n\
               Output:       {output:>10}\n\
               Cache write:  {cache_creation:>10}\n\
               Cache read:   {cache_read:>10}\n\
               Total:        {total:>10}\n\n\
             Estimated cost: ${cost:.4}\n\n\
             Use /cost for session cost details.",
            account_info = account_info,
            model = ctx.config.effective_model(),
            input = input,
            output = output,
            cache_creation = cache_creation,
            cache_read = cache_read,
            total = total,
            cost = cost,
        ))
    }
}

// ---- /plugin -------------------------------------------------------------

async fn loaded_plugin_registry(project_dir: &std::path::Path) -> claurst_plugins::PluginRegistry {
    if let Some(global) = claurst_plugins::global_plugin_registry() {
        (*global).clone()
    } else {
        let registry = claurst_plugins::load_plugins(project_dir, &[]).await;
        publish_plugin_registry(&registry);
        registry
    }
}

fn publish_plugin_registry(registry: &claurst_plugins::PluginRegistry) {
    claurst_plugins::set_global_hooks(registry.build_hook_registry());
    claurst_plugins::set_global_registry(registry.clone());
}

fn merge_plugin_mcp_servers(
    config: &mut claurst_core::config::Config,
    registry: &claurst_plugins::PluginRegistry,
) -> usize {
    let mut existing_names: std::collections::HashSet<String> =
        config.mcp_servers.iter().map(|s| s.name.clone()).collect();
    let mut added = 0;

    for mcp_server in registry.all_mcp_servers() {
        if existing_names.insert(mcp_server.name.clone()) {
            config.mcp_servers.push(mcp_server);
            added += 1;
        }
    }

    added
}

#[async_trait]
impl SlashCommand for PluginCommand {
    fn name(&self) -> &str {
        "plugin"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["plugins"]
    }
    fn description(&self) -> &str {
        "Manage plugins"
    }
    fn help(&self) -> &str {
        "Usage: /plugin [list|info <name>|enable <name>|disable <name>|install <path>|reload]\n\
         Manage Coven Code plugins.\n\n\
         Subcommands:\n\
           /plugin              — list all installed plugins\n\
           /plugin list         — list all installed plugins\n\
           /plugin info <name>  — show detailed info about a plugin\n\
           /plugin enable <name>   — enable a plugin (persisted to settings)\n\
           /plugin disable <name>  — disable a plugin (persisted to settings)\n\
           /plugin install <path>  — install a plugin from a local directory\n\
           /plugin reload       — reload plugins from disk"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let project_dir = ctx.working_dir.clone();

        let parsed = claurst_plugins::parse_plugin_args(args);
        match parsed {
            claurst_plugins::PluginSubCommand::List => {
                let registry = loaded_plugin_registry(&project_dir).await;
                CommandResult::Message(claurst_plugins::format_plugin_list(&registry))
            }
            claurst_plugins::PluginSubCommand::Enable(ref name) if name.is_empty() => {
                CommandResult::Error(
                    "Usage: /plugin enable <name>\nRun /plugin list to see installed plugins."
                        .to_string(),
                )
            }
            claurst_plugins::PluginSubCommand::Enable(name) => {
                let registry = loaded_plugin_registry(&project_dir).await;
                if registry.get(&name).is_none() {
                    return CommandResult::Error(format!(
                        "Plugin '{}' not found. Use `/plugin list` to see installed plugins.",
                        name
                    ));
                }
                let mut settings = claurst_core::config::Settings::load_sync().unwrap_or_default();
                settings.enabled_plugins.insert(name.clone());
                settings.disabled_plugins.remove(&name);
                let _ = settings.save_sync();
                CommandResult::Message(format!(
                    "Plugin '{}' enabled. Run `/plugin reload` to apply changes in this session.",
                    name
                ))
            }
            claurst_plugins::PluginSubCommand::Disable(ref name) if name.is_empty() => {
                CommandResult::Error(
                    "Usage: /plugin disable <name>\nRun /plugin list to see installed plugins."
                        .to_string(),
                )
            }
            claurst_plugins::PluginSubCommand::Disable(name) => {
                let registry = loaded_plugin_registry(&project_dir).await;
                if registry.get(&name).is_none() {
                    return CommandResult::Error(format!(
                        "Plugin '{}' not found. Use `/plugin list` to see installed plugins.",
                        name
                    ));
                }
                let mut settings = claurst_core::config::Settings::load_sync().unwrap_or_default();
                settings.disabled_plugins.insert(name.clone());
                settings.enabled_plugins.remove(&name);
                let _ = settings.save_sync();
                CommandResult::Message(format!(
                    "Plugin '{}' disabled. Run `/plugin reload` to apply changes in this session.",
                    name
                ))
            }
            claurst_plugins::PluginSubCommand::Info(ref name) if name.is_empty() => {
                CommandResult::Error(
                    "Usage: /plugin info <name>\nRun /plugin list to see installed plugins."
                        .to_string(),
                )
            }
            claurst_plugins::PluginSubCommand::Info(name) => {
                let registry = loaded_plugin_registry(&project_dir).await;
                CommandResult::Message(claurst_plugins::format_plugin_info(&registry, &name))
            }
            claurst_plugins::PluginSubCommand::Install(ref path) if path.is_empty() => {
                CommandResult::Error(
                    "Usage: /plugin install <path>\nProvide the path to a local plugin directory."
                        .to_string(),
                )
            }
            claurst_plugins::PluginSubCommand::Install(path) => {
                let result = claurst_plugins::install_plugin_from_path(std::path::Path::new(&path));
                match result {
                    Ok(name) => CommandResult::Message(format!(
                        "Plugin '{}' installed successfully. Run `/plugin reload` to activate it.",
                        name
                    )),
                    Err(e) => CommandResult::Error(format!("Install failed: {}", e)),
                }
            }
            claurst_plugins::PluginSubCommand::Reload => {
                let old_registry = loaded_plugin_registry(&project_dir).await;
                let (new_registry, diff) =
                    claurst_plugins::reload_plugins(&old_registry, &project_dir, &[]).await;
                merge_plugin_mcp_servers(&mut ctx.config, &new_registry);
                let summary = claurst_plugins::format_reload_summary(&new_registry, &diff);
                publish_plugin_registry(&new_registry);
                CommandResult::Message(summary)
            }
            claurst_plugins::PluginSubCommand::Help => CommandResult::Message(
                "Plugin commands:\n\
                     /plugin              — list all installed plugins\n\
                     /plugin list         — list all installed plugins\n\
                     /plugin info <name>  — show plugin details\n\
                     /plugin enable <name>   — enable a plugin\n\
                     /plugin disable <name>  — disable a plugin\n\
                     /plugin install <path>  — install plugin from local path\n\
                     /plugin reload       — reload plugins from disk"
                    .to_string(),
            ),
        }
    }
}

// ---- /reload-plugins -----------------------------------------------------

#[async_trait]
impl SlashCommand for ReloadPluginsCommand {
    fn name(&self) -> &str {
        "reload-plugins"
    }
    fn description(&self) -> &str {
        "Reload all plugins without restarting"
    }
    fn help(&self) -> &str {
        "Usage: /reload-plugins\n\
         Reloads all plugins and shows what changed."
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let project_dir = ctx.working_dir.clone();

        let old_registry = loaded_plugin_registry(&project_dir).await;
        let (new_registry, diff) =
            claurst_plugins::reload_plugins(&old_registry, &project_dir, &[]).await;
        merge_plugin_mcp_servers(&mut ctx.config, &new_registry);
        let summary = claurst_plugins::format_reload_summary(&new_registry, &diff);
        publish_plugin_registry(&new_registry);

        CommandResult::Message(summary)
    }
}

// ---- Plugin slash command adapter ----------------------------------------

/// Wraps a plugin-defined `PluginCommandDef` so it can be executed like a
/// built-in slash command.  The adapter is created on-the-fly inside
/// `execute_command` when no built-in matches the input.
pub struct PluginSlashCommandAdapter {
    pub def: claurst_plugins::PluginCommandDef,
}

#[async_trait]
impl SlashCommand for PluginSlashCommandAdapter {
    fn name(&self) -> &str {
        &self.def.name
    }

    fn description(&self) -> &str {
        &self.def.description
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        // Enforce capability grants before the action runs.
        if let Err(reason) = claurst_plugins::check_plugin_capability(&self.def) {
            return CommandResult::Error(reason);
        }

        match &self.def.run_action {
            claurst_plugins::CommandRunAction::StaticResponse(msg) => {
                CommandResult::Message(msg.clone())
            }
            claurst_plugins::CommandRunAction::MarkdownPrompt {
                file_path,
                plugin_root: _,
            } => {
                // Read the markdown file and inject it into the conversation
                match std::fs::read_to_string(file_path) {
                    Ok(content) => {
                        let full_prompt = if args.is_empty() {
                            content
                        } else {
                            format!("{}\n\nArguments: {}", content, args)
                        };
                        CommandResult::UserMessage(full_prompt)
                    }
                    Err(e) => CommandResult::Error(format!(
                        "Could not read plugin command file '{}': {}",
                        file_path, e
                    )),
                }
            }
            claurst_plugins::CommandRunAction::ShellCommand {
                command,
                plugin_root,
            } => {
                let full_cmd = if args.is_empty() {
                    command.clone()
                } else {
                    format!("{} {}", command, args)
                };
                let cmd_result =
                    std::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" })
                        .args(if cfg!(windows) {
                            vec!["/C", &full_cmd]
                        } else {
                            vec!["-c", &full_cmd]
                        })
                        .env("CLAUDE_PLUGIN_ROOT", plugin_root)
                        .output();
                match cmd_result {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        if out.status.success() {
                            CommandResult::Message(stdout.to_string())
                        } else {
                            CommandResult::Error(format!("Command failed:\n{}", stderr))
                        }
                    }
                    Err(e) => CommandResult::Error(format!("Failed to run command: {}", e)),
                }
            }
        }
    }
}

// ---- /doctor -------------------------------------------------------------

#[async_trait]
impl SlashCommand for DoctorCommand {
    fn name(&self) -> &str {
        "doctor"
    }
    fn description(&self) -> &str {
        "Check system health and diagnose issues"
    }
    fn help(&self) -> &str {
        "Usage: /doctor\n\
         Runs a comprehensive system diagnostics check:\n\
         - API key validation (live GET /v1/models call)\n\
         - Git availability\n\
         - MCP server connection status\n\
         - Disk space\n\
         - Config file integrity\n\
         - Tool permission summary\n\
         - Session lock state\n\
         - Coven Substrate (daemon health, api version, sessions, familiars)\n\
         - Coven Code version"
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let mut lines: Vec<String> = Vec::new();

        // ── Header ─────────────────────────────────────────────────────────
        lines.push(format!(
            "Coven Code v{}  |  {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
        ));
        lines.push(String::new());

        // ── API / Auth ──────────────────────────────────────────────────────
        lines.push("Authentication".to_string());
        let anthropic_auth = ctx
            .config
            .resolve_anthropic_auth_async()
            .await
            .unwrap_or((String::new(), false));
        let client_config = claurst_api::client::ClientConfig {
            api_key: anthropic_auth.0,
            api_base: ctx.config.resolve_anthropic_api_base(),
            use_bearer_auth: anthropic_auth.1,
            ..Default::default()
        };
        let provider_registry =
            claurst_api::ProviderRegistry::from_config(&ctx.config, client_config);
        let provider_id = claurst_core::ProviderId::new(ctx.config.selected_provider_id());
        match provider_registry.get(&provider_id) {
            Some(provider) => match provider.health_check().await {
                Ok(claurst_api::provider_types::ProviderStatus::Healthy) => {
                    lines.push(format!("  ✓ {} is healthy", provider.name()));
                }
                Ok(claurst_api::provider_types::ProviderStatus::Degraded { reason }) => {
                    lines.push(format!("  ⚠ {} is degraded: {}", provider.name(), reason));
                }
                Ok(claurst_api::provider_types::ProviderStatus::Unavailable { reason }) => {
                    lines.push(format!(
                        "  ✗ {} is unavailable: {}",
                        provider.name(),
                        reason
                    ));
                }
                Err(err) => {
                    lines.push(format!(
                        "  ✗ {} health check failed: {}",
                        provider.name(),
                        err
                    ));
                }
            },
            None => {
                let hint = claurst_core::config::primary_api_key_env_var_for_provider(
                    ctx.config.selected_provider_id(),
                )
                .map(|env| format!("set {env}"))
                .unwrap_or_else(|| "configure credentials".to_string());
                lines.push(format!(
                    "  ✗ No active provider runtime found — {} or use /connect",
                    hint
                ));
            }
        }
        // Show which model is active
        lines.push(format!(
            "  • Active model: {}",
            ctx.config.effective_model()
        ));
        lines.push(String::new());

        // ── Git ─────────────────────────────────────────────────────────────
        lines.push("Tools".to_string());
        let git_out = tokio::process::Command::new("git")
            .arg("--version")
            .output()
            .await;
        match git_out {
            Ok(o) if o.status.success() => {
                let ver = String::from_utf8_lossy(&o.stdout).trim().to_string();
                lines.push(format!("  ✓ {ver}"));
            }
            _ => lines.push("  ✗ git not found — many features require git".to_string()),
        }

        // Ripgrep
        let rg_out = tokio::process::Command::new("rg")
            .arg("--version")
            .output()
            .await;
        match rg_out {
            Ok(o) if o.status.success() => {
                let first = String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                lines.push(format!("  ✓ ripgrep: {first}"));
            }
            _ => lines.push(
                "  ⚠ ripgrep (rg) not found — Grep tool will fall back to built-in".to_string(),
            ),
        }
        lines.push(String::new());

        // ── Disk space ──────────────────────────────────────────────────────
        lines.push("Disk Space".to_string());
        #[cfg(windows)]
        {
            // On Windows use PowerShell to get free space for the current drive
            let ps_out = tokio::process::Command::new("powershell")
                .args(["-NoProfile", "-Command",
                    "Get-PSDrive -Name (Split-Path -Qualifier (Get-Location)) | \
                     Select-Object Name,@{N='Used(GB)';E={[math]::Round($_.Used/1GB,1)}},\
                     @{N='Free(GB)';E={[math]::Round($_.Free/1GB,1)}} | Format-Table -HideTableHeaders"])
                .output()
                .await;
            match ps_out {
                Ok(o) if o.status.success() => {
                    let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if out.is_empty() {
                        lines.push("  • Disk info unavailable".to_string());
                    } else {
                        for l in out.lines().take(3) {
                            lines.push(format!("  • {}", l.trim()));
                        }
                    }
                }
                _ => lines.push("  ⚠ Could not query disk space".to_string()),
            }
        }
        #[cfg(not(windows))]
        {
            let df_out = tokio::process::Command::new("df")
                .args(["-h", "."])
                .output()
                .await;
            match df_out {
                Ok(o) if o.status.success() => {
                    let out = String::from_utf8_lossy(&o.stdout);
                    // Print the header + the first data line (current filesystem)
                    for (i, l) in out.lines().enumerate().take(2) {
                        if i == 0 {
                            lines.push(format!("  • {}", l));
                        } else {
                            lines.push(format!("  ✓ {}", l));
                        }
                    }
                }
                _ => lines.push("  ⚠ Could not query disk space (`df -h .` failed)".to_string()),
            }
        }
        lines.push(String::new());

        // ── Config directory ────────────────────────────────────────────────
        lines.push("Configuration".to_string());
        let config_dir = claurst_core::config::Settings::config_dir();
        if config_dir.exists() {
            lines.push(format!("  ✓ Config dir: {}", config_dir.display()));
        } else {
            lines.push(format!("  ✗ Config dir missing: {}", config_dir.display()));
        }

        // Settings validation — try loading ~/.coven-code/settings.json
        let settings_path = config_dir.join("settings.json");
        if settings_path.exists() {
            match std::fs::read_to_string(&settings_path)
                .ok()
                .and_then(|s| serde_json::from_str::<claurst_core::config::Settings>(&s).ok())
            {
                Some(_) => lines.push("  ✓ settings.json valid".to_string()),
                None => {
                    // Try as raw JSON to distinguish missing vs invalid
                    match std::fs::read_to_string(&settings_path)
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    {
                        Some(_) => lines.push(
                            "  ⚠ settings.json is JSON but has unexpected structure".to_string(),
                        ),
                        None => lines.push(
                            "  ✗ settings.json is invalid JSON — run /config to repair".to_string(),
                        ),
                    }
                }
            }
        } else {
            lines.push("  • settings.json not found (defaults will be used)".to_string());
        }

        // AGENTS.md
        let claude_md = ctx.working_dir.join("AGENTS.md");
        if claude_md.exists() {
            lines.push("  ✓ AGENTS.md present in working directory".to_string());
        } else {
            lines.push(
                "  • No AGENTS.md in working directory (run /init to create one)".to_string(),
            );
        }
        lines.push(String::new());

        // ── MCP servers ─────────────────────────────────────────────────────
        lines.push("MCP Servers".to_string());
        let mcp_count = ctx.config.mcp_servers.len();
        if mcp_count == 0 {
            lines.push("  • No MCP servers configured".to_string());
        } else if let Some(mgr) = ctx.mcp_manager.as_ref() {
            // Report live connection status from the manager
            let statuses = mgr.all_statuses();
            for srv in ctx.config.mcp_servers.iter().take(12) {
                let status_str = match statuses.get(&srv.name) {
                    Some(claurst_mcp::McpServerStatus::Connected { tool_count }) => {
                        format!(
                            "  ✓ {} — connected ({} tool{})",
                            srv.name,
                            tool_count,
                            if *tool_count == 1 { "" } else { "s" }
                        )
                    }
                    Some(claurst_mcp::McpServerStatus::Connecting) => {
                        format!("  ⚠ {} — connecting…", srv.name)
                    }
                    Some(claurst_mcp::McpServerStatus::Disconnected {
                        last_error: Some(e),
                    }) => {
                        format!("  ✗ {} — failed: {}", srv.name, e)
                    }
                    Some(claurst_mcp::McpServerStatus::Disconnected { last_error: None }) => {
                        format!("  ✗ {} — disconnected", srv.name)
                    }
                    Some(claurst_mcp::McpServerStatus::Failed { error, .. }) => {
                        format!("  ✗ {} — failed: {}", srv.name, error)
                    }
                    None => format!("  ⚠ {} — not started", srv.name),
                };
                lines.push(status_str);
            }
            if mcp_count > 12 {
                lines.push(format!("    … and {} more", mcp_count - 12));
            }
        } else {
            // No live manager — just show configured names
            lines.push(format!(
                "  ✓ {mcp_count} MCP server(s) configured (not yet connected):"
            ));
            for srv in ctx.config.mcp_servers.iter().take(8) {
                lines.push(format!("    - {}", srv.name));
            }
            if mcp_count > 8 {
                lines.push(format!("    … and {} more", mcp_count - 8));
            }
        }
        lines.push(String::new());

        // ── Hooks ───────────────────────────────────────────────────────────
        lines.push("Hooks".to_string());
        let hook_count: usize = ctx.config.hooks.values().map(|v| v.len()).sum();
        if hook_count == 0 {
            lines.push("  • No hooks configured".to_string());
        } else {
            lines.push(format!(
                "  ✓ {hook_count} hook(s) configured across {} event(s)",
                ctx.config.hooks.len()
            ));
        }
        lines.push(String::new());

        // ── Tool permissions ─────────────────────────────────────────────────
        lines.push("Tool Permissions".to_string());
        let all_tool_names = claurst_tools::all_tool_names();
        let total_tools = all_tool_names.len();
        let allowed_count = ctx.config.allowed_tools.len();
        let denied_count = ctx.config.disallowed_tools.len();
        // Tools not in allowed or denied lists require user confirmation
        let explicit_tools: std::collections::HashSet<&str> = ctx
            .config
            .allowed_tools
            .iter()
            .chain(ctx.config.disallowed_tools.iter())
            .map(|s| s.as_str())
            .collect();
        let confirm_count = all_tool_names
            .iter()
            .filter(|n| !explicit_tools.contains(**n))
            .count();
        let mode_label = match ctx.config.permission_mode {
            claurst_core::PermissionMode::BypassPermissions => {
                "bypass-permissions (no confirmation required)"
            }
            claurst_core::PermissionMode::AcceptEdits => "accept-edits (file edits auto-approved)",
            claurst_core::PermissionMode::Plan => "plan (read-only, no writes)",
            claurst_core::PermissionMode::Default => "default (confirm destructive actions)",
        };
        lines.push(format!("  • Mode: {mode_label}"));
        lines.push(format!("  • Total built-in tools: {total_tools}"));
        if allowed_count > 0 {
            lines.push(format!(
                "  ✓ Always allowed: {} tool(s) — {}",
                allowed_count,
                ctx.config.allowed_tools.join(", ")
            ));
        }
        if denied_count > 0 {
            lines.push(format!(
                "  ✗ Always denied: {} tool(s) — {}",
                denied_count,
                ctx.config.disallowed_tools.join(", ")
            ));
        }
        if ctx.config.permission_mode == claurst_core::PermissionMode::Default {
            lines.push(format!(
                "  ⚠ Require confirmation: {} tool(s)",
                confirm_count
            ));
        }
        lines.push(String::new());

        // ── Session / lock ──────────────────────────────────────────────────
        lines.push("Session".to_string());
        let lock_path = config_dir.join("claude.lock");
        if lock_path.exists() {
            lines.push("  ⚠ Lock file exists — another instance may be running".to_string());
        } else {
            lines.push("  ✓ No stale lock file".to_string());
        }
        lines.push(format!("  • Session ID: {}", ctx.session_id));
        lines.push(format!("  • Working dir: {}", ctx.working_dir.display()));
        lines.push(String::new());

        // ── Coven Substrate ────────────────────────────────────────────────
        // The Coven daemon is optional — coven-code works standalone. This
        // section reports whether the substrate is reachable and surfaces
        // the api version + structured-error capability so users can tell
        // at a glance whether `/coven` integration is fully wired up.
        lines.push("Coven Substrate".to_string());
        match claurst_core::coven_shared::DaemonClient::new() {
            None => {
                lines.push("  • Daemon not installed (no ~/.coven/coven.sock)".to_string());
                lines.push(
                    "    Install: npm install -g @opencoven/coven  →  coven daemon start"
                        .to_string(),
                );
            }
            Some(client) => {
                if !client.is_online() {
                    lines.push("  ✗ Daemon socket present but not responding".to_string());
                    lines.push("    Try: coven daemon restart".to_string());
                } else {
                    match client.health() {
                        Ok(h) => {
                            let api_ok = h.api_version == COVEN_DAEMON_EXPECTED_API;
                            let marker = if api_ok { "✓" } else { "⚠" };
                            lines.push(format!(
                                "  {} Daemon online ({}, coven {})",
                                marker,
                                if h.api_version.is_empty() {
                                    "unknown api"
                                } else {
                                    h.api_version.as_str()
                                },
                                if h.coven_version.is_empty() {
                                    "?"
                                } else {
                                    h.coven_version.as_str()
                                },
                            ));
                            if !api_ok && !h.api_version.is_empty() {
                                lines.push(format!(
                                    "    Expected {COVEN_DAEMON_EXPECTED_API}; \
                                     `/coven` features may not work."
                                ));
                            }
                            match client.active_sessions() {
                                Ok(sessions) => {
                                    lines.push(format!(
                                        "  • Active daemon sessions: {}",
                                        sessions.len()
                                    ));
                                }
                                Err(e) => {
                                    lines.push(format!("  ⚠ Could not list sessions: {e}"));
                                }
                            }
                            match client.familiar_statuses() {
                                Ok(fams) => {
                                    lines.push(format!("  • Familiars registered: {}", fams.len()));
                                }
                                Err(_) => {
                                    // Familiar listing is best-effort.
                                }
                            }
                        }
                        Err(e) => {
                            lines.push(format!("  ⚠ Health check failed: {e}"));
                        }
                    }
                }
            }
        }

        CommandResult::Message(lines.join("\n"))
    }
}

// ---- /login --------------------------------------------------------------

#[async_trait]
impl SlashCommand for LoginCommand {
    fn name(&self) -> &str {
        "login"
    }
    fn description(&self) -> &str {
        "Authenticate with Anthropic or Codex (multi-account)"
    }
    fn help(&self) -> &str {
        "Usage: /login [--console] [--codex] [--label <name>]\n\n\
         Start an OAuth login. Anthropic OAuth requires a configured Coven Code\n\
         client ID via COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID; use\n\
         ANTHROPIC_API_KEY until that client is configured. Pass `--codex` to\n\
         add a ChatGPT/Codex account. `--label work` names the saved profile so\n\
         you can `switch` to it later by that name."
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let tokens: Vec<&str> = args.split_whitespace().collect();
        let use_codex = tokens.contains(&"--codex");
        let login_with_claude_ai = !tokens.contains(&"--console");
        let label = parse_label_arg(&tokens);

        let provider = if use_codex {
            claurst_core::accounts::PROVIDER_CODEX
        } else {
            claurst_core::accounts::PROVIDER_ANTHROPIC
        };

        CommandResult::StartLoginForProvider {
            provider: provider.to_string(),
            login_with_claude_ai,
            label,
        }
    }
}

fn parse_label_arg(tokens: &[&str]) -> Option<String> {
    let mut it = tokens.iter();
    while let Some(t) = it.next() {
        if *t == "--label" || *t == "-l" {
            return it.next().map(|s| s.to_string());
        }
        if let Some(rest) = t.strip_prefix("--label=") {
            return Some(rest.to_string());
        }
    }
    None
}

// ---- /logout -------------------------------------------------------------

#[async_trait]
impl SlashCommand for LogoutCommand {
    fn name(&self) -> &str {
        "logout"
    }
    fn description(&self) -> &str {
        "Clear credentials for the active account"
    }
    fn help(&self) -> &str {
        "Usage: /logout [--codex] [--all]\n\n\
         By default removes the active Anthropic account. `--codex` targets\n\
         Codex instead. `--all` purges every stored credential for the chosen\n\
         provider and clears any API key in settings."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let tokens: Vec<&str> = args.split_whitespace().collect();
        let use_codex = tokens.contains(&"--codex");
        let purge_all = tokens.contains(&"--all");

        if use_codex {
            if purge_all {
                let mut registry = claurst_core::accounts::AccountRegistry::load();
                let ids: Vec<String> = registry
                    .list(claurst_core::accounts::PROVIDER_CODEX)
                    .into_iter()
                    .map(|p| p.id)
                    .collect();
                for id in &ids {
                    let _ = registry.remove(claurst_core::accounts::PROVIDER_CODEX, id);
                }
                return CommandResult::Message(format!(
                    "Removed {} stored Codex account(s).",
                    ids.len()
                ));
            }
            if let Err(e) = claurst_core::oauth_config::clear_codex_tokens() {
                return CommandResult::Error(format!("Failed to clear Codex tokens: {}", e));
            }
            return CommandResult::Message("Logged out of the active Codex account.".to_string());
        }

        // Anthropic logout.
        if purge_all {
            let mut registry = claurst_core::accounts::AccountRegistry::load();
            let ids: Vec<String> = registry
                .list(claurst_core::accounts::PROVIDER_ANTHROPIC)
                .into_iter()
                .map(|p| p.id)
                .collect();
            for id in &ids {
                let _ = registry.remove(claurst_core::accounts::PROVIDER_ANTHROPIC, id);
            }
            let mut settings = claurst_core::config::Settings::load()
                .await
                .unwrap_or_default();
            settings.config.api_key = None;
            let _ = settings.save().await;
            ctx.config.api_key = None;
            return CommandResult::Message(format!(
                "Removed {} stored Anthropic account(s) and cleared API key.",
                ids.len()
            ));
        }

        if let Err(e) = claurst_core::oauth::OAuthTokens::clear().await {
            return CommandResult::Error(format!("Failed to clear OAuth tokens: {}", e));
        }
        let mut settings = claurst_core::config::Settings::load()
            .await
            .unwrap_or_default();
        settings.config.api_key = None;
        if let Err(e) = settings.save().await {
            return CommandResult::Error(format!("Failed to update settings: {}", e));
        }
        ctx.config.api_key = None;
        CommandResult::Message("Logged out of the active Anthropic account.".to_string())
    }
}

// ---- /accounts ------------------------------------------------------------

pub struct AccountsCommand;

#[async_trait]
impl SlashCommand for AccountsCommand {
    fn name(&self) -> &str {
        "accounts"
    }
    fn description(&self) -> &str {
        "List stored Anthropic and Codex accounts"
    }
    fn help(&self) -> &str {
        "Usage: /accounts\n\n\
         Lists every stored Anthropic and Codex account along with the\n\
         currently active one (marked with `*`). Use /switch to change\n\
         accounts, /login to add a new one, /logout to remove one."
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let registry = claurst_core::accounts::AccountRegistry::load();
        let mut out = String::new();
        for (provider, label) in [
            (claurst_core::accounts::PROVIDER_ANTHROPIC, "Anthropic"),
            (claurst_core::accounts::PROVIDER_CODEX, "Codex"),
        ] {
            let profiles = registry.list(provider);
            let active = registry.active(provider);
            if profiles.is_empty() {
                out.push_str(&format!("{}: (no accounts stored)\n", label));
                continue;
            }
            out.push_str(&format!("{}:\n", label));
            for p in profiles {
                let marker = if active == Some(&p.id) { "*" } else { " " };
                let email = p.email.as_deref().unwrap_or("");
                let tier = p
                    .subscription_tier
                    .as_deref()
                    .map(|t| format!(" [{}]", t))
                    .unwrap_or_default();
                out.push_str(&format!("  {} {}{}  {}\n", marker, p.id, tier, email));
            }
        }
        if out.is_empty() {
            out.push_str("No accounts stored. Use /login to add one.");
        }
        CommandResult::Message(out.trim_end().to_string())
    }
}

// ---- /switch --------------------------------------------------------------

pub struct SwitchCommand;

#[async_trait]
impl SlashCommand for SwitchCommand {
    fn name(&self) -> &str {
        "switch"
    }
    fn description(&self) -> &str {
        "Switch the active account for a provider"
    }
    fn help(&self) -> &str {
        "Usage: /switch [--codex] <profile-id>\n\n\
         Make a stored account active. Defaults to Anthropic; pass `--codex`\n\
         to switch the Codex account instead. Run /switch with no arguments\n\
         to list stored accounts and see available profile ids."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        // /switch with no arguments lists accounts (absorbs the former /accounts).
        if args.trim().is_empty() {
            return AccountsCommand.execute("", ctx).await;
        }
        let tokens: Vec<&str> = args.split_whitespace().collect();
        let use_codex = tokens.contains(&"--codex");
        let provider = if use_codex {
            claurst_core::accounts::PROVIDER_CODEX
        } else {
            claurst_core::accounts::PROVIDER_ANTHROPIC
        };
        let display = if use_codex { "Codex" } else { "Anthropic" };
        let id = tokens.iter().find(|t| !t.starts_with("--"));

        let Some(id) = id else {
            return CommandResult::Error(format!(
                "Usage: /switch {}<profile-id> (try /accounts to see options)",
                if use_codex { "--codex " } else { "" }
            ));
        };

        let mut registry = claurst_core::accounts::AccountRegistry::load();
        match registry.switch_to(provider, id) {
            Ok(()) => {
                CommandResult::Message(format!("Switched {} active account to '{}'.", display, id))
            }
            Err(e) => CommandResult::Error(format!("{}", e)),
        }
    }
}

// ---- /refresh ------------------------------------------------------------

#[async_trait]
impl SlashCommand for RefreshCommand {
    fn name(&self) -> &str {
        "refresh"
    }
    fn description(&self) -> &str {
        "Clear saved provider auth and model caches"
    }
    fn help(&self) -> &str {
        "Usage: /refresh\n\n\
         Clears saved provider credentials, provider/model selection, and model caches, then rebuilds the live runtime state.\n\
         After refreshing, run /connect to authenticate and choose a provider again."
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        if !args.trim().is_empty() {
            return CommandResult::Error("Usage: /refresh".to_string());
        }
        CommandResult::RefreshProviderState
    }
}

// ---- /caveman, /rocky, /normal -------------------------------------------

fn parse_speech_level(args: &str) -> String {
    match args.trim().to_lowercase().as_str() {
        "lite" | "light" => "lite".to_string(),
        "ultra" | "heavy" => "ultra".to_string(),
        "" | "full" | "moderate" | "default" => "full".to_string(),
        other => {
            // Unknown level, default to full
            tracing::warn!(level = other, "Unknown speech level, using full");
            "full".to_string()
        }
    }
}

#[async_trait]
impl SlashCommand for IncantCommand {
    fn name(&self) -> &str {
        "incant"
    }
    fn description(&self) -> &str {
        "Cast a speech incantation — change the model's voice (caveman, rocky) or lift it"
    }
    fn help(&self) -> &str {
        "Usage: /incant <voice> [lite|full|ultra] | /incant off\n\n\
         Incantations change how the model speaks, trading flourish for tokens.\n\
         Voices:\n\
         - caveman: why use many token when few token do trick (~75% reduction)\n\
         - rocky:   Eridian engineer from Project Hail Mary. Save big token. Good good good.\n\
         Intensity:\n\
         - lite:  light touch (~40% reduction)\n\
         - full:  the default (~75% reduction)\n\
         - ultra: maximum compression\n\n\
         /incant off lifts the active incantation and returns to normal speech."
    }
    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let mut words = args.split_whitespace();
        let voice = words.next().unwrap_or("");
        let rest = words.next().unwrap_or("");
        match voice {
            "" => CommandResult::Error(
                "Usage: /incant <caveman|rocky> [lite|full|ultra] | /incant off".to_string(),
            ),
            "off" | "none" | "normal" => CommandResult::SpeechMode {
                mode: None,
                level: "full".to_string(),
            },
            "caveman" | "rocky" => CommandResult::SpeechMode {
                mode: Some(voice.to_string()),
                level: parse_speech_level(rest),
            },
            other => CommandResult::Error(format!(
                "Unknown incantation '{}'. Known voices: caveman, rocky. Use /incant off to lift.",
                other
            )),
        }
    }
}

// ---- /init ---------------------------------------------------------------

#[async_trait]
impl SlashCommand for InitCommand {
    fn name(&self) -> &str {
        "init"
    }
    fn description(&self) -> &str {
        "Initialize a new project with AGENTS.md"
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let path = ctx.working_dir.join("AGENTS.md");
        if path.exists() {
            return CommandResult::Message(format!(
                "AGENTS.md already exists at {}",
                path.display()
            ));
        }

        let default_content = "# Project Instructions\n\n\
            Add project-specific instructions and context here.\n\n\
            ## Guidelines\n\n\
            - Describe your project structure\n\
            - Note any coding conventions\n\
            - List important files and their purposes\n";

        match tokio::fs::write(&path, default_content).await {
            Ok(()) => CommandResult::Message(format!("Created AGENTS.md at {}", path.display())),
            Err(e) => CommandResult::Error(format!("Failed to create AGENTS.md: {}", e)),
        }
    }
}

// ---- /review -------------------------------------------------------------

#[async_trait]
impl SlashCommand for ReviewCommand {
    fn name(&self) -> &str {
        "review"
    }
    fn description(&self) -> &str {
        "Review code changes via LLM and optionally post to GitHub PR"
    }
    fn help(&self) -> &str {
        "Usage: /review [base-ref] | /review security [path] | /review ultra [path]\n\n\
         Runs `git diff <base>...HEAD` (or `git diff --cached` when no base is given),\n\
         sends the diff to the LLM for a structured review, then optionally posts the\n\
         review as a comment to the associated GitHub PR.\n\n\
         Variants:\n\
           /review security  — security-focused review (vulnerabilities, secrets, injection)\n\
           /review ultra     — exhaustive multi-dimensional review\n\n\
         GitHub posting requires:\n\
           GITHUB_TOKEN  — a personal access token with repo scope\n\
           CLAUDE_PR_NUMBER — the PR number (auto-detected from `git remote` if absent)\n\n\
         Examples:\n\
           /review            # diff of staged changes\n\
           /review main       # diff from main..HEAD\n\
           /review origin/main"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let trimmed = args.trim();
        // /review security and /review ultra absorb the former standalone
        // /security-review and /ultrareview commands.
        let (head, rest) = match trimmed.split_once(char::is_whitespace) {
            Some((h, r)) => (h, r.trim()),
            None => (trimmed, ""),
        };
        match head {
            "security" => return SecurityReviewCommand.execute(rest, ctx).await,
            "ultra" | "ultrareview" => return UltrareviewCommand.execute(rest, ctx).await,
            _ => {}
        }
        let base = trimmed;

        // ------------------------------------------------------------------
        // 1. Collect the diff
        // ------------------------------------------------------------------
        let repo_root = claurst_core::git_utils::get_repo_root(&ctx.working_dir)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let diff = if base.is_empty() {
            // No base given — use staged changes; fall back to unstaged if empty.
            let staged = claurst_core::git_utils::get_staged_diff(&repo_root);
            if staged.is_empty() {
                claurst_core::git_utils::get_unstaged_diff(&repo_root)
            } else {
                staged
            }
        } else {
            // Run `git diff <base>...HEAD`
            let out = std::process::Command::new("git")
                .current_dir(&repo_root)
                .args(["diff", &format!("{}...HEAD", base)])
                .output();
            match out {
                Ok(o) if o.status.success() => {
                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    return CommandResult::Error(format!("git diff failed: {}", stderr.trim()));
                }
                Err(e) => return CommandResult::Error(format!("Failed to run git: {}", e)),
            }
        };

        if diff.is_empty() {
            return CommandResult::Message(
                "No diff found. Stage some changes or provide a base ref (e.g. /review main)."
                    .to_string(),
            );
        }

        // ------------------------------------------------------------------
        // 2. Summarise changed files for the TUI header
        // ------------------------------------------------------------------
        let changed_files: Vec<&str> = diff
            .lines()
            .filter(|l| l.starts_with("diff --git "))
            .filter_map(|l| {
                // "diff --git a/foo/bar.rs b/foo/bar.rs"  -> "foo/bar.rs"
                let parts: Vec<&str> = l.split(' ').collect();
                parts.get(3).map(|p| p.trim_start_matches("b/"))
            })
            .collect();

        let file_summary = if changed_files.is_empty() {
            "Changed files: (unknown)".to_string()
        } else {
            format!(
                "Changed files ({}):\n{}",
                changed_files.len(),
                changed_files
                    .iter()
                    .map(|f| format!("  - {}", f))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        // Truncate diff to a sensible size for the LLM (≈ 100 k chars).
        const MAX_DIFF_CHARS: usize = 100_000;
        let diff_for_llm = if diff.len() > MAX_DIFF_CHARS {
            format!(
                "{}\n\n[... diff truncated at {} chars ...]",
                &diff[..MAX_DIFF_CHARS],
                MAX_DIFF_CHARS
            )
        } else {
            diff.clone()
        };

        // ------------------------------------------------------------------
        // 3. Call the LLM for a structured PR review
        // ------------------------------------------------------------------
        let model = ctx.config.effective_model().to_string();
        let provider = match provider_for_config(&ctx.config).await {
            Some(provider) => provider,
            None => {
                return CommandResult::Error(
                    "Cannot initialise provider client for code review.".to_string(),
                );
            }
        };

        let review_prompt = format!(
            "You are a senior software engineer performing a pull-request code review.\n\
             Provide a concise, actionable review of the following diff.\n\n\
             Structure your response as:\n\
             ## Summary\n\
             (1-3 sentences describing what changed)\n\n\
             ## Issues\n\
             (bulleted list: [CRITICAL|MAJOR|MINOR] file:line — description; \
             omit section if none)\n\n\
             ## Suggestions\n\
             (bulleted list of optional improvements; omit section if none)\n\n\
             ## Verdict\n\
             APPROVE / REQUEST_CHANGES / COMMENT — one line with brief rationale\n\n\
             ---\n\
             {}\n\n\
             ```diff\n\
             {}\n\
             ```",
            file_summary, diff_for_llm
        );

        let request = claurst_api::ProviderRequest {
            model,
            messages: vec![Message::user(review_prompt)],
            system_prompt: Some(claurst_api::SystemPrompt::Text(
                "You are a thorough, constructive code reviewer. \
                 Be concise but precise. Focus on correctness, security, and maintainability."
                    .to_string(),
            )),
            tools: vec![],
            max_tokens: 4096,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: vec![],
            thinking: None,
            provider_options: serde_json::Value::Object(Default::default()),
        };

        let review_text = match provider.create_message(request).await {
            Err(e) => {
                return CommandResult::Error(format!("LLM call failed: {}", e));
            }
            Ok(response) => {
                let text = text_from_content_blocks(&response.content);
                if text.trim().is_empty() {
                    return CommandResult::Error("LLM returned an empty review.".to_string());
                }
                text
            }
        };

        // ------------------------------------------------------------------
        // 4. Optionally post to GitHub PR
        // ------------------------------------------------------------------
        let github_token = std::env::var("GITHUB_TOKEN").ok();
        let mut github_post_result: Option<String> = None;

        if let Some(ref token) = github_token {
            // Determine PR number
            let pr_number: Option<u64> = std::env::var("CLAUDE_PR_NUMBER")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .or_else(|| detect_pr_number_from_git(&repo_root));

            if let Some(pr_num) = pr_number {
                // Determine owner/repo from git remote
                if let Some((owner, repo)) = detect_github_owner_repo(&repo_root) {
                    let comment_body = format!(
                        "## Coven Code Code Review\n\n{}\n\n---\n*Generated by [Coven Code](https://claude.ai/claude-code)*",
                        review_text
                    );

                    let url = format!(
                        "https://api.github.com/repos/{}/{}/issues/{}/comments",
                        owner, repo, pr_num
                    );

                    let http = reqwest::Client::new();
                    let post_result = http
                        .post(&url)
                        .header("Authorization", format!("Bearer {}", token))
                        .header("User-Agent", "coven-code/1.0")
                        .header("Accept", "application/vnd.github+json")
                        .json(&serde_json::json!({ "body": comment_body }))
                        .send()
                        .await;

                    match post_result {
                        Ok(resp) if resp.status().is_success() => {
                            github_post_result = Some(format!(
                                "\nPosted review comment to PR #{} ({}/{}).",
                                pr_num, owner, repo
                            ));
                        }
                        Ok(resp) => {
                            let status = resp.status().as_u16();
                            let body = resp.text().await.unwrap_or_default();
                            github_post_result =
                                Some(format!("\nGitHub API returned {}: {}", status, body));
                        }
                        Err(e) => {
                            github_post_result = Some(format!("\nFailed to post to GitHub: {}", e));
                        }
                    }
                } else {
                    github_post_result = Some(
                        "\n(Could not detect GitHub owner/repo from git remote — \
                         review not posted.)"
                            .to_string(),
                    );
                }
            } else {
                github_post_result = Some(
                    "\n(GITHUB_TOKEN set but no PR number found. \
                     Set CLAUDE_PR_NUMBER=<n> to post the review.)"
                        .to_string(),
                );
            }
        }

        // ------------------------------------------------------------------
        // 5. Compose and return the final output
        // ------------------------------------------------------------------
        let mut output = format!("## Code Review\n\n{}\n\n{}", file_summary, review_text);

        if let Some(ref note) = github_post_result {
            output.push_str(note);
        }

        CommandResult::Message(output)
    }
}

/// Try to detect the PR number from the GitHub API via `gh` CLI, then fall
/// back to parsing the upstream tracking branch name (e.g. `pr/42/head`).
fn detect_pr_number_from_git(repo_root: &std::path::Path) -> Option<u64> {
    // Attempt `gh pr view --json number -q .number`
    let out = std::process::Command::new("gh")
        .current_dir(repo_root)
        .args(["pr", "view", "--json", "number", "-q", ".number"])
        .output()
        .ok()?;

    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout);
        return s.trim().parse::<u64>().ok();
    }

    // Fallback: look at the upstream tracking ref for a pattern like
    // `refs/pull/42/head` or branch name `pr/42`.
    let tracking = std::process::Command::new("git")
        .current_dir(repo_root)
        .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Pattern: "origin/pr/42" or "refs/pull/42/head"
    for segment in tracking.split('/') {
        if let Ok(n) = segment.parse::<u64>() {
            return Some(n);
        }
    }

    None
}

/// Parse `origin` remote URL to extract GitHub owner and repo name.
/// Handles both HTTPS (`https://github.com/owner/repo.git`) and
/// SSH (`git@github.com:owner/repo.git`) formats.
fn detect_github_owner_repo(repo_root: &std::path::Path) -> Option<(String, String)> {
    let remote_url = std::process::Command::new("git")
        .current_dir(repo_root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;

    parse_github_remote_url(&remote_url)
}

fn parse_github_remote_url(url: &str) -> Option<(String, String)> {
    // HTTPS: https://github.com/owner/repo.git  or  https://github.com/owner/repo
    if let Some(rest) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
    {
        let clean = rest.trim_end_matches(".git");
        let mut parts = clean.splitn(2, '/');
        let owner = parts.next()?.to_string();
        let repo = parts.next()?.to_string();
        return Some((owner, repo));
    }

    // SSH: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let clean = rest.trim_end_matches(".git");
        let mut parts = clean.splitn(2, '/');
        let owner = parts.next()?.to_string();
        let repo = parts.next()?.to_string();
        return Some((owner, repo));
    }

    None
}

// ---- /import-config ------------------------------------------------------

#[async_trait]
impl SlashCommand for ImportConfigCommand {
    fn name(&self) -> &str {
        "import-config"
    }
    fn description(&self) -> &str {
        "Import CLAUDE.md and settings.json from ~/.claude"
    }
    fn help(&self) -> &str {
        "Usage: /import-config\n\
         Import user-level Claude Code configuration from ~/.claude:\n\
           - ~/.claude/CLAUDE.md\n\
           - ~/.claude/settings.json\n\n\
         This command opens an interactive import dialog with preview and confirmation."
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        CommandResult::OpenImportConfigOverlay
    }
}

// ---- /hooks --------------------------------------------------------------

#[async_trait]
impl SlashCommand for HooksCommand {
    fn name(&self) -> &str {
        "hooks"
    }
    fn description(&self) -> &str {
        "Show configured event hooks"
    }
    fn help(&self) -> &str {
        "Usage: /hooks\n\
         Show hooks configured in settings.json under 'hooks'.\n\
         Hooks fire shell commands on events: PreToolUse, PostToolUse, Stop, UserPromptSubmit."
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        // In TUI mode this command is intercepted by intercept_slash_command("hooks")
        // before execute() is ever called, so this path only runs in non-TUI
        // contexts (e.g., `claude hooks` on the CLI, pipes, or tests).
        //
        // Signal to the CLI driver that it should open the TUI overlay if possible;
        // the CLI will fall back to the text listing when no TUI is active.
        if ctx.config.hooks.is_empty() {
            // If there is nothing to show in the overlay, emit a helpful message
            // so the user knows what to do.
            return CommandResult::Message(
                "No hooks configured.\n\
                 Add hooks to ~/.coven-code/settings.json under the 'hooks' key.\n\
                 Example:\n\
                 \x20 \"hooks\": {\n\
                 \x20   \"PreToolUse\": [{ \"matcher\": \"*\", \"hooks\": [{ \"type\": \"command\", \"command\": \"echo $STDIN\" }] }]\n\
                 \x20 }"
                    .to_string(),
            );
        }

        // Return the overlay-open signal; the CLI driver will call
        // app.hooks_config_menu.open() or fall back to text output if running
        // without a TUI.
        CommandResult::OpenHooksOverlay
    }
}

// ---- /mcp ----------------------------------------------------------------

#[async_trait]
impl SlashCommand for McpCommand {
    fn name(&self) -> &str {
        "mcp"
    }
    fn description(&self) -> &str {
        "Show MCP server status and manage connections"
    }
    fn help(&self) -> &str {
        "Usage: /mcp [list|status|auth <server>|connect <server>|logs <server>|resources|prompts|get-prompt ...]\n\n\
         Manages Model Context Protocol (MCP) servers.\n\
         MCP servers extend Coven Code with external tools, resources, and prompt templates.\n\n\
         Subcommands:\n\
           /mcp                        — list configured servers with live status\n\
           /mcp list                   — same as above\n\
           /mcp status                 — detailed connection status for all servers\n\
           /mcp auth <server>          — show OAuth auth instructions for a server\n\
           /mcp connect <server>       — reconnect a disconnected server\n\
           /mcp logs <server>          — show recent errors/logs for a server\n\
           /mcp resources [server]     — list resources from connected servers\n\
           /mcp prompts [server]       — list prompt templates from connected servers\n\
           /mcp get-prompt <server> <prompt> [key=value ...]  — expand a prompt template\n\n\
         To add/remove MCP servers, edit ~/.coven-code/settings.json\n\
         under the 'mcpServers' key.\n\
         Docs: https://docs.anthropic.com/claude-code/mcp"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let sub = args.trim();
        let first_word = sub.split_whitespace().next().unwrap_or("");

        // Delegate live-server subcommands (resources/prompts/get-prompt) to the async helper.
        if matches!(first_word, "resources" | "prompts" | "get-prompt") {
            if let Some(result) = McpCommand::handle_live_subcommand(sub, ctx).await {
                return result;
            }
            // Manager not available — fall through to show configured servers
        }

        // /mcp auth <server-name>
        if first_word == "auth" {
            let server_name = sub["auth".len()..].trim();
            if server_name.is_empty() {
                return CommandResult::Error(
                    "Usage: /mcp auth <server-name>\n\
                     Example: /mcp auth my-server"
                        .to_string(),
                );
            }
            return McpCommand::handle_auth(server_name, ctx).await;
        }

        // /mcp tools [server-name]
        if first_word == "tools" {
            let rest = sub["tools".len()..].trim();
            let server_filter = if rest.is_empty() { None } else { Some(rest) };
            return McpCommand::handle_tools(server_filter, ctx);
        }

        // /mcp connect <server-name>
        if first_word == "connect" {
            let server_name = sub["connect".len()..].trim();
            if server_name.is_empty() {
                return CommandResult::Error(
                    "Usage: /mcp connect <server-name>\n\
                     Example: /mcp connect my-server"
                        .to_string(),
                );
            }
            return McpCommand::handle_connect(server_name, ctx).await;
        }

        // /mcp logs <server-name>
        if first_word == "logs" {
            let server_name = sub["logs".len()..].trim();
            if server_name.is_empty() {
                return CommandResult::Error(
                    "Usage: /mcp logs <server-name>\n\
                     Example: /mcp logs my-server"
                        .to_string(),
                );
            }
            return McpCommand::handle_logs(server_name, ctx);
        }

        if ctx.config.mcp_servers.is_empty() {
            return CommandResult::Message(
                "No MCP servers configured.\n\n\
                 To add a MCP server, edit ~/.coven-code/settings.json:\n\
                 {\n\
                   \"mcpServers\": [\n\
                     {\n\
                       \"name\": \"my-server\",\n\
                       \"command\": \"npx\",\n\
                       \"args\": [\"-y\", \"@modelcontextprotocol/server-filesystem\", \"/tmp\"]\n\
                     }\n\
                   ]\n\
                 }\n\n\
                 Docs: https://docs.anthropic.com/claude-code/mcp"
                    .to_string(),
            );
        }

        // /mcp status — detailed status table
        if sub == "status" {
            let mut output = String::from("MCP Server Status\n─────────────────\n");
            for srv in &ctx.config.mcp_servers {
                let kind = srv.server_type.as_str();
                let endpoint = srv
                    .url
                    .as_deref()
                    .or(srv.command.as_deref())
                    .unwrap_or("(unknown)");

                // Fetch live status from the manager if available.
                let live_status = ctx
                    .mcp_manager
                    .as_ref()
                    .map(|m| m.server_status(&srv.name).display())
                    .unwrap_or_else(|| "unknown (manager not active)".to_string());

                output.push_str(&format!(
                    "  {name:20} [{kind:10}] {status}\n    endpoint: {endpoint}\n",
                    name = srv.name,
                    kind = kind,
                    status = live_status,
                    endpoint = endpoint,
                ));
            }
            if ctx.mcp_manager.is_none() {
                output.push_str(
                    "\nNote: MCP manager is not active in this session.\n\
                     Restart Coven Code to connect to MCP servers.\n\
                     Use /mcp connect <server> to retry a single server.",
                );
            }
            return CommandResult::Message(output);
        }

        // Default: /mcp or /mcp list — show configured servers with live status inline
        let manager = ctx.mcp_manager.as_ref();
        let mut output = format!(
            "Configured MCP Servers ({})\n──────────────────────────\n",
            ctx.config.mcp_servers.len()
        );
        for srv in &ctx.config.mcp_servers {
            let cmd_display = if let Some(ref url) = srv.url {
                format!("url={}", url)
            } else if let Some(ref cmd) = srv.command {
                let args_str = srv.args.join(" ");
                if args_str.is_empty() {
                    cmd.clone()
                } else {
                    format!("{} {}", cmd, args_str)
                }
            } else {
                "(no command)".to_string()
            };

            let status_str = manager
                .map(|m| m.server_status(&srv.name).display())
                .unwrap_or_else(|| "not running".to_string());

            output.push_str(&format!(
                "  {name}  [{status}]\n    type: {type_}  |  {cmd}\n",
                name = srv.name,
                status = status_str,
                type_ = srv.server_type,
                cmd = cmd_display,
            ));
        }
        output.push_str(
            "\nSubcommands: status | auth <server> | connect <server> | logs <server>\n\
             Also: resources | prompts | get-prompt <server> <prompt> [key=val ...]",
        );
        CommandResult::Message(output)
    }
}

impl McpCommand {
    /// Handle `/mcp auth <server>` — initiate OAuth or show auth instructions.
    ///
    /// For HTTP/SSE servers: runs the browser-based OAuth flow, stores the
    /// resulting token, and requests the runtime to reconnect.
    ///
    /// For stdio servers: shows env-var auth instructions.
    async fn handle_auth(server_name: &str, ctx: &CommandContext) -> CommandResult {
        let srv = match ctx
            .config
            .mcp_servers
            .iter()
            .find(|s| s.name == server_name)
        {
            Some(s) => s,
            None => {
                let configured: Vec<&str> = ctx
                    .config
                    .mcp_servers
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect();
                return CommandResult::Error(format!(
                    "No MCP server named '{}' is configured.\n\
                     Configured servers: {}",
                    server_name,
                    if configured.is_empty() {
                        "(none)".to_string()
                    } else {
                        configured.join(", ")
                    }
                ));
            }
        };

        let is_http = matches!(srv.server_type.as_str(), "http" | "sse");

        if !is_http {
            // stdio — env-var / API-key auth
            let env_keys: Vec<&str> = srv.env.keys().map(|k| k.as_str()).collect();
            let env_note = if env_keys.is_empty() {
                "No environment variables configured.".to_string()
            } else {
                format!("Configured env vars: {}", env_keys.join(", "))
            };
            let token_note = match claurst_mcp::oauth::get_mcp_token(server_name) {
                Some(tok) if !tok.is_expired(60) => " (valid token stored)".to_string(),
                Some(_) => " (stored token is expired)".to_string(),
                None => " (no token stored)".to_string(),
            };
            return CommandResult::Message(format!(
                "MCP Server '{}' (stdio){}\n\
                 {}\n\n\
                 stdio servers authenticate via environment variables (API keys etc.).\n\
                 Add required variables to the 'env' block in ~/.coven-code/settings.json,\n\
                 then restart Coven Code or run /mcp connect {} to reconnect.",
                server_name, token_note, env_note, server_name
            ));
        }

        if let Some(manager) = &ctx.mcp_manager {
            use claurst_mcp::McpServerStatus;
            if matches!(
                manager.server_status(server_name),
                McpServerStatus::Connecting
            ) {
                return CommandResult::Message(format!(
                    "MCP server '{}' is currently connecting — try again shortly.",
                    server_name
                ));
            }

            if let Some(run_auth) = &ctx.mcp_auth_runner {
                // In the interactive CLI/TUI we start browser auth in the background
                // so the event loop stays responsive while waiting for the callback.
                match manager.begin_auth(server_name).await {
                    Ok(session) => {
                        let auth_url = session.auth_url.clone();
                        let redirect_uri = session.redirect_uri.clone();
                        run_auth(session);
                        return CommandResult::McpAuthFlow {
                            server_name: server_name.to_string(),
                            auth_url,
                            redirect_uri,
                        };
                    }
                    Err(e) => {
                        let server_url = srv.url.as_deref().unwrap_or("(URL not configured)");
                        return CommandResult::Message(format!(
                            "MCP OAuth — '{}'\n\
                             Could not initiate OAuth flow: {}\n\n\
                             Manual authentication fallback:\n  Open {} in your browser and complete the OAuth flow.\n\
                             Then run /mcp connect {} to reconnect.",
                            server_name, e, server_url, server_name
                        ));
                    }
                }
            }

            match manager.authenticate(server_name).await {
                Ok(result) => {
                    return CommandResult::Message(format!(
                        "MCP OAuth — '{}'\n\
                         Browser authentication completed; token saved to:\n  {}\n\n\
                         The runtime will attempt to reload the MCP connection; if it still does not reconnect, run /mcp connect {} manually.",
                        server_name,
                        result.token_path.display(),
                        server_name
                    ));
                }
                Err(e) => {
                    let server_url = srv.url.as_deref().unwrap_or("(URL not configured)");
                    return CommandResult::Message(format!(
                        "MCP OAuth — '{}'\n\
                         Could not complete OAuth flow: {}\n\n\
                         Manual authentication fallback:\n  Open {} in your browser and complete the OAuth flow.\n\
                         Then run /mcp connect {} to reconnect.",
                        server_name, e, server_url, server_name
                    ));
                }
            }
        }

        // No live manager — static instructions.
        let server_url = srv.url.as_deref().unwrap_or("(URL not configured)");
        let token_note = match claurst_mcp::oauth::get_mcp_token(server_name) {
            Some(tok) if !tok.is_expired(60) => " (valid token stored)".to_string(),
            Some(_) => " (stored token is expired)".to_string(),
            None => " (no token stored)".to_string(),
        };
        CommandResult::Message(format!(
            "MCP OAuth Authentication — '{}'{}\n\
             Server URL: {}\n\n\
             To authenticate:\n\
             1. Open the server URL in your browser and complete OAuth\n\
             2. The token is saved to ~/.coven-code/mcp-tokens/{}.json\n\
             3. Restart Coven Code — the token will be used automatically\n\n\
             Token storage: ~/.coven-code/mcp-tokens/{}.json",
            server_name, token_note, server_url, server_name, server_name
        ))
    }

    /// Handle `/mcp tools [server]` — list available tools.
    fn handle_tools(server_filter: Option<&str>, ctx: &CommandContext) -> CommandResult {
        let manager = match ctx.mcp_manager.as_ref() {
            Some(m) => m,
            None => {
                return CommandResult::Message(
                    "MCP manager is not active. No tool information available.\n\
                 Restart Coven Code to connect to MCP servers."
                        .to_string(),
                )
            }
        };

        let all_tools = manager.all_tool_definitions();
        let tools: Vec<_> = if let Some(filter) = server_filter {
            all_tools
                .iter()
                .filter(|(srv, _)| srv.as_str() == filter)
                .collect()
        } else {
            all_tools.iter().collect()
        };

        if tools.is_empty() {
            return CommandResult::Message(if let Some(filter) = server_filter {
                format!(
                    "No tools available from server '{}' (not connected or has no tools).",
                    filter
                )
            } else {
                "No tools available from any connected MCP server.".to_string()
            });
        }

        let title = if let Some(filter) = server_filter {
            format!("MCP Tools — '{}' ({})", filter, tools.len())
        } else {
            format!("MCP Tools — all servers ({})", tools.len())
        };
        let mut out = format!("{}\n{}\n", title, "─".repeat(title.len()));
        let mut last_server = "";
        for (server, tool) in &tools {
            if server.as_str() != last_server && server_filter.is_none() {
                out.push_str(&format!("[{}]\n", server));
                last_server = server.as_str();
            }
            // Strip the "servername_" prefix for display
            let bare = tool
                .name
                .strip_prefix(&format!("{}_", server))
                .unwrap_or(&tool.name);
            let preview: String = tool.description.chars().take(80).collect();
            let ellipsis = if tool.description.len() > 80 {
                "…"
            } else {
                ""
            };
            out.push_str(&format!("  {}\n    {}{}\n", bare, preview, ellipsis));
        }
        CommandResult::Message(out)
    }

    /// Handle `/mcp connect <server>` — attempt to reconnect a server.
    async fn handle_connect(server_name: &str, ctx: &CommandContext) -> CommandResult {
        // Validate that the server is configured.
        if !ctx.config.mcp_servers.iter().any(|s| s.name == server_name) {
            let names: Vec<&str> = ctx
                .config
                .mcp_servers
                .iter()
                .map(|s| s.name.as_str())
                .collect();
            return CommandResult::Error(format!(
                "No MCP server named '{}' is configured.\n\
                 Configured servers: {}",
                server_name,
                if names.is_empty() {
                    "(none)".to_string()
                } else {
                    names.join(", ")
                }
            ));
        }

        match &ctx.mcp_manager {
            None => {
                // No live manager — give useful instructions.
                CommandResult::Message(format!(
                    "The MCP manager is not running in this session.\n\
                     To connect '{}', restart Coven Code — servers connect automatically\n\
                     on startup using the configuration in ~/.coven-code/settings.json.\n\
                     \n\
                     If the server requires authentication, run /mcp auth {} first.",
                    server_name, server_name
                ))
            }
            Some(manager) => {
                let current = manager.server_status(server_name);
                use claurst_mcp::McpServerStatus;
                match current {
                    McpServerStatus::Connected { tool_count } => CommandResult::Message(format!(
                        "MCP server '{}' is already connected ({} tool{} available).",
                        server_name,
                        tool_count,
                        if tool_count == 1 { "" } else { "s" }
                    )),
                    McpServerStatus::Connecting => CommandResult::Message(format!(
                        "MCP server '{}' is already in the process of connecting.\n\
                             Check back in a moment.",
                        server_name
                    )),
                    McpServerStatus::Disconnected { .. } | McpServerStatus::Failed { .. } => {
                        // The McpManager doesn't expose a reconnect method — it's built at
                        // startup.  Inform the user and suggest a restart.
                        CommandResult::Message(format!(
                            "MCP server '{}' is currently disconnected.\n\
                             Status: {}\n\
                             \n\
                             The runtime MCP manager reconnects servers automatically.\n\
                             If the server stays disconnected:\n\
                             1. Check authentication: /mcp auth {}\n\
                             2. Verify the command/URL in ~/.coven-code/settings.json\n\
                             3. Restart Coven Code to force a full reconnect",
                            server_name,
                            manager.server_status(server_name).display(),
                            server_name
                        ))
                    }
                }
            }
        }
    }

    /// Handle `/mcp logs <server>` — show recent error/log information.
    fn handle_logs(server_name: &str, ctx: &CommandContext) -> CommandResult {
        // Validate server name.
        if !ctx.config.mcp_servers.iter().any(|s| s.name == server_name) {
            let names: Vec<&str> = ctx
                .config
                .mcp_servers
                .iter()
                .map(|s| s.name.as_str())
                .collect();
            return CommandResult::Error(format!(
                "No MCP server named '{}' is configured.\n\
                 Configured servers: {}",
                server_name,
                if names.is_empty() {
                    "(none)".to_string()
                } else {
                    names.join(", ")
                }
            ));
        }

        let mut lines = vec![format!(
            "MCP Server Logs — '{}'\n──────────────────────",
            server_name
        )];

        if let Some(manager) = &ctx.mcp_manager {
            use claurst_mcp::McpServerStatus;
            let status = manager.server_status(server_name);
            lines.push(format!("Current status:  {}", status.display()));

            match &status {
                McpServerStatus::Disconnected {
                    last_error: Some(e),
                } => {
                    lines.push(format!("\nLast connection error:\n  {}", e));
                    lines.push(String::new());
                    lines.push("Troubleshooting:".to_string());
                    lines.push(format!(
                        "  /mcp auth {}    — check authentication",
                        server_name
                    ));
                    lines.push(format!(
                        "  /mcp connect {} — attempt reconnect",
                        server_name
                    ));
                }
                McpServerStatus::Failed { error, retry_at } => {
                    lines.push(format!("\nConnection failure:\n  {}", error));
                    let retry_secs = retry_at
                        .saturating_duration_since(std::time::Instant::now())
                        .as_secs();
                    if retry_secs > 0 {
                        lines.push(format!("  Automatic retry in {}s", retry_secs));
                    }
                    let _ = retry_at; // used above
                }
                McpServerStatus::Connected { tool_count } => {
                    lines.push(format!(
                        "\nServer is healthy — {} tool{} available.",
                        tool_count,
                        if *tool_count == 1 { "" } else { "s" }
                    ));
                    // Show catalog info if available.
                    if let Some(catalog) = manager.server_catalog(server_name) {
                        if !catalog.resources.is_empty() {
                            lines.push(format!(
                                "Resources ({}): {}",
                                catalog.resource_count,
                                catalog.resources.join(", ")
                            ));
                        }
                        if !catalog.prompts.is_empty() {
                            lines.push(format!(
                                "Prompts ({}): {}",
                                catalog.prompt_count,
                                catalog.prompts.join(", ")
                            ));
                        }
                    }
                }
                McpServerStatus::Disconnected { last_error: None } => {
                    lines.push("\nServer disconnected cleanly (no error recorded).".to_string());
                    lines.push(format!("Run /mcp connect {} to reconnect.", server_name));
                }
                McpServerStatus::Connecting => {
                    lines.push("\nConnection in progress…".to_string());
                }
            }

            // Show failed server errors from the initial connect_all pass.
            for (name, err) in manager.failed_servers() {
                if name == server_name {
                    lines.push(format!("\nStartup connection error:\n  {}", err));
                    break;
                }
            }
        } else {
            lines.push("MCP manager is not active in this session.".to_string());
            lines.push("Restart Coven Code to start the MCP runtime.".to_string());
        }

        // Hint about log files.
        lines.push(String::new());
        lines.push(
            "Note: Detailed stdio output from MCP server processes is not\n\
                    captured by the manager. Run the server command directly in a\n\
                    terminal to see its full output."
                .to_string(),
        );

        CommandResult::Message(lines.join("\n"))
    }
}

// Helper: handle async /mcp resources|prompts|get-prompt subcommands via a separate trait impl.
// These need the mcp_manager from CommandContext.
impl McpCommand {
    async fn handle_live_subcommand(sub: &str, ctx: &CommandContext) -> Option<CommandResult> {
        let manager = ctx.mcp_manager.as_ref()?;
        let parts: Vec<&str> = sub.splitn(4, ' ').collect();
        match parts[0] {
            "resources" => {
                let filter = parts.get(1).copied();
                let resources = manager.list_all_resources(filter).await;
                if resources.is_empty() {
                    return Some(CommandResult::Message(
                        "No resources available (servers may not support resources/list)."
                            .to_string(),
                    ));
                }
                let mut out = format!("MCP Resources ({})\n──────────────────\n", resources.len());
                for r in &resources {
                    let server = r.get("server").and_then(|v| v.as_str()).unwrap_or("?");
                    let uri = r.get("uri").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = r.get("name").and_then(|v| v.as_str()).unwrap_or(uri);
                    let desc = r.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    if desc.is_empty() {
                        out.push_str(&format!("  [{server}] {name}\n    {uri}\n"));
                    } else {
                        out.push_str(&format!("  [{server}] {name} — {desc}\n    {uri}\n"));
                    }
                }
                Some(CommandResult::Message(out))
            }
            "prompts" => {
                let filter = parts.get(1).copied();
                let prompts = manager.list_all_prompts(filter).await;
                if prompts.is_empty() {
                    return Some(CommandResult::Message(
                        "No prompt templates available (servers may not support prompts/list)."
                            .to_string(),
                    ));
                }
                let mut out = format!(
                    "MCP Prompt Templates ({})\n─────────────────────────\n",
                    prompts.len()
                );
                for p in &prompts {
                    let server = p.get("server").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let desc = p.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    let args: Vec<String> = p
                        .get("arguments")
                        .and_then(|a| a.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|a| {
                                    a.get("name")
                                        .and_then(|n| n.as_str())
                                        .map(|s| s.to_string())
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let args_display = if args.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", args.join(", "))
                    };
                    if desc.is_empty() {
                        out.push_str(&format!("  [{server}] {name}{args_display}\n"));
                    } else {
                        out.push_str(&format!("  [{server}] {name}{args_display} — {desc}\n"));
                    }
                }
                out.push_str("\nUse: /mcp get-prompt <server> <prompt> [key=value ...]\n");
                Some(CommandResult::Message(out))
            }
            "get-prompt" => {
                // /mcp get-prompt <server> <prompt-name> [key=val key2=val2 ...]
                let server = match parts.get(1) {
                    Some(s) => *s,
                    None => {
                        return Some(CommandResult::Error(
                            "Usage: /mcp get-prompt <server> <prompt> [key=value ...]".to_string(),
                        ))
                    }
                };
                let prompt_name = match parts.get(2) {
                    Some(p) => *p,
                    None => {
                        return Some(CommandResult::Error(
                            "Usage: /mcp get-prompt <server> <prompt> [key=value ...]".to_string(),
                        ))
                    }
                };
                let mut args: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                if let Some(kv_str) = parts.get(3) {
                    for kv in kv_str.split_whitespace() {
                        if let Some((k, v)) = kv.split_once('=') {
                            args.insert(k.to_string(), v.to_string());
                        }
                    }
                }
                let arguments = if args.is_empty() { None } else { Some(args) };
                match manager.get_prompt(server, prompt_name, arguments).await {
                    Ok(result) => {
                        let mut injected = String::new();
                        for msg in &result.messages {
                            let text = match &msg.content {
                                claurst_mcp::PromptMessageContent::Text { text } => text.clone(),
                                claurst_mcp::PromptMessageContent::Image { .. } => {
                                    "[image]".to_string()
                                }
                                claurst_mcp::PromptMessageContent::Resource { resource } => {
                                    resource.to_string()
                                }
                            };
                            injected.push_str(&format!("[{}]: {}\n", msg.role, text));
                        }
                        Some(CommandResult::UserMessage(injected.trim().to_string()))
                    }
                    Err(e) => Some(CommandResult::Error(format!(
                        "Failed to get prompt '{}' from '{}': {}",
                        prompt_name, server, e
                    ))),
                }
            }
            _ => None,
        }
    }
}

// ---- /permissions --------------------------------------------------------

#[async_trait]
impl SlashCommand for PermissionsCommand {
    fn name(&self) -> &str {
        "permissions"
    }
    fn description(&self) -> &str {
        "View or change tool permission settings"
    }
    fn help(&self) -> &str {
        "Usage: /permissions [set <mode>|allow <tool>|deny <tool>|reset]\n\n\
         Modes: default, accept-edits, bypass-permissions, plan\n\n\
         Examples:\n\
           /permissions                    — show current permissions\n\
           /permissions set accept-edits   — auto-accept file edits\n\
           /permissions allow Bash         — allow a specific tool\n\
           /permissions deny Write         — deny a specific tool\n\
           /permissions reset              — clear overrides"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let args = args.trim();

        if args.is_empty() {
            let allowed_display = if ctx.config.allowed_tools.is_empty() {
                "(all tools allowed)".to_string()
            } else {
                ctx.config.allowed_tools.join(", ")
            };
            let denied_display = if ctx.config.disallowed_tools.is_empty() {
                "(none)".to_string()
            } else {
                ctx.config.disallowed_tools.join(", ")
            };
            return CommandResult::Message(format!(
                "Permission Settings\n\
                 ───────────────────\n\
                 Mode:          {:?}\n\
                 Allowed tools: {}\n\
                 Denied tools:  {}\n\n\
                 Use /permissions set <mode> to change the permission mode.\n\
                 Use /permissions allow|deny <tool> to override individual tools.\n\
                 Use /permissions reset to clear all overrides.",
                ctx.config.permission_mode, allowed_display, denied_display,
            ));
        }

        let mut parts = args.splitn(2, ' ');
        let sub = parts.next().unwrap_or("").trim();
        let arg = parts.next().unwrap_or("").trim();

        match sub {
            "set" => {
                let mode = match arg.to_lowercase().as_str() {
                    "default" => claurst_core::config::PermissionMode::Default,
                    "accept-edits" | "accept_edits" => {
                        claurst_core::config::PermissionMode::AcceptEdits
                    }
                    "bypass-permissions" | "bypass_permissions" => {
                        claurst_core::config::PermissionMode::BypassPermissions
                    }
                    "plan" => claurst_core::config::PermissionMode::Plan,
                    _ => {
                        return CommandResult::Error(
                            "Mode must be: default, accept-edits, bypass-permissions, or plan"
                                .to_string(),
                        )
                    }
                };
                let mut new_config = ctx.config.clone();
                new_config.permission_mode = mode.clone();
                if let Err(e) = save_settings_mutation(|s| s.config.permission_mode = mode.clone())
                {
                    return CommandResult::Error(format!("Failed to save: {}", e));
                }
                CommandResult::ConfigChangeMessage(
                    new_config,
                    format!("Permission mode set to {:?}.", mode),
                )
            }
            "allow" => {
                if arg.is_empty() {
                    return CommandResult::Error("Usage: /permissions allow <tool>".to_string());
                }
                let tool = arg.to_string();
                let mut new_config = ctx.config.clone();
                if !new_config.allowed_tools.contains(&tool) {
                    new_config.allowed_tools.push(tool.clone());
                }
                new_config.disallowed_tools.retain(|t| t != &tool);
                if let Err(e) = save_settings_mutation(|s| {
                    if !s.config.allowed_tools.contains(&tool) {
                        s.config.allowed_tools.push(tool.clone());
                    }
                    s.config.disallowed_tools.retain(|t| t != &tool);
                }) {
                    return CommandResult::Error(format!("Failed to save: {}", e));
                }
                CommandResult::ConfigChangeMessage(new_config, format!("Allowed tool: {}", tool))
            }
            "deny" => {
                if arg.is_empty() {
                    return CommandResult::Error("Usage: /permissions deny <tool>".to_string());
                }
                let tool = arg.to_string();
                let mut new_config = ctx.config.clone();
                if !new_config.disallowed_tools.contains(&tool) {
                    new_config.disallowed_tools.push(tool.clone());
                }
                new_config.allowed_tools.retain(|t| t != &tool);
                if let Err(e) = save_settings_mutation(|s| {
                    if !s.config.disallowed_tools.contains(&tool) {
                        s.config.disallowed_tools.push(tool.clone());
                    }
                    s.config.allowed_tools.retain(|t| t != &tool);
                }) {
                    return CommandResult::Error(format!("Failed to save: {}", e));
                }
                CommandResult::ConfigChangeMessage(new_config, format!("Denied tool: {}", tool))
            }
            "reset" => {
                let mut new_config = ctx.config.clone();
                new_config.allowed_tools.clear();
                new_config.disallowed_tools.clear();
                new_config.permission_mode = claurst_core::config::PermissionMode::Default;
                if let Err(e) = save_settings_mutation(|s| {
                    s.config.allowed_tools.clear();
                    s.config.disallowed_tools.clear();
                    s.config.permission_mode = claurst_core::config::PermissionMode::Default;
                }) {
                    return CommandResult::Error(format!("Failed to save: {}", e));
                }
                CommandResult::ConfigChangeMessage(
                    new_config,
                    "Permissions reset to defaults.".to_string(),
                )
            }
            other => CommandResult::Error(format!(
                "Unknown subcommand '{}'. Use: /permissions [set|allow|deny|reset]",
                other
            )),
        }
    }
}

// ---- /plan ---------------------------------------------------------------

#[async_trait]
impl SlashCommand for PlanCommand {
    fn name(&self) -> &str {
        "plan"
    }
    fn description(&self) -> &str {
        "Enter plan mode – model outputs a plan for approval before acting"
    }
    fn help(&self) -> &str {
        "Usage: /plan [description]\n\n\
         Switches to plan mode where the model will create a detailed plan before executing.\n\
         The plan must be approved before any file writes or command executions are performed.\n\
         Use /plan exit to leave plan mode."
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        if args.trim() == "exit" {
            return CommandResult::UserMessage(
                "[Exiting plan mode. Resuming normal execution.]".to_string(),
            );
        }
        let task_desc = if args.is_empty() {
            "the current task".to_string()
        } else {
            args.to_string()
        };
        CommandResult::UserMessage(format!(
            "[Entering plan mode for: {}]\n\
             Please create a detailed step-by-step plan. Do not execute any commands or \
             write any files until the plan has been reviewed and approved.",
            task_desc
        ))
    }
}

// ---- /tasks --------------------------------------------------------------

#[async_trait]
impl SlashCommand for TasksCommand {
    fn name(&self) -> &str {
        "tasks"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["bashes"]
    }
    fn description(&self) -> &str {
        "List and manage background tasks"
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        CommandResult::UserMessage(
            "Please list all current tasks using the TaskList tool and show their status."
                .to_string(),
        )
    }
}

// ---- /session ------------------------------------------------------------

#[async_trait]
impl SlashCommand for SessionCommand {
    fn name(&self) -> &str {
        "session"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["remote"]
    }
    fn description(&self) -> &str {
        "Browse and manage sessions (rename, fork, branch, tag, add-dir)"
    }
    fn help(&self) -> &str {
        "Usage: /session [list|rename|fork|branch|tag|add-dir] [...]\n\n\
         Without arguments, shows the current session and recent sessions.\n\n\
         Subcommands (absorbing the former standalone commands):\n\
           /session list                       — list saved sessions\n\
           /session rename <title>             — rename the current session\n\
           /session fork [message_index]       — fork into a new session\n\
           /session branch [create|switch|list] [name]\n\
           /session tag [list|add|remove] [tag]\n\
           /session add-dir <path>             — add a workspace root"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let trimmed = args.trim();
        let (head, rest) = match trimmed.split_once(char::is_whitespace) {
            Some((h, r)) => (h, r.trim()),
            None => (trimmed, ""),
        };
        match head {
            // Each arm absorbs a former standalone command.
            "rename" => return RenameCommand.execute(rest, ctx).await,
            "fork" => return ForkCommand.execute(rest, ctx).await,
            "branch" => return execute_named_command_from_slash("branch", rest, ctx),
            "tag" => return execute_named_command_from_slash("tag", rest, ctx),
            "add-dir" => return execute_named_command_from_slash("add-dir", rest, ctx),
            _ => {}
        }
        match trimmed {
            "list" => {
                let sessions = claurst_core::history::list_sessions().await;
                if sessions.is_empty() {
                    CommandResult::Message("No saved sessions found.".to_string())
                } else {
                    let mut output = String::from("Recent sessions:\n\n");
                    for sess in sessions.iter().take(10) {
                        let updated = sess.updated_at.format("%Y-%m-%d %H:%M").to_string();
                        let id_short = &sess.id[..sess.id.len().min(8)];
                        output.push_str(&format!(
                            "  {} | {} | {} messages | {}\n",
                            id_short,
                            updated,
                            sess.messages.len(),
                            sess.title.as_deref().unwrap_or("(untitled)")
                        ));
                    }
                    output.push_str("\nUse /resume <id> to resume a session.");
                    CommandResult::Message(output)
                }
            }
            "" => {
                // If a bridge remote URL is active, show it prominently.
                if let Some(ref url) = ctx.remote_session_url {
                    let border = "─".repeat(url.len().min(60) + 4);
                    let display_url = if url.len() > 60 {
                        format!("{}…", &url[..60])
                    } else {
                        url.clone()
                    };
                    CommandResult::Message(format!(
                        "Remote session active\n\
                         ┌{border}┐\n\
                         │  {display_url}  │\n\
                         └{border}┘\n\n\
                         Open the URL above on any device to connect remotely.\n\
                         Session ID: {}",
                        ctx.session_id,
                    ))
                } else {
                    // Show current session info + recent sessions list.
                    let sessions = claurst_core::history::list_sessions().await;
                    let mut output = format!(
                        "Current session\n\
                         ───────────────\n\
                         ID:       {}\n\
                         Title:    {}\n\
                         Messages: {}\n\
                         Model:    {}\n",
                        ctx.session_id,
                        ctx.session_title.as_deref().unwrap_or("(untitled)"),
                        ctx.messages.len(),
                        ctx.config.effective_model()
                    );

                    if !sessions.is_empty() {
                        output.push_str("\nRecent sessions:\n\n");
                        for sess in sessions.iter().take(5) {
                            let updated = sess.updated_at.format("%Y-%m-%d %H:%M").to_string();
                            let id_short = &sess.id[..sess.id.len().min(8)];
                            let marker = if sess.id == ctx.session_id {
                                " ◀ current"
                            } else {
                                ""
                            };
                            output.push_str(&format!(
                                "  {} | {} | {} messages | {}{}\n",
                                id_short,
                                updated,
                                sess.messages.len(),
                                sess.title.as_deref().unwrap_or("(untitled)"),
                                marker,
                            ));
                        }
                        output.push_str(
                            "\nUse /session list for all sessions, /resume <id> to switch.",
                        );
                    }

                    CommandResult::Message(output)
                }
            }
            _ => CommandResult::Error(format!(
                "Unknown subcommand: {}\n\nUsage: /session [list|rename|fork|branch|tag|add-dir]",
                args
            )),
        }
    }
}

// ---- /fork ---------------------------------------------------------------

#[async_trait]
impl SlashCommand for ForkCommand {
    fn name(&self) -> &str {
        "fork"
    }
    fn hidden(&self) -> bool {
        true // folded into /session fork; one-release compatibility alias
    }
    fn description(&self) -> &str {
        "Fork the current session into a new branch"
    }
    fn help(&self) -> &str {
        "Usage: /session fork [message_index]\n\n\
         Fork the current session at the specified message index (or at the\n\
         current point if no index is given).  Creates a new session containing\n\
         messages up to the fork point.\n\n\
         Examples:\n\
           /fork        \u{2014} fork at the current end of the conversation\n\
           /fork 5      \u{2014} fork after message 5"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let fork_index: Option<usize> = args.trim().parse().ok();
        let messages = &ctx.messages;
        let fork_at = fork_index.unwrap_or(messages.len()).min(messages.len());
        let forked_messages: Vec<_> = messages[..fork_at].to_vec();

        let mut new_session = claurst_core::history::ConversationSession::new(
            ctx.config.effective_model().to_string(),
        );
        new_session.messages = forked_messages;
        new_session.parent_session_id = Some(ctx.session_id.clone());
        new_session.fork_point_message_index = Some(fork_at);
        new_session.title = Some(format!(
            "Fork of {}",
            ctx.session_title.as_deref().unwrap_or("session")
        ));
        new_session.working_dir = Some(ctx.working_dir.to_string_lossy().to_string());

        let new_id = new_session.id.clone();
        match claurst_core::history::save_session(&new_session).await {
            Ok(()) => CommandResult::Message(format!(
                "Session forked at message {}. New session: {}\nUse /resume {} to switch to it.",
                fork_at, new_id, new_id
            )),
            Err(e) => CommandResult::Error(format!("Failed to save forked session: {}", e)),
        }
    }
}

// ---- /thinking -----------------------------------------------------------

#[async_trait]
impl SlashCommand for ThinkingCommand {
    fn name(&self) -> &str {
        "thinking"
    }
    fn description(&self) -> &str {
        "Toggle extended thinking mode"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["think"]
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        // Extended thinking is configured through the model; just inform the user
        let model = ctx.config.effective_model();
        if model.contains("claude-3-5") || model.contains("claude-3.5") {
            CommandResult::Message(
                "Extended thinking is not available for Claude 3.5 models.\n\
                 Use claude-opus-4-6 or claude-sonnet-4-6 for extended thinking."
                    .to_string(),
            )
        } else {
            CommandResult::Message(format!(
                "Extended thinking is available with {}.\n\
                 You can request thinking by asking Coven Code to 'think step by step' or \
                 'think carefully before answering'.",
                model
            ))
        }
    }
}

// ---- /export -------------------------------------------------------------

/// Format a single `Message` as a Markdown section.
///
/// User messages render as `## User\n<text>`.
/// Assistant messages render as `## Assistant\n<text>` followed by
/// `### Tool: <name>\n**Input:** …\n**Output:** …` for each tool call pair.
fn export_message_to_markdown(
    msg: &claurst_core::types::Message,
    all_messages: &[claurst_core::types::Message],
    msg_idx: usize,
) -> String {
    use claurst_core::types::{ContentBlock, MessageContent, Role, ToolResultContent};

    let role_label = match msg.role {
        Role::User => "User",
        Role::Assistant => "Assistant",
    };

    let mut out = format!("## {}\n", role_label);

    match &msg.content {
        MessageContent::Text(t) => {
            out.push_str(t);
            out.push('\n');
        }
        MessageContent::Blocks(blocks) => {
            // Collect text first
            let mut text_parts: Vec<&str> = Vec::new();
            let mut tool_uses: Vec<(&str, &str, &serde_json::Value)> = Vec::new(); // (id, name, input)

            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        text_parts.push(text.as_str());
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_uses.push((id.as_str(), name.as_str(), input));
                    }
                    ContentBlock::Thinking { thinking, .. } => {
                        // Include thinking blocks as a collapsible hint
                        out.push_str("\n<details><summary>Thinking</summary>\n\n");
                        out.push_str(thinking);
                        out.push_str("\n</details>\n\n");
                    }
                    _ => {}
                }
            }

            if !text_parts.is_empty() {
                out.push_str(&text_parts.join(""));
                out.push('\n');
            }

            // For each tool use, look for the matching ToolResult in the NEXT user message
            for (tool_id, tool_name, tool_input) in &tool_uses {
                out.push_str(&format!("\n### Tool: {}\n", tool_name));
                let input_str = serde_json::to_string_pretty(tool_input)
                    .unwrap_or_else(|_| tool_input.to_string());
                out.push_str(&format!("**Input:** `{}`\n", input_str.replace('\n', " ")));

                // Search the next user message for a matching ToolResult
                let mut found_output: Option<String> = None;
                'search: for next_msg in all_messages.iter().skip(msg_idx + 1) {
                    if let MessageContent::Blocks(next_blocks) = &next_msg.content {
                        for nb in next_blocks {
                            if let ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                            } = nb
                            {
                                if tool_use_id.as_str() == *tool_id {
                                    let text = match content {
                                        ToolResultContent::Text(t) => t.clone(),
                                        ToolResultContent::Blocks(bs) => bs
                                            .iter()
                                            .filter_map(|b| {
                                                if let ContentBlock::Text { text } = b {
                                                    Some(text.as_str())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect::<Vec<_>>()
                                            .join(""),
                                    };
                                    let label = if is_error.unwrap_or(false) {
                                        "Error"
                                    } else {
                                        "Output"
                                    };
                                    found_output = Some(format!(
                                        "**{}:** `{}`\n",
                                        label,
                                        text.lines().next().unwrap_or(&text).trim()
                                    ));
                                    break 'search;
                                }
                            }
                        }
                    }
                }
                out.push_str(
                    found_output
                        .as_deref()
                        .unwrap_or("**Output:** *(pending)*\n"),
                );
            }
        }
    }

    out
}

/// Build the full markdown export string.
fn build_markdown_export(ctx: &CommandContext) -> String {
    let mut out = String::new();
    out.push_str("# Conversation Export\n\n");
    out.push_str(&format!("- **Session ID:** {}\n", ctx.session_id));
    out.push_str(&format!("- **Model:** {}\n", ctx.config.effective_model()));
    out.push_str(&format!(
        "- **Exported:** {}\n",
        chrono::Utc::now().to_rfc3339()
    ));
    if let Some(ref title) = ctx.session_title {
        out.push_str(&format!("- **Title:** {}\n", title));
    }
    out.push_str(&format!("- **Messages:** {}\n", ctx.messages.len()));
    out.push_str("\n---\n\n");

    let messages = ctx.messages.clone();
    for (i, msg) in messages.iter().enumerate() {
        out.push_str(&export_message_to_markdown(msg, &messages, i));
        out.push_str("\n---\n\n");
    }
    out
}

/// Build the full JSON export value.
fn build_json_export(ctx: &CommandContext) -> serde_json::Value {
    serde_json::json!({
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "session_id": ctx.session_id,
        "session_title": ctx.session_title,
        "model": ctx.config.effective_model(),
        "message_count": ctx.messages.len(),
        "messages": ctx.messages.iter().map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content,
                "uuid": m.uuid,
            })
        }).collect::<Vec<_>>(),
    })
}

#[async_trait]
impl SlashCommand for ExportCommand {
    fn name(&self) -> &str {
        "export"
    }
    fn description(&self) -> &str {
        "Export conversation to markdown or JSON"
    }
    fn help(&self) -> &str {
        "Usage: /export [--format markdown|json] [--output <file>]\n\n\
         Export the current conversation.\n\n\
         Flags:\n\
           --format markdown   Render as readable Markdown (default for .md files)\n\
           --format json       Full structured JSON export (default)\n\
           --output <path>     Write to file; if omitted, prints to the terminal\n\n\
         Examples:\n\
           /export\n\
           /export --format markdown\n\
           /export --format json --output chat.json\n\
           /export --output conversation.md"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        // ── Parse flags ────────────────────────────────────────────────────
        let args = args.trim();
        let mut format: Option<&str> = None; // "markdown" | "json"
        let mut output_path: Option<String> = None;

        // Simple hand-rolled flag parser (no clap dep in commands crate)
        let tokens: Vec<&str> = args.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            match tokens[i] {
                "--format" | "-f" => {
                    if i + 1 < tokens.len() {
                        format = Some(tokens[i + 1]);
                        i += 2;
                    } else {
                        return CommandResult::Error(
                            "--format requires a value: markdown or json".to_string(),
                        );
                    }
                }
                "--output" | "-o" => {
                    if i + 1 < tokens.len() {
                        output_path = Some(tokens[i + 1].to_string());
                        i += 2;
                    } else {
                        return CommandResult::Error("--output requires a file path".to_string());
                    }
                }
                other if !other.starts_with('-') => {
                    // Bare filename as positional arg (legacy compat)
                    if output_path.is_none() {
                        output_path = Some(other.to_string());
                    }
                    i += 1;
                }
                other => {
                    return CommandResult::Error(format!("Unknown flag: {}", other));
                }
            }
        }

        // ── Determine format from output path extension if not explicit ─────
        let resolved_format = match format {
            Some("markdown") | Some("md") => "markdown",
            Some("json") => "json",
            Some(other) => {
                return CommandResult::Error(format!(
                    "Unknown format '{}'. Use 'markdown' or 'json'.",
                    other
                ));
            }
            None => {
                // Infer from output file extension
                if let Some(ref path) = output_path {
                    if path.ends_with(".md") || path.ends_with(".markdown") {
                        "markdown"
                    } else {
                        "json"
                    }
                } else {
                    "json"
                }
            }
        };

        // ── Build content ───────────────────────────────────────────────────
        let content: String = match resolved_format {
            "markdown" => build_markdown_export(ctx),
            _ => {
                let val = build_json_export(ctx);
                match serde_json::to_string_pretty(&val) {
                    Ok(j) => j,
                    Err(e) => return CommandResult::Error(format!("Serialization error: {}", e)),
                }
            }
        };

        // ── Write or return ─────────────────────────────────────────────────
        match output_path {
            Some(ref filename) => {
                // Default extension if the user didn't provide one
                let filename = if !filename.contains('.') {
                    format!(
                        "{}.{}",
                        filename,
                        if resolved_format == "markdown" {
                            "md"
                        } else {
                            "json"
                        }
                    )
                } else {
                    filename.to_string()
                };

                let path = if std::path::Path::new(&filename).is_absolute() {
                    std::path::PathBuf::from(&filename)
                } else {
                    ctx.working_dir.join(&filename)
                };

                match tokio::fs::write(&path, &content).await {
                    Ok(()) => CommandResult::Message(format!(
                        "Conversation exported to {} ({} messages, {} format)",
                        path.display(),
                        ctx.messages.len(),
                        resolved_format,
                    )),
                    Err(e) => {
                        CommandResult::Error(format!("Failed to write {}: {}", path.display(), e))
                    }
                }
            }
            None => {
                // Print to terminal
                CommandResult::Message(content)
            }
        }
    }
}

// ---- /share --------------------------------------------------------------

#[async_trait]
impl SlashCommand for ShareCommand {
    fn name(&self) -> &str {
        "share"
    }
    fn description(&self) -> &str {
        "Upload the current session as a secret GitHub gist and return a shareable URL"
    }
    fn help(&self) -> &str {
        "Usage: /share\n\n\
         Renders the current session as a single self-contained HTML file,\n\
         uploads it as a secret GitHub gist via the `gh` CLI, and prints a\n\
         viewer URL of the form https://opencoven.github.io/coven-code/session/#<gist-id>.\n\n\
         Requirements:\n  \
           - GitHub CLI (gh) installed and logged in (`gh auth login`).\n\n\
         The viewer base URL can be overridden with COVEN_CODE_SHARE_VIEWER_URL.\n\
         Secret gists are unlisted but readable by anyone who has the link."
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        use claurst_core::share_export::{share_viewer_url, write_session_html, SessionExportMeta};

        // 1. Check that `gh` is installed and authenticated. Uses tokio::process
        //    so the TUI event loop keeps animating during the (occasionally
        //    slow) network round-trip.
        match tokio::process::Command::new("gh")
            .args(["auth", "status"])
            .output()
            .await
        {
            Err(_) => {
                return CommandResult::Error(
                    "GitHub CLI (gh) is not installed. Install it from https://cli.github.com/"
                        .to_string(),
                );
            }
            Ok(out) if !out.status.success() => {
                return CommandResult::Error(
                    "GitHub CLI is not logged in. Run `gh auth login` first.".to_string(),
                );
            }
            Ok(_) => {}
        }

        // 2. Build metadata + render HTML to a temp file.
        let meta = SessionExportMeta {
            session_id: ctx.session_id.clone(),
            title: ctx.session_title.clone(),
            model: ctx.config.effective_model().to_string(),
            working_dir: ctx.working_dir.display().to_string(),
            exported_at: chrono::Utc::now().to_rfc3339(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let safe_id: String = ctx
            .session_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let stem = if safe_id.is_empty() {
            "session".to_string()
        } else {
            safe_id
        };
        let tmp = std::env::temp_dir().join(format!("claurst-session-{stem}.html"));

        if let Err(e) = write_session_html(&tmp, &ctx.messages, &meta) {
            return CommandResult::Error(format!("Failed to render session HTML: {e}"));
        }

        tracing::info!(target: "share", path = %tmp.display(), "Uploading session HTML as secret gist");

        // 3. Upload as a secret gist (async, so the TUI stays responsive).
        let result = tokio::process::Command::new("gh")
            .args(["gist", "create", "--public=false"])
            .arg(&tmp)
            .output()
            .await;

        // Best-effort tmp cleanup.
        let _ = std::fs::remove_file(&tmp);

        let output = match result {
            Ok(o) => o,
            Err(e) => return CommandResult::Error(format!("Failed to spawn gh: {e}")),
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = stderr.trim();
            return CommandResult::Error(format!(
                "gh gist create failed: {}",
                if msg.is_empty() { "unknown error" } else { msg }
            ));
        }

        // 4. Parse gist URL and derive the viewer URL.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let gist_url = stdout.trim();
        let gist_id = gist_url.rsplit('/').next().unwrap_or("").trim();
        if gist_id.is_empty() {
            return CommandResult::Error(format!(
                "Could not parse gist id from gh output: {gist_url:?}"
            ));
        }
        let viewer = share_viewer_url(gist_id);

        // Auto-open in the system browser unless the user opted out — saves the
        // copy/paste dance after a /share. Skipped when `COVEN_CODE_SHARE_NO_OPEN`
        // is set (e.g. on a headless box) or when `open` can't find a handler.
        let opted_out = std::env::var_os("COVEN_CODE_SHARE_NO_OPEN")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false);
        let opened = if opted_out {
            false
        } else {
            open::that(&viewer).is_ok()
        };

        let footer = if opened {
            "Opened in your browser. The gist is secret (unlisted); delete it to revoke access."
        } else if opted_out {
            "The gist is secret (unlisted). Anyone with the link can view it; delete the gist to revoke access."
        } else {
            "Could not auto-open the link. Copy the URL above. The gist is secret (unlisted); delete the gist to revoke access."
        };

        CommandResult::Message(format!("Share URL: {viewer}\nGist: {gist_url}\n\n{footer}"))
    }
}

// ---- /skills -------------------------------------------------------------

#[async_trait]
impl SlashCommand for SkillsCommand {
    fn name(&self) -> &str {
        "skills"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["skill"]
    }
    fn description(&self) -> &str {
        "List available skills in .coven-code/commands/"
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let mut found: Vec<String> = Vec::new();
        let dirs = [
            ctx.working_dir.join(".coven-code").join("commands"),
            dirs::home_dir()
                .unwrap_or_default()
                .join(".coven-code")
                .join("commands"),
        ];

        for dir in &dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().is_some_and(|e| e == "md") {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            let name = stem.to_string();
                            if !found.contains(&name) {
                                found.push(name);
                            }
                        }
                    }
                }
            }
        }

        // Include skills contributed by installed plugins.
        if let Some(registry) = claurst_plugins::global_plugin_registry() {
            for skill_dir in registry.all_skill_paths() {
                if let Ok(entries) = std::fs::read_dir(&skill_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        // Skills can be individual .md files or subdirs with SKILL.md.
                        if p.is_dir() {
                            if p.join("SKILL.md").exists() || p.join("skill.md").exists() {
                                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                                    let skill_name = name.to_string();
                                    if !found.contains(&skill_name) {
                                        found.push(skill_name);
                                    }
                                }
                            }
                        } else if p.extension().is_some_and(|e| e == "md") {
                            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                                let name = stem.to_string();
                                if !found.contains(&name) {
                                    found.push(name);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Include discovered skills from .coven-code/skills/ and configured paths/URLs.
        let discovered = claurst_core::discover_skills(&ctx.working_dir, &ctx.config.skills);

        let mut output = if found.is_empty() && discovered.is_empty() {
            return CommandResult::Message(
                "No skills found.\nCreate .md files in .coven-code/commands/ to define skills.\n\
                 Example: .coven-code/commands/review.md"
                    .to_string(),
            );
        } else if found.is_empty() {
            String::new()
        } else {
            found.sort();
            format!(
                "Available skills ({}):\n{}",
                found.len(),
                found
                    .iter()
                    .map(|s| format!("  /{}", s))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        if !discovered.is_empty() {
            let mut disc_list: Vec<(&String, &claurst_core::DiscoveredSkill)> =
                discovered.iter().collect();
            disc_list.sort_by_key(|(name, _)| name.as_str());

            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("\nDiscovered skills ({}):\n", disc_list.len()));
            for (name, skill) in disc_list {
                output.push_str(&format!(
                    "  /{} — {} ({})\n",
                    name,
                    skill.description,
                    skill.source_path.display()
                ));
            }
        }

        CommandResult::Message(output.trim_end().to_string())
    }
}

// ---- /rewind -------------------------------------------------------------

#[async_trait]
impl SlashCommand for RewindCommand {
    fn name(&self) -> &str {
        "rewind"
    }
    fn description(&self) -> &str {
        "Rewind the conversation or roll back a turn's file changes"
    }
    fn help(&self) -> &str {
        "Usage: /rewind [list|diff [n]|last|<n>|<uuid>]\n\n\
         Without arguments, opens an interactive overlay to pick the message to\n\
         rewind the conversation to (↑↓ to navigate, Enter to select, y/n to confirm).\n\n\
         With arguments, rolls back file changes recorded by the shadow-git\n\
         snapshot system (absorbing the former /undo and /revert):\n\
           /rewind list     — list assistant turns with recorded file changes\n\
           /rewind diff [n] — preview a turn's diff without reverting\n\
           /rewind last     — revert the most recent assistant turn\n\
           /rewind <n>      — revert the n-th most recent assistant turn\n\
           /rewind <uuid>   — revert the turn whose message id starts with <uuid>\n\n\
         The legacy /undo and /revert commands remain hidden one-release\n\
         compatibility aliases for the argument forms."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            if ctx.messages.is_empty() {
                return CommandResult::Message(
                    "Nothing to rewind — conversation is empty.".to_string(),
                );
            }
            return CommandResult::OpenRewindOverlay;
        }
        // File-rollback forms absorbed from /undo and /revert. RevertCommand
        // already handles list / diff / <n> / <uuid>.
        match trimmed {
            "last" | "undo" => RevertCommand.execute("", ctx).await,
            _ => RevertCommand.execute(trimmed, ctx).await,
        }
    }
}

// ---- /stats --------------------------------------------------------------

#[async_trait]
impl SlashCommand for StatsCommand {
    fn name(&self) -> &str {
        "stats"
    }
    fn hidden(&self) -> bool {
        true // folded into /usage stats; one-release compatibility alias
    }
    fn description(&self) -> &str {
        "Show token usage and cost statistics"
    }
    fn help(&self) -> &str {
        "Usage: /usage stats\n\n\
         Shows detailed token usage and cost breakdown for the current session,\n\
         including cache creation/read token counts, turn counts, and session duration.\n\
         Use /usage for quota and account info, /usage cost for a quick cost summary."
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let input = ctx.cost_tracker.input_tokens();
        let output = ctx.cost_tracker.output_tokens();
        let cache_creation = ctx.cost_tracker.cache_creation_tokens();
        let cache_read = ctx.cost_tracker.cache_read_tokens();
        let total = ctx.cost_tracker.total_tokens();
        let cost = ctx.cost_tracker.total_cost_usd();
        let model = ctx.config.effective_model();

        // Count user/assistant turns separately.
        let user_turns = ctx
            .messages
            .iter()
            .filter(|m| m.role == claurst_core::types::Role::User)
            .count();
        let assistant_turns = ctx
            .messages
            .iter()
            .filter(|m| m.role == claurst_core::types::Role::Assistant)
            .count();

        // Count tool-use invocations.
        let tool_calls: usize = ctx
            .messages
            .iter()
            .map(|m| m.get_tool_use_blocks().len())
            .sum();

        // Cost breakdown note: cache-read tokens are cheaper than input, and
        // cache-creation tokens are slightly more expensive. Provide a note if
        // caching is active.
        let cache_note = if cache_creation > 0 || cache_read > 0 {
            format!(
                "\n  (Cache write: {:>10}    Cache read: {:>10})",
                cache_creation, cache_read
            )
        } else {
            String::new()
        };

        CommandResult::Message(format!(
            "Session Statistics\n\
             ══════════════════\n\
             Model:          {model}\n\
             \n\
             Conversation:\n\
               User turns:     {user_turns:>10}\n\
               Assistant turns:{assistant_turns:>10}\n\
               Tool calls:     {tool_calls:>10}\n\
             \n\
             Token usage:\n\
               Input:          {input:>10}\n\
               Output:         {output:>10}\n\
               Total:          {total:>10}{cache_note}\n\
             \n\
             Estimated cost:   ${cost:.4}\n\
             \n\
             Use /usage for quota info · /cost for quick cost",
            model = model,
            user_turns = user_turns,
            assistant_turns = assistant_turns,
            tool_calls = tool_calls,
            input = input,
            output = output,
            total = total,
            cache_note = cache_note,
            cost = cost,
        ))
    }
}

// ---- /rename -------------------------------------------------------------

#[async_trait]
impl SlashCommand for RenameCommand {
    fn name(&self) -> &str {
        "rename"
    }
    fn description(&self) -> &str {
        "Rename the current session"
    }
    fn help(&self) -> &str {
        "Usage: /rename [new name]\n\n\
         With a name: sets the session title immediately.\n\
         With no argument: auto-generates a kebab-case name from the conversation.\n\n\
         Examples:\n\
           /rename fix-login-bug\n\
           /rename              — auto-generate from conversation history"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let name = args.trim();

        if !name.is_empty() {
            // Explicit name provided: rename immediately.
            return CommandResult::RenameSession(name.to_string());
        }

        // No name given — auto-generate from conversation context.
        if ctx.messages.is_empty() {
            return CommandResult::Error(
                "No conversation context yet. Usage: /rename <name>".to_string(),
            );
        }

        // Build a short conversation excerpt (up to ~2000 chars) for the model.
        let excerpt: String = ctx
            .messages
            .iter()
            .take(20)
            .filter_map(|m| {
                let text = m.get_all_text();
                if text.is_empty() {
                    return None;
                }
                let role = match m.role {
                    claurst_core::types::Role::User => "User",
                    claurst_core::types::Role::Assistant => "Assistant",
                };
                Some(format!(
                    "{}: {}",
                    role,
                    text.chars().take(300).collect::<String>()
                ))
            })
            .collect::<Vec<_>>()
            .join("\n");

        if excerpt.is_empty() {
            return CommandResult::Error(
                "No text content in conversation. Usage: /rename <name>".to_string(),
            );
        }

        let provider = match provider_for_config(&ctx.config).await {
            Some(provider) => provider,
            None => {
                return CommandResult::Error(
                    "Could not create a provider client for auto-naming.\n\
                     Use /rename <name> to set the name manually."
                        .to_string(),
                );
            }
        };
        let rename_model = resolve_fast_model_id(&ctx.config);

        let system_prompt = "Generate a short kebab-case name (2-4 words) that captures the \
            main topic of this conversation. Use lowercase words separated by hyphens. \
            Examples: fix-login-bug, add-auth-feature, refactor-api-client. \
            Respond with ONLY the name, nothing else.";

        let request = claurst_api::ProviderRequest {
            model: rename_model,
            messages: vec![Message::user(format!(
                "Conversation to name:\n\n{}",
                &excerpt[..excerpt.len().min(2000)]
            ))],
            system_prompt: Some(claurst_api::SystemPrompt::Text(system_prompt.to_string())),
            tools: vec![],
            max_tokens: 64,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: vec![],
            thinking: None,
            provider_options: serde_json::Value::Object(Default::default()),
        };

        match provider.create_message(request).await {
            Ok(response) => {
                let raw_text = text_from_content_blocks(&response.content)
                    .trim()
                    .to_string();

                let generated = raw_text
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '-')
                    .collect::<String>();

                // Trim leading/trailing hyphens and ensure non-empty.
                let cleaned = generated.trim_matches('-').to_string();
                if cleaned.is_empty() {
                    return CommandResult::Error(
                        "Could not generate a valid name from conversation. \
                         Use /rename <name> to set manually."
                            .to_string(),
                    );
                }

                CommandResult::RenameSession(cleaned)
            }
            Err(e) => CommandResult::Error(format!(
                "Auto-name generation failed: {e}\n\
                 Use /rename <name> to set the name manually."
            )),
        }
    }
}

// ---- /effort -------------------------------------------------------------

#[async_trait]
impl SlashCommand for EffortCommand {
    fn name(&self) -> &str {
        "effort"
    }
    fn description(&self) -> &str {
        "Set the model's thinking effort (low | normal | high)"
    }
    fn help(&self) -> &str {
        "Usage: /effort [low|normal|high]\n\
         Sets how much computation the model uses for reasoning.\n\
         'high' enables extended thinking with a larger budget."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        match args.trim() {
            "" => CommandResult::Message(
                "Current effort: normal\nUse /effort [low|normal|high] to change.".to_string(),
            ),
            "low" => {
                // Low effort: smaller max_tokens
                ctx.config.max_tokens = Some(4096);
                CommandResult::ConfigChange(ctx.config.clone())
            }
            "normal" => {
                ctx.config.max_tokens = None; // use default
                CommandResult::ConfigChange(ctx.config.clone())
            }
            "high" => {
                ctx.config.max_tokens = Some(32768);
                CommandResult::ConfigChange(ctx.config.clone())
            }
            other => CommandResult::Error(format!(
                "Unknown effort level '{}'. Use: low | normal | high",
                other
            )),
        }
    }
}

// ---- /commit -------------------------------------------------------------

#[async_trait]
impl SlashCommand for CommitCommand {
    fn name(&self) -> &str {
        "commit"
    }
    fn description(&self) -> &str {
        "Ask Coven Code to commit staged changes"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let extra = if args.trim().is_empty() {
            String::new()
        } else {
            format!(" with message: {}", args.trim())
        };

        CommandResult::UserMessage(format!(
            "Please commit the currently staged git changes{}. \
             Run `git diff --cached` to see what's staged, \
             write an appropriate commit message following the repository's conventions, \
             and run `git commit`.",
            extra
        ))
    }
}

// ---------------------------------------------------------------------------
// UI settings helpers (stored in ~/.coven-code/ui-settings.json)
// These hold things not present in the core Config struct.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct UiSettings {
    #[serde(default)]
    pub editor_mode: Option<String>, // "vim" or "normal"
    #[serde(default)]
    pub fast_mode: Option<bool>,
    #[serde(default)]
    pub voice_enabled: Option<bool>,
    #[serde(default)]
    pub statusline_show_cost: Option<bool>,
    #[serde(default)]
    pub statusline_show_tokens: Option<bool>,
    #[serde(default)]
    pub statusline_show_model: Option<bool>,
    #[serde(default)]
    pub statusline_show_time: Option<bool>,
    #[serde(default)]
    pub prompt_color: Option<String>,
    #[serde(default)]
    pub sandbox_mode: Option<bool>,
    /// Shell command patterns excluded from sandboxing (glob-style strings).
    /// Mirrors TS `excludedCommands` in settings.local.json.
    #[serde(default)]
    pub sandbox_excluded_commands: Vec<String>,
}

fn ui_settings_path() -> std::path::PathBuf {
    claurst_core::config::Settings::config_dir().join("ui-settings.json")
}

fn load_ui_settings() -> UiSettings {
    let path = ui_settings_path();
    if !path.exists() {
        return UiSettings::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_ui_settings(settings: &UiSettings) -> anyhow::Result<()> {
    let path = ui_settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)?;
    std::fs::write(&path, json)?;
    Ok(())
}

fn mutate_ui_settings<F>(f: F) -> anyhow::Result<UiSettings>
where
    F: FnOnce(&mut UiSettings),
{
    let mut s = load_ui_settings();
    f(&mut s);
    save_ui_settings(&s)?;
    Ok(s)
}

// ---- /context ------------------------------------------------------------

#[async_trait]
impl SlashCommand for ContextCommand {
    fn name(&self) -> &str {
        "context"
    }
    fn hidden(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Show context window usage (tokens used / available)"
    }
    fn help(&self) -> &str {
        "Usage: /context\n\n\
         Displays the current context window utilization:\n\
         - Estimated tokens consumed by current conversation\n\
         - Context window limit for the active model\n\
         - Percentage used"
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let model = ctx.config.effective_model();

        // Determine context window size from known model names
        let context_window: u64 = 200_000;

        let used_tokens = ctx.cost_tracker.total_tokens();
        let pct = if context_window > 0 {
            (used_tokens as f64 / context_window as f64) * 100.0
        } else {
            0.0
        };

        let bar_width = 40usize;
        let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
        let bar: String = "█".repeat(filled) + &"░".repeat(bar_width.saturating_sub(filled));

        // Estimate approximate message tokens from the message list
        let msg_char_count: usize = ctx.messages.iter().map(|m| m.get_all_text().len()).sum();
        // Rough estimate: ~4 chars per token for message text
        let msg_token_estimate = msg_char_count / 4;

        CommandResult::Message(format!(
            "Context Window Usage\n\
             ────────────────────\n\
             Model:          {model}\n\
             Context window: {window:>10} tokens\n\
             API tokens used:{used:>10} tokens  ({pct:.1}%)\n\
             Est. msg size:  {msg:>10} tokens  (approx)\n\
             Messages:       {msgs:>10}\n\n\
             [{bar}] {pct:.1}%\n\n\
             Use /compact to reduce context usage.",
            model = model,
            window = context_window,
            used = used_tokens,
            pct = pct,
            msg = msg_token_estimate,
            msgs = ctx.messages.len(),
            bar = bar,
        ))
    }
}

// ---- /copy ---------------------------------------------------------------

#[async_trait]
impl SlashCommand for CopyCommand {
    fn name(&self) -> &str {
        "copy"
    }
    fn description(&self) -> &str {
        "Copy the last assistant response to the clipboard"
    }
    fn help(&self) -> &str {
        "Usage: /copy [n]\n\n\
         Copies the most recent assistant response to the system clipboard.\n\
         Optionally pass a number to copy the Nth most-recent response."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let n: usize = args.trim().parse().unwrap_or(1).max(1);

        // Find the Nth most recent assistant message
        let assistant_msgs: Vec<&claurst_core::types::Message> = ctx
            .messages
            .iter()
            .rev()
            .filter(|m| m.role == claurst_core::types::Role::Assistant)
            .take(n)
            .collect();

        let msg = match assistant_msgs.last() {
            Some(m) => m,
            None => {
                return CommandResult::Message(
                    "No assistant messages found in conversation.".to_string(),
                )
            }
        };

        let text = msg.get_all_text();
        if text.is_empty() {
            return CommandResult::Message("Last assistant message is empty.".to_string());
        }

        // Try system clipboard via arboard
        #[cfg(not(target_os = "linux"))]
        {
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone())) {
                Ok(()) => {
                    let preview: String = text.chars().take(80).collect();
                    let ellipsis = if text.len() > 80 { "…" } else { "" };
                    return CommandResult::Message(format!(
                        "Copied {} chars to clipboard.\nPreview: {}{}",
                        text.len(),
                        preview,
                        ellipsis
                    ));
                }
                Err(e) => {
                    tracing::warn!("Clipboard write failed: {}", e);
                    // Fall through to file fallback
                }
            }
        }

        // Fallback: write to a temp file and inform the user
        let tmp_path = std::env::temp_dir().join("claude_copy.md");
        match std::fs::write(&tmp_path, &text) {
            Ok(()) => {
                let preview: String = text.chars().take(80).collect();
                let ellipsis = if text.len() > 80 { "…" } else { "" };
                CommandResult::Message(format!(
                    "Clipboard not available; saved {} chars to {}\nPreview: {}{}",
                    text.len(),
                    tmp_path.display(),
                    preview,
                    ellipsis
                ))
            }
            Err(e) => CommandResult::Error(format!("Failed to copy: {}", e)),
        }
    }
}

// ---- /chrome -------------------------------------------------------------
//
// Real CDP-over-WebSocket implementation.
//
// Chrome must be launched with:
//   chrome --remote-debugging-port=9222 --no-first-run
//
// The connection is stored in a process-wide lazy mutex so subsequent
// subcommand calls reuse the same WebSocket session.

mod chrome_cdp {
    use base64::Engine as _;
    use futures::{SinkExt, StreamExt};
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::net::TcpStream;
    use tokio_tungstenite::{
        connect_async, tungstenite::Message as WsMessage, MaybeTlsStream, WebSocketStream,
    };

    // -----------------------------------------------------------------------
    // Global session state
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    pub struct ChromeSession {
        pub ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
        pub port: u16,
        pub tab_url: String,
    }

    static SESSION: Lazy<Mutex<Option<ChromeSession>>> = Lazy::new(|| Mutex::new(None));
    static MSG_ID: AtomicU64 = AtomicU64::new(1);

    fn next_id() -> u64 {
        MSG_ID.fetch_add(1, Ordering::Relaxed)
    }

    // -----------------------------------------------------------------------
    // Low-level CDP helpers
    // -----------------------------------------------------------------------

    /// Send a CDP method call and wait for the matching response.
    /// Returns the full response object (including `result` / `error`).
    async fn cdp_call(
        ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        method: &str,
        params: Value,
    ) -> anyhow::Result<Value> {
        let id = next_id();
        let request = json!({ "id": id, "method": method, "params": params });
        ws.send(WsMessage::Text(request.to_string())).await?;

        // Drain messages until we get the one with our id (ignore events).
        loop {
            let raw = ws
                .next()
                .await
                .ok_or_else(|| anyhow::anyhow!("WebSocket closed unexpectedly"))??;
            let text: String = match raw {
                WsMessage::Text(t) => t.to_string(),
                WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
                WsMessage::Close(_) => {
                    return Err(anyhow::anyhow!("WebSocket closed by Chrome"));
                }
                _ => continue,
            };
            let val: Value = serde_json::from_str(&text)?;
            if val["id"] == id {
                if let Some(err) = val.get("error") {
                    return Err(anyhow::anyhow!("CDP error: {}", err));
                }
                return Ok(val);
            }
            // It's an event or different response — keep waiting.
        }
    }

    // -----------------------------------------------------------------------
    // Session take/restore helpers
    //
    // We avoid holding a MutexGuard across await points by taking ownership
    // of the session, performing all async operations with it, then putting
    // it back into the global.
    // -----------------------------------------------------------------------

    fn take_session() -> anyhow::Result<ChromeSession> {
        SESSION.lock().take().ok_or_else(|| {
            anyhow::anyhow!("No active Chrome session. Run `/chrome connect` first.")
        })
    }

    fn store_session(s: ChromeSession) {
        *SESSION.lock() = Some(s);
    }

    // -----------------------------------------------------------------------
    // Public helpers called from the SlashCommand impl
    // -----------------------------------------------------------------------

    /// Connect to Chrome at the given port.
    /// Picks the first available target (tab/page).
    pub async fn connect(port: u16) -> anyhow::Result<String> {
        let http_url = format!("http://localhost:{}/json/list", port);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()?;
        let tabs: Value = client.get(&http_url).send().await?.json().await?;

        let ws_url = tabs
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|t| t["type"] == "page")
                    .and_then(|t| t["webSocketDebuggerUrl"].as_str().map(|s| s.to_string()))
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No debuggable page found on port {}. \
                     Make sure Chrome has at least one open tab.",
                    port
                )
            })?;

        let tab_url = tabs
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|t| t["type"] == "page")
                    .and_then(|t| t["url"].as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default();

        let (ws, _) = connect_async(&ws_url)
            .await
            .map_err(|e| anyhow::anyhow!("WebSocket connect to {} failed: {}", ws_url, e))?;

        let mut session = ChromeSession {
            ws,
            port,
            tab_url: tab_url.clone(),
        };
        // Enable Page domain so captureScreenshot etc. work.
        cdp_call(&mut session.ws, "Page.enable", json!({})).await?;
        // Enable Runtime domain for eval/click/fill.
        cdp_call(&mut session.ws, "Runtime.enable", json!({})).await?;

        store_session(session);

        Ok(format!(
            "Connected to Chrome on port {} (tab: {})",
            port, tab_url
        ))
    }

    /// Disconnect the current session.
    pub fn disconnect() -> String {
        let mut guard = SESSION.lock();
        if guard.is_some() {
            *guard = None;
            "Disconnected from Chrome.".to_string()
        } else {
            "No active Chrome session.".to_string()
        }
    }

    /// Navigate to a URL.
    pub async fn navigate(url: &str) -> anyhow::Result<String> {
        let url = url.to_string();
        let mut s = take_session()?;
        let result = async {
            let resp = cdp_call(&mut s.ws, "Page.navigate", json!({ "url": url })).await?;
            let frame_id = resp["result"]["frameId"].as_str().unwrap_or("unknown");
            Ok(format!("Navigated. frameId={}", frame_id))
        }
        .await;
        store_session(s);
        result
    }

    /// Take a screenshot, write PNG to a temp file, return the path.
    pub async fn screenshot() -> anyhow::Result<String> {
        let mut s = take_session()?;
        let result = async {
            let resp = cdp_call(
                &mut s.ws,
                "Page.captureScreenshot",
                json!({ "format": "png", "captureBeyondViewport": false }),
            )
            .await?;
            let b64 = resp["result"]["data"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("No screenshot data in response"))?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;

            let tmp = tempfile::Builder::new()
                .prefix("cc-chrome-")
                .suffix(".png")
                .tempfile()?;
            let path = tmp.path().to_path_buf();
            std::fs::write(&path, &bytes)?;
            // Persist file past the NamedTempFile drop.
            let _ = tmp.keep()?;
            Ok(format!("Screenshot saved to {}", path.display()))
        }
        .await;
        store_session(s);
        result
    }

    /// Click the first element matching a CSS selector.
    pub async fn click(selector: &str) -> anyhow::Result<String> {
        let sel_json = serde_json::to_string(selector)?;
        let js = format!(
            r#"(function(){{
                var el=document.querySelector({sel});
                if(!el)return 'ELEMENT_NOT_FOUND';
                var r=el.getBoundingClientRect();
                return JSON.stringify({{x:r.left+r.width/2,y:r.top+r.height/2}});
            }})()"#,
            sel = sel_json
        );
        let selector = selector.to_string();
        let mut s = take_session()?;
        let result = async {
            let resp = cdp_call(
                &mut s.ws,
                "Runtime.evaluate",
                json!({ "expression": js, "returnByValue": true }),
            )
            .await?;
            let val_str = resp["result"]["result"]["value"].as_str().unwrap_or("");
            if val_str == "ELEMENT_NOT_FOUND" {
                return Err(anyhow::anyhow!(
                    "No element found for selector: {}",
                    selector
                ));
            }
            let coords: Value = serde_json::from_str(val_str)?;
            let x = coords["x"].as_f64().unwrap_or(0.0);
            let y = coords["y"].as_f64().unwrap_or(0.0);

            cdp_call(
                &mut s.ws,
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mousePressed", "x": x, "y": y,
                    "button": "left", "clickCount": 1
                }),
            )
            .await?;
            cdp_call(
                &mut s.ws,
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mouseReleased", "x": x, "y": y,
                    "button": "left", "clickCount": 1
                }),
            )
            .await?;

            Ok(format!("Clicked '{}' at ({:.0}, {:.0})", selector, x, y))
        }
        .await;
        store_session(s);
        result
    }

    /// Fill an input field.
    pub async fn fill(selector: &str, text: &str) -> anyhow::Result<String> {
        let js = format!(
            r#"(function(){{
                var el=document.querySelector({sel});
                if(!el)return false;
                el.focus();
                el.value={val};
                el.dispatchEvent(new Event('input',{{bubbles:true}}));
                el.dispatchEvent(new Event('change',{{bubbles:true}}));
                return true;
            }})()"#,
            sel = serde_json::to_string(selector)?,
            val = serde_json::to_string(text)?
        );
        let selector = selector.to_string();
        let text = text.to_string();
        let mut s = take_session()?;
        let result = async {
            let resp = cdp_call(
                &mut s.ws,
                "Runtime.evaluate",
                json!({ "expression": js, "returnByValue": true }),
            )
            .await?;
            let ok = resp["result"]["result"]["value"].as_bool().unwrap_or(false);
            if ok {
                Ok(format!("Filled '{}' with {:?}", selector, text))
            } else {
                Err(anyhow::anyhow!(
                    "No element found for selector: {}",
                    selector
                ))
            }
        }
        .await;
        store_session(s);
        result
    }

    /// Evaluate arbitrary JavaScript and return the result as a string.
    pub async fn eval(js: &str) -> anyhow::Result<String> {
        let js = js.to_string();
        let mut s = take_session()?;
        let result = async {
            let resp = cdp_call(
                &mut s.ws,
                "Runtime.evaluate",
                json!({ "expression": js, "returnByValue": true }),
            )
            .await?;
            let result_val = &resp["result"]["result"];
            let out = if let Some(v) = result_val["value"].as_str() {
                v.to_string()
            } else if !result_val["value"].is_null() {
                result_val["value"].to_string()
            } else if let Some(desc) = result_val["description"].as_str() {
                desc.to_string()
            } else {
                result_val.to_string()
            };
            Ok(out)
        }
        .await;
        store_session(s);
        result
    }
}

// ---- SlashCommand impl -------------------------------------------------------

#[async_trait]
impl SlashCommand for ChromeCommand {
    fn name(&self) -> &str {
        "chrome"
    }
    fn description(&self) -> &str {
        "Browser automation via Chrome DevTools Protocol (CDP)"
    }
    fn help(&self) -> &str {
        "Usage: /chrome <subcommand> [args]\n\n\
         Control a running Chrome/Chromium browser via CDP.\n\n\
         First, launch Chrome with remote debugging enabled:\n\
           chrome --remote-debugging-port=9222 --no-first-run\n\n\
         Subcommands:\n\
           /chrome connect [--port 9222]      — connect to Chrome\n\
           /chrome navigate <url>             — navigate to URL\n\
           /chrome screenshot                 — take screenshot, save to temp file\n\
           /chrome click <selector>           — click CSS selector\n\
           /chrome fill <selector> <text>     — fill input field\n\
           /chrome eval <js>                  — evaluate JavaScript\n\
           /chrome disconnect                 — disconnect"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let mut parts = args.trim().splitn(2, char::is_whitespace);
        let sub = parts.next().unwrap_or("").trim();
        let rest = parts.next().unwrap_or("").trim();

        match sub {
            // ------------------------------------------------------------------
            // /chrome connect [--port <N>]
            // ------------------------------------------------------------------
            "connect" => {
                let port: u16 = if let Some(p) = rest.strip_prefix("--port ").map(str::trim) {
                    match p.parse() {
                        Ok(n) => n,
                        Err(_) => {
                            return CommandResult::Error(format!("Invalid port number: {}", p));
                        }
                    }
                } else if rest.is_empty() {
                    9222
                } else {
                    match rest.parse() {
                        Ok(n) => n,
                        Err(_) => {
                            return CommandResult::Error(format!(
                                "Usage: /chrome connect [--port <N>]\nInvalid argument: {}",
                                rest
                            ));
                        }
                    }
                };

                match chrome_cdp::connect(port).await {
                    Ok(msg) => CommandResult::Message(msg),
                    Err(e) => CommandResult::Error(format!(
                        "Failed to connect to Chrome on port {}: {}\n\n\
                         Make sure Chrome is running with:\n\
                           chrome --remote-debugging-port={} --no-first-run",
                        port, e, port
                    )),
                }
            }

            // ------------------------------------------------------------------
            // /chrome navigate <url>
            // ------------------------------------------------------------------
            "navigate" => {
                if rest.is_empty() {
                    return CommandResult::Error(
                        "Usage: /chrome navigate <url>\nExample: /chrome navigate https://example.com"
                            .to_string(),
                    );
                }
                match chrome_cdp::navigate(rest).await {
                    Ok(msg) => CommandResult::Message(msg),
                    Err(e) => CommandResult::Error(e.to_string()),
                }
            }

            // ------------------------------------------------------------------
            // /chrome screenshot
            // ------------------------------------------------------------------
            "screenshot" => match chrome_cdp::screenshot().await {
                Ok(msg) => CommandResult::Message(msg),
                Err(e) => CommandResult::Error(e.to_string()),
            },

            // ------------------------------------------------------------------
            // /chrome click <selector>
            // ------------------------------------------------------------------
            "click" => {
                if rest.is_empty() {
                    return CommandResult::Error(
                        "Usage: /chrome click <css-selector>\nExample: /chrome click button#submit"
                            .to_string(),
                    );
                }
                match chrome_cdp::click(rest).await {
                    Ok(msg) => CommandResult::Message(msg),
                    Err(e) => CommandResult::Error(e.to_string()),
                }
            }

            // ------------------------------------------------------------------
            // /chrome fill <selector> <text>
            // ------------------------------------------------------------------
            "fill" => {
                // Split selector and text at first whitespace.
                let mut fill_parts = rest.splitn(2, char::is_whitespace);
                let selector = fill_parts.next().unwrap_or("").trim();
                let text = fill_parts.next().unwrap_or("").trim();
                if selector.is_empty() {
                    return CommandResult::Error(
                        "Usage: /chrome fill <css-selector> <text>\nExample: /chrome fill input#email user@example.com"
                            .to_string(),
                    );
                }
                match chrome_cdp::fill(selector, text).await {
                    Ok(msg) => CommandResult::Message(msg),
                    Err(e) => CommandResult::Error(e.to_string()),
                }
            }

            // ------------------------------------------------------------------
            // /chrome eval <js>
            // ------------------------------------------------------------------
            "eval" => {
                if rest.is_empty() {
                    return CommandResult::Error(
                        "Usage: /chrome eval <javascript>\nExample: /chrome eval document.title"
                            .to_string(),
                    );
                }
                match chrome_cdp::eval(rest).await {
                    Ok(result) => CommandResult::Message(format!("=> {}", result)),
                    Err(e) => CommandResult::Error(e.to_string()),
                }
            }

            // ------------------------------------------------------------------
            // /chrome disconnect
            // ------------------------------------------------------------------
            "disconnect" => CommandResult::Message(chrome_cdp::disconnect()),

            // ------------------------------------------------------------------
            // No subcommand or unknown
            // ------------------------------------------------------------------
            "" => CommandResult::Message(self.help().to_string()),
            other => CommandResult::Error(format!(
                "Unknown subcommand: '{}'\n\n{}",
                other,
                self.help()
            )),
        }
    }
}

// ---- /vim (/vi) ----------------------------------------------------------

#[async_trait]
impl SlashCommand for VimCommand {
    fn name(&self) -> &str {
        "vim"
    }
    fn hidden(&self) -> bool {
        true
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["vi"]
    }
    fn description(&self) -> &str {
        "Toggle vim keybinding mode on/off"
    }
    fn help(&self) -> &str {
        "Usage: /vim [on|off]\n\n\
         Toggles vim keybinding mode in the REPL input.\n\
         When enabled, use Esc to switch between INSERT and NORMAL modes.\n\n\
         The setting is persisted to ~/.coven-code/ui-settings.json."
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let current = load_ui_settings();
        let current_mode = current.editor_mode.as_deref().unwrap_or("normal");

        let new_mode = match args.trim() {
            "on" | "vim" => "vim",
            "off" | "normal" => "normal",
            "" => {
                // Toggle
                if current_mode == "vim" {
                    "normal"
                } else {
                    "vim"
                }
            }
            other => {
                return CommandResult::Error(format!(
                    "Unknown argument '{}'. Use: /vim [on|off]",
                    other
                ))
            }
        };

        match mutate_ui_settings(|s| s.editor_mode = Some(new_mode.to_string())) {
            Ok(_) => CommandResult::Message(format!(
                "Editor mode set to {}.\n{}",
                new_mode,
                if new_mode == "vim" {
                    "Use Esc to switch between INSERT and NORMAL modes.\n\
                     Restart the REPL for the change to take effect."
                } else {
                    "Using standard (readline-style) keyboard bindings.\n\
                     Restart the REPL for the change to take effect."
                }
            )),
            Err(e) => CommandResult::Error(format!("Failed to save setting: {}", e)),
        }
    }
}

// ---- /voice --------------------------------------------------------------

#[async_trait]
impl SlashCommand for VoiceCommand {
    fn name(&self) -> &str {
        "voice"
    }
    fn hidden(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Toggle voice input mode on/off"
    }
    fn help(&self) -> &str {
        "Usage: /voice [on|off|status]\n\n\
         Enables or disables voice input (push-to-talk).\n\
         Setting is persisted to ~/.coven-code/ui-settings.json.\n\n\
         Transcription is performed via a Whisper-compatible API.\n\
         Set one of these env vars for the API key:\n\
           OPENAI_API_KEY   — OpenAI Whisper (default endpoint)\n\
           ANTHROPIC_API_KEY — used as a fallback key\n\n\
         To use a local Whisper server instead of OpenAI:\n\
           export WHISPER_ENDPOINT_URL=http://localhost:8080/v1/audio/transcriptions\n\
           export OPENAI_API_KEY=any-value  (local servers often ignore the key)\n\n\
         On Linux, ALSA must be set up: sudo apt install libasound2-dev\n\
         Check available devices with: arecord -l\n\n\
         Controls:\n\
           Alt+V — start recording; Alt+V or Esc — stop and transcribe"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let current = load_ui_settings();
        let currently_enabled = current.voice_enabled.unwrap_or(false);

        let enable = match args.trim() {
            "on" | "enable" | "enabled" | "true" | "1" => true,
            "off" | "disable" | "disabled" | "false" | "0" => false,
            "" => !currently_enabled, // toggle
            "status" => {
                let state = if currently_enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                let endpoint = std::env::var("WHISPER_ENDPOINT_URL").unwrap_or_else(|_| {
                    "https://api.openai.com/v1/audio/transcriptions (default)".to_string()
                });
                let key_source = if std::env::var("OPENAI_API_KEY").is_ok() {
                    "OPENAI_API_KEY"
                } else if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                    "ANTHROPIC_API_KEY"
                } else {
                    "(none — transcription will fail)"
                };
                return CommandResult::Message(format!(
                    "Voice mode: {}\n\
                     Endpoint:   {}\n\
                     API key:    {}",
                    state, endpoint, key_source
                ));
            }
            other => {
                return CommandResult::Error(format!(
                    "Unknown argument '{}'. Use: /voice [on|off|status]",
                    other
                ))
            }
        };

        match mutate_ui_settings(|s| s.voice_enabled = Some(enable)) {
            Ok(_) => {
                if enable {
                    let endpoint = std::env::var("WHISPER_ENDPOINT_URL")
                        .unwrap_or_else(|_| "OpenAI Whisper (default)".to_string());
                    let key_hint = if std::env::var("OPENAI_API_KEY").is_ok()
                        || std::env::var("ANTHROPIC_API_KEY").is_ok()
                    {
                        String::new()
                    } else {
                        "\nWarning: no OPENAI_API_KEY found — transcription will fail. \
                         Set OPENAI_API_KEY or WHISPER_ENDPOINT_URL for a local server."
                            .to_string()
                    };
                    CommandResult::Message(format!(
                        "Voice recording activated.\n\
                         Press Alt+V to start recording; Alt+V or Esc to stop and transcribe.\n\
                         Endpoint: {}{}",
                        endpoint, key_hint
                    ))
                } else {
                    CommandResult::Message("Voice recording deactivated.".to_string())
                }
            }
            Err(e) => CommandResult::Error(format!("Failed to save voice setting: {}", e)),
        }
    }
}

// ---- /upgrade ------------------------------------------------------------

#[async_trait]
impl SlashCommand for UpgradeCommand {
    fn name(&self) -> &str {
        "update"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["upgrade"]
    }
    fn description(&self) -> &str {
        "Check for updates and download the latest release"
    }
    fn help(&self) -> &str {
        "Usage: /update\n\n\
         Checks GitHub releases for the latest version of Coven Code.\n\
         If a newer version is available, shows where to download it."
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let current = claurst_core::constants::APP_VERSION;

        // Check GitHub releases API for latest version
        let client = reqwest::Client::builder()
            .user_agent(format!("coven-code/{}", current))
            .timeout(std::time::Duration::from_secs(8))
            .build();

        let client = match client {
            Ok(c) => c,
            Err(e) => {
                return CommandResult::Message(format!(
                    "Current version: {current}\n\
                     Could not check for updates (HTTP client error: {e})\n\
                     Visit https://github.com/OpenCoven/coven-code/releases for updates."
                ))
            }
        };

        let resp = client
            .get("https://api.github.com/repos/OpenCoven/coven-code/releases/latest")
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let json: serde_json::Value = r.json().await.unwrap_or(serde_json::Value::Null);

                let tag = json
                    .get("tag_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .trim_start_matches('v');

                let url = json
                    .get("html_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://github.com/OpenCoven/coven-code/releases");

                if tag == current || tag == "unknown" {
                    CommandResult::Message(format!(
                        "Coven Code v{current} - you are up to date.\n\
                         Release page: {url}"
                    ))
                } else {
                    CommandResult::Message(format!(
                        "Update available!\n\
                         Current version:  v{current}\n\
                         Latest version:   v{tag}\n\
                         Release page:     {url}\n\n\
                         Upgrade in place (recommended):\n\
                           coven-code upgrade\n\n\
                         Or build from source:\n\
                           cargo install coven-code --force"
                    ))
                }
            }
            Ok(r) => {
                let status = r.status();
                CommandResult::Message(format!(
                    "Current version: v{current}\n\
                     Could not check for updates (HTTP {status}).\n\
                     Visit https://github.com/OpenCoven/coven-code/releases for updates."
                ))
            }
            Err(e) => CommandResult::Message(format!(
                "Current version: v{current}\n\
                 Could not check for updates: {e}\n\
                 Visit https://github.com/OpenCoven/coven-code/releases for updates."
            )),
        }
    }
}

// ---- /statusline ---------------------------------------------------------

#[async_trait]
impl SlashCommand for StatuslineCommand {
    fn name(&self) -> &str {
        "statusline"
    }
    fn hidden(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Configure what is shown in the status line"
    }
    fn help(&self) -> &str {
        "Usage: /statusline [show|hide] [cost|tokens|model|time|all]\n\n\
         Controls which items appear in the TUI status bar at the bottom.\n\
         Settings are persisted to ~/.coven-code/ui-settings.json.\n\n\
         Examples:\n\
           /statusline               — show current configuration\n\
           /statusline show cost     — show cost in status line\n\
           /statusline hide tokens   — hide token count\n\
           /statusline show all      — show everything\n\
           /statusline hide all      — hide everything"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let args = args.trim();
        let current = load_ui_settings();

        if args.is_empty() {
            return CommandResult::Message(format!(
                "Status line configuration\n\
                 ─────────────────────────\n\
                 Show cost:   {cost}\n\
                 Show tokens: {tokens}\n\
                 Show model:  {model}\n\
                 Show time:   {time}\n\n\
                 Use /statusline [show|hide] [cost|tokens|model|time|all] to change.",
                cost = fmt_bool(current.statusline_show_cost.unwrap_or(true)),
                tokens = fmt_bool(current.statusline_show_tokens.unwrap_or(true)),
                model = fmt_bool(current.statusline_show_model.unwrap_or(true)),
                time = fmt_bool(current.statusline_show_time.unwrap_or(true)),
            ));
        }

        let mut parts = args.splitn(2, ' ');
        let verb = parts.next().unwrap_or("").trim();
        let item = parts.next().unwrap_or("").trim();

        let show = match verb {
            "show" | "enable" | "on" => true,
            "hide" | "disable" | "off" => false,
            _ => {
                return CommandResult::Error(
                    "Usage: /statusline [show|hide] [cost|tokens|model|time|all]".to_string(),
                )
            }
        };

        if item.is_empty() || item == "all" {
            match mutate_ui_settings(|s| {
                s.statusline_show_cost = Some(show);
                s.statusline_show_tokens = Some(show);
                s.statusline_show_model = Some(show);
                s.statusline_show_time = Some(show);
            }) {
                Ok(_) => {
                    return CommandResult::Message(format!(
                        "Status line: all items {}.",
                        if show { "shown" } else { "hidden" }
                    ))
                }
                Err(e) => return CommandResult::Error(format!("Failed to save: {}", e)),
            }
        }

        let result = match item {
            "cost" => mutate_ui_settings(|s| s.statusline_show_cost = Some(show)),
            "tokens" | "token" => mutate_ui_settings(|s| s.statusline_show_tokens = Some(show)),
            "model" => mutate_ui_settings(|s| s.statusline_show_model = Some(show)),
            "time" | "clock" => mutate_ui_settings(|s| s.statusline_show_time = Some(show)),
            other => {
                return CommandResult::Error(format!(
                    "Unknown item '{}'. Use: cost, tokens, model, time, or all.",
                    other
                ))
            }
        };

        match result {
            Ok(_) => CommandResult::Message(format!(
                "Status line: {} {}.",
                item,
                if show { "shown" } else { "hidden" }
            )),
            Err(e) => CommandResult::Error(format!("Failed to save: {}", e)),
        }
    }
}

fn fmt_bool(v: bool) -> &'static str {
    if v {
        "on"
    } else {
        "off"
    }
}

// ---- /security-review ----------------------------------------------------

#[async_trait]
impl SlashCommand for SecurityReviewCommand {
    fn name(&self) -> &str {
        "security-review"
    }
    fn description(&self) -> &str {
        "Run a security review of the current project"
    }
    fn help(&self) -> &str {
        "Usage: /security-review [path]\n\n\
         Asks Coven Code to perform a security review of the codebase.\n\
         Analyzes for common vulnerabilities: injection attacks, auth issues,\n\
         secrets exposure, unsafe deserialization, path traversal, etc."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let target = if args.trim().is_empty() {
            ctx.working_dir.display().to_string()
        } else {
            args.trim().to_string()
        };

        CommandResult::UserMessage(format!(
            "Please perform a comprehensive security review of the code in `{target}`.\n\n\
             Focus on identifying:\n\
             1. Injection vulnerabilities (SQL, command, LDAP, XSS, SSTI)\n\
             2. Authentication and authorization flaws\n\
             3. Hardcoded secrets, API keys, or passwords\n\
             4. Insecure deserialization\n\
             5. Path traversal or file inclusion vulnerabilities\n\
             6. Cryptographic weaknesses (weak algorithms, bad IV usage, key reuse)\n\
             7. Dependency vulnerabilities (check for outdated packages)\n\
             8. Race conditions and TOCTOU issues\n\
             9. Information disclosure (verbose errors, debug endpoints)\n\
             10. Any OWASP Top 10 issues relevant to this codebase\n\n\
             For each finding, provide:\n\
             - Severity: Critical/High/Medium/Low/Informational\n\
             - File and line number\n\
             - Description of the vulnerability\n\
             - Proof of concept or reproduction steps\n\
             - Recommended remediation\n\n\
             Start by reading the main source files and any dependency manifests.",
            target = target,
        ))
    }
}

// ---- /terminal-setup -----------------------------------------------------

#[async_trait]
impl SlashCommand for TerminalSetupCommand {
    fn name(&self) -> &str {
        "terminal-setup"
    }
    fn hidden(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Help configure your terminal for optimal Coven Code use"
    }
    fn help(&self) -> &str {
        "Usage: /terminal-setup\n\n\
         Diagnoses your terminal environment and gives recommendations for\n\
         optimal Coven Code display (font, color support, Unicode, etc.)."
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let mut checks: Vec<String> = Vec::new();

        // Check TERM variable
        let term = std::env::var("TERM").unwrap_or_default();
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();

        // Terminal identification
        let terminal_name = if !term_program.is_empty() {
            term_program.clone()
        } else {
            term.clone()
        };
        checks.push(format!("Terminal:      {}", terminal_name));

        // Color depth
        let color_depth = if colorterm == "truecolor" || colorterm == "24bit" {
            "24-bit true color (optimal)"
        } else if term.contains("256color") || colorterm == "256color" {
            "256 colors (good)"
        } else if !term.is_empty() {
            "Basic colors (limited)"
        } else {
            "Unknown"
        };
        checks.push(format!("Colors:        {}", color_depth));

        // Check if UNICODE is likely supported
        let lang = std::env::var("LANG").unwrap_or_default();
        let lc_all = std::env::var("LC_ALL").unwrap_or_default();
        let unicode_env =
            lang.to_lowercase().contains("utf") || lc_all.to_lowercase().contains("utf");
        checks.push(format!(
            "Unicode/UTF-8: {}",
            if unicode_env {
                "likely supported (LANG/LC_ALL contains UTF)"
            } else {
                "check LANG env var"
            }
        ));

        // Check for known good terminals
        let is_good_terminal = matches!(
            term_program.to_lowercase().as_str(),
            "iterm.app" | "iterm2" | "hyper" | "warp" | "alacritty" | "kitty" | "wezterm"
        ) || term_program.to_lowercase().contains("vscode")
            || term_program.to_lowercase().contains("terminal");

        checks.push(format!(
            "Terminal type: {}",
            if is_good_terminal {
                "well-known terminal (good)"
            } else {
                "verify settings below"
            }
        ));

        // Shell detection
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
        checks.push(format!("Shell:         {}", shell));

        // Check for Nerd Fonts (heuristic: environment variable set by some terminals)
        let nerd_font =
            std::env::var("NERD_FONT").is_ok() || std::env::var("TERM_NERD_FONT").is_ok();

        CommandResult::Message(format!(
            "Terminal Setup Diagnostic\n\
             ─────────────────────────\n\
             {checks}\n\n\
             Recommendations for optimal Coven Code experience:\n\
             ─────────────────────────────────────────────────\n\
             1. Font: Use a Nerd Font for box-drawing characters and icons\n\
                {nerd_hint}\n\
                Download: https://www.nerdfonts.com/\n\
             2. Color: Enable 24-bit true color:\n\
                export COLORTERM=truecolor\n\
             3. Unicode: Ensure UTF-8 locale:\n\
                export LANG=en_US.UTF-8\n\
             4. Recommended terminals:\n\
                - WezTerm (all platforms)\n\
                - Alacritty (all platforms)\n\
                - Kitty (macOS/Linux)\n\
                - Windows Terminal (Windows)\n\
                - iTerm2 (macOS)\n\
             5. Set terminal to unlimited scrollback for long conversations",
            checks = checks.join("\n  "),
            nerd_hint = if nerd_font {
                "[ok] Nerd Font detected"
            } else {
                "[!] Nerd Font not detected — box-drawing may appear broken"
            },
        ))
    }
}

// ---- /advisor ------------------------------------------------------------

#[async_trait]
impl SlashCommand for AdvisorCommand {
    fn name(&self) -> &str {
        "advisor"
    }
    fn description(&self) -> &str {
        "Set or unset the server-side advisor model"
    }
    fn help(&self) -> &str {
        "Usage: /advisor [<model>|off|unset]\n\n\
         Sets the advisor model used for server-side suggestions.\n\
         Examples:\n\
           /advisor claude-opus-4-6   — set advisor model\n\
           /advisor off               — disable the advisor\n\
           /advisor                   — show current advisor setting"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let arg = args.trim();
        let settings_dir = claurst_core::config::Settings::config_dir();
        let settings_path = settings_dir.join("settings.json");

        // Read or create settings JSON
        let mut settings_val: serde_json::Value = settings_path
            .exists()
            .then(|| std::fs::read_to_string(&settings_path).ok())
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}));

        match arg {
            "" => {
                let current = settings_val
                    .get("advisorModel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(not set)");
                CommandResult::Message(format!("Advisor model: {current}"))
            }
            "off" | "unset" | "none" => {
                settings_val
                    .as_object_mut()
                    .map(|m| m.remove("advisorModel"));
                if let Ok(json) = serde_json::to_string_pretty(&settings_val) {
                    let _ = std::fs::write(&settings_path, json);
                }
                CommandResult::Message("Advisor model unset.".to_string())
            }
            model => {
                // Basic validation: must look like a model identifier
                if model.starts_with("claude-") || model.contains('/') {
                    settings_val["advisorModel"] = serde_json::Value::String(model.to_string());
                    if let Ok(json) = serde_json::to_string_pretty(&settings_val) {
                        let _ = std::fs::write(&settings_path, json);
                    }
                    CommandResult::Message(format!("Advisor model set to: {model}"))
                } else {
                    CommandResult::Message(format!(
                        "Unknown model '{model}'. Model IDs should start with 'claude-'.\n\
                         Use /model to see available models."
                    ))
                }
            }
        }
    }
}

// ---- /fast (/speed) ------------------------------------------------------

#[async_trait]
impl SlashCommand for FastCommand {
    fn name(&self) -> &str {
        "fast"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["speed"]
    }
    fn description(&self) -> &str {
        "Toggle fast mode (uses a faster/cheaper model)"
    }
    fn help(&self) -> &str {
        "Usage: /fast [on|off]\n\n\
         Fast mode switches to the active provider's smaller, faster model\n\
         for quick responses. Toggle without argument to switch.\n\
         The setting is persisted to ~/.coven-code/ui-settings.json."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let current = load_ui_settings();
        let currently_on = current.fast_mode.unwrap_or(false);

        let enable = match args.trim() {
            "on" | "enable" | "true" | "1" => true,
            "off" | "disable" | "false" | "0" => false,
            "" => !currently_on,
            other => {
                return CommandResult::Error(format!(
                    "Unknown argument '{}'. Use: /fast [on|off]",
                    other
                ))
            }
        };

        if let Err(e) = mutate_ui_settings(|s| s.fast_mode = Some(enable)) {
            return CommandResult::Error(format!("Failed to save setting: {}", e));
        }

        let provider_id = ctx.config.selected_provider_id();
        let fast_model = resolve_fast_model_id(&ctx.config);
        let normal_model =
            stripped_model_for_provider(provider_id, ctx.config.effective_model()).to_string();

        if enable {
            let mut new_config = ctx.config.clone();
            new_config.model = Some(canonical_model_for_provider(provider_id, &fast_model));
            CommandResult::ConfigChangeMessage(
                new_config,
                format!(
                    "Fast mode ON. Using {} for quicker, cheaper responses.\n\
                     Use /fast off to return to {}.",
                    fast_model, normal_model
                ),
            )
        } else {
            let mut new_config = ctx.config.clone();
            // Restore default / saved model
            new_config.model = None;
            let restored_model =
                stripped_model_for_provider(provider_id, new_config.effective_model()).to_string();
            CommandResult::ConfigChangeMessage(
                new_config,
                format!(
                    "Fast mode OFF. Restored to default model ({}).",
                    restored_model
                ),
            )
        }
    }
}

// ---- /think-back ---------------------------------------------------------

#[async_trait]
impl SlashCommand for ThinkBackCommand {
    fn name(&self) -> &str {
        "think-back"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["thinkback"]
    }
    fn description(&self) -> &str {
        "Show thinking traces from previous responses in this session"
    }
    fn help(&self) -> &str {
        "Usage: /think-back [n] | /think-back play [n]\n\n\
         Displays the thinking/reasoning traces from the most recent model responses.\n\
         Pass a number to show the Nth most recent thinking block.\n\
         `/think-back play` replays a trace as an animated walkthrough."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let trimmed = args.trim();
        // /think-back play absorbs the former standalone /thinkback-play command.
        if let Some(rest) = trimmed.strip_prefix("play") {
            return ThinkBackPlayCommand.execute(rest.trim(), ctx).await;
        }
        let n: usize = trimmed.parse().unwrap_or(1).max(1);

        // Scan messages for thinking blocks
        let thinking_blocks: Vec<(usize, String)> = ctx
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == claurst_core::types::Role::Assistant)
            .filter_map(|(idx, m)| {
                let blocks = m.get_thinking_blocks();
                if blocks.is_empty() {
                    return None;
                }
                let thinking: String = blocks
                    .iter()
                    .filter_map(|b| {
                        if let claurst_core::types::ContentBlock::Thinking { thinking, .. } = b {
                            Some(thinking.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if thinking.is_empty() {
                    None
                } else {
                    Some((idx, thinking))
                }
            })
            .collect();

        if thinking_blocks.is_empty() {
            return CommandResult::Message(
                "No thinking traces found in this session.\n\
                 Thinking traces appear when the model uses extended thinking mode.\n\
                 Try asking Coven Code to 'think step by step' or 'think carefully'."
                    .to_string(),
            );
        }

        // Show the Nth most recent (1-indexed)
        let total = thinking_blocks.len();
        let target_idx = total.saturating_sub(n);
        let (msg_idx, trace) = &thinking_blocks[target_idx];

        CommandResult::Message(format!(
            "Thinking trace ({n} of {total} found, from message {msg}):\n\
             ─────────────────────────────────────\n\
             {trace}\n\
             ─────────────────────────────────────\n\
             Use /think-back <n> to see older traces.",
            n = n,
            total = total,
            msg = msg_idx + 1,
            trace = trace,
        ))
    }
}

// ---- /thinkback-play -----------------------------------------------------

#[async_trait]
impl SlashCommand for ThinkBackPlayCommand {
    fn name(&self) -> &str {
        "thinkback-play"
    }
    fn description(&self) -> &str {
        "Replay a thinking trace as an animated walkthrough"
    }
    fn help(&self) -> &str {
        "Usage: /thinkback-play [n]\n\n\
         Replays a previous thinking trace, formatted for easy reading.\n\
         Pass a number to replay the Nth most recent trace."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let n: usize = args.trim().parse().unwrap_or(1).max(1);

        let thinking_blocks: Vec<String> = ctx
            .messages
            .iter()
            .filter(|m| m.role == claurst_core::types::Role::Assistant)
            .filter_map(|m| {
                let blocks = m.get_thinking_blocks();
                if blocks.is_empty() {
                    return None;
                }
                let t: String = blocks
                    .iter()
                    .filter_map(|b| {
                        if let claurst_core::types::ContentBlock::Thinking { thinking, .. } = b {
                            Some(thinking.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            })
            .collect();

        if thinking_blocks.is_empty() {
            return CommandResult::Message(
                "No thinking traces to replay in this session.".to_string(),
            );
        }

        let total = thinking_blocks.len();
        let idx = total.saturating_sub(n);
        let trace = &thinking_blocks[idx];

        // Format the trace with step numbering
        let steps: Vec<&str> = trace.split('\n').filter(|l| !l.trim().is_empty()).collect();
        let mut formatted = format!(
            "Thinking Trace Replay ({}/{total})\n\
             ══════════════════════════════════\n",
            n,
            total = total
        );
        for (i, step) in steps.iter().enumerate() {
            formatted.push_str(&format!("  Step {}: {}\n", i + 1, step));
        }
        formatted.push_str("══════════════════════════════════\n");
        formatted.push_str(&format!(
            "{} steps shown. Use /think-back for raw traces.",
            steps.len()
        ));

        CommandResult::Message(formatted)
    }
}

// ---- /search -------------------------------------------------------------

#[async_trait]
impl SlashCommand for SearchCommand {
    fn name(&self) -> &str {
        "search"
    }
    fn description(&self) -> &str {
        "Search across all sessions"
    }
    fn help(&self) -> &str {
        "Usage: /search <query>\n\n\
         Searches session titles and message content in the local SQLite\n\
         session database (~/.coven-code/sessions.db).  Returns the 50 best\n\
         matching sessions, ordered by most recently updated.\n\n\
         Example: /search refactor authentication"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let query = args.trim();
        if query.is_empty() {
            return CommandResult::Error(
                "Usage: /search <query>\n\
                 Provide a search term to look up across all sessions."
                    .to_string(),
            );
        }

        let db_path = claurst_core::config::Settings::config_dir().join("sessions.db");

        let store = match claurst_core::SqliteSessionStore::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                return CommandResult::Error(format!(
                    "Failed to open session database: {}\n\
                     The database is created automatically once sessions are stored.",
                    e
                ))
            }
        };

        let results = match store.search_sessions(query) {
            Ok(r) => r,
            Err(e) => return CommandResult::Error(format!("Search failed: {}", e)),
        };

        if results.is_empty() {
            return CommandResult::Message(format!("No sessions found matching \"{}\".", query));
        }

        let mut out = format!(
            "Search results for \"{}\": {} session(s)\n\n",
            query,
            results.len()
        );
        for s in &results {
            let title = s.title.as_deref().unwrap_or("(untitled)");
            out.push_str(&format!(
                "  [{}] {} — {} ({} messages, updated {})\n",
                &s.id[..s.id.len().min(12)],
                title,
                s.model,
                s.message_count,
                &s.updated_at[..s.updated_at.len().min(10)],
            ));
        }
        out.push_str("\nTip: use /resume <session-id> to continue a session.");
        CommandResult::Message(out)
    }
}

// ---- /btw ----------------------------------------------------------------

#[async_trait]
impl SlashCommand for BtwCommand {
    fn name(&self) -> &str {
        "whisper"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["btw"]
    }
    fn description(&self) -> &str {
        "Whisper a side question to your familiar without adding it to history"
    }
    fn help(&self) -> &str {
        "Usage: /whisper <question>\n\n\
         Whispers a background question to the model without it becoming part of\n\
         the main conversation context. The response is shown inline but not\n\
         stored in the message history.\n\n\
         Example:\n\
           /whisper what is the capital of France?"
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let question = args.trim();
        if question.is_empty() {
            return CommandResult::Error(
                "Usage: /whisper <question>  — provide a question after /whisper".to_string(),
            );
        }

        // Surface as a special user message tagged as a side-question so the
        // REPL/TUI can handle it as a non-history query. We inject a system tag
        // that tells the backend to answer but not record the exchange.
        CommandResult::UserMessage(format!(
            "[/btw side-question — answer inline, do not store in history]: {}",
            question
        ))
    }
}

// ---- /sandbox-toggle -----------------------------------------------------

#[async_trait]
impl SlashCommand for SandboxToggleCommand {
    fn name(&self) -> &str {
        "sandbox"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["sandbox-toggle"]
    }
    fn description(&self) -> &str {
        "Enable or disable sandboxed execution of shell commands"
    }
    fn help(&self) -> &str {
        "Usage: /sandbox-toggle [on|off|exclude <pattern>|status]\n\n\
         Toggles sandboxed execution of bash/shell commands.\n\
         When sandbox mode is enabled, shell commands run in an isolated\n\
         environment to prevent unintended side effects.\n\n\
         Subcommands:\n\
           /sandbox-toggle           — toggle the current state\n\
           /sandbox-toggle on        — enable sandbox mode\n\
           /sandbox-toggle off       — disable sandbox mode\n\
           /sandbox-toggle status    — show current state and excluded patterns\n\
           /sandbox-toggle exclude <pattern>  — add a command pattern to exclusions\n\n\
         Sandbox is supported on macOS, Linux, and WSL2.\n\
         Note: A restart is recommended for full effect."
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let args = args.trim();

        // Platform support check: sandbox requires macOS or Linux (not Windows native).
        let platform = std::env::consts::OS;
        let is_wsl =
            std::env::var("WSL_DISTRO_NAME").is_ok() || std::env::var("WSL_INTEROP").is_ok();
        let is_supported = matches!(platform, "linux" | "macos") || is_wsl;

        // Handle subcommand: status
        if args == "status" {
            let ui = load_ui_settings();
            let mode = if ui.sandbox_mode.unwrap_or(false) {
                "enabled"
            } else {
                "disabled"
            };
            let excl = if ui.sandbox_excluded_commands.is_empty() {
                "(none)".to_string()
            } else {
                ui.sandbox_excluded_commands
                    .iter()
                    .map(|p| format!("  - {}", p))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let platform_note = if is_supported {
                format!("\u{2713} Supported on this platform ({})", platform)
            } else {
                format!(
                    "\u{2717} Not supported on this platform ({}). Requires macOS, Linux, or WSL2.",
                    platform
                )
            };
            return CommandResult::Message(format!(
                "Sandbox mode: {}\n\
                 Platform:     {}\n\
                 Excluded command patterns:\n{}\n\n\
                 Use /sandbox-toggle [on|off] to change mode.\n\
                 Use /sandbox-toggle exclude <pattern> to add exclusions.",
                mode, platform_note, excl
            ));
        }

        // Handle subcommand: exclude <pattern>
        if let Some(rest) = args.strip_prefix("exclude").map(str::trim) {
            if rest.is_empty() {
                return CommandResult::Error(
                    "Usage: /sandbox-toggle exclude <command-pattern>\n\
                     Example: /sandbox-toggle exclude \"npm run test:*\""
                        .to_string(),
                );
            }
            // Strip surrounding quotes if present
            let pattern = rest.trim_matches(|c| c == '"' || c == '\'').to_string();
            if pattern.is_empty() {
                return CommandResult::Error("Pattern cannot be empty.".to_string());
            }
            match mutate_ui_settings(|s| {
                if !s.sandbox_excluded_commands.contains(&pattern) {
                    s.sandbox_excluded_commands.push(pattern.clone());
                }
            }) {
                Ok(_) => {
                    let settings_path = ui_settings_path();
                    return CommandResult::Message(format!(
                        "Added \"{}\" to sandbox excluded commands.\n\
                         Saved to: {}",
                        pattern,
                        settings_path.display()
                    ));
                }
                Err(e) => return CommandResult::Error(format!("Failed to save exclusion: {}", e)),
            }
        }

        // Platform guard for toggling on/off
        if !is_supported
            && (args == "on"
                || args == "enable"
                || args == "enabled"
                || args == "true"
                || args == "1"
                || args.is_empty())
        {
            let msg = if is_wsl {
                "Error: Sandboxing requires WSL2. WSL1 is not supported.".to_string()
            } else {
                format!(
                    "Error: Sandboxing is currently only supported on macOS, Linux, and WSL2.\n\
                     Current platform: {}",
                    platform
                )
            };
            // Only hard-block enabling; allow off/status even on unsupported platforms.
            if args != "off"
                && args != "disable"
                && args != "disabled"
                && args != "false"
                && args != "0"
            {
                return CommandResult::Error(msg);
            }
        }

        // Read current sandbox state from ui-settings
        let current_ui = load_ui_settings();
        let currently_enabled = current_ui.sandbox_mode.unwrap_or(false);

        let enable = match args {
            "on" | "enable" | "enabled" | "true" | "1" => true,
            "off" | "disable" | "disabled" | "false" | "0" => false,
            "" => !currently_enabled,
            other => {
                return CommandResult::Error(format!(
                    "Unknown argument '{}'. Use: /sandbox-toggle [on|off|status|exclude <pattern>]",
                    other
                ))
            }
        };

        match mutate_ui_settings(|s| s.sandbox_mode = Some(enable)) {
            Ok(_) => {
                let state = if enable { "enabled" } else { "disabled" };
                CommandResult::Message(format!(
                    "Sandbox mode {}. Restart recommended for full effect.\n\
                     Use /sandbox-toggle exclude <pattern> to bypass sandboxing for specific commands.",
                    state
                ))
            }
            Err(e) => CommandResult::Error(format!("Failed to save sandbox setting: {}", e)),
        }
    }
}

// ---- /ultrareview --------------------------------------------------------

#[async_trait]
impl SlashCommand for UltrareviewCommand {
    fn name(&self) -> &str {
        "ultrareview"
    }
    fn description(&self) -> &str {
        "Run an exhaustive multi-dimensional code review"
    }
    fn help(&self) -> &str {
        "Usage: /ultrareview [path]\n\n\
         Runs a comprehensive code review that goes beyond /review and\n\
         /security-review. Covers: security (OWASP Top 10), performance,\n\
         maintainability, test coverage, error handling, API design,\n\
         documentation, accessibility, and architectural concerns.\n\
         Each finding is tagged by category and severity."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let target = if args.trim().is_empty() {
            ctx.working_dir.display().to_string()
        } else {
            args.trim().to_string()
        };

        CommandResult::UserMessage(format!(
            "Please perform an **ultra-comprehensive code review** of the code in `{target}`.\n\n\
             This review must go beyond a standard review and cover ALL of the following dimensions:\n\n\
             ## 1. Security (OWASP Top 10 + extras)\n\
             - Injection vulnerabilities (SQL, command, LDAP, XSS, SSTI, CRLF)\n\
             - Broken authentication / session management\n\
             - Sensitive data exposure (secrets, PII, tokens in logs or source)\n\
             - XML/JSON External Entity (XXE) processing\n\
             - Broken access control and privilege escalation paths\n\
             - Security misconfiguration (default creds, open ports, verbose errors)\n\
             - Cross-site scripting (Stored, Reflected, DOM-based)\n\
             - Insecure deserialization\n\
             - Using components with known vulnerabilities (outdated deps)\n\
             - Insufficient logging and monitoring\n\
             - Path traversal and file inclusion\n\
             - Race conditions, TOCTOU, deadlocks\n\
             - Cryptographic weaknesses (weak algorithms, key reuse, bad IV)\n\
             - Supply chain / dependency confusion risks\n\n\
             ## 2. Performance\n\
             - Algorithmic complexity: O(n²) or worse in hot paths\n\
             - Unnecessary allocations, copies, or clones\n\
             - Database N+1 query patterns\n\
             - Missing indexes on frequently queried fields\n\
             - Blocking I/O in async contexts\n\
             - Unbounded loops or recursion\n\
             - Memory leaks or resource leaks (file handles, sockets)\n\
             - Caching opportunities\n\n\
             ## 3. Maintainability & Code Quality\n\
             - Functions / methods exceeding 50 lines\n\
             - Deep nesting (>4 levels)\n\
             - Duplicated logic (DRY violations)\n\
             - Magic numbers and strings without named constants\n\
             - Misleading names (variables, functions, types)\n\
             - Dead code and unused imports\n\
             - Overly complex conditionals\n\
             - Coupling: tight coupling between unrelated modules\n\n\
             ## 4. Error Handling\n\
             - Swallowed errors (empty catch blocks, `unwrap()` without context)\n\
             - Panic-able paths in library code\n\
             - Missing input validation at trust boundaries\n\
             - Unclear error messages that hinder debugging\n\
             - Error type inconsistency across the codebase\n\n\
             ## 5. Test Coverage\n\
             - Missing unit tests for critical logic\n\
             - Missing integration tests for external boundaries\n\
             - Tests with no assertions\n\
             - Tests that are brittle (time-dependent, order-dependent)\n\
             - Missing negative / edge-case tests\n\
             - Mocking strategy concerns\n\n\
             ## 6. API Design\n\
             - Unclear or inconsistent naming conventions\n\
             - Functions with too many parameters (>5)\n\
             - Mutable global state\n\
             - Missing or incorrect use of visibility modifiers\n\
             - Breaking changes risk in public interfaces\n\
             - Lack of builder or fluent patterns where appropriate\n\n\
             ## 7. Documentation\n\
             - Missing doc comments on public items\n\
             - Outdated or misleading comments\n\
             - Undocumented panics, unsafe blocks, or invariants\n\
             - Missing README or high-level architectural overview\n\n\
             ## 8. Architectural Concerns\n\
             - Single Responsibility Principle violations\n\
             - Circular dependencies\n\
             - Missing abstraction layers\n\
             - Hardcoded configuration that should be externalised\n\
             - Observability gaps (missing tracing, metrics, structured logs)\n\n\
             ## Output Format\n\
             For **every** finding, provide:\n\
             - **Category** (from the dimensions above)\n\
             - **Severity**: Critical / High / Medium / Low / Informational\n\
             - **File** and **line number** (if applicable)\n\
             - **Description** of the issue\n\
             - **Impact**: what can go wrong\n\
             - **Recommended fix** with a code snippet where helpful\n\n\
             Start by reading the main source files, dependency manifests, and any CI/CD configuration.\n\
             Group findings by severity (Critical first). Conclude with a prioritised action plan.",
            target = target,
        ))
    }
}

// ---- Named-command slash adapters ----------------------------------------

#[async_trait]
impl SlashCommand for NamedCommandAdapter {
    fn name(&self) -> &str {
        self.slash_name
    }

    fn hidden(&self) -> bool {
        self.slash_hidden
    }

    fn aliases(&self) -> Vec<&str> {
        self.slash_aliases.to_vec()
    }

    fn description(&self) -> &str {
        self.slash_description
    }

    fn help(&self) -> &str {
        self.slash_help
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        execute_named_command_from_slash(self.target_name, args, ctx)
    }
}

// ---- /revert ---------------------------------------------------------------

#[async_trait]
impl SlashCommand for RevertCommand {
    fn name(&self) -> &str {
        "revert"
    }
    fn hidden(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Revert file changes from an assistant turn back to pre-turn state"
    }
    fn help(&self) -> &str {
        "Usage: /revert [<n>|<uuid>] | /revert list | /revert diff [<n>]\n\n\
         Without args: revert the most recent assistant turn.\n\
         With a number n: revert the n-th most recent assistant turn (1 = latest).\n\
         With a uuid: revert the turn whose message id starts with that string.\n\
         `/revert list` shows turns that have recorded file changes.\n\
         `/revert diff` previews the shadow-git diff for a turn without reverting.\n\n\
         This uses the shadow-git snapshot to restore all files that were\n\
         changed during the target turn, and removes that turn (and any later\n\
         turns) from the session transcript.\n\n\
         Examples:\n\
           /revert        — revert last turn\n\
           /revert 2      — revert the second-to-last turn\n\
           /revert abc123 — revert the turn with uuid starting 'abc123'"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let trimmed = args.trim();
        // /revert list and /revert diff absorb the former standalone
        // /checkpoints and /snapshot commands.
        if trimmed == "list" {
            return CheckpointsCommand.execute("", ctx).await;
        }
        if let Some(rest) = trimmed.strip_prefix("diff") {
            return SnapshotDiffCommand.execute(rest.trim(), ctx).await;
        }
        let snap = match claurst_core::snapshot::get_or_create(&ctx.working_dir) {
            Some(s) => s,
            None => {
                return CommandResult::Error(
                    "Snapshot system unavailable (git not found or not a git repo).".into(),
                )
            }
        };

        // Collect assistant messages that have a snapshot patch (newest last).
        let checkpoints: Vec<&claurst_core::types::Message> = ctx
            .messages
            .iter()
            .filter(|m| {
                m.role == claurst_core::types::Role::Assistant && m.snapshot_patch.is_some()
            })
            .collect();

        if checkpoints.is_empty() {
            return CommandResult::Message(
                "No revertible turns found. Run /checkpoints to see recorded file changes.".into(),
            );
        }

        // Select the target turn.
        let args = args.trim();
        let target = if args.is_empty() {
            checkpoints.last().copied()
        } else if let Ok(n) = args.parse::<usize>() {
            if n == 0 || n > checkpoints.len() {
                return CommandResult::Error(format!(
                    "Turn {} out of range (1–{}).",
                    n,
                    checkpoints.len()
                ));
            }
            Some(checkpoints[checkpoints.len() - n])
        } else {
            checkpoints
                .iter()
                .copied()
                .find(|m| m.uuid.as_deref().is_some_and(|u| u.starts_with(args)))
        };

        let target = match target {
            Some(m) => m,
            None => return CommandResult::Error(format!("No turn found matching '{args}'.")),
        };

        // Collect all patches from this turn onward to revert.
        let target_uuid = match target.uuid.clone() {
            Some(u) => u,
            None => return CommandResult::Error("Target turn has no uuid; cannot revert.".into()),
        };

        let patches: Vec<claurst_core::snapshot::Patch> = ctx
            .messages
            .iter()
            .skip_while(|m| m.uuid.as_deref() != Some(&target_uuid))
            .filter_map(|m| m.snapshot_patch.clone())
            .collect();

        if patches.is_empty() {
            return CommandResult::Message("No file changes recorded for that turn.".into());
        }

        // Revert files.
        snap.revert(&patches).await;

        // Truncate the session transcript at the target turn.
        let project_root = claurst_core::git_utils::get_repo_root(&ctx.working_dir)
            .unwrap_or_else(|| ctx.working_dir.clone());
        let path = claurst_core::session_storage::transcript_path(&project_root, &ctx.session_id);
        if path.exists() {
            if let Err(e) = claurst_core::session_storage::truncate_after(&path, &target_uuid).await
            {
                return CommandResult::Error(format!(
                    "Reverted files but could not trim transcript: {e}"
                ));
            }
        }

        let file_count: usize = patches.iter().map(|p| p.files.len()).sum();
        CommandResult::Message(format!(
            "Reverted {} file(s) changed during turn {}. Transcript trimmed.",
            file_count,
            &target_uuid[..target_uuid.len().min(8)],
        ))
    }
}

// ---- /checkpoints ----------------------------------------------------------

#[async_trait]
impl SlashCommand for CheckpointsCommand {
    fn name(&self) -> &str {
        "checkpoints"
    }
    fn description(&self) -> &str {
        "List assistant turns that have recorded file changes"
    }
    fn help(&self) -> &str {
        "Usage: /checkpoints\n\nShows all assistant turns in this session that modified files,\n\
         with file counts.  Use /revert <n> to roll back to a specific turn."
    }

    async fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let checkpoints: Vec<(usize, &claurst_core::types::Message)> = ctx
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                m.role == claurst_core::types::Role::Assistant && m.snapshot_patch.is_some()
            })
            .collect();

        if checkpoints.is_empty() {
            return CommandResult::Message(
                "No file-change checkpoints recorded yet for this session.\n\
                 Checkpoints are created automatically when the assistant modifies files."
                    .into(),
            );
        }

        let total = checkpoints.len();
        let mut lines = vec![format!("{} checkpoint(s):", total)];
        for (rank, (_, msg)) in checkpoints.iter().rev().enumerate() {
            let uuid_short = msg
                .uuid
                .as_deref()
                .map(|u| &u[..u.len().min(8)])
                .unwrap_or("?");
            let file_count = msg.snapshot_patch.as_ref().map_or(0, |p| p.files.len());
            let preview: Vec<String> = msg
                .snapshot_patch
                .as_ref()
                .map(|p| {
                    p.files
                        .iter()
                        .take(3)
                        .map(|f| {
                            f.file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default()
                        })
                        .collect()
                })
                .unwrap_or_default();
            let preview_str = if preview.len() == file_count {
                preview.join(", ")
            } else {
                format!("{}, …", preview.join(", "))
            };
            lines.push(format!(
                "  [{}] {} — {} file(s): {}",
                rank + 1,
                uuid_short,
                file_count,
                preview_str
            ));
        }
        lines.push(String::new());
        lines.push("Use /revert <n> to revert to before turn [n].".into());
        CommandResult::Message(lines.join("\n"))
    }
}

// ---- /snapshot (show snapshot diff for a recorded turn) ------------------

#[async_trait]
impl SlashCommand for SnapshotDiffCommand {
    fn name(&self) -> &str {
        "snapshot"
    }
    fn description(&self) -> &str {
        "Show shadow-git diff of file changes from an assistant turn"
    }
    fn help(&self) -> &str {
        "Usage: /snapshot [<n>|<hash>]\n\n\
         Without args: show unified diff for the most recent assistant turn.\n\
         With a number: show diff for the n-th most recent turn (1 = latest).\n\
         With a hash: show diff against that explicit snapshot tree hash.\n\n\
         See also: /checkpoints (list turns), /revert (roll back files)."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let snap = match claurst_core::snapshot::get_or_create(&ctx.working_dir) {
            Some(s) => s,
            None => {
                return CommandResult::Error(
                    "Snapshot system unavailable (git not found or not a git repo).".into(),
                )
            }
        };

        let args = args.trim();

        // If a raw hash was passed, use it directly.
        let hash = if !args.is_empty()
            && args.chars().all(|c| c.is_ascii_hexdigit())
            && args.len() >= 8
        {
            args.to_string()
        } else {
            // Otherwise find the n-th most recent checkpoint.
            let checkpoints: Vec<&claurst_core::snapshot::Patch> = ctx
                .messages
                .iter()
                .filter_map(|m| {
                    if m.role == claurst_core::types::Role::Assistant {
                        m.snapshot_patch.as_ref()
                    } else {
                        None
                    }
                })
                .collect();

            if checkpoints.is_empty() {
                return CommandResult::Message(
                    "No snapshot checkpoints recorded yet. File changes will appear here after the next assistant turn.".into()
                );
            }

            let idx = if args.is_empty() {
                0
            } else {
                match args.parse::<usize>() {
                    Ok(n) if n >= 1 && n <= checkpoints.len() => n - 1,
                    _ => {
                        return CommandResult::Error(format!(
                            "Turn '{}' out of range (1–{}).",
                            args,
                            checkpoints.len()
                        ))
                    }
                }
            };
            // Reverse so idx=0 is newest.
            let patch = checkpoints[checkpoints.len() - 1 - idx];
            patch.hash.clone()
        };

        let diff = snap.diff(&hash).await;
        if diff.is_empty() {
            CommandResult::Message(format!(
                "No changes since snapshot {}.",
                &hash[..hash.len().min(8)]
            ))
        } else {
            CommandResult::Message(diff)
        }
    }
}

// ---- /providers -------------------------------------------------------------

#[async_trait]
impl SlashCommand for ProvidersCommand {
    fn name(&self) -> &str {
        "providers"
    }
    fn description(&self) -> &str {
        "List available AI providers and their status"
    }
    fn help(&self) -> &str {
        "Usage: /providers\n\nList all providers registered in the model registry with their\nmodel counts, context windows, and pricing information."
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let registry = claurst_api::ModelRegistry::new();
        let all = registry.list_all();

        if all.is_empty() {
            return CommandResult::Message("No providers available.".to_string());
        }

        // Group by provider
        use std::collections::HashMap;
        let mut by_provider: HashMap<String, Vec<_>> = HashMap::new();
        for entry in &all {
            by_provider
                .entry(entry.info.provider_id.to_string())
                .or_default()
                .push(entry);
        }

        // Sort providers alphabetically for stable output
        let mut provider_keys: Vec<String> = by_provider.keys().cloned().collect();
        provider_keys.sort();

        let mut lines = vec!["Available providers:\n".to_string()];
        for provider in &provider_keys {
            let models = &by_provider[provider];
            lines.push(format!(
                "\n{} ({} model{})",
                provider.to_uppercase(),
                models.len(),
                if models.len() == 1 { "" } else { "s" }
            ));
            for m in models.iter().take(3) {
                let cost_str = match (m.cost_input, m.cost_output) {
                    (Some(i), Some(o)) => format!("${:.2}/${:.2} per 1M", i, o),
                    _ => "free/local".to_string(),
                };
                lines.push(format!(
                    "  {} — {}K ctx, {}",
                    m.info.id,
                    m.info.context_window / 1000,
                    cost_str
                ));
            }
            if models.len() > 3 {
                lines.push(format!("  ... and {} more", models.len() - 3));
            }
        }

        CommandResult::Message(lines.join("\n"))
    }
}

// ---- /connect -------------------------------------------------------------

#[async_trait]
impl SlashCommand for ConnectCommand {
    fn name(&self) -> &str {
        "connect"
    }
    fn description(&self) -> &str {
        "Connect an AI provider"
    }
    fn help(&self) -> &str {
        "Usage: /connect\n\nOpens the interactive provider picker dialog.\nSelect a provider to see setup instructions."
    }

    async fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        // This is handled by the TUI interceptor — opening the connect dialog.
        CommandResult::Message("Use the connect dialog to set up a provider.".to_string())
    }
}

// ---- /agent ---------------------------------------------------------------

#[async_trait]
impl SlashCommand for AgentCommand {
    fn name(&self) -> &str {
        "agent"
    }
    fn description(&self) -> &str {
        "Browse, inspect, and manage familiars and sub-agents"
    }
    fn help(&self) -> &str {
        "Usage: /agent [name] | /agent [list|create|edit|delete|reset] [name] | /agent managed [...]\n\n\
         Without arguments, lists all available named agents (in the TUI, opens the agents browser).\n\
         With a name, shows details for that agent.\n\
         /agent list|create|edit|delete|reset manage sub-agent definitions (formerly /agents).\n\
         /agent managed configures the manager-executor system (formerly /managed-agents).\n\n\
         To use an agent, start Coven Code with: --agent <name>"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        use std::collections::HashMap;

        let trimmed = args.trim();
        let (head, rest) = match trimmed.split_once(char::is_whitespace) {
            Some((h, r)) => (h, r.trim()),
            None => (trimmed, ""),
        };
        match head {
            // /agent managed … absorbs the former standalone /managed-agents.
            "managed" => return ManagedAgentsCommand.execute(rest, ctx).await,
            // /agent list|create|… absorbs the former standalone /agents.
            "list" | "create" | "edit" | "delete" | "reset" => {
                return execute_named_command_from_slash("agents", trimmed, ctx);
            }
            _ => {}
        }

        // Merge built-in defaults with user-defined agents (user wins on collision).
        let mut all_agents: HashMap<String, claurst_core::AgentDefinition> =
            claurst_core::default_agents();
        all_agents.extend(ctx.config.agents.clone());

        let agent_name = args.trim();

        if agent_name.is_empty() {
            // List all visible agents.
            let mut keys: Vec<&String> = all_agents
                .iter()
                .filter(|(_, d)| d.visible)
                .map(|(k, _)| k)
                .collect();
            keys.sort();

            let mut output = "Available agents:\n\n".to_string();
            for name in keys {
                let def = &all_agents[name];
                output.push_str(&format!(
                    "  @{} — {}\n    access: {}{}\n",
                    name,
                    def.description.as_deref().unwrap_or(""),
                    def.access,
                    def.max_turns
                        .map(|t| format!(", max_turns: {}", t))
                        .unwrap_or_default(),
                ));
            }
            output.push_str("\nUse --agent <name> when starting Coven Code to activate an agent.");
            CommandResult::Message(output)
        } else if let Some(def) = all_agents.get(agent_name) {
            // Show details for the named agent.
            let mut output = format!("Agent: @{}\n", agent_name);
            if let Some(ref desc) = def.description {
                output.push_str(&format!("Description: {}\n", desc));
            }
            output.push_str(&format!("Access: {}\n", def.access));
            if let Some(ref model) = def.model {
                output.push_str(&format!("Model: {}\n", model));
            }
            if let Some(t) = def.max_turns {
                output.push_str(&format!("Max turns: {}\n", t));
            }
            if let Some(ref color) = def.color {
                output.push_str(&format!("Color: {}\n", color));
            }
            if let Some(ref prompt) = def.prompt {
                output.push_str(&format!("\nSystem prompt prefix:\n  {}\n", prompt));
            }
            output.push_str(&format!("\nTo activate: coven-code --agent {}", agent_name));
            CommandResult::Message(output)
        } else {
            CommandResult::Error(format!(
                "Unknown agent '{}'. Run /agent to see available agents.",
                agent_name
            ))
        }
    }
}

// ---- /familiar -----------------------------------------------------------
//
// Sets the active familiar — updates settings.json AND live-swaps app.config
// so the mascot changes immediately without restart.
//
// Usage:
//   /familiar          — show current familiar + roster
//   /familiar <id>     — switch to a saved familiar (persists to settings.json)
//   /familiar reset    — clear setting

/// Return daemon-declared familiars from `~/.coven/familiars.toml`.
fn current_familiar_roster() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();

    if let Some(daemon) = claurst_core::coven_shared::load_familiars() {
        for f in daemon {
            let desc = format_daemon_familiar(&f);
            out.push((f.id, desc));
        }
    }
    out
}

fn format_daemon_familiar(f: &claurst_core::coven_shared::CovenFamiliar) -> String {
    let emoji = f.emoji.as_deref().unwrap_or("").trim();
    let role = f.role.as_deref().unwrap_or("").trim();
    let desc = f.description.as_deref().unwrap_or("").trim();
    let mut parts: Vec<String> = Vec::new();
    if !emoji.is_empty() {
        parts.push(emoji.to_string());
    }
    let tail = match (role.is_empty(), desc.is_empty()) {
        (true, true) => f.display_name.clone().unwrap_or_else(|| f.id.clone()),
        (false, true) => role.to_string(),
        (true, false) => desc.to_string(),
        (false, false) => format!("{} — {}", role, desc),
    };
    parts.push(tail);
    parts.join(" ")
}

fn current_familiar_display<'a>(
    configured: Option<&'a str>,
    roster: &'a [(String, String)],
) -> &'a str {
    match configured {
        Some(current) if roster.iter().any(|(name, _)| name == current) => current,
        _ => "none",
    }
}

/// Infer a familiar from the system username using the saved Coven roster.
fn infer_familiar_from_env() -> Option<String> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()?;
    let user_lc = user.to_lowercase();
    claurst_core::coven_shared::load_familiars().and_then(|familiars| {
        familiars.into_iter().find_map(|fam| {
            if user_lc.contains(&fam.id.to_lowercase()) {
                return Some(fam.id);
            }
            if let Some(display_name) = &fam.display_name {
                if user_lc.contains(&display_name.to_lowercase()) {
                    return Some(fam.id);
                }
            }
            None
        })
    })
}

#[async_trait]
impl SlashCommand for FamiliarCommand {
    fn name(&self) -> &str {
        "familiar"
    }
    fn description(&self) -> &str {
        "Set your active familiar — changes the TUI mascot live"
    }
    fn aliases(&self) -> Vec<&str> {
        vec!["familiars"]
    }
    fn help(&self) -> &str {
        "Usage: /familiar [name|reset|auto]\n\n\
         Without arguments, shows the current familiar and roster.\n\
         With a saved familiar name,\n\
         switches immediately and persists to settings.json.\n\n\
         /familiar reset  — clear the familiar setting\n\
         /familiar auto   — infer from your system username\n\n\
         Or set \"familiar\" to a saved roster id directly in ~/.coven-code/settings.json."
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let arg = args.trim().to_lowercase();
        let roster = current_familiar_roster();
        let from_daemon = claurst_core::coven_shared::coven_home().is_some()
            && claurst_core::coven_shared::load_familiars().is_some();

        if arg.is_empty() {
            // Show current + roster.
            let current = current_familiar_display(ctx.config.familiar.as_deref(), &roster);
            let auto_hint = infer_familiar_from_env()
                .map(|f| format!(" (auto-detect suggests: {})", f))
                .unwrap_or_default();
            let source = if from_daemon {
                " (roster from ~/.coven/familiars.toml)"
            } else {
                ""
            };
            let mut out = format!(
                "Current familiar: {}{}\n\nRoster{}:\n",
                current, auto_hint, source
            );
            for (name, desc) in &roster {
                let marker = if name == current { " ◀" } else { "" };
                out.push_str(&format!("  {:8} {}{}", name, desc, marker));
                out.push('\n');
            }
            out.push_str("\nUsage: /familiar <name>  to switch");
            return CommandResult::Message(out);
        }

        // Resolve name.
        let target = if arg == "reset" {
            None
        } else if arg == "auto" {
            match infer_familiar_from_env() {
                Some(familiar) => Some(familiar),
                None => {
                    return CommandResult::Error(
                        "Could not infer a familiar from USER/USERNAME and the saved roster."
                            .to_string(),
                    );
                }
            }
        } else if roster.iter().any(|(n, _)| n == &arg) {
            Some(arg.clone())
        } else {
            let names: Vec<&str> = roster.iter().map(|(n, _)| n.as_str()).collect();
            return CommandResult::Error(format!(
                "Unknown familiar '{}'. Choose from: {}",
                arg,
                names.join(", ")
            ));
        };

        // Persist to settings.json.
        let target_clone = target.clone();
        if let Err(e) = save_settings_mutation(|s| s.config.familiar = target_clone) {
            return CommandResult::Error(format!("Failed to save familiar: {}", e));
        }

        // Live-swap config so render picks it up immediately.
        let mut new_config = ctx.config.clone();
        new_config.familiar = target.clone();

        let msg = match &target {
            Some(name) => format!(
                "Familiar set to {}. {} ",
                name,
                roster
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, d)| d.as_str())
                    .unwrap_or("")
            ),
            None => "Familiar reset to none.".to_string(),
        };

        CommandResult::ConfigChangeMessage(new_config, msg)
    }
}

// ---- /managed-agents -----------------------------------------------------

#[async_trait]
impl SlashCommand for ManagedAgentsCommand {
    fn name(&self) -> &str {
        "managed-agents"
    }
    fn hidden(&self) -> bool {
        true // folded into /agent managed; one-release compatibility alias
    }
    fn description(&self) -> &str {
        "Configure and manage the manager-executor agent architecture"
    }
    fn help(&self) -> &str {
        "Usage: /managed-agents [subcommand]\n\n\
         Subcommands:\n\
           (none) | status                        — show current config\n\
           presets                                — list built-in presets\n\
           preset <name>                          — apply a named preset\n\
           setup                                  — show setup instructions\n\
           configure manager-model <value>        — set manager model\n\
           configure executor-model <value>       — set executor model\n\
           configure executor-turns <n>           — set executor max turns\n\
           configure concurrent <n>               — set max concurrent executors\n\
           configure isolation on|off             — set executor isolation\n\
           configure budget-split shared|percentage:<pct>|fixed:<mgr>:<exe>\n\
           budget <amount>                        — set total budget in USD (0 to clear)\n\
           enable                                 — enable managed agents\n\
           disable                                — disable managed agents\n\
           reset                                  — remove config entirely"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        use claurst_core::{builtin_managed_agent_presets, BudgetSplitPolicy, ManagedAgentConfig};

        let args = args.trim();

        // Helper to format current config as status string
        fn format_status(cfg: &Option<ManagedAgentConfig>) -> String {
            match cfg {
                None => {
                    "Managed Agents: NOT CONFIGURED\n\nRun /managed-agents setup to get started."
                        .to_string()
                }
                Some(c) => {
                    let state = if c.enabled {
                        "ACTIVE"
                    } else {
                        "CONFIGURED but inactive"
                    };
                    let budget_str = match c.total_budget_usd {
                        Some(b) => format!("${:.2} total", b),
                        None => "no cap".to_string(),
                    };
                    let split_str = match &c.budget_split {
                        BudgetSplitPolicy::SharedPool => "shared pool".to_string(),
                        BudgetSplitPolicy::Percentage { manager_pct } => {
                            format!("{}% manager", manager_pct)
                        }
                        BudgetSplitPolicy::FixedCaps {
                            manager_usd,
                            executor_usd,
                        } => {
                            format!("${:.2} mgr / ${:.2} exe", manager_usd, executor_usd)
                        }
                    };
                    let preset = c.preset_name.as_deref().unwrap_or("custom");
                    let isolation = if c.executor_isolation { "on" } else { "off" };
                    format!(
                        "Managed Agents: {}\n  Manager:    {}\n  Executor:   {}\n  Preset:     {}\n  Budget:     {}  |  split: {}\n  Exec limits: {} turns, {} concurrent, isolation: {}\n\nRun /managed-agents <subcommand> — presets | setup | configure | enable | disable | budget | reset",
                        state,
                        c.manager_model,
                        c.executor_model,
                        preset,
                        budget_str,
                        split_str,
                        c.executor_max_turns,
                        c.max_concurrent_executors,
                        isolation,
                    )
                }
            }
        }

        if args.is_empty() || args == "status" {
            return CommandResult::Message(format_status(&ctx.config.managed_agents));
        }

        if args == "presets" {
            let presets = builtin_managed_agent_presets();
            let mut out = "Built-in managed agent presets:\n\n".to_string();
            for p in &presets {
                out.push_str(&format!(
                    "  {:<28} — {}\n    Manager:  {}\n    Executor: {}\n\n",
                    p.name, p.description, p.manager_model, p.executor_model
                ));
            }
            out.push_str("Use: /managed-agents preset <name> to apply a preset.");
            return CommandResult::Message(out);
        }

        if args == "setup" {
            let presets = builtin_managed_agent_presets();
            let mut out = "Managed Agents Setup\n\nQuickstart — apply a preset:\n\n".to_string();
            for p in &presets {
                out.push_str(&format!(
                    "  /managed-agents preset {}\n    {}\n\n",
                    p.name, p.description
                ));
            }
            out.push_str("\nOr configure manually:\n  /managed-agents configure manager-model <provider/model>\n  /managed-agents configure executor-model <provider/model>\n  /managed-agents enable\n\nModel format: provider/model (e.g. anthropic/claude-opus-4-6, openai/gpt-4o, google/gemini-2.5-flash)\nAny provider registered in the ProviderRegistry can be used.");
            return CommandResult::Message(out);
        }

        if let Some(preset_name) = args.strip_prefix("preset ").map(str::trim) {
            let presets = builtin_managed_agent_presets();
            let found = presets
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(preset_name));
            match found {
                None => {
                    let names: Vec<&str> = presets.iter().map(|p| p.name).collect();
                    return CommandResult::Error(format!(
                        "Unknown preset '{}'. Available: {}",
                        preset_name,
                        names.join(", ")
                    ));
                }
                Some(p) => {
                    let new_cfg = ManagedAgentConfig {
                        enabled: true,
                        manager_model: p.manager_model.to_string(),
                        executor_model: p.executor_model.to_string(),
                        executor_max_turns: p.executor_max_turns,
                        max_concurrent_executors: p.max_concurrent_executors,
                        budget_split: BudgetSplitPolicy::SharedPool,
                        total_budget_usd: None,
                        preset_name: Some(p.name.to_string()),
                        executor_isolation: false,
                    };
                    let name = p.name.to_string();
                    if let Err(e) = save_settings_mutation(|settings| {
                        settings.managed_agents = Some(new_cfg.clone());
                        settings.config.managed_agents = Some(new_cfg.clone());
                    }) {
                        return CommandResult::Error(format!("Failed to save: {}", e));
                    }
                    let mut new_config = ctx.config.clone();
                    new_config.managed_agents = Some(new_cfg);
                    return CommandResult::ConfigChangeMessage(
                        new_config,
                        format!("Applied preset '{}'. Managed agents ENABLED.", name),
                    );
                }
            }
        }

        if let Some(rest) = args.strip_prefix("configure ").map(str::trim) {
            let mut cfg = ctx
                .config
                .managed_agents
                .clone()
                .unwrap_or(ManagedAgentConfig {
                    enabled: false,
                    manager_model: String::new(),
                    executor_model: String::new(),
                    executor_max_turns: 10,
                    max_concurrent_executors: 4,
                    budget_split: BudgetSplitPolicy::SharedPool,
                    total_budget_usd: None,
                    preset_name: None,
                    executor_isolation: false,
                });

            if let Some(val) = rest.strip_prefix("manager-model ").map(str::trim) {
                cfg.manager_model = val.to_string();
                cfg.preset_name = None;
            } else if let Some(val) = rest.strip_prefix("executor-model ").map(str::trim) {
                cfg.executor_model = val.to_string();
                cfg.preset_name = None;
            } else if let Some(val) = rest.strip_prefix("executor-turns ").map(str::trim) {
                match val.parse::<u32>() {
                    Ok(n) => cfg.executor_max_turns = n,
                    Err(_) => return CommandResult::Error(format!("Invalid number: '{}'", val)),
                }
            } else if let Some(val) = rest.strip_prefix("concurrent ").map(str::trim) {
                match val.parse::<u32>() {
                    Ok(n) => cfg.max_concurrent_executors = n,
                    Err(_) => return CommandResult::Error(format!("Invalid number: '{}'", val)),
                }
            } else if let Some(val) = rest.strip_prefix("isolation ").map(str::trim) {
                match val {
                    "on" => cfg.executor_isolation = true,
                    "off" => cfg.executor_isolation = false,
                    _ => return CommandResult::Error("Use 'on' or 'off'".to_string()),
                }
            } else if let Some(val) = rest.strip_prefix("budget-split ").map(str::trim) {
                if val == "shared" {
                    cfg.budget_split = BudgetSplitPolicy::SharedPool;
                } else if let Some(pct_str) = val.strip_prefix("percentage:") {
                    match pct_str.parse::<u8>() {
                        Ok(pct) => {
                            cfg.budget_split = BudgetSplitPolicy::Percentage { manager_pct: pct }
                        }
                        Err(_) => {
                            return CommandResult::Error(format!(
                                "Invalid percentage: '{}'",
                                pct_str
                            ))
                        }
                    }
                } else if let Some(caps_str) = val.strip_prefix("fixed:") {
                    let parts: Vec<&str> = caps_str.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        match (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
                            (Ok(m), Ok(e)) => {
                                cfg.budget_split = BudgetSplitPolicy::FixedCaps {
                                    manager_usd: m,
                                    executor_usd: e,
                                }
                            }
                            _ => {
                                return CommandResult::Error(
                                    "Invalid fixed caps format. Use fixed:<manager>:<executor>"
                                        .to_string(),
                                )
                            }
                        }
                    } else {
                        return CommandResult::Error(
                            "Invalid fixed caps format. Use fixed:<manager>:<executor>".to_string(),
                        );
                    }
                } else {
                    return CommandResult::Error(
                        "Use: shared | percentage:<pct> | fixed:<manager>:<executor>".to_string(),
                    );
                }
            } else {
                return CommandResult::Error(format!(
                    "Unknown configure option: '{}'\nOptions: manager-model, executor-model, executor-turns, concurrent, isolation, budget-split",
                    rest
                ));
            }

            if let Err(e) = save_settings_mutation(|settings| {
                settings.managed_agents = Some(cfg.clone());
                settings.config.managed_agents = Some(cfg.clone());
            }) {
                return CommandResult::Error(format!("Failed to save: {}", e));
            }
            let mut new_config = ctx.config.clone();
            new_config.managed_agents = Some(cfg);
            return CommandResult::ConfigChangeMessage(
                new_config,
                "Managed agents configuration updated.".to_string(),
            );
        }

        if let Some(amount_str) = args.strip_prefix("budget ").map(str::trim) {
            match amount_str.parse::<f64>() {
                Err(_) => return CommandResult::Error(format!("Invalid amount: '{}'", amount_str)),
                Ok(amount) => {
                    let mut cfg = match ctx.config.managed_agents.clone() {
                        None => {
                            return CommandResult::Error(
                                "No managed agents config. Run /managed-agents setup first."
                                    .to_string(),
                            )
                        }
                        Some(c) => c,
                    };
                    cfg.total_budget_usd = if amount <= 0.0 { None } else { Some(amount) };
                    if let Err(e) = save_settings_mutation(|settings| {
                        settings.managed_agents = Some(cfg.clone());
                        settings.config.managed_agents = Some(cfg.clone());
                    }) {
                        return CommandResult::Error(format!("Failed to save: {}", e));
                    }
                    let mut new_config = ctx.config.clone();
                    let msg = if amount <= 0.0 {
                        "Budget cap cleared.".to_string()
                    } else {
                        format!("Budget set to ${:.2}.", amount)
                    };
                    new_config.managed_agents = Some(cfg);
                    return CommandResult::ConfigChangeMessage(new_config, msg);
                }
            }
        }

        if args == "enable" {
            let mut cfg = match ctx.config.managed_agents.clone() {
                None => {
                    return CommandResult::Error(
                        "No managed agents config. Run /managed-agents setup first.".to_string(),
                    )
                }
                Some(c) => c,
            };
            if cfg.manager_model.is_empty() || cfg.executor_model.is_empty() {
                return CommandResult::Error(
                    "manager_model and executor_model must be set before enabling.".to_string(),
                );
            }
            cfg.enabled = true;
            if let Err(e) = save_settings_mutation(|settings| {
                settings.managed_agents = Some(cfg.clone());
                settings.config.managed_agents = Some(cfg.clone());
            }) {
                return CommandResult::Error(format!("Failed to save: {}", e));
            }
            let mut new_config = ctx.config.clone();
            new_config.managed_agents = Some(cfg);
            return CommandResult::ConfigChangeMessage(
                new_config,
                "Managed agents ENABLED.".to_string(),
            );
        }

        if args == "disable" {
            let mut cfg = match ctx.config.managed_agents.clone() {
                None => return CommandResult::Error("No managed agents config.".to_string()),
                Some(c) => c,
            };
            cfg.enabled = false;
            if let Err(e) = save_settings_mutation(|settings| {
                settings.managed_agents = Some(cfg.clone());
                settings.config.managed_agents = Some(cfg.clone());
            }) {
                return CommandResult::Error(format!("Failed to save: {}", e));
            }
            let mut new_config = ctx.config.clone();
            new_config.managed_agents = Some(cfg);
            return CommandResult::ConfigChangeMessage(
                new_config,
                "Managed agents disabled.".to_string(),
            );
        }

        if args == "reset" {
            if let Err(e) = save_settings_mutation(|settings| {
                settings.managed_agents = None;
                settings.config.managed_agents = None;
            }) {
                return CommandResult::Error(format!("Failed to save: {}", e));
            }
            let mut new_config = ctx.config.clone();
            new_config.managed_agents = None;
            return CommandResult::ConfigChangeMessage(
                new_config,
                "Managed agents configuration removed.".to_string(),
            );
        }

        CommandResult::Error(format!(
            "Unknown subcommand: '{}'\nRun /managed-agents to see usage.",
            args
        ))
    }
}

// ---------------------------------------------------------------------------
// /coven — substrate (daemon) control surface
// ---------------------------------------------------------------------------
//
// The `/coven` command lets users drive the local Coven daemon
// (`~/.coven/coven.sock`) from inside Coven Code without leaving the TUI.
// Read-only operations go through the in-process `DaemonClient`; mutating
// rituals that the daemon does not currently expose over HTTP (archive,
// summon, sacrifice, doctor, patch, pc, daemon lifecycle) shell out to the
// `coven` binary, matching the exact verbs `coven-cli`'s slash TUI uses.

fn coven_help_text() -> &'static str {
    "Usage: /coven <subcommand> [args]\n\
     \n\
     Status & lifecycle\n\
       /coven                          Daemon health + active session count\n\
       /coven status                   Same as /coven\n\
       /coven capabilities             Daemon capability catalog (harness manifests)\n\
       /coven familiars                List familiar statuses\n\
       /coven doctor                   Detect installed harness CLIs\n\
       /coven daemon start|status|stop|restart\n\
     \n\
     Sessions (read-only)\n\
       /coven sessions [--all]         List active (or all) daemon sessions\n\
       /coven info <session-id>        Show full session record\n\
       /coven log <session-id>         Fetch the redacted log preview\n\
       /coven events <id> [--after N] [--limit M]   Stream a page of session events\n\
     \n\
     Sessions (live control)\n\
       /coven run <harness> <prompt>   Launch a new harness session\n\
       /coven send <session-id> <txt>  Forward input to a live session\n\
       /coven kill <session-id>        Terminate a live session\n\
       /coven attach <session-id>      Replay/follow a running session\n\
     \n\
     Session rituals\n\
       /coven summon <session-id>      Restore an archived session\n\
       /coven archive <session-id>     Archive a non-running session\n\
       /coven sacrifice <session-id>   Permanently delete a session\n\
     \n\
     Control plane\n\
       /coven actions <action> [json]  POST a control-plane action\n\
     \n\
     Coven Calls (delegation ledger)\n\
       /coven calls [--limit N]        Read ~/.coven/cave-coven-calls.json\n\
     \n\
     Parallel-work protocol\n\
       /coven claim acquire|release|status|heartbeat|canary [args]\n\
       /coven hooks-install            Install git hooks for the claim protocol\n\
     \n\
     Harness adapters & maintenance\n\
       /coven adapter list|doctor [id]\n\
       /coven logs prune [--days N]\n\
       /coven wt <branch>|--list|--doctor|--prune-merged|--prune-stale [DAYS]\n\
     \n\
     Workflows\n\
       /coven patch [name] [issue]     Open the OpenClaw repair flow\n\
       /coven pc [status|top|disk|...] macOS system diagnostics\n\
     \n\
     /coven help                       Show this help"
}

/// Expected API version negotiated against `coven.daemon.v1`. Bump this
/// when coven-code is ready to talk to a newer daemon contract.
const COVEN_DAEMON_EXPECTED_API: &str = "coven.daemon.v1";

/// Render a [`DaemonError`] into a user-facing string. Offline errors are
/// rewritten to suggest `/coven daemon start`; other errors surface the
/// structured code so users can distinguish `session_not_live` from
/// `session_not_found` etc.
fn coven_render_error(err: &claurst_core::coven_shared::DaemonError) -> String {
    if err.is_offline() {
        return "Coven daemon offline. Try `/coven daemon start`.".to_string();
    }
    err.to_string()
}

fn coven_format_health(client: &claurst_core::coven_shared::DaemonClient) -> String {
    let mut out = String::new();
    if !client.is_online() {
        out.push_str("Coven daemon: offline\n");
        out.push_str("  Start it with `/coven daemon start` (or run `coven daemon start`).\n");
        return out;
    }
    match client.health() {
        Ok(h) => {
            out.push_str(&format!(
                "Coven daemon: online ({api}, coven {ver})\n",
                api = if h.api_version.is_empty() {
                    "unknown api"
                } else {
                    h.api_version.as_str()
                },
                ver = if h.coven_version.is_empty() {
                    "?"
                } else {
                    h.coven_version.as_str()
                },
            ));
            if !h.api_version.is_empty() && h.api_version != COVEN_DAEMON_EXPECTED_API {
                out.push_str(&format!(
                    "  warning: client expects {} — features may misbehave on this daemon.\n",
                    COVEN_DAEMON_EXPECTED_API
                ));
            }
            if let Some(pid) = h.pid {
                out.push_str(&format!("  pid: {pid}\n"));
            }
            if let Some(sock) = h.socket.as_deref() {
                out.push_str(&format!("  socket: {sock}\n"));
            }
            if let Some(started) = h.started_at.as_deref() {
                out.push_str(&format!("  started: {started}\n"));
            }
        }
        Err(e) => {
            out.push_str(&format!("Coven daemon: online (health unavailable: {e})\n"));
        }
    }
    match client.active_sessions() {
        Ok(active) => out.push_str(&format!("  active sessions: {}\n", active.len())),
        Err(e) => out.push_str(&format!("  active sessions: ? ({e})\n")),
    }
    out
}

fn coven_format_sessions(
    sessions: &[claurst_core::coven_shared::DaemonSession],
    include_archived: bool,
) -> String {
    if sessions.is_empty() {
        return if include_archived {
            "No sessions found in the daemon ledger.".to_string()
        } else {
            "No active daemon sessions. Launch one with `/coven run <harness> <prompt>`."
                .to_string()
        };
    }
    let mut out = String::new();
    out.push_str(&format!(
        "{:<36}  {:<8}  {:<10}  {}\n",
        "id", "harness", "status", "title"
    ));
    out.push_str(&format!("{}\n", "-".repeat(78)));
    for s in sessions {
        let title = if s.title.is_empty() {
            "(untitled)"
        } else {
            s.title.as_str()
        };
        let status = if s.archived_at.is_some() {
            "archived"
        } else {
            s.status.as_str()
        };
        out.push_str(&format!(
            "{:<36}  {:<8}  {:<10}  {}\n",
            s.id, s.harness, status, title
        ));
    }
    out
}

/// Read and pretty-print the Coven Calls delegation ledger written by
/// `coven-cli/src/coven_calls.rs`.  Returns a user-facing message —
/// either a rendered table of the most recent `limit` calls, or an
/// explanatory string when the file is missing/empty/unparsable.
///
/// The on-disk shape (camelCase JSON, ledger version 1) is:
/// `{ "version": 1, "calls": [ { id, callerFamiliarId, calleeFamiliarId,
/// request, status, createdAt, endedAt?, sessionId?, artifact? } ] }`.
fn coven_read_calls_ledger(limit: usize) -> String {
    let Some(home) = claurst_core::coven_shared::coven_home() else {
        return "Could not determine ~/.coven; is the daemon installed?".to_string();
    };
    let path = home.join("cave-coven-calls.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return format!(
                "No delegation ledger yet at {}. Familiars only write here once they cast a Coven Call.",
                path.display()
            );
        }
        Err(e) => return format!("Could not read {}: {e}", path.display()),
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return format!("Could not parse {}: {e}", path.display()),
    };
    let calls = value
        .get("calls")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if calls.is_empty() {
        return "Delegation ledger is empty.".to_string();
    }
    let total = calls.len();
    let take = limit.max(1).min(total);
    let mut out = String::new();
    out.push_str(&format!(
        "{:<8}  {:<8}  {:<10}  {:<20}  request\n",
        "caller", "callee", "status", "createdAt"
    ));
    out.push_str(&format!("{}\n", "-".repeat(78)));
    for call in calls.iter().rev().take(take) {
        let caller = call
            .get("callerFamiliarId")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let callee = call
            .get("calleeFamiliarId")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let status = call.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let created = call
            .get("createdAt")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let request = call.get("request").and_then(|v| v.as_str()).unwrap_or("");
        let trimmed = if request.chars().count() > 36 {
            let s: String = request.chars().take(36).collect();
            format!("{s}…")
        } else {
            request.to_string()
        };
        out.push_str(&format!(
            "{:<8}  {:<8}  {:<10}  {:<20}  {}\n",
            caller, callee, status, created, trimmed
        ));
    }
    if take < total {
        out.push_str(&format!(
            "\n…showing {take} most recent of {total} calls. Use `/coven calls --limit N` to widen.\n"
        ));
    }
    out
}

/// Spawn the `coven` binary with the given argv tail and capture stdout/stderr.
/// Returns the combined human-readable output (or an explanatory error if the
/// binary is missing).
fn coven_shell_out(args: &[&str]) -> String {
    let mut cmd = std::process::Command::new("coven");
    cmd.args(args);
    match cmd.output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let mut buf = String::new();
            if !stdout.is_empty() {
                buf.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str(&stderr);
            }
            if !out.status.success() {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str(&format!(
                    "(coven exited with status {})",
                    out.status.code().unwrap_or(-1)
                ));
            }
            if buf.is_empty() {
                "(no output)".to_string()
            } else {
                buf
            }
        }
        Err(e) => format!(
            "Could not launch the `coven` binary: {e}\n\
             Install it from https://github.com/OpenCoven/coven or `npm install -g @opencoven/cli`."
        ),
    }
}

#[async_trait]
impl SlashCommand for CovenCommand {
    fn name(&self) -> &str {
        "coven"
    }
    fn description(&self) -> &str {
        "Drive the local Coven daemon (sessions, harness runs, rituals)"
    }
    fn help(&self) -> &str {
        coven_help_text()
    }
    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        use claurst_core::coven_shared::DaemonClient;

        let trimmed = args.trim();
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let sub = parts.next().unwrap_or("").trim();
        let rest = parts.next().unwrap_or("").trim();

        // No subcommand → status. Matches `coven` (default) UX in coven-cli.
        if sub.is_empty() || sub == "status" {
            match DaemonClient::new() {
                Some(client) => return CommandResult::Message(coven_format_health(&client)),
                None => {
                    return CommandResult::Message(
                        "Coven daemon socket not found at ~/.coven/coven.sock.\n\
                         Start it with `/coven daemon start` (or `coven daemon start`)."
                            .to_string(),
                    );
                }
            }
        }

        if sub == "help" || sub == "--help" || sub == "-h" {
            return CommandResult::Message(coven_help_text().to_string());
        }

        match sub {
            // Read-only listings — use the in-process client.
            "sessions" => {
                let include_archived = rest.split_whitespace().any(|t| t == "--all" || t == "-a");
                let Some(client) = DaemonClient::new() else {
                    return CommandResult::Message(
                        "Coven daemon offline. Try `/coven daemon start`.".to_string(),
                    );
                };
                let result = if include_archived {
                    client.all_sessions()
                } else {
                    client.active_sessions()
                };
                match result {
                    Ok(sessions) => {
                        CommandResult::Message(coven_format_sessions(&sessions, include_archived))
                    }
                    Err(e) => CommandResult::Error(coven_render_error(&e)),
                }
            }

            // Native-API verbs. The daemon's HTTP contract exposes these
            // directly, so we hit it in-process and bypass the shell-out
            // path entirely.
            "familiars" => {
                let Some(client) = DaemonClient::new() else {
                    return CommandResult::Message(
                        "Coven daemon offline. Try `/coven daemon start`.".to_string(),
                    );
                };
                let statuses = match client.familiar_statuses() {
                    Ok(s) => s,
                    Err(e) => return CommandResult::Error(coven_render_error(&e)),
                };
                if statuses.is_empty() {
                    return CommandResult::Message(
                        "No familiars reported by the daemon.".to_string(),
                    );
                }
                let mut out = String::new();
                out.push_str(&format!(
                    "{:<12}  {:<14}  {:<8}  active  freshness\n",
                    "id", "display", "status"
                ));
                out.push_str(&format!("{}\n", "-".repeat(64)));
                for f in &statuses {
                    out.push_str(&format!(
                        "{:<12}  {:<14}  {:<8}  {:>6}  {}\n",
                        f.id, f.display_name, f.status, f.active_sessions, f.memory_freshness,
                    ));
                }
                CommandResult::Message(out)
            }
            "log" => {
                if rest.is_empty() {
                    return CommandResult::Error("Usage: /coven log <session-id>".to_string());
                }
                let Some(client) = DaemonClient::new() else {
                    return CommandResult::Message(
                        "Coven daemon offline. Try `/coven daemon start`.".to_string(),
                    );
                };
                match client.session_log(rest) {
                    Ok(log) if !log.is_empty() => CommandResult::Message(log),
                    Ok(_) => CommandResult::Message("(session log is empty)".to_string()),
                    Err(e) => CommandResult::Error(coven_render_error(&e)),
                }
            }
            "send" => {
                let mut split = rest.splitn(2, char::is_whitespace);
                let id = split.next().unwrap_or("");
                let input = split.next().unwrap_or("").trim();
                if id.is_empty() || input.is_empty() {
                    return CommandResult::Error(
                        "Usage: /coven send <session-id> <text>".to_string(),
                    );
                }
                let Some(client) = DaemonClient::new() else {
                    return CommandResult::Message(
                        "Coven daemon offline. Try `/coven daemon start`.".to_string(),
                    );
                };
                match client.send_input(id, input) {
                    Ok(()) => CommandResult::Message(format!("Sent input to {id}.")),
                    Err(e) => CommandResult::Error(coven_render_error(&e)),
                }
            }
            "kill" => {
                if rest.is_empty() {
                    return CommandResult::Error("Usage: /coven kill <session-id>".to_string());
                }
                let Some(client) = DaemonClient::new() else {
                    return CommandResult::Message(
                        "Coven daemon offline. Try `/coven daemon start`.".to_string(),
                    );
                };
                match client.kill_session(rest) {
                    Ok(()) => CommandResult::Message(format!("Killed session {rest}.")),
                    Err(e) => CommandResult::Error(coven_render_error(&e)),
                }
            }
            "capabilities" => {
                let Some(client) = DaemonClient::new() else {
                    return CommandResult::Message(
                        "Coven daemon offline. Try `/coven daemon start`.".to_string(),
                    );
                };
                match client.capabilities() {
                    Ok(body) => {
                        let pretty = serde_json::from_str::<serde_json::Value>(&body)
                            .ok()
                            .and_then(|v| serde_json::to_string_pretty(&v).ok())
                            .unwrap_or(body);
                        CommandResult::Message(pretty)
                    }
                    Err(e) => CommandResult::Error(coven_render_error(&e)),
                }
            }
            "info" => {
                if rest.is_empty() {
                    return CommandResult::Error("Usage: /coven info <session-id>".to_string());
                }
                let Some(client) = DaemonClient::new() else {
                    return CommandResult::Message(
                        "Coven daemon offline. Try `/coven daemon start`.".to_string(),
                    );
                };
                match client.get_session(rest) {
                    Ok(s) => {
                        let archived = s
                            .archived_at
                            .as_deref()
                            .map(|t| format!("\n  archived: {t}"))
                            .unwrap_or_default();
                        let title = if s.title.is_empty() {
                            "(untitled)"
                        } else {
                            s.title.as_str()
                        };
                        CommandResult::Message(format!(
                            "Session {id}\n  harness: {h}\n  status: {st}\n  project: {root}\n  title: {t}{a}",
                            id = s.id,
                            h = s.harness,
                            st = s.status,
                            root = s.project_root,
                            t = title,
                            a = archived,
                        ))
                    }
                    Err(e) => CommandResult::Error(coven_render_error(&e)),
                }
            }
            "events" => {
                if rest.is_empty() {
                    return CommandResult::Error(
                        "Usage: /coven events <session-id> [--after N] [--limit M]".to_string(),
                    );
                }
                let mut tokens = rest.split_whitespace();
                let id = tokens.next().unwrap_or("");
                let mut after: Option<i64> = None;
                // Default page size so a user doesn't accidentally fetch
                // every event in a long session. `/coven events <id>
                // --limit 0` opts back into the daemon-default (unset).
                let mut limit: Option<u32> = Some(50);
                while let Some(tok) = tokens.next() {
                    match tok {
                        "--after" | "--after-seq" => {
                            if let Some(v) = tokens.next() {
                                after = v.parse().ok();
                            }
                        }
                        "--limit" => {
                            if let Some(v) = tokens.next() {
                                match v.parse::<u32>() {
                                    Ok(0) => limit = None,
                                    Ok(n) => limit = Some(n),
                                    Err(_) => {}
                                }
                            }
                        }
                        _ => {
                            return CommandResult::Error(format!(
                                "Unknown flag '{tok}'.\nUsage: /coven events <session-id> [--after N] [--limit M]"
                            ));
                        }
                    }
                }
                let Some(client) = DaemonClient::new() else {
                    return CommandResult::Message(
                        "Coven daemon offline. Try `/coven daemon start`.".to_string(),
                    );
                };
                match client.session_events(id, after, limit) {
                    Ok(page) => {
                        if page.events.is_empty() {
                            return CommandResult::Message("End of events stream.".to_string());
                        }
                        let mut out = String::new();
                        out.push_str(&format!(
                            "{:<5}  {:<16}  {:<20}  {}\n",
                            "seq", "kind", "createdAt", "payload (truncated)"
                        ));
                        out.push_str(&format!("{}\n", "-".repeat(78)));
                        for ev in &page.events {
                            let seq = ev
                                .seq
                                .map(|n| n.to_string())
                                .unwrap_or_else(|| "?".to_string());
                            let payload = if ev.payload_json.len() > 36 {
                                format!("{}…", &ev.payload_json[..36])
                            } else {
                                ev.payload_json.clone()
                            };
                            out.push_str(&format!(
                                "{:<5}  {:<16}  {:<20}  {}\n",
                                seq, ev.kind, ev.created_at, payload
                            ));
                        }
                        if page.has_more {
                            if let Some(next) = page.next_after_seq {
                                out.push_str(&format!(
                                    "\nMore events available — `/coven events {id} --after {next}`."
                                ));
                            } else {
                                out.push_str("\nMore events available.");
                            }
                        } else {
                            out.push_str("\nEnd of events stream.");
                        }
                        CommandResult::Message(out)
                    }
                    Err(e) => CommandResult::Error(coven_render_error(&e)),
                }
            }
            "actions" => {
                if rest.is_empty() {
                    return CommandResult::Error(
                        "Usage: /coven actions <action> [json-args]".to_string(),
                    );
                }
                let mut split = rest.splitn(2, char::is_whitespace);
                let action = split.next().unwrap_or("").trim();
                let args_raw = split.next().unwrap_or("").trim();
                let args = if args_raw.is_empty() {
                    None
                } else {
                    match serde_json::from_str::<serde_json::Value>(args_raw) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            return CommandResult::Error(format!(
                                "Could not parse args as JSON: {e}\nUsage: /coven actions <action> [json-args]"
                            ));
                        }
                    }
                };
                let Some(client) = DaemonClient::new() else {
                    return CommandResult::Message(
                        "Coven daemon offline. Try `/coven daemon start`.".to_string(),
                    );
                };
                match client.control_action(action, args) {
                    Ok(res) => {
                        let mut out = format!(
                            "action: {a}\nstatus: {st}\naccepted: {acc}\nok: {ok}",
                            a = res.action,
                            st = res.status,
                            acc = res.accepted,
                            ok = res.ok,
                        );
                        if let Some(r) = res.reason {
                            out.push_str(&format!("\nreason: {r}"));
                        }
                        CommandResult::Message(out)
                    }
                    Err(e) => CommandResult::Error(coven_render_error(&e)),
                }
            }

            // Verbs that shell out — the daemon's HTTP API does not expose
            // these yet, but the `coven` CLI implements them against the same
            // store. We keep the verb names identical to coven-cli/src/tui/shell.rs.
            "run" => {
                if rest.is_empty() {
                    return CommandResult::Error(
                        "Usage: /coven run <harness> <prompt>".to_string(),
                    );
                }
                let mut harness_split = rest.splitn(2, char::is_whitespace);
                let harness = harness_split.next().unwrap_or("");
                let prompt = harness_split.next().unwrap_or("").trim();
                if harness.is_empty() || prompt.is_empty() {
                    return CommandResult::Error(
                        "Usage: /coven run <harness> <prompt>".to_string(),
                    );
                }
                CommandResult::Message(coven_shell_out(&["run", harness, prompt]))
            }
            "attach" => {
                if rest.is_empty() {
                    return CommandResult::Error("Usage: /coven attach <session-id>".to_string());
                }
                CommandResult::Message(coven_shell_out(&["attach", rest]))
            }
            "summon" => {
                if rest.is_empty() {
                    return CommandResult::Error("Usage: /coven summon <session-id>".to_string());
                }
                CommandResult::Message(coven_shell_out(&["summon", rest]))
            }
            "archive" => {
                if rest.is_empty() {
                    return CommandResult::Error("Usage: /coven archive <session-id>".to_string());
                }
                CommandResult::Message(coven_shell_out(&["archive", rest]))
            }
            "sacrifice" => {
                if rest.is_empty() {
                    return CommandResult::Error(
                        "Usage: /coven sacrifice <session-id>\n\
                         Sacrifice is irreversible; the underlying CLI requires --yes."
                            .to_string(),
                    );
                }
                // The coven CLI requires --yes for sacrifice; injecting it
                // mirrors the slash-TUI behavior in coven-cli where users
                // confirm by typing the slash command.
                CommandResult::Message(coven_shell_out(&["sacrifice", rest, "--yes"]))
            }
            "doctor" => CommandResult::Message(coven_shell_out(&["doctor"])),
            "daemon" => {
                let action = rest.split_whitespace().next().unwrap_or("status");
                if !matches!(action, "start" | "status" | "stop" | "restart") {
                    return CommandResult::Error(format!(
                        "Unknown daemon action: '{action}'.\nUse start | status | stop | restart."
                    ));
                }
                CommandResult::Message(coven_shell_out(&["daemon", action]))
            }
            "patch" => {
                // Pass the remainder through verbatim — `coven patch` takes a
                // flexible mix of positional name + issue text + flags.
                let mut argv: Vec<&str> = vec!["patch"];
                argv.extend(rest.split_whitespace());
                CommandResult::Message(coven_shell_out(&argv))
            }
            "pc" => {
                let mut argv: Vec<&str> = vec!["pc"];
                argv.extend(rest.split_whitespace());
                CommandResult::Message(coven_shell_out(&argv))
            }

            // Coven-specific integrations.
            "calls" => {
                // Native FS read of the delegation ledger written by
                // coven-cli/src/coven_calls.rs.  The shape is documented in
                // that file and in coven-cave's lib/coven-calls-types.ts.
                let mut limit: usize = 20;
                let mut tokens = rest.split_whitespace();
                while let Some(tok) = tokens.next() {
                    if tok == "--limit" {
                        if let Some(v) = tokens.next() {
                            limit = v.parse().unwrap_or(limit);
                        }
                    }
                }
                CommandResult::Message(coven_read_calls_ledger(limit))
            }
            "claim" => {
                if rest.is_empty() {
                    return CommandResult::Error(
                        "Usage: /coven claim acquire|release|status|heartbeat|canary [args]"
                            .to_string(),
                    );
                }
                let mut argv: Vec<&str> = vec!["claim"];
                argv.extend(rest.split_whitespace());
                CommandResult::Message(coven_shell_out(&argv))
            }
            "hooks-install" | "hooks" => {
                // Both names map to `coven hooks install`. We accept "hooks"
                // as an alias to mirror the coven-cli subcommand shape.
                CommandResult::Message(coven_shell_out(&["hooks", "install"]))
            }
            "adapter" => {
                if rest.is_empty() {
                    return CommandResult::Error(
                        "Usage: /coven adapter list [--json] | adapter doctor [id]".to_string(),
                    );
                }
                let mut argv: Vec<&str> = vec!["adapter"];
                argv.extend(rest.split_whitespace());
                CommandResult::Message(coven_shell_out(&argv))
            }
            "logs" => {
                if rest.is_empty() {
                    return CommandResult::Error("Usage: /coven logs prune [--days N]".to_string());
                }
                let mut argv: Vec<&str> = vec!["logs"];
                argv.extend(rest.split_whitespace());
                CommandResult::Message(coven_shell_out(&argv))
            }
            "wt" => {
                let mut argv: Vec<&str> = vec!["wt"];
                argv.extend(rest.split_whitespace());
                CommandResult::Message(coven_shell_out(&argv))
            }

            other => CommandResult::Error(format!(
                "Unknown /coven subcommand: '{other}'.\nRun `/coven help` for usage."
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

static COMMANDS: Lazy<Vec<Box<dyn SlashCommand>>> = Lazy::new(|| {
    vec![
        Box::new(HelpCommand),
        Box::new(ClearCommand),
        Box::new(CompactCommand),
        Box::new(ExitCommand),
        Box::new(ModelCommand),
        Box::new(ConfigCommand),
        Box::new(PluginCommand),
        Box::new(VersionCommand),
        Box::new(ResumeCommand),
        Box::new(ReloadPluginsCommand),
        Box::new(StatusCommand),
        Box::new(DiffCommand),
        Box::new(MemoryCommand),
        Box::new(BugCommand),
        Box::new(UsageCommand),
        Box::new(DoctorCommand),
        Box::new(LoginCommand),
        Box::new(LogoutCommand),
        Box::new(SwitchCommand),
        Box::new(RefreshCommand),
        Box::new(IncantCommand),
        Box::new(InitCommand),
        Box::new(ReviewCommand),
        Box::new(HooksCommand),
        Box::new(ImportConfigCommand),
        Box::new(McpCommand),
        Box::new(PermissionsCommand),
        Box::new(PlanCommand),
        Box::new(TasksCommand),
        Box::new(SessionCommand),
        Box::new(ForkCommand),
        Box::new(ThinkingCommand),
        Box::new(ThemeCommand),
        Box::new(OutputStyleCommand),
        Box::new(KeybindingsCommand),
        // New commands
        Box::new(ExportCommand),
        Box::new(ShareCommand),
        Box::new(SkillsCommand),
        Box::new(RewindCommand),
        Box::new(EffortCommand),
        Box::new(CommitCommand),
        Box::new(NamedCommandAdapter {
            slash_name: "add-dir",
            slash_hidden: true,
            target_name: "add-dir",
            slash_aliases: &[],
            slash_description: "Add a directory to Coven Code's allowed workspace paths",
            slash_help: "Usage: /add-dir <path>",
        }),
        Box::new(NamedCommandAdapter {
            slash_name: "agents",
            slash_hidden: true,
            target_name: "agents",
            slash_aliases: &[],
            slash_description: "Manage and configure sub-agents",
            slash_help: "Usage: /agents [list|create|edit|delete|reset] [name]",
        }),
        Box::new(NamedCommandAdapter {
            slash_name: "branch",
            slash_hidden: true,
            target_name: "branch",
            slash_aliases: &[],
            slash_description: "Create a branch of the current conversation at this point",
            slash_help: "Usage: /branch [create|switch|list] [name]",
        }),
        Box::new(NamedCommandAdapter {
            slash_name: "tag",
            slash_hidden: true,
            target_name: "tag",
            slash_aliases: &[],
            slash_description: "Toggle a searchable tag on the current session",
            slash_help: "Usage: /tag [list|add|remove] [tag]",
        }),
        Box::new(NamedCommandAdapter {
            slash_name: "pr-comments",
            slash_hidden: false,
            target_name: "pr-comments",
            slash_aliases: &[],
            slash_description: "Get comments from a GitHub pull request",
            slash_help: "Usage: /pr-comments <PR-number>",
        }),
        // Batch-1 new commands
        Box::new(CopyCommand),
        Box::new(ChromeCommand),
        Box::new(UpgradeCommand),
        Box::new(FastCommand),
        Box::new(ThinkBackCommand),
        // /whisper (BtwCommand) and /sandbox (SandboxToggleCommand)
        Box::new(BtwCommand),
        Box::new(SandboxToggleCommand),
        // Advisor
        Box::new(AdvisorCommand),
        // Multi-provider support
        Box::new(ProvidersCommand),
        Box::new(ConnectCommand),
        // Named agent system
        Box::new(AgentCommand),
        Box::new(FamiliarCommand),
        // Session search (SQLite)
        Box::new(SearchCommand),
        // Managed agent (manager-executor) architecture
        Box::new(ManagedAgentsCommand),
        // Durable long-running goals
        Box::new(GoalCommand),
        // Coven daemon control surface (sessions, harness runs, rituals)
        Box::new(CovenCommand),
    ]
});

/// Return all built-in slash commands.
pub fn all_commands() -> &'static [Box<dyn SlashCommand>] {
    &COMMANDS
}

/// Find a command by name or alias.
pub fn find_command(name: &str) -> Option<&'static dyn SlashCommand> {
    let name = name.trim_start_matches('/');
    all_commands()
        .iter()
        .find(|c| c.name() == name || c.aliases().contains(&name))
        .map(|c| c.as_ref())
}

/// Build `HelpEntry` values for all non-hidden commands, suitable for
/// populating `HelpOverlay::commands` at startup.
pub fn build_help_entries() -> Vec<claurst_tui::overlays::HelpEntry> {
    all_commands()
        .iter()
        .filter(|c| !c.hidden())
        .map(|c| claurst_tui::overlays::HelpEntry {
            name: c.name().to_string(),
            aliases: c.aliases().join(", "),
            description: c.description().to_string(),
            category: command_category(c.name()).to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// User-defined command templates (Feature 2)
// ---------------------------------------------------------------------------

/// A slash command backed by a user-defined template in `settings.json`.
struct TemplateCommand {
    name: String,
    template: claurst_core::CommandTemplate,
}

#[async_trait]
impl SlashCommand for TemplateCommand {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        self.template
            .description
            .as_deref()
            .unwrap_or("Custom command")
    }
    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let mut words = args.split_whitespace();
        let arg1 = words.next().unwrap_or("");
        let arg2 = words.next().unwrap_or("");
        let prompt = self
            .template
            .template
            .replace("$ARGUMENTS", args)
            .replace("$1", arg1)
            .replace("$2", arg2);
        CommandResult::UserMessage(prompt)
    }
}

/// Build slash commands from user-defined command templates stored in
/// `settings.commands`.
pub fn commands_from_settings(settings: &claurst_core::Settings) -> Vec<Box<dyn SlashCommand>> {
    settings
        .commands
        .iter()
        .map(|(name, template)| {
            Box::new(TemplateCommand {
                name: name.clone(),
                template: template.clone(),
            }) as Box<dyn SlashCommand>
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Discovered skill commands (from .coven-code/skills/ and git URLs)
// ---------------------------------------------------------------------------

/// A slash command backed by a discovered skill markdown file.
struct SkillCommand {
    name: String,
    description: String,
    template: String,
}

#[async_trait]
impl SlashCommand for SkillCommand {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }

    async fn execute(&self, args: &str, _ctx: &mut CommandContext) -> CommandResult {
        let mut words = args.split_whitespace();
        let arg1 = words.next().unwrap_or("");
        let arg2 = words.next().unwrap_or("");
        let prompt = self
            .template
            .replace("$ARGUMENTS", args)
            .replace("$1", arg1)
            .replace("$2", arg2);
        CommandResult::UserMessage(prompt)
    }
}

/// Build slash commands from skill markdown files discovered on the filesystem
/// and from configured git URLs.
///
/// Pass the project `cwd` and the `skills` section of the effective config.
/// Bundled skills take precedence — any discovered skill whose name clashes
/// with a built-in command will be silently skipped.
pub fn commands_from_discovered_skills(
    cwd: &std::path::Path,
    skills_config: &claurst_core::SkillsConfig,
) -> Vec<Box<dyn SlashCommand>> {
    let discovered = claurst_core::discover_skills(cwd, skills_config);
    // Build a set of built-in command names so we can skip collisions.
    let all_cmds = all_commands();
    let builtin_names: std::collections::HashSet<&str> =
        all_cmds.iter().map(|c| c.name()).collect();

    discovered
        .into_values()
        .filter(|skill| !builtin_names.contains(skill.name.as_str()))
        .map(|skill| {
            Box::new(SkillCommand {
                name: skill.name,
                description: skill.description,
                template: skill.template,
            }) as Box<dyn SlashCommand>
        })
        .collect()
}

/// Execute a slash command string (with leading /).
pub async fn execute_command(input: &str, ctx: &mut CommandContext) -> Option<CommandResult> {
    if !claurst_tui::input::is_slash_command(input) {
        return None;
    }
    let (name, args) = claurst_tui::input::parse_slash_command(input);

    // First check built-in commands.
    if let Some(cmd) = find_command(name) {
        return Some(cmd.execute(args, ctx).await);
    }

    // Check user-defined command templates from settings.
    let cmd_name = name.trim_start_matches('/');
    if let Some(tmpl) = ctx.config.commands.get(cmd_name).cloned() {
        let tc = TemplateCommand {
            name: cmd_name.to_string(),
            template: tmpl,
        };
        return Some(tc.execute(args, ctx).await);
    }

    // Check discovered skill commands (from .coven-code/skills/, git URLs, etc.).
    {
        let discovered = claurst_core::discover_skills(&ctx.working_dir, &ctx.config.skills);
        if let Some(skill) = discovered.get(cmd_name) {
            let sc = SkillCommand {
                name: skill.name.clone(),
                description: skill.description.clone(),
                template: skill.template.clone(),
            };
            return Some(sc.execute(args, ctx).await);
        }
    }

    // Then check plugin-defined slash commands.
    if let Some(registry) = claurst_plugins::global_plugin_registry() {
        for cmd_def in registry.all_command_defs() {
            if cmd_def.name == cmd_name {
                let adapter = PluginSlashCommandAdapter { def: cmd_def };
                return Some(adapter.execute(args, ctx).await);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Named commands module (top-level `claude <name>` subcommands)
// ---------------------------------------------------------------------------
pub mod named_commands;

// ---------------------------------------------------------------------------
// Stats analytics (persisted transcript aggregation) — backs `coven-code stats`.
// The current-session `/stats` slash command lives above; this module reads
// JSONL transcripts on disk.
// ---------------------------------------------------------------------------
pub mod stats;

#[cfg(test)]
pub(crate) mod test_env {
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) struct CommandEnvGuard {
        old_home: Option<String>,
        old_coven_home: Option<String>,
        old_user: Option<String>,
        old_username: Option<String>,
        coven_home: PathBuf,
        _tmp: Option<tempfile::TempDir>,
        _lock: MutexGuard<'static, ()>,
    }

    impl CommandEnvGuard {
        pub(crate) fn set(home: &Path, coven_home: &Path, user: Option<&str>) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let guard = Self {
                old_home: std::env::var("HOME").ok(),
                old_coven_home: std::env::var("COVEN_HOME").ok(),
                old_user: std::env::var("USER").ok(),
                old_username: std::env::var("USERNAME").ok(),
                coven_home: coven_home.to_path_buf(),
                _tmp: None,
                _lock: lock,
            };
            std::env::set_var("HOME", home);
            std::env::set_var("COVEN_HOME", coven_home);
            match user {
                Some(value) => std::env::set_var("USER", value),
                None => std::env::remove_var("USER"),
            }
            std::env::remove_var("USERNAME");
            guard
        }

        pub(crate) fn with_coven_home(user: Option<&str>) -> Self {
            let tmp = tempfile::tempdir().expect("tempdir");
            let home = tmp.path().join("home");
            let coven_home = tmp.path().join("coven");
            std::fs::create_dir_all(&home).expect("home dir");
            std::fs::create_dir_all(&coven_home).expect("coven home dir");
            let mut guard = Self::set(&home, &coven_home, user);
            guard._tmp = Some(tmp);
            guard
        }

        pub(crate) fn coven_home(&self) -> &Path {
            &self.coven_home
        }
    }

    impl Drop for CommandEnvGuard {
        fn drop(&mut self) {
            match &self.old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.old_coven_home {
                Some(value) => std::env::set_var("COVEN_HOME", value),
                None => std::env::remove_var("COVEN_HOME"),
            }
            match &self.old_user {
                Some(value) => std::env::set_var("USER", value),
                None => std::env::remove_var("USER"),
            }
            match &self.old_username {
                Some(value) => std::env::set_var("USERNAME", value),
                None => std::env::remove_var("USERNAME"),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::CommandEnvGuard;
    use claurst_core::cost::CostTracker;

    fn make_ctx() -> CommandContext {
        CommandContext {
            config: claurst_core::config::Config::default(),
            cost_tracker: CostTracker::new(),
            messages: vec![],
            working_dir: std::path::PathBuf::from("."),
            session_id: "test-session".to_string(),
            session_title: None,
            remote_session_url: None,
            mcp_manager: None,
            mcp_auth_runner: None,
        }
    }

    // ---- Command registry tests ---------------------------------------------

    #[test]
    fn test_all_commands_non_empty() {
        assert!(!all_commands().is_empty());
    }

    #[test]
    fn test_all_commands_have_unique_names() {
        let mut names = std::collections::HashSet::new();
        for cmd in all_commands() {
            assert!(
                names.insert(cmd.name().to_string()),
                "Duplicate command name: {}",
                cmd.name()
            );
        }
    }

    #[test]
    fn test_find_command_by_name() {
        assert!(find_command("help").is_some());
        assert!(find_command("clear").is_some());
        assert!(find_command("exit").is_some());
        assert!(find_command("model").is_some());
        assert!(find_command("refresh").is_some());
        assert!(find_command("version").is_some());
    }

    #[test]
    fn test_find_command_with_slash_prefix() {
        // find_command should strip the leading / before lookup
        assert!(find_command("/help").is_some());
        assert!(find_command("/clear").is_some());
    }

    #[test]
    fn phase_two_legacy_commands_are_fully_removed() {
        // The Phase 2 hidden aliases' one-release grace period has ended:
        // the names no longer resolve at all. Their impls survive only as
        // delegation targets of /config, /usage, and /rewind.
        let removed_legacy = [
            "color",
            "context",
            "cost",
            "revert",
            "statusline",
            "terminal-setup",
            "undo",
            "vim",
            "voice",
        ];

        for name in removed_legacy {
            assert!(
                find_command(name).is_none(),
                "{name} should no longer resolve after the alias grace period"
            );
        }
    }

    #[tokio::test]
    async fn config_command_owns_folded_ui_settings() {
        let _guard = CommandEnvGuard::with_coven_home(None);
        let mut ctx = make_ctx();
        let command = find_command("config").unwrap();

        let result = command.execute("color", &mut ctx).await;
        match result {
            CommandResult::Message(message) => assert!(message.contains("Current prompt color")),
            other => panic!("expected color status message, got {:?}", other),
        }

        let result = command.execute("statusline", &mut ctx).await;
        match result {
            CommandResult::Message(message) => {
                assert!(message.contains("Status line configuration"))
            }
            other => panic!("expected statusline message, got {:?}", other),
        }

        let result = command.execute("voice status", &mut ctx).await;
        match result {
            CommandResult::Message(message) => assert!(message.contains("Voice mode")),
            other => panic!("expected voice status message, got {:?}", other),
        }

        let result = command.execute("vim on", &mut ctx).await;
        match result {
            CommandResult::Message(message) => assert!(message.contains("Editor mode set to vim")),
            other => panic!("expected vim setting message, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn usage_command_owns_cost_and_context_views() {
        let mut ctx = make_ctx();
        let command = find_command("usage").unwrap();

        let result = command.execute("cost", &mut ctx).await;
        match result {
            CommandResult::Message(message) => assert!(message.contains("Session Cost")),
            other => panic!("expected cost message, got {:?}", other),
        }

        let result = command.execute("context", &mut ctx).await;
        match result {
            CommandResult::Message(message) => assert!(message.contains("Context Window Usage")),
            other => panic!("expected context message, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn rewind_with_args_routes_to_file_rollback_not_overlay() {
        let _guard = CommandEnvGuard::with_coven_home(None);
        let mut ctx = make_ctx();
        ctx.messages.push(claurst_core::types::Message::user("hi"));
        let command = find_command("rewind").unwrap();

        // `list` must reach the checkpoints path, never the overlay.
        let result = command.execute("list", &mut ctx).await;
        assert!(
            !matches!(result, CommandResult::OpenRewindOverlay),
            "/rewind list must route to the revert/checkpoints path"
        );

        // `diff` must reach the snapshot-diff path, never the overlay.
        let result = command.execute("diff", &mut ctx).await;
        assert!(
            !matches!(result, CommandResult::OpenRewindOverlay),
            "/rewind diff must route to the snapshot diff path"
        );
    }

    #[tokio::test]
    async fn rewind_without_args_keeps_existing_behavior() {
        let mut ctx = make_ctx();
        let command = find_command("rewind").unwrap();
        let result = command.execute("", &mut ctx).await;
        match result {
            CommandResult::Message(message) => assert!(message.contains("Nothing to rewind")),
            other => panic!("expected empty-conversation message, got {:?}", other),
        }
    }

    #[test]
    fn test_find_command_by_alias() {
        // /help has aliases "h" and "?"
        assert!(find_command("h").is_some());
        assert!(find_command("?").is_some());
        // /clear has alias "c"
        assert!(find_command("c").is_some());
        assert!(find_command("settings").is_some());
        assert!(find_command("continue").is_some());
        assert!(find_command("bug").is_some());
        assert!(find_command("bashes").is_some());
        assert!(find_command("remote").is_some());
        assert!(find_command("sandbox-toggle").is_some());
        assert!(find_command("btw").is_some());
    }

    #[test]
    fn test_find_command_not_found() {
        assert!(find_command("nonexistent_command_xyz").is_none());
    }

    #[test]
    fn test_core_commands_present() {
        let expected = [
            "help",
            "clear",
            "compact",
            "exit",
            "model",
            "config",
            "version",
            "status",
            "diff",
            "memory",
            "hooks",
            "permissions",
            "plan",
            "tasks",
            "session",
            "login",
            "logout",
            "refresh",
            "feedback",
            "usage",
            "plugin",
            "reload-plugins",
            "add-dir",
            "agents",
            "branch",
            "tag",
            "pr-comments",
            "whisper",
            "incant",
            "sandbox",
            "switch",
            "review",
            "coven",
            "familiar",
        ];
        for name in &expected {
            assert!(
                find_command(name).is_some(),
                "Expected command '{}' not in all_commands()",
                name
            );
        }
    }

    // ---- Command execution tests --------------------------------------------

    #[tokio::test]
    async fn test_clear_command_returns_clear_conversation() {
        let mut ctx = make_ctx();
        let cmd = find_command("clear").unwrap();
        let result = cmd.execute("", &mut ctx).await;
        assert!(matches!(result, CommandResult::ClearConversation));
    }

    #[tokio::test]
    async fn test_refresh_command_requests_provider_reset() {
        let mut ctx = make_ctx();
        let cmd = find_command("refresh").unwrap();
        let result = cmd.execute("", &mut ctx).await;
        assert!(matches!(result, CommandResult::RefreshProviderState));
    }

    #[tokio::test]
    async fn test_exit_command_returns_exit() {
        let mut ctx = make_ctx();
        let cmd = find_command("exit").unwrap();
        let result = cmd.execute("", &mut ctx).await;
        assert!(matches!(result, CommandResult::Exit));
    }

    #[tokio::test]
    async fn test_doctor_command_reports_coven_substrate() {
        // /doctor must surface a "Coven Substrate" section regardless of
        // whether the daemon is online. This pins the Phase 7 diagnostics-
        // coverage requirement so future doctor refactors don't accidentally
        // drop the substrate report and silently regress the gap closure.
        let mut ctx = make_ctx();
        let cmd = find_command("doctor").expect("/doctor must be registered");
        let result = cmd.execute("", &mut ctx).await;
        match result {
            CommandResult::Message(msg) => {
                assert!(
                    msg.contains("Coven Substrate"),
                    "/doctor output must include a 'Coven Substrate' section, got:\n{msg}"
                );
                // Either the daemon is installed and we report status/version,
                // or it isn't and we point at the install hint. Both paths
                // must produce a non-empty section.
                let has_status = msg.contains("Daemon online")
                    || msg.contains("Daemon socket present")
                    || msg.contains("Daemon not installed");
                assert!(
                    has_status,
                    "/doctor substrate section is empty, got:\n{msg}"
                );
            }
            other => panic!("expected Message from /doctor, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_doctor_help_mentions_substrate() {
        // The /doctor help text must document the substrate check so users
        // can tell what `/doctor` covers without running it.
        let cmd = find_command("doctor").expect("/doctor must be registered");
        assert!(
            cmd.help().contains("Coven Substrate"),
            "/doctor help should mention Coven Substrate, got:\n{}",
            cmd.help()
        );
    }

    #[tokio::test]
    async fn test_version_command_returns_message() {
        let mut ctx = make_ctx();
        let cmd = find_command("version").unwrap();
        let result = cmd.execute("", &mut ctx).await;
        assert!(matches!(result, CommandResult::Message(_)));
        if let CommandResult::Message(msg) = result {
            assert!(
                msg.contains("claude") || msg.contains("Coven Code") || msg.contains('.'),
                "Version message should contain version number, got: {}",
                msg
            );
        }
    }

    #[tokio::test]
    async fn test_cost_command_returns_message() {
        // /cost is no longer registered (expired Phase 2 alias); the impl
        // remains as the delegation target of /usage cost.
        let mut ctx = make_ctx();
        assert!(find_command("cost").is_none());
        let result = UsageCommand.execute("cost", &mut ctx).await;
        assert!(matches!(result, CommandResult::Message(_)));
    }

    #[tokio::test]
    async fn test_login_command_starts_oauth_flow() {
        let mut ctx = make_ctx();
        let cmd = find_command("login").unwrap();
        // Default (no --console) → Anthropic, login_with_claude_ai = true
        let result = cmd.execute("", &mut ctx).await;
        match result {
            CommandResult::StartLoginForProvider {
                provider,
                login_with_claude_ai,
                label,
            } => {
                assert_eq!(provider, claurst_core::accounts::PROVIDER_ANTHROPIC);
                assert!(login_with_claude_ai);
                assert!(label.is_none());
            }
            other => panic!("expected StartLoginForProvider, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_login_command_console_flag() {
        let mut ctx = make_ctx();
        let cmd = find_command("login").unwrap();
        let result = cmd.execute("--console", &mut ctx).await;
        match result {
            CommandResult::StartLoginForProvider {
                provider,
                login_with_claude_ai,
                ..
            } => {
                assert_eq!(provider, claurst_core::accounts::PROVIDER_ANTHROPIC);
                assert!(!login_with_claude_ai);
            }
            other => panic!("expected StartLoginForProvider, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_login_command_codex_flag() {
        let mut ctx = make_ctx();
        let cmd = find_command("login").unwrap();
        let result = cmd.execute("--codex --label work", &mut ctx).await;
        match result {
            CommandResult::StartLoginForProvider {
                provider, label, ..
            } => {
                assert_eq!(provider, claurst_core::accounts::PROVIDER_CODEX);
                assert_eq!(label.as_deref(), Some("work"));
            }
            other => panic!("expected StartLoginForProvider, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_switch_no_args_lists_accounts() {
        let mut ctx = make_ctx();
        let cmd = find_command("switch").unwrap();
        let result = cmd.execute("", &mut ctx).await;
        // /switch with no args lists stored accounts (absorbs old /accounts).
        assert!(matches!(result, CommandResult::Message(_)));
    }

    #[tokio::test]
    async fn familiar_empty_args_uses_saved_roster_only() {
        let _guard = CommandEnvGuard::with_coven_home(Some("kitty"));
        let mut ctx = make_ctx();
        ctx.config.familiar = Some("kitty".to_string());

        assert!(current_familiar_roster().is_empty());

        let cmd = find_command("familiar").unwrap();
        let result = cmd.execute("", &mut ctx).await;

        match result {
            CommandResult::Message(msg) => {
                assert!(msg.contains("Current familiar: none"), "{msg}");
                assert!(!msg.contains("kitty"), "{msg}");
            }
            other => panic!("expected Message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn familiar_auto_without_saved_roster_returns_error() {
        let _guard = CommandEnvGuard::with_coven_home(Some("sage"));
        let mut ctx = make_ctx();
        ctx.config.familiar = Some("existing".to_string());

        let cmd = find_command("familiar").unwrap();
        let result = cmd.execute("auto", &mut ctx).await;

        match result {
            CommandResult::Error(msg) => assert_eq!(
                msg,
                "Could not infer a familiar from USER/USERNAME and the saved roster."
            ),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_switch_command_requires_id() {
        let mut ctx = make_ctx();
        let cmd = find_command("switch").unwrap();
        // A flag with no profile id is still an error.
        let result = cmd.execute("--codex", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_help_command_returns_message() {
        let mut ctx = make_ctx();
        let cmd = find_command("help").unwrap();
        let result = cmd.execute("", &mut ctx).await;
        // help returns either Message or Silent
        assert!(
            matches!(result, CommandResult::Message(_) | CommandResult::Silent),
            "help should return Message or Silent"
        );
    }

    #[tokio::test]
    async fn test_removed_upsell_commands_are_gone() {
        for name in [
            "web-setup",
            "passes",
            "desktop",
            "mobile",
            "stickers",
            "install-github-app",
            "install-slack-app",
            "privacy-settings",
        ] {
            assert!(find_command(name).is_none(), "'{}' should be removed", name);
        }
    }

    #[tokio::test]
    async fn test_import_config_command_opens_overlay() {
        let mut ctx = make_ctx();
        let cmd = find_command("import-config").unwrap();
        let result = cmd.execute("", &mut ctx).await;
        assert!(matches!(result, CommandResult::OpenImportConfigOverlay));
    }

    #[tokio::test]
    async fn test_coven_command_registered() {
        // `/coven` must be wired into the registry so the TUI autocompletes it.
        assert!(find_command("coven").is_some());
    }

    #[tokio::test]
    async fn test_coven_help_returns_usage_message() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("help", &mut ctx).await;
        match result {
            CommandResult::Message(msg) => {
                assert!(msg.contains("Usage: /coven"));
                assert!(msg.contains("sessions"));
                assert!(msg.contains("attach"));
                assert!(msg.contains("sacrifice"));
            }
            other => panic!("expected Message, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_coven_attach_without_id_errors() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("attach", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_unknown_subcommand_errors() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("nonsense", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_daemon_rejects_bad_action() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("daemon explode", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_send_requires_id_and_text() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        assert!(matches!(
            cmd.execute("send", &mut ctx).await,
            CommandResult::Error(_)
        ));
        assert!(matches!(
            cmd.execute("send abc-123", &mut ctx).await,
            CommandResult::Error(_)
        ));
    }

    #[tokio::test]
    async fn test_coven_kill_requires_id() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("kill", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_log_requires_id() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("log", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_help_lists_native_subcommands() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("help", &mut ctx).await;
        match result {
            CommandResult::Message(msg) => {
                for verb in [
                    "familiars",
                    "log",
                    "send",
                    "kill",
                    "capabilities",
                    "info",
                    "events",
                    "actions",
                ] {
                    assert!(msg.contains(verb), "help should mention {verb}: {msg}");
                }
            }
            other => panic!("expected Message, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_coven_info_requires_id() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("info", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_events_requires_id() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("events", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_events_rejects_unknown_flag() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("events abc-123 --wat 1", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_actions_requires_name() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("actions", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_actions_rejects_bad_json_args() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd
            .execute("actions coven.capabilities.refresh not-json", &mut ctx)
            .await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    /// The TUI's slash-command autocomplete and `/help` overlay both read
    /// from `claurst_tui::app::PROMPT_SLASH_COMMANDS`. That list is hand-
    /// maintained — when a new command is added to the registry but not to
    /// the list, the TUI silently hides it from autocomplete. This test is
    /// the safety net.
    #[test]
    fn prompt_slash_commands_covers_registry() {
        use std::collections::HashSet;

        // Names that appear in autocomplete on purpose even though they're
        // not (or not primarily) registered SlashCommands:
        //  - quit / settings / survey: user-facing aliases of exit / config /
        //    feedback.
        //  - handoff: intercepted directly by the TUI via
        //    `App::intercept_slash_command_with_args` (sends the current
        //    session context to a Coven familiar). It is documented in
        //    docs/familiars.md and lives in tui/src/handoff.rs.
        const ALLOWED_ALIAS_NAMES: &[&str] = &["quit", "settings", "survey", "handoff"];

        let prompt_names: HashSet<&str> = claurst_tui::app::PROMPT_SLASH_COMMANDS
            .iter()
            .map(|(name, _)| *name)
            .collect();

        // Every non-hidden registered command must show up in autocomplete.
        let mut missing: Vec<&str> = all_commands()
            .iter()
            .filter(|c| !c.hidden())
            .map(|c| c.name())
            .filter(|name| !prompt_names.contains(name))
            .collect();
        missing.sort();
        assert!(
            missing.is_empty(),
            "PROMPT_SLASH_COMMANDS is missing entries for non-hidden \
             registered commands: {missing:?}. Add them to \
             crates/tui/src/app.rs::PROMPT_SLASH_COMMANDS."
        );

        // No orphans in the prompt list (every entry must either match a
        // registered command name/alias or be in the allow-list).
        let known_names_and_aliases: HashSet<&str> = all_commands()
            .iter()
            .flat_map(|c| std::iter::once(c.name()).chain(c.aliases()))
            .collect();
        let mut orphans: Vec<&str> = prompt_names
            .iter()
            .copied()
            .filter(|name| {
                !known_names_and_aliases.contains(name) && !ALLOWED_ALIAS_NAMES.contains(name)
            })
            .collect();
        orphans.sort();
        assert!(
            orphans.is_empty(),
            "PROMPT_SLASH_COMMANDS has orphan entries with no matching \
             command: {orphans:?}. Either register the command or drop \
             the entry."
        );
    }

    #[tokio::test]
    async fn test_coven_help_lists_integration_subcommands() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("help", &mut ctx).await;
        match result {
            CommandResult::Message(msg) => {
                for verb in ["calls", "claim", "hooks-install", "adapter", "logs", "wt"] {
                    assert!(msg.contains(verb), "help should mention {verb}: {msg}");
                }
            }
            other => panic!("expected Message, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_coven_claim_requires_action() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("claim", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_adapter_requires_subaction() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("adapter", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_coven_logs_requires_subaction() {
        let mut ctx = make_ctx();
        let cmd = find_command("coven").unwrap();
        let result = cmd.execute("logs", &mut ctx).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn coven_calls_ledger_returns_message_when_file_missing() {
        let _guard = CommandEnvGuard::with_coven_home(None);
        let msg = coven_read_calls_ledger(20);
        assert!(
            msg.contains("No delegation ledger yet") || msg.contains("Could not"),
            "expected friendly message, got: {msg}"
        );
    }

    #[test]
    fn coven_calls_ledger_renders_recent_entries() {
        let guard = CommandEnvGuard::with_coven_home(None);
        let ledger = r#"{
          "version": 1,
          "calls": [
            {"id":"a","callerFamiliarId":"nova","calleeFamiliarId":"sage",
             "request":"first task","status":"completed","createdAt":"2026-06-07T00:00:00Z"},
            {"id":"b","callerFamiliarId":"sage","calleeFamiliarId":"kitty",
             "request":"second task","status":"running","createdAt":"2026-06-07T00:01:00Z"}
          ]
        }"#;
        std::fs::write(guard.coven_home().join("cave-coven-calls.json"), ledger).unwrap();
        let out = coven_read_calls_ledger(20);
        assert!(out.contains("nova"));
        assert!(out.contains("sage"));
        assert!(out.contains("kitty"));
        assert!(out.contains("running"));
    }

    /// End-to-end integration test for `/coven` against a mock daemon
    /// running on a real Unix socket. We boot a `UnixListener` in a
    /// scratch tempdir, point `COVEN_HOME` at it, and exercise the two
    /// structured-error scenarios from the daemon's HTTP contract:
    /// 409 `session_not_live` (live-control endpoints) and 404
    /// `session_not_found` (per-session reads). The assertions pin
    /// that the daemon's error code reaches the user-visible
    /// `CommandResult::Error` payload instead of being collapsed to a
    /// generic "daemon offline" fallback.
    //
    // The MutexGuard is held across `.await` on purpose: `COVEN_HOME`
    // is a process-global env var that the synchronous DaemonClient
    // reads each time, and we must keep it pinned to our tempdir for
    // the entire test. The lock only serializes tests in this same
    // file, so there is no real deadlock risk — only a clippy
    // false-positive.
    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn coven_command_integrates_with_daemon_mock() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;

        // Spawn a single thread that serves `responses` sequentially —
        // one connection per response, in order. Each connection reads
        // the HTTP request and writes the canned body. Returns when all
        // responses have been delivered.
        fn serve_sequence(
            listener: UnixListener,
            responses: Vec<&'static str>,
        ) -> std::thread::JoinHandle<()> {
            std::thread::spawn(move || {
                for response in responses {
                    let (mut stream, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(_) => return,
                    };
                    let mut buf = [0_u8; 4096];
                    let _ = stream.read(&mut buf);
                    // Best-effort write: the client may have hung up
                    // already on a 4xx/5xx scenario, so a broken pipe
                    // here is not the test's failure mode.
                    let _ = stream.write_all(response.as_bytes());
                }
            })
        }

        let guard = CommandEnvGuard::with_coven_home(None);
        let dir = guard.coven_home().to_path_buf();

        let sock_path = dir.join("coven.sock");
        let coven = find_command("coven").expect("/coven command must be registered");

        // ---- /coven send against 409 session_not_live ----
        {
            let listener = UnixListener::bind(&sock_path).unwrap();
            let server = serve_sequence(
                listener,
                vec![
                    "HTTP/1.0 409 Conflict\r\nContent-Type: application/json\r\n\r\n\
                     {\"error\":{\"code\":\"session_not_live\",\
                     \"message\":\"Session is not running.\"}}",
                ],
            );
            let mut ctx = make_ctx();
            let result = coven.execute("send dead-session hello", &mut ctx).await;
            let _ = server.join();
            match result {
                CommandResult::Error(msg) => {
                    assert!(
                        msg.contains("session_not_live"),
                        "expected session_not_live code, got: {msg}"
                    );
                    assert!(msg.contains("409"), "expected status, got: {msg}");
                    assert!(
                        !msg.contains("daemon offline"),
                        "structured 409 must not be reported as 'daemon offline': {msg}"
                    );
                }
                other => panic!("expected Error from /coven send, got {other:?}"),
            }
            std::fs::remove_file(&sock_path).ok();
        }

        // ---- /coven info <missing> against 404 session_not_found ----
        {
            let listener = UnixListener::bind(&sock_path).unwrap();
            let server = serve_sequence(
                listener,
                vec![
                    "HTTP/1.0 404 Not Found\r\nContent-Type: application/json\r\n\r\n\
                     {\"error\":{\"code\":\"session_not_found\",\
                     \"message\":\"Session was not found.\"}}",
                ],
            );
            let mut ctx = make_ctx();
            let result = coven.execute("info ghost", &mut ctx).await;
            let _ = server.join();
            match result {
                CommandResult::Error(msg) => {
                    assert!(
                        msg.contains("session_not_found"),
                        "expected session_not_found code, got: {msg}"
                    );
                    assert!(msg.contains("404"), "expected status, got: {msg}");
                }
                other => panic!("expected Error from /coven info, got {other:?}"),
            }
            std::fs::remove_file(&sock_path).ok();
        }

        // ---- /coven sessions against a healthy daemon ----
        {
            let listener = UnixListener::bind(&sock_path).unwrap();
            let server = serve_sequence(
                listener,
                vec![
                    "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n\
                     [{\"id\":\"abc-123\",\"harness\":\"openclaw\",\
                     \"title\":\"hello world\",\"status\":\"running\",\
                     \"project_root\":\"/tmp/p\"}]",
                ],
            );
            let mut ctx = make_ctx();
            let result = coven.execute("sessions", &mut ctx).await;
            let _ = server.join();
            match result {
                CommandResult::Message(msg) => {
                    assert!(
                        msg.contains("abc-123"),
                        "expected session id in list, got: {msg}"
                    );
                    assert!(msg.contains("openclaw"), "expected harness, got: {msg}");
                    assert!(msg.contains("hello world"), "expected title, got: {msg}");
                }
                other => panic!("expected Message from /coven sessions, got {other:?}"),
            }
            std::fs::remove_file(&sock_path).ok();
        }
    }

    #[test]
    fn test_split_command_args_preserves_quoted_segments() {
        assert_eq!(
            split_command_args("create \"agent alpha\" 'second value'"),
            vec![
                "create".to_string(),
                "agent alpha".to_string(),
                "second value".to_string(),
            ]
        );
    }
}
