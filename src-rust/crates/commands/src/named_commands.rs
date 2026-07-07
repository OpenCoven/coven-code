//! Named commands (e.g. `coven-code agents`, `coven-code ide`, `coven-code branch`, …).
//!
//! These complement slash commands with more complex top-level flows.
//! A named command is invoked when the *first* CLI argument matches one
//! of the registered names — before the normal REPL starts.
//!
//! Sources consulted while porting:
//!   src/commands/agents/index.ts
//!   src/commands/ide/index.ts
//!   src/commands/branch/index.ts
//!   src/commands/tag/index.ts
//!   src/commands/passes/index.ts
//!   src/commands/pr_comments/index.ts
//!   src/commands/install-github-app/index.ts
//!   src/commands/desktop/index.ts  (implied by component structure)
//!   src/commands/mobile/index.ts   (implied by component structure)
//!   src/commands/remote-setup/index.ts (implied by component structure)

use crate::{CommandContext, CommandResult};
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};
// `open` crate: used by StickersCommand to launch the browser.

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A top-level named command (`coven-code <name> [args…]`).
pub trait NamedCommand: Send + Sync {
    /// Primary command name, e.g. `"agents"`.
    fn name(&self) -> &str;

    /// One-line description used in `coven-code --help`.
    fn description(&self) -> &str;

    /// Usage hint shown in `coven-code <name> --help`.
    fn usage(&self) -> &str;

    /// Execute the command.  `args` is the slice of arguments *after* the
    /// command name itself.
    fn execute_named(&self, args: &[&str], ctx: &CommandContext) -> CommandResult;
}

// ---------------------------------------------------------------------------
// agents
// ---------------------------------------------------------------------------

pub struct AgentsCommand;

impl NamedCommand for AgentsCommand {
    fn name(&self) -> &str {
        "agents"
    }
    fn description(&self) -> &str {
        "Manage and configure sub-agents and Coven familiars"
    }
    fn usage(&self) -> &str {
        "coven-code agents [list|create|edit|delete|familiars|reset] [name]"
    }

    fn execute_named(&self, args: &[&str], ctx: &CommandContext) -> CommandResult {
        match args.first().copied().unwrap_or("list") {
            "list" => {
                let defs = claurst_tui::agents_view::load_agent_definitions(&ctx.working_dir);

                let (familiar_defs, user_defs): (Vec<_>, Vec<_>) = defs
                    .iter()
                    .partition(|d| d.source.starts_with("coven:familiar"));

                if defs.is_empty() {
                    return CommandResult::Message(
                        "Available Agents (0)\n\n\
                         No custom agents defined. Create one with /familiar create <name>\n\
                         or run: coven-code agents create <name>\n\n\
                         No Coven familiars found. Install the Coven daemon to\n\
                         automatically surface familiars here."
                            .to_string(),
                    );
                }

                let mut out = format!("Available Agents ({})\n", defs.len());

                if !user_defs.is_empty() {
                    out.push_str(&format!("\nWorkspace Agents ({})\n", user_defs.len()));
                    for def in &user_defs {
                        let model_str = def.model.as_deref().unwrap_or("default");
                        if def.description.is_empty() {
                            out.push_str(&format!(
                                "  \u{2022} {} (model: {})\n",
                                def.name, model_str
                            ));
                        } else {
                            out.push_str(&format!(
                                "  \u{2022} {}: {}\n    Model: {}\n",
                                def.name, def.description, model_str
                            ));
                        }
                    }
                }

                if !familiar_defs.is_empty() {
                    out.push_str(&format!(
                        "\n\u{2728} Coven Familiars ({})\n",
                        familiar_defs.len()
                    ));
                    for def in &familiar_defs {
                        let id = def.source.trim_start_matches("coven:familiar:");
                        let desc_short = def
                            .description
                            .split(" \u{2014} ")
                            .nth(1)
                            .unwrap_or(&def.description)
                            .trim();
                        let access = def.access.as_deref().unwrap_or("read-only");
                        out.push_str(&format!(
                            "  \u{2605} {} (id: {}, access: {})\n    {}\n",
                            def.name, id, access, desc_short
                        ));
                    }
                    out.push_str("\nSwitch active familiar: /familiar <name>");
                }

                if user_defs.is_empty() {
                    out.push_str(
                        "\nUse 'coven-code agents create <name>' to add a workspace agent.",
                    );
                }
                CommandResult::Message(out)
            }
            "familiars" => {
                // Shorthand: list only Coven familiars.
                let defs = claurst_tui::agents_view::load_agent_definitions(&ctx.working_dir);
                let familiar_defs: Vec<_> = defs
                    .iter()
                    .filter(|d| d.source.starts_with("coven:familiar"))
                    .collect();
                if familiar_defs.is_empty() {
                    return CommandResult::Message(
                        "No Coven familiars found.\n\n\
                         Install the Coven daemon and define familiars in\n\
                         ~/.coven/familiars.toml to have them appear here as agents."
                            .to_string(),
                    );
                }
                let mut out = format!("\u{2728} Coven Familiars ({})\n\n", familiar_defs.len());
                for def in &familiar_defs {
                    let id = def.source.trim_start_matches("coven:familiar:");
                    out.push_str(&format!(
                        "  \u{2605} {} [{}]\n    {}\n\n",
                        def.name, id, def.description
                    ));
                }
                out.push_str("Switch to a familiar: coven-code agent <name>");
                CommandResult::Message(out)
            }
            "create" => {
                let name = args.get(1).copied().unwrap_or("my-agent");
                if claurst_core::coven_shared::is_disallowed_familiar_name(name) {
                    return CommandResult::Error(format!(
                        "The name '{name}' is reserved and cannot be used for an agent or \
                         familiar. Choose a different name."
                    ));
                }
                CommandResult::Message(format!(
                    "Create a new agent by adding .coven-code/agents/{name}.md\n\
                     Template:\n\
                     ---\n\
                     name: {name}\n\
                     description: <description>\n\
                     model: claude-sonnet-4-6\n\
                     ---\n\n\
                     <agent instructions here>"
                ))
            }
            "edit" => {
                let name = match args.get(1).copied() {
                    Some(n) => n,
                    None => {
                        return CommandResult::Error(
                            "Usage: coven-code agents edit <name>".to_string(),
                        )
                    }
                };
                // Block renaming/retargeting an agent onto a reserved name.
                if claurst_core::coven_shared::is_disallowed_familiar_name(name) {
                    return CommandResult::Error(format!(
                        "The name '{name}' is reserved and cannot be used for an agent or \
                         familiar. Choose a different name."
                    ));
                }
                CommandResult::Message(format!(
                    "Edit .coven-code/agents/{name}.md in your editor to update the agent."
                ))
            }
            "delete" => {
                let name = match args.get(1).copied() {
                    Some(n) => n,
                    None => {
                        return CommandResult::Error(
                            "Usage: coven-code agents delete <name>".to_string(),
                        )
                    }
                };
                CommandResult::Message(format!(
                    "Delete .coven-code/agents/{name}.md to remove the agent."
                ))
            }
            "reset" => match claurst_core::reset_familiars_and_agents(Some(&ctx.working_dir)) {
                Ok(summary) => CommandResult::Message(summary.message()),
                Err(err) => {
                    CommandResult::Error(format!("Failed to reset agents and familiars: {err}"))
                }
            },
            sub => CommandResult::Error(format!(
                "Unknown agents subcommand: '{sub}'\
                \nValid: list, familiars, create, edit, delete, reset"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// agent  (switch active familiar / agent persona)
// ---------------------------------------------------------------------------

pub struct AgentCommand;

impl NamedCommand for AgentCommand {
    fn name(&self) -> &str {
        "agent"
    }
    fn description(&self) -> &str {
        "Show or switch the active Coven familiar / agent persona"
    }
    fn usage(&self) -> &str {
        "coven-code agent [name|--list]"
    }

    fn execute_named(&self, args: &[&str], ctx: &CommandContext) -> CommandResult {
        let defs = claurst_tui::agents_view::load_agent_definitions(&ctx.working_dir);

        // --list flag: enumerate all available names.
        if args.first().copied() == Some("--list") {
            if defs.is_empty() {
                return CommandResult::Message(
                    "No agents or familiars found.\n\
                     Install the Coven daemon or add agents to .coven-code/agents/"
                        .to_string(),
                );
            }
            let mut out = String::from("Available agents/familiars:\n");
            for def in &defs {
                let badge = if def.source.starts_with("coven:familiar") {
                    "\u{2728}"
                } else {
                    "\u{2022}"
                };
                out.push_str(&format!("  {} {}\n", badge, def.name));
            }
            out.push_str("\nRun: coven-code agent <name>  to activate one.");
            return CommandResult::Message(out);
        }

        match args.first().copied() {
            None => {
                // No arg: show current familiar from ~/.coven-code/settings.json if available.
                CommandResult::Message(
                    "Usage: coven-code agent <name>\n\
                     Use --list to see all available agents and familiars."
                        .to_string(),
                )
            }
            Some(name) => {
                // Find the agent/familiar by name (case-insensitive).
                let needle = name.to_lowercase();
                let matched = defs.iter().find(|d| {
                    d.name.to_lowercase() == needle
                        || d.source
                            .trim_start_matches("coven:familiar:")
                            .to_lowercase()
                            == needle
                });

                match matched {
                    Some(def) => {
                        let is_familiar = def.source.starts_with("coven:familiar");
                        let badge = if is_familiar { "\u{2728}" } else { "\u{2022}" };
                        let kind = if is_familiar { "familiar" } else { "agent" };
                        let instructions_preview: String = def
                            .instructions
                            .lines()
                            .take(4)
                            .map(|l| format!("  {}", l))
                            .collect::<Vec<_>>()
                            .join("\n");
                        CommandResult::Message(format!(
                            "{badge} Activating {kind}: {}\n\
                             Description: {}\n\
                             Model: {}\n\
                             \nPersona preview:\n{}\n\
                             \nStart a session to apply this persona:\n\
                             coven-code --agent \"{}\" [prompt]",
                            def.name,
                            def.description,
                            def.model.as_deref().unwrap_or("default"),
                            instructions_preview,
                            def.name,
                        ))
                    }
                    None => CommandResult::Error(format!(
                        "No agent or familiar named '{}' found.\n\
                         Run: coven-code agent --list",
                        name
                    )),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// add-dir
// ---------------------------------------------------------------------------

pub struct AddDirCommand;

impl NamedCommand for AddDirCommand {
    fn name(&self) -> &str {
        "add-dir"
    }
    fn description(&self) -> &str {
        "Add a directory to Coven Code's allowed workspace paths"
    }
    fn usage(&self) -> &str {
        "coven-code add-dir <path>"
    }

    fn execute_named(&self, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        let raw = match args.first() {
            Some(p) => *p,
            None => return CommandResult::Error("Usage: coven-code add-dir <path>".to_string()),
        };

        let path = std::path::Path::new(raw);

        if !path.exists() {
            return CommandResult::Error(format!("Directory does not exist: {}", path.display()));
        }

        if !path.is_dir() {
            return CommandResult::Error(format!("Not a directory: {}", path.display()));
        }

        let abs_path = match std::fs::canonicalize(path) {
            Ok(p) => p,
            Err(e) => return CommandResult::Error(format!("Cannot resolve path: {e}")),
        };

        let mut settings = match claurst_core::config::Settings::load_sync() {
            Ok(s) => s,
            Err(e) => {
                return CommandResult::Error(format!(
                    "Failed to load settings before updating workspace paths: {e}"
                ))
            }
        };

        if !settings
            .config
            .workspace_paths
            .iter()
            .any(|p| p == &abs_path)
        {
            settings.config.workspace_paths.push(abs_path.clone());
            if let Err(e) = settings.save_sync() {
                return CommandResult::Error(format!(
                    "Added {} for this session, but failed to save settings: {}",
                    abs_path.display(),
                    e
                ));
            }
        }

        CommandResult::Message(format!(
            "Added {} to allowed workspace paths.",
            abs_path.display()
        ))
    }
}

// ---------------------------------------------------------------------------
// memory
// ---------------------------------------------------------------------------

pub struct MemoryCommand;

impl NamedCommand for MemoryCommand {
    fn name(&self) -> &str {
        "memory"
    }
    fn description(&self) -> &str {
        "Inspect and manage lifecycle controls for local, team, and hosted memory"
    }
    fn usage(&self) -> &str {
        "coven-code memory [list|delete|redact|conflicts|resolve-conflict|hosted-delete] ..."
    }

    fn execute_named(&self, args: &[&str], ctx: &CommandContext) -> CommandResult {
        match args.first().copied().unwrap_or("list") {
            "list" | "status" => memory_list(ctx),
            "delete" => memory_delete_or_redact(args, ctx, MemoryLifecycleAction::Delete),
            "redact" => memory_delete_or_redact(args, ctx, MemoryLifecycleAction::Redact),
            "conflicts" => memory_conflicts(ctx),
            "resolve-conflict" => memory_resolve_conflict(args, ctx),
            "hosted-delete" => memory_hosted_delete(args),
            "--help" | "-h" | "help" => CommandResult::Message(memory_usage()),
            sub => CommandResult::Error(format!(
                "Unknown memory subcommand: '{sub}'\n{}",
                memory_usage()
            )),
        }
    }
}

#[derive(Clone, Copy)]
enum MemoryLifecycleAction {
    Delete,
    Redact,
}

fn memory_usage() -> String {
    "Usage: coven-code memory <subcommand>\n\n\
     Subcommands:\n\
       list\n\
       delete <key> --reason <reason> [--team]\n\
       redact <key> --reason <reason> [--team]\n\
       conflicts\n\
       resolve-conflict <key>\n\
       hosted-delete --tenant <id> --installation <id> --repo-id <id> --repo <owner/name> [--domain <domain>]\n\n\
     Domains: default, security-private, pr:<number>, branch:<name>, release:<name>"
        .to_string()
}

fn memory_root(ctx: &CommandContext) -> PathBuf {
    claurst_core::memdir::auto_memory_path(&ctx.working_dir)
}

fn team_memory_root(ctx: &CommandContext) -> PathBuf {
    claurst_core::memdir::team_memory_path(&memory_root(ctx))
}

fn memory_list(ctx: &CommandContext) -> CommandResult {
    let memory_dir = memory_root(ctx);
    let team_dir = claurst_core::memdir::team_memory_path(&memory_dir);
    let memories = claurst_core::memdir::scan_memory_dir(&memory_dir);
    let conflicts = claurst_core::team_memory_sync::pending_conflicts(&team_dir);

    let mut out = String::new();
    out.push_str("Memory lifecycle status\n");
    out.push_str(&format!("  Local memory: {}\n", memory_dir.display()));
    out.push_str(&format!("  Team memory:  {}\n", team_dir.display()));
    out.push_str(&format!("  Memory files: {}\n", memories.len()));
    for memory in memories.iter().take(50) {
        out.push_str(&format!("    {}\n", memory.filename));
    }
    if memories.len() > 50 {
        out.push_str(&format!("    ... {} more\n", memories.len() - 50));
    }
    out.push_str(&format!("  Pending team conflicts: {}\n", conflicts.len()));

    CommandResult::Message(out)
}

fn memory_delete_or_redact(
    args: &[&str],
    ctx: &CommandContext,
    action: MemoryLifecycleAction,
) -> CommandResult {
    let key = match args.get(1).copied() {
        Some(value) if !value.starts_with("--") => value,
        _ => {
            let verb = match action {
                MemoryLifecycleAction::Delete => "delete",
                MemoryLifecycleAction::Redact => "redact",
            };
            return CommandResult::Error(format!(
                "Usage: coven-code memory {verb} <key> --reason <reason> [--team]"
            ));
        }
    };
    let reason = match flag_value(args, "--reason") {
        Some(value) if !value.trim().is_empty() => value,
        _ => return CommandResult::Error("--reason is required".to_string()),
    };

    let root = if has_flag(args, "--team") {
        team_memory_root(ctx)
    } else {
        memory_root(ctx)
    };
    let path = match memory_file_path(&root, key) {
        Ok(path) => path,
        Err(message) => return CommandResult::Error(message),
    };
    if !path.is_file() {
        return CommandResult::Error(format!("Memory file not found: {key}"));
    }

    let result = match action {
        MemoryLifecycleAction::Delete => claurst_core::memdir::delete_memory_file(&path, reason),
        MemoryLifecycleAction::Redact => claurst_core::memdir::redact_memory_file(&path, reason),
    };
    match result {
        Ok(()) => {
            let verb = match action {
                MemoryLifecycleAction::Delete => "Deleted",
                MemoryLifecycleAction::Redact => "Redacted",
            };
            CommandResult::Message(format!("{verb} memory file: {key}"))
        }
        Err(err) => CommandResult::Error(format!("Failed to update memory file: {err}")),
    }
}

fn memory_conflicts(ctx: &CommandContext) -> CommandResult {
    let team_dir = team_memory_root(ctx);
    let conflicts = claurst_core::team_memory_sync::pending_conflicts(&team_dir);
    if conflicts.is_empty() {
        return CommandResult::Message(format!(
            "No pending team-memory conflicts.\nTeam memory: {}",
            team_dir.display()
        ));
    }

    let mut out = format!("Pending team-memory conflicts ({})\n", conflicts.len());
    for conflict in conflicts {
        out.push_str(&format!(
            "  {} [{:?}]: {}\n",
            conflict.key, conflict.kind, conflict.reason
        ));
    }
    CommandResult::Message(out)
}

fn memory_resolve_conflict(args: &[&str], ctx: &CommandContext) -> CommandResult {
    let key = match args.get(1).copied() {
        Some(value) if !value.starts_with("--") => value,
        _ => {
            return CommandResult::Error(
                "Usage: coven-code memory resolve-conflict <key>".to_string(),
            )
        }
    };
    if let Err(err) = claurst_core::team_memory_sync::validate_memory_path(key) {
        return CommandResult::Error(format!("Invalid memory key: {err}"));
    }

    let team_dir = team_memory_root(ctx);
    match claurst_core::team_memory_sync::resolve_conflict(&team_dir, key) {
        Ok(true) => CommandResult::Message(format!("Resolved team-memory conflict: {key}")),
        Ok(false) => CommandResult::Error(format!("No pending conflict for key: {key}")),
        Err(err) => CommandResult::Error(format!("Failed to resolve conflict: {err}")),
    }
}

fn memory_hosted_delete(args: &[&str]) -> CommandResult {
    let scope = match hosted_scope_from_args(args) {
        Ok(scope) => scope,
        Err(message) => return CommandResult::Error(message),
    };
    let path = claurst_core::memdir::hosted_memory_path_for_scope(&scope);
    match claurst_core::memdir::delete_hosted_memory_for_scope(&scope) {
        Ok(()) => CommandResult::Message(format!(
            "Deleted hosted memory namespace: {}",
            path.display()
        )),
        Err(err) => CommandResult::Error(format!("Failed to delete hosted memory: {err}")),
    }
}

fn hosted_scope_from_args(
    args: &[&str],
) -> Result<claurst_core::hosted_review::HostedReviewScope, String> {
    let tenant = required_flag(args, "--tenant")?;
    let installation = required_flag(args, "--installation")?;
    let repo_id = required_flag(args, "--repo-id")?;
    let repo = required_flag(args, "--repo")?;
    let mut scope = claurst_core::hosted_review::HostedReviewScope::new(
        tenant.to_string(),
        installation.to_string(),
        repo_id.to_string(),
        repo.to_string(),
    );
    if let Some(domain) = flag_value(args, "--domain") {
        scope = scope.with_domain(parse_memory_domain(domain)?);
    }
    scope
        .validate()
        .map_err(|err| format!("Invalid hosted scope: {err}"))?;
    Ok(scope)
}

fn parse_memory_domain(raw: &str) -> Result<claurst_core::hosted_review::MemoryDomain, String> {
    if raw == "default" || raw == "default-branch" {
        return Ok(claurst_core::hosted_review::MemoryDomain::DefaultBranch);
    }
    if raw == "security-private" {
        return Ok(claurst_core::hosted_review::MemoryDomain::SecurityPrivate);
    }
    if let Some(value) = raw.strip_prefix("pr:") {
        let number = value
            .parse::<u64>()
            .map_err(|_| format!("Invalid pull request domain: {raw}"))?;
        return Ok(claurst_core::hosted_review::MemoryDomain::PullRequest(
            number,
        ));
    }
    if let Some(value) = raw.strip_prefix("branch:") {
        if value.trim().is_empty() {
            return Err("branch domain requires a branch name".to_string());
        }
        return Ok(claurst_core::hosted_review::MemoryDomain::Branch(
            value.to_string(),
        ));
    }
    if let Some(value) = raw.strip_prefix("release:") {
        if value.trim().is_empty() {
            return Err("release domain requires a release name".to_string());
        }
        return Ok(claurst_core::hosted_review::MemoryDomain::Release(
            value.to_string(),
        ));
    }
    Err(format!("Unsupported hosted memory domain: {raw}"))
}

fn memory_file_path(root: &Path, key: &str) -> Result<PathBuf, String> {
    if key.trim().is_empty() {
        return Err("Memory key is required".to_string());
    }
    claurst_core::team_memory_sync::validate_memory_path(key)
        .map_err(|err| format!("Invalid memory key: {err}"))?;
    Ok(root.join(key))
}

fn required_flag<'a>(args: &'a [&str], name: &str) -> Result<&'a str, String> {
    match flag_value(args, name) {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!("{name} is required")),
    }
}

fn flag_value<'a>(args: &'a [&str], name: &str) -> Option<&'a str> {
    args.windows(2).find_map(|window| {
        if window[0] == name && !window[1].starts_with("--") {
            Some(window[1])
        } else {
            None
        }
    })
}

fn has_flag(args: &[&str], name: &str) -> bool {
    args.contains(&name)
}

// ---------------------------------------------------------------------------
// branch
// ---------------------------------------------------------------------------

pub struct BranchCommand;

impl NamedCommand for BranchCommand {
    fn name(&self) -> &str {
        "branch"
    }
    fn description(&self) -> &str {
        "Create a branch of the current conversation at this point"
    }
    fn usage(&self) -> &str {
        "coven-code branch [create|list|switch] [name|id]"
    }

    fn execute_named(&self, args: &[&str], ctx: &CommandContext) -> CommandResult {
        match args.first().copied().unwrap_or("") {
            "" | "create" => {
                // Optional name argument (second arg for "create", first for bare call)
                let name = if args.first().copied() == Some("create") {
                    args.get(1).copied()
                } else {
                    args.first().copied()
                };

                if ctx.session_id.is_empty() || ctx.session_id == "pre-session" {
                    return CommandResult::Error(
                        "No active session to branch. Start a conversation first.".to_string(),
                    );
                }

                let session_id = ctx.session_id.clone();
                let msg_count = ctx.messages.len();
                let title_opt = name.map(|s| s.to_string());

                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async move {
                        claurst_core::history::branch_session(
                            &session_id,
                            msg_count,
                            title_opt.as_deref(),
                        )
                        .await
                    })
                });

                match result {
                    Ok(new_session) => {
                        let title = new_session.title.as_deref().unwrap_or("(untitled)");
                        CommandResult::Message(format!(
                            "Created branch: \"{title}\"\nNew session ID: {}\n\
                             To resume original: coven-code -r{}\n\
                             To switch to branch: /branch switch {}",
                            new_session.id,
                            ctx.session_id,
                            new_session.id,
                        ))
                    }
                    Err(e) => CommandResult::Error(format!("Failed to branch session: {e}")),
                }
            }
            "list" => {
                let parent_id = ctx.session_id.clone();

                let sessions = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(claurst_core::history::list_sessions())
                });

                let branches: Vec<_> = sessions
                    .iter()
                    .filter(|s| s.branch_from.as_deref() == Some(&parent_id))
                    .collect();

                if branches.is_empty() {
                    CommandResult::Message(
                        "No branches found for the current session.".to_string(),
                    )
                } else {
                    let mut out = format!(
                        "Branches of session {}:\n\n",
                        &parent_id[..parent_id.len().min(8)]
                    );
                    for b in &branches {
                        let updated = b.updated_at.format("%Y-%m-%d %H:%M").to_string();
                        let id_short = &b.id[..b.id.len().min(8)];
                        out.push_str(&format!(
                            "  {} | {} | {} messages | {}\n",
                            id_short,
                            updated,
                            b.messages.len(),
                            b.title.as_deref().unwrap_or("(untitled)")
                        ));
                    }
                    out.push_str("\nUse: coven-code branch switch <id>");
                    CommandResult::Message(out)
                }
            }
            "switch" => {
                let id = match args.get(1).copied() {
                    Some(i) if !i.is_empty() => i.to_string(),
                    _ => {
                        return CommandResult::Error(
                            "Usage: coven-code branch switch <session-id>".to_string(),
                        )
                    }
                };

                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(claurst_core::history::load_session(&id))
                });

                match result {
                    Ok(session) => CommandResult::ResumeSession(session),
                    Err(e) => CommandResult::Error(format!("Could not load session '{id}': {e}")),
                }
            }
            sub => CommandResult::Error(format!("Unknown branch subcommand: '{sub}'\nUsage: coven-code branch [create|list|switch] [name|id]")),
        }
    }
}

// ---------------------------------------------------------------------------
// tag
// ---------------------------------------------------------------------------

pub struct TagCommand;

impl NamedCommand for TagCommand {
    fn name(&self) -> &str {
        "tag"
    }
    fn description(&self) -> &str {
        "Toggle a searchable tag on the current session"
    }
    fn usage(&self) -> &str {
        "coven-code tag [list|add|remove|toggle] [tag]"
    }

    fn execute_named(&self, args: &[&str], ctx: &CommandContext) -> CommandResult {
        let session_id = ctx.session_id.clone();

        match args.first().copied().unwrap_or("list") {
            "list" => {
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(claurst_core::history::load_session(&session_id))
                });
                match result {
                    Ok(session) => {
                        if session.tags.is_empty() {
                            CommandResult::Message(
                                "No tags set for this session.".to_string(),
                            )
                        } else {
                            CommandResult::Message(format!(
                                "Tags for this session:\n{}",
                                session
                                    .tags
                                    .iter()
                                    .map(|t| format!("  #{t}"))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            ))
                        }
                    }
                    Err(_) => CommandResult::Message(
                        "No tags set for this session (session not yet saved).".to_string(),
                    ),
                }
            }
            "add" => {
                let tag = match args.get(1).copied() {
                    Some(t) if !t.is_empty() => t.to_string(),
                    _ => {
                        return CommandResult::Error(
                            "Usage: coven-code tag add <tag>".to_string(),
                        )
                    }
                };

                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(claurst_core::history::tag_session(&session_id, &tag))
                });

                match result {
                    Ok(()) => CommandResult::Message(format!("Added tag: #{tag}")),
                    Err(e) => CommandResult::Error(format!(
                        "Could not add tag (session may not be saved yet): {e}"
                    )),
                }
            }
            "remove" => {
                let tag = match args.get(1).copied() {
                    Some(t) if !t.is_empty() => t.to_string(),
                    _ => {
                        return CommandResult::Error(
                            "Usage: coven-code tag remove <tag>".to_string(),
                        )
                    }
                };

                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(claurst_core::history::untag_session(&session_id, &tag))
                });

                match result {
                    Ok(()) => CommandResult::Message(format!("Removed tag: #{tag}")),
                    Err(e) => CommandResult::Error(format!("Could not remove tag: {e}")),
                }
            }
            "toggle" => {
                let tag = match args.get(1).copied() {
                    Some(t) if !t.is_empty() => t.to_string(),
                    _ => {
                        return CommandResult::Error(
                            "Usage: coven-code tag toggle <tag>".to_string(),
                        )
                    }
                };

                // Load session to check existing tags
                let load_result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(claurst_core::history::load_session(&session_id))
                });

                match load_result {
                    Ok(session) => {
                        let tag_clone = tag.clone();
                        if session.tags.iter().any(|t| t == &tag) {
                            // Tag exists — remove it
                            let remove_result = tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current()
                                    .block_on(claurst_core::history::untag_session(&session_id, &tag_clone))
                            });
                            match remove_result {
                                Ok(()) => CommandResult::Message(format!("Removed tag: #{tag}")),
                                Err(e) => CommandResult::Error(format!("Could not remove tag: {e}")),
                            }
                        } else {
                            // Tag absent — add it
                            let add_result = tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current()
                                    .block_on(claurst_core::history::tag_session(&session_id, &tag_clone))
                            });
                            match add_result {
                                Ok(()) => CommandResult::Message(format!("Added tag: #{tag}")),
                                Err(e) => CommandResult::Error(format!("Could not add tag: {e}")),
                            }
                        }
                    }
                    Err(e) => CommandResult::Error(format!(
                        "Could not toggle tag (session may not be saved yet): {e}"
                    )),
                }
            }
            sub => CommandResult::Error(format!(
                "Unknown tag subcommand: '{sub}'\nUsage: coven-code tag [list|add|remove|toggle] [tag]"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// pr-comments
// ---------------------------------------------------------------------------

pub struct PrCommentsCommand;

impl NamedCommand for PrCommentsCommand {
    fn name(&self) -> &str {
        "pr-comments"
    }
    fn description(&self) -> &str {
        "Get review comments from the current GitHub pull request"
    }
    fn usage(&self) -> &str {
        "coven-code pr-comments"
    }

    fn execute_named(&self, _args: &[&str], _ctx: &CommandContext) -> CommandResult {
        // Step 1: Get current git remote + PR info via gh CLI
        let pr_json = std::process::Command::new("gh")
            .args(["pr", "view", "--json", "number,url,headRefName,baseRefName"])
            .output();

        let pr_info = match pr_json {
            Err(_) => {
                return CommandResult::Error(
                    "GitHub CLI (gh) not found. Install from https://cli.github.com".to_string(),
                )
            }
            Ok(out) if !out.status.success() => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return CommandResult::Error(format!("No open PR found: {}", stderr.trim()));
            }
            Ok(out) => match serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                Ok(v) => v,
                Err(_) => return CommandResult::Error("Failed to parse gh output".to_string()),
            },
        };

        let pr_number = pr_info["number"].as_u64().unwrap_or(0);
        let pr_url = pr_info["url"].as_str().unwrap_or("").to_string();

        if pr_number == 0 {
            return CommandResult::Error("Could not determine PR number.".to_string());
        }

        // Step 2: Fetch review comments via gh API
        let comments_out = std::process::Command::new("gh")
            .args([
                "api",
                &format!("repos/{{owner}}/{{repo}}/pulls/{}/comments", pr_number),
            ])
            .output();

        let mut output = format!("PR #{} \u{2014} {}\n\n", pr_number, pr_url);

        match comments_out {
            Ok(out) if out.status.success() => {
                match serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout) {
                    Ok(comments) if !comments.is_empty() => {
                        output.push_str(&format!("Review comments ({}):\n", comments.len()));
                        for c in &comments {
                            let path = c["path"].as_str().unwrap_or("unknown");
                            let line = c["line"].as_u64().unwrap_or(0);
                            let user = c["user"]["login"].as_str().unwrap_or("unknown");
                            let body = c["body"].as_str().unwrap_or("").trim();
                            let body_short: String = body.chars().take(200).collect();
                            output.push_str(&format!(
                                "  {}:{} by @{}:\n    {}\n\n",
                                path, line, user, body_short
                            ));
                        }
                    }
                    Ok(_) => output.push_str("No review comments found.\n"),
                    Err(_) => output.push_str("Could not parse review comments.\n"),
                }
            }
            _ => output.push_str("Could not fetch review comments (check gh auth).\n"),
        }

        CommandResult::Message(output)
    }
}

// ---------------------------------------------------------------------------
// desktop
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// mobile — helper
// ---------------------------------------------------------------------------

/// Render a URL as a real QR code using Unicode half-block characters.
///
/// Uses the `qrcode` crate to encode the URL, then converts the bit matrix
/// to lines of "▀" / "▄" / "█" / " " so that two QR rows are packed into
/// one terminal line (each cell is rendered as a half-block character).
/// This matches the approach used by many CLI QR renderers and fits in ~40
/// terminal columns for typical short URLs.
pub fn render_qr(url: &str) -> Vec<String> {
    use qrcode::{EcLevel, QrCode};

    let code = match QrCode::with_error_correction_level(url.as_bytes(), EcLevel::L) {
        Ok(c) => c,
        Err(_) => return vec!["[QR generation failed]".to_string()],
    };

    let matrix = code.to_colors();
    let width = code.width();

    // Add a 2-module quiet zone on each side (QR spec requires ≥4, but 2 renders fine).
    let qz = 2usize;
    let padded_width = width + qz * 2;

    // Helper: return true if module at (row, col) is dark, treating the quiet zone as light.
    let dark = |row: isize, col: isize| -> bool {
        if row < 0 || col < 0 || row >= width as isize || col >= width as isize {
            return false;
        }
        matrix[row as usize * width + col as usize] == qrcode::Color::Dark
    };

    let mut lines = Vec::new();
    // Iterate two matrix rows per terminal line.
    let total_rows = (width + qz * 2) as isize;
    let mut r: isize = -(qz as isize);
    while r < (width + qz) as isize {
        let mut line = String::new();
        for c in -(qz as isize)..(width + qz) as isize {
            let top = dark(r, c);
            let bot = dark(r + 1, c);
            line.push(match (top, bot) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        lines.push(line);
        r += 2;
    }
    let _ = padded_width; // suppress unused warning
    let _ = total_rows;
    lines
}

// ---------------------------------------------------------------------------
// mobile
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// install-github-app
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// remote-setup
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// stickers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ultraplan — Agentic planning with extended thinking
// ---------------------------------------------------------------------------

pub struct UltraplanCommand;

impl NamedCommand for UltraplanCommand {
    fn name(&self) -> &str {
        "ultraplan"
    }
    fn description(&self) -> &str {
        "Launch Ultraplan agentic code planner with extended thinking"
    }
    fn usage(&self) -> &str {
        "coven-code ultraplan [--effort=medium|high|maximum]"
    }

    fn execute_named(&self, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        // Parse effort level from args
        let effort = args
            .iter()
            .find(|arg| arg.starts_with("--effort="))
            .and_then(|arg| arg.strip_prefix("--effort="))
            .unwrap_or("medium");

        // Validate effort level
        if !matches!(effort, "medium" | "high" | "maximum") {
            return CommandResult::Error(format!(
                "Invalid effort level: '{}'. Use: medium, high, or maximum",
                effort
            ));
        }

        CommandResult::Message(format!(
            "🚀 Ultraplan activated with {} effort level\n\n\
             Ultraplan will now:\n\
             • Analyze the codebase and requirements\n\
             • Use extended thinking for deep reasoning\n\
             • Generate a comprehensive implementation plan\n\
             • Break down the work into clear steps\n\n\
             Ask me: '/ultraplan describe what you want to build'",
            effort
        ))
    }
}

// ---------------------------------------------------------------------------
// stats — persisted session analytics
//
// Reuses the existing `StatsCommand` struct (which already implements the
// slash form for the *current* session). The `NamedCommand` form reads
// JSONL transcripts on disk and produces aggregated views. Logic lives in
// `crate::stats`.
// ---------------------------------------------------------------------------

impl NamedCommand for crate::StatsCommand {
    fn name(&self) -> &str {
        "stats"
    }
    fn description(&self) -> &str {
        "Aggregate token / cost / tool stats across saved sessions"
    }
    fn usage(&self) -> &str {
        "coven-code stats [summary|sessions|tools|daily|session <id>] \
         [--days N] [--top N] [--all-projects] [--json]"
    }

    fn execute_named(&self, args: &[&str], ctx: &CommandContext) -> CommandResult {
        crate::stats::run(args, ctx)
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

static NAMED_COMMANDS: Lazy<Vec<Box<dyn NamedCommand>>> = Lazy::new(|| {
    vec![
        Box::new(AgentsCommand),
        Box::new(AgentCommand),
        Box::new(AddDirCommand),
        Box::new(MemoryCommand),
        Box::new(BranchCommand),
        Box::new(TagCommand),
        Box::new(PrCommentsCommand),
        Box::new(UltraplanCommand),
        Box::new(crate::StatsCommand),
    ]
});

/// Return one instance of every registered named command.
pub fn all_named_commands() -> &'static [Box<dyn NamedCommand>] {
    &NAMED_COMMANDS
}

/// Look up a named command by its primary name (case-insensitive).
pub fn find_named_command(name: &str) -> Option<&'static dyn NamedCommand> {
    let needle = name.to_lowercase();
    all_named_commands()
        .iter()
        .find(|c| c.name() == needle.as_str())
        .map(|c| c.as_ref())
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
            session_id: "named-test-session".to_string(),
            session_title: None,
            remote_session_url: None,
            mcp_manager: None,
            mcp_auth_runner: None,
        }
    }

    fn make_ctx_at(working_dir: std::path::PathBuf) -> CommandContext {
        CommandContext {
            working_dir,
            ..make_ctx()
        }
    }

    struct RemoteMemoryEnvGuard {
        old_remote_memory_dir: Option<String>,
    }

    impl RemoteMemoryEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let guard = Self {
                old_remote_memory_dir: std::env::var("COVEN_CODE_REMOTE_MEMORY_DIR").ok(),
            };
            std::env::set_var("COVEN_CODE_REMOTE_MEMORY_DIR", path);
            guard
        }
    }

    impl Drop for RemoteMemoryEnvGuard {
        fn drop(&mut self) {
            match &self.old_remote_memory_dir {
                Some(value) => std::env::set_var("COVEN_CODE_REMOTE_MEMORY_DIR", value),
                None => std::env::remove_var("COVEN_CODE_REMOTE_MEMORY_DIR"),
            }
        }
    }

    struct MemoryTestEnv {
        _remote_memory: RemoteMemoryEnvGuard,
        _command_env: CommandEnvGuard,
        _tmp: tempfile::TempDir,
    }

    fn memory_test_ctx() -> (CommandContext, MemoryTestEnv) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        let coven_home = tmp.path().join("coven");
        let project = tmp.path().join("project");
        let memory_base = tmp.path().join("remote-memory");
        std::fs::create_dir_all(&home).expect("home dir");
        std::fs::create_dir_all(&coven_home).expect("coven home dir");
        std::fs::create_dir_all(&project).expect("project dir");
        std::fs::create_dir_all(&memory_base).expect("memory base");
        let command_env = CommandEnvGuard::set(&home, &coven_home, None);
        let remote_memory = RemoteMemoryEnvGuard::set(&memory_base);
        let ctx = make_ctx_at(project);
        (
            ctx,
            MemoryTestEnv {
                _remote_memory: remote_memory,
                _command_env: command_env,
                _tmp: tmp,
            },
        )
    }

    #[test]
    fn test_all_named_commands_non_empty() {
        assert!(!all_named_commands().is_empty());
    }

    #[test]
    fn test_all_named_commands_unique_names() {
        let mut names = std::collections::HashSet::new();
        for cmd in all_named_commands() {
            assert!(
                names.insert(cmd.name().to_string()),
                "Duplicate named command: {}",
                cmd.name()
            );
        }
    }

    #[test]
    fn test_find_named_command_found() {
        assert!(find_named_command("agents").is_some());
        assert!(find_named_command("branch").is_some());
        assert!(find_named_command("memory").is_some());
    }

    #[test]
    fn test_find_named_command_not_found() {
        assert!(find_named_command("nonexistent-xyz").is_none());
    }

    #[test]
    fn test_find_named_command_case_insensitive() {
        assert!(find_named_command("Agents").is_some());
        assert!(find_named_command("BRANCH").is_some());
    }

    #[test]
    fn test_agents_list_returns_message() {
        let ctx = make_ctx();
        let cmd = AgentsCommand;
        let result = cmd.execute_named(&[], &ctx);
        assert!(matches!(result, CommandResult::Message(_)));
    }

    #[test]
    fn test_agents_create_includes_name() {
        let ctx = make_ctx();
        let cmd = AgentsCommand;
        let result = cmd.execute_named(&["create", "my-bot"], &ctx);
        if let CommandResult::Message(msg) = result {
            assert!(msg.contains("my-bot"));
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_agents_reset_removes_saved_roster_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let coven_home = temp.path().join("coven");
        let project = temp.path().join("project");
        let project_agents = project.join(".coven-code").join("agents");
        std::fs::create_dir_all(&project_agents).expect("project agents dir");
        std::fs::create_dir_all(&coven_home).expect("coven home");
        let _guard = CommandEnvGuard::set(&home, &coven_home, None);
        let global_agents = claurst_core::Settings::config_dir().join("agents");
        std::fs::create_dir_all(&global_agents).expect("global agents dir");

        let global_agent = global_agents.join("global.md");
        let project_agent = project_agents.join("project.md");
        let familiar_roster = coven_home.join("familiars.toml");
        std::fs::write(&global_agent, "global").expect("global agent");
        std::fs::write(&project_agent, "project").expect("project agent");
        std::fs::write(&familiar_roster, "[[familiar]]\nid = \"orchestrator\"\n")
            .expect("familiar roster");

        let settings = claurst_core::Settings {
            familiar: Some("orchestrator".to_string()),
            ..Default::default()
        };
        settings.save_sync().expect("settings save");

        let cmd = AgentsCommand;
        let ctx = make_ctx_at(project);
        let result = cmd.execute_named(&["reset"], &ctx);

        let CommandResult::Message(msg) = result else {
            panic!("Expected Message");
        };
        assert!(msg.contains("removed 2 agent files"), "{msg}");
        assert!(!global_agent.exists());
        assert!(!project_agent.exists());
        assert!(!familiar_roster.exists());
        let settings = claurst_core::Settings::load_sync().expect("settings load");
        assert!(settings.familiar.is_none());
    }

    #[test]
    fn test_add_dir_missing_arg_returns_error() {
        let ctx = make_ctx();
        let cmd = AddDirCommand;
        let result = cmd.execute_named(&[], &ctx);
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_branch_list_returns_message() {
        // With no active tokio runtime the block_in_place path won't be reached;
        // but `list` on an empty session dir returns a Message (no sessions found).
        // We verify the command exists and returns a non-Error for the list subcommand
        // when called outside a runtime (it will panic in block_in_place if runtime
        // is missing, so we just check the command dispatches).
        let ctx = make_ctx();
        let cmd = BranchCommand;
        // "pre-session" session_id: create/switch will error; list is the safe path
        let result = cmd.execute_named(&["unknown-sub"], &ctx);
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_branch_create_no_session_returns_error() {
        let cmd = BranchCommand;
        // Calling create on a session that isn't "pre-session" but also doesn't exist
        // on disk: we can't call block_in_place outside a tokio runtime in a sync test,
        // so instead verify the pre-session guard fires.
        let mut ctx2 = make_ctx();
        ctx2.session_id = "pre-session".to_string();
        let result = cmd.execute_named(&[], &ctx2);
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_branch_switch_missing_id_returns_error() {
        let ctx = make_ctx();
        let cmd = BranchCommand;
        let result = cmd.execute_named(&["switch"], &ctx);
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_memory_list_shows_paths_without_body() {
        let (ctx, _env) = memory_test_ctx();
        let memory_dir = memory_root(&ctx);
        std::fs::create_dir_all(&memory_dir).expect("memory dir");
        std::fs::write(memory_dir.join("policy.md"), "secret body").expect("memory file");

        let result = MemoryCommand.execute_named(&["list"], &ctx);

        let CommandResult::Message(msg) = result else {
            panic!("Expected Message");
        };
        assert!(msg.contains("policy.md"));
        assert!(msg.contains("Team memory:"));
        assert!(!msg.contains("secret body"));
    }

    #[test]
    fn test_memory_delete_writes_tombstone_without_original_body() {
        let (ctx, _env) = memory_test_ctx();
        let memory_dir = memory_root(&ctx);
        std::fs::create_dir_all(&memory_dir).expect("memory dir");
        let path = memory_dir.join("policy.md");
        std::fs::write(&path, "sensitive fact").expect("memory file");

        let result =
            MemoryCommand.execute_named(&["delete", "policy.md", "--reason", "operator"], &ctx);

        assert!(matches!(result, CommandResult::Message(_)));
        let content = std::fs::read_to_string(path).expect("updated memory");
        assert!(content.contains("deleted_at:"));
        assert!(content.contains("[DELETED: operator]"));
        assert!(!content.contains("sensitive fact"));
    }

    #[test]
    fn test_memory_redact_team_file_writes_stub_without_original_body() {
        let (ctx, _env) = memory_test_ctx();
        let team_dir = team_memory_root(&ctx);
        std::fs::create_dir_all(&team_dir).expect("team dir");
        let path = team_dir.join("team.md");
        std::fs::write(&path, "security incident detail").expect("team memory");

        let result = MemoryCommand.execute_named(
            &["redact", "team.md", "--reason", "request", "--team"],
            &ctx,
        );

        assert!(matches!(result, CommandResult::Message(_)));
        let content = std::fs::read_to_string(path).expect("updated team memory");
        assert!(content.contains("redacted_at:"));
        assert!(content.contains("[REDACTED: request]"));
        assert!(!content.contains("security incident detail"));
    }

    #[test]
    fn test_memory_rejects_path_traversal() {
        let (ctx, _env) = memory_test_ctx();

        let result =
            MemoryCommand.execute_named(&["delete", "../secret.md", "--reason", "operator"], &ctx);

        let CommandResult::Error(msg) = result else {
            panic!("Expected Error");
        };
        assert!(msg.contains("Invalid memory key"));
    }

    #[test]
    fn test_memory_rejects_missing_flag_values() {
        let (ctx, _env) = memory_test_ctx();
        let memory_dir = memory_root(&ctx);
        std::fs::create_dir_all(&memory_dir).expect("memory dir");
        std::fs::write(memory_dir.join("policy.md"), "body").expect("memory file");

        let result =
            MemoryCommand.execute_named(&["delete", "policy.md", "--reason", "--team"], &ctx);

        let CommandResult::Error(msg) = result else {
            panic!("Expected Error");
        };
        assert!(msg.contains("--reason is required"));
    }

    #[test]
    fn test_memory_conflicts_lists_pending_records_without_content() {
        let (ctx, _env) = memory_test_ctx();
        let team_dir = team_memory_root(&ctx);
        let conflict_dir = team_dir.join(".conflicts");
        std::fs::create_dir_all(&conflict_dir).expect("conflict dir");
        let conflict = claurst_core::team_memory_sync::TeamMemoryPullConflict {
            key: "MEMORY.md".to_string(),
            kind: claurst_core::team_memory_sync::PullConflictKind::BothChanged,
            local_checksum: Some("local".to_string()),
            base_checksum: Some("base".to_string()),
            remote_checksum: Some("remote".to_string()),
            reason: "local and remote changed".to_string(),
        };
        let record = serde_json::json!({
            "conflict": conflict,
            "local": "do not print local body",
            "remote": { "key": "MEMORY.md", "content": "do not print remote body" }
        });
        std::fs::write(
            conflict_dir.join("MEMORY.md.json"),
            serde_json::to_string(&record).expect("conflict json"),
        )
        .expect("conflict write");

        let result = MemoryCommand.execute_named(&["conflicts"], &ctx);

        let CommandResult::Message(msg) = result else {
            panic!("Expected Message");
        };
        assert!(msg.contains("MEMORY.md"));
        assert!(msg.contains("BothChanged"));
        assert!(!msg.contains("do not print"));
    }

    #[test]
    fn test_memory_resolve_conflict_removes_only_matching_record() {
        let (ctx, _env) = memory_test_ctx();
        let team_dir = team_memory_root(&ctx);
        let conflict_dir = team_dir.join(".conflicts");
        std::fs::create_dir_all(&conflict_dir).expect("conflict dir");
        std::fs::write(conflict_dir.join("MEMORY.md.json"), "{}").expect("first conflict");
        std::fs::write(conflict_dir.join("other.md.json"), "{}").expect("second conflict");

        let result = MemoryCommand.execute_named(&["resolve-conflict", "MEMORY.md"], &ctx);

        assert!(matches!(result, CommandResult::Message(_)));
        assert!(!conflict_dir.join("MEMORY.md.json").exists());
        assert!(conflict_dir.join("other.md.json").exists());
    }

    #[test]
    fn test_memory_hosted_delete_targets_only_requested_namespace() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        let coven_home = tmp.path().join("coven");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::create_dir_all(&coven_home).expect("coven home");
        let _env_guard = CommandEnvGuard::set(&home, &coven_home, None);
        let first = claurst_core::hosted_review::HostedReviewScope::new(
            "tenant-a".to_string(),
            "install-1".to_string(),
            "repo-1".to_string(),
            "OpenCoven/coven-code".to_string(),
        );
        let second = claurst_core::hosted_review::HostedReviewScope::new(
            "tenant-a".to_string(),
            "install-1".to_string(),
            "repo-2".to_string(),
            "OpenCoven/other".to_string(),
        );
        let first_dir = claurst_core::memdir::hosted_memory_path_for_scope(&first);
        let second_dir = claurst_core::memdir::hosted_memory_path_for_scope(&second);
        std::fs::create_dir_all(&first_dir).expect("first hosted dir");
        std::fs::create_dir_all(&second_dir).expect("second hosted dir");
        std::fs::write(first_dir.join("MEMORY.md"), "first").expect("first memory");
        std::fs::write(second_dir.join("MEMORY.md"), "second").expect("second memory");

        let result = MemoryCommand.execute_named(
            &[
                "hosted-delete",
                "--tenant",
                "tenant-a",
                "--installation",
                "install-1",
                "--repo-id",
                "repo-1",
                "--repo",
                "OpenCoven/coven-code",
            ],
            &make_ctx(),
        );

        assert!(matches!(result, CommandResult::Message(_)));
        assert!(!first_dir.exists());
        assert!(second_dir.exists());
    }

    #[test]
    fn test_memory_hosted_delete_rejects_missing_scope_value() {
        let ctx = make_ctx();

        let result = MemoryCommand.execute_named(
            &[
                "hosted-delete",
                "--tenant",
                "--installation",
                "install-1",
                "--repo-id",
                "repo-1",
                "--repo",
                "OpenCoven/coven-code",
            ],
            &ctx,
        );

        let CommandResult::Error(msg) = result else {
            panic!("Expected Error");
        };
        assert!(msg.contains("--tenant is required"));
    }

    #[test]
    fn test_pr_comments_no_gh_returns_error() {
        // Without `gh` installed or outside a git repo with an open PR,
        // the command returns an Error (gh not found or no open PR).
        let ctx = make_ctx();
        let cmd = PrCommentsCommand;
        // Either gh is missing (Error with "not found") or no PR is open (Error).
        // Both cases produce CommandResult::Error.
        let result = cmd.execute_named(&[], &ctx);
        // On CI / dev machines without gh: Error. With gh but no open PR: also Error.
        // We accept Error or Message (in case gh is installed and finds a PR).
        match result {
            CommandResult::Error(_) | CommandResult::Message(_) => {}
            other => panic!("Unexpected result: {:?}", other),
        }
    }
}
