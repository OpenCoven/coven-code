use claurst_core::CommandContextSignal;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Stable groupings for built-in commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandCategory {
    StartNavigate,
    WorkReview,
    Conversation,
    AccountModels,
    Customize,
    Extend,
    CovenAdvanced,
    Other,
}

impl CommandCategory {
    /// Human-readable heading used by command discovery surfaces.
    pub const fn label(self) -> &'static str {
        match self {
            Self::StartNavigate => "Start & Navigate",
            Self::WorkReview => "Work & Review",
            Self::Conversation => "Conversation",
            Self::AccountModels => "Account & Models",
            Self::Customize => "Customize",
            Self::Extend => "Extend",
            Self::CovenAdvanced => "Coven & Advanced",
            Self::Other => "Other",
        }
    }

    /// The stable order of user-facing command groups.
    pub const fn ordered() -> &'static [Self] {
        &[
            Self::StartNavigate,
            Self::WorkReview,
            Self::Conversation,
            Self::AccountModels,
            Self::Customize,
            Self::Extend,
            Self::CovenAdvanced,
        ]
    }

    /// Position in the canonical command-discovery order.
    ///
    /// `Other` is deliberately last because it is not a user-facing section.
    pub fn rank(self) -> usize {
        Self::ordered()
            .iter()
            .position(|category| *category == self)
            .unwrap_or(Self::ordered().len())
    }
}

/// Where a command was defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    BuiltIn,
    User,
    Skill,
    Plugin,
}

/// Availability and migration state of a command path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CommandLifecycle {
    Active,
    Advanced,
    Removed { replacement: String },
}

impl CommandLifecycle {
    /// The path users should move to, if this lifecycle supersedes a command.
    pub fn replacement(&self) -> Option<&str> {
        match self {
            Self::Removed { replacement } => Some(replacement),
            Self::Active | Self::Advanced => None,
        }
    }
}

/// Owned metadata for a canonical command path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub path: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub usage: String,
    pub category: CommandCategory,
    pub keywords: Vec<String>,
    pub lifecycle: CommandLifecycle,
    pub source: CommandSource,
    pub suggest_when: Vec<CommandContextSignal>,
    pub children: Vec<CommandSpec>,
}

impl CommandSpec {
    /// Construct an active built-in command.
    pub fn active(
        path: &str,
        summary: &str,
        usage: &str,
        category: CommandCategory,
        keywords: &[&str],
    ) -> Self {
        Self::with_lifecycle(
            path,
            summary,
            usage,
            category,
            keywords,
            CommandLifecycle::Active,
        )
    }

    /// Construct an advanced built-in command.
    pub fn advanced(
        path: &str,
        summary: &str,
        usage: &str,
        category: CommandCategory,
        keywords: &[&str],
    ) -> Self {
        Self::with_lifecycle(
            path,
            summary,
            usage,
            category,
            keywords,
            CommandLifecycle::Advanced,
        )
    }

    /// Retain detailed replacement guidance for a removed path.
    pub fn removed_with_details(
        path: &str,
        replacement: &str,
        summary: &str,
        usage: &str,
        category: CommandCategory,
        keywords: &[&str],
    ) -> Self {
        Self::with_lifecycle(
            path,
            summary,
            usage,
            category,
            keywords,
            CommandLifecycle::Removed {
                replacement: replacement.to_string(),
            },
        )
    }

    /// Construct metadata for an already-removed compatibility path.
    pub fn removed(path: &str, replacement: &str, category: CommandCategory) -> Self {
        Self::with_lifecycle(
            path,
            "Removed compatibility command",
            &format!("Use /{replacement}"),
            category,
            &["legacy", "compatibility"],
            CommandLifecycle::Removed {
                replacement: replacement.to_string(),
            },
        )
    }

    fn with_lifecycle(
        path: &str,
        summary: &str,
        usage: &str,
        category: CommandCategory,
        keywords: &[&str],
        lifecycle: CommandLifecycle,
    ) -> Self {
        Self {
            path: path.to_string(),
            aliases: Vec::new(),
            summary: summary.to_string(),
            usage: usage.to_string(),
            category,
            keywords: keywords
                .iter()
                .map(|keyword| (*keyword).to_string())
                .collect(),
            lifecycle,
            source: CommandSource::BuiltIn,
            suggest_when: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Add accepted aliases to a focused command specification.
    pub fn with_aliases(mut self, aliases: &[&str]) -> Self {
        self.aliases = aliases.iter().map(|alias| (*alias).to_string()).collect();
        self
    }

    /// Set the context signals that make this command useful.
    pub fn with_suggestions(mut self, signals: &[CommandContextSignal]) -> Self {
        self.suggest_when = signals.to_vec();
        self
    }

    /// Attach canonical subcommand specifications.
    pub fn with_children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }

    /// Set the origin of this specification.
    pub fn with_source(mut self, source: CommandSource) -> Self {
        self.source = source;
        self
    }

    /// Return this command and all of its descendants in path order.
    pub fn flattened(&self) -> Vec<&Self> {
        let mut specs = vec![self];
        for child in &self.children {
            specs.extend(child.flattened());
        }
        specs
    }
}

/// Flatten a catalog, preserving parent-before-child order.
pub fn flatten_catalog(catalog: &[CommandSpec]) -> Vec<&CommandSpec> {
    catalog.iter().flat_map(CommandSpec::flattened).collect()
}

/// Combine catalog layers using slash-command execution precedence.
///
/// A later layer loses the complete entry when either its canonical path or
/// any alias collides with an earlier canonical path or alias.
pub fn compose_catalogs(
    builtin: Vec<CommandSpec>,
    user_templates: Vec<CommandSpec>,
    discovered_skills: Vec<CommandSpec>,
    plugin_commands: Vec<CommandSpec>,
) -> Vec<CommandSpec> {
    let mut seen = HashSet::new();
    let mut composed = Vec::new();

    for spec in builtin
        .into_iter()
        .chain(user_templates)
        .chain(discovered_skills)
        .chain(plugin_commands)
    {
        let identifiers = spec
            .flattened()
            .into_iter()
            .flat_map(|entry| {
                std::iter::once(entry.path.as_str()).chain(entry.aliases.iter().map(String::as_str))
            })
            .collect::<Vec<_>>();
        if identifiers
            .iter()
            .any(|identifier| seen.contains(*identifier))
        {
            continue;
        }
        seen.extend(identifiers.into_iter().map(str::to_string));
        composed.push(spec);
    }

    composed
}

/// Find a canonical path or alias in a catalog.
pub fn find_catalog_spec<'a>(catalog: &'a [CommandSpec], path: &str) -> Option<&'a CommandSpec> {
    let path = path.trim().trim_start_matches('/');
    flatten_catalog(catalog)
        .into_iter()
        .find(|spec| spec.path == path || spec.aliases.iter().any(|alias| alias == path))
}

/// Resolve a registered slash command name to its catalog category.
pub fn category_for_path(path: &str) -> CommandCategory {
    find_catalog_spec(&builtin_catalog_template(), path)
        .map(|spec| spec.category)
        .unwrap_or(CommandCategory::Other)
}

/// Return the stable user-facing category order.
pub const fn category_order() -> &'static [CommandCategory] {
    CommandCategory::ordered()
}

/// Render the checked-in reference from the same metadata used by command discovery.
pub fn render_reference() -> String {
    let catalog = crate::builtin_catalog();
    let entries = flatten_catalog(&catalog);
    let mut output = String::from("# Slash commands\n\nGenerated from the command catalog. Run `cargo run --package claurst-commands --example command_reference` from `src-rust/` to regenerate.\n\nType `/` for contextual suggestions, or open the command palette with Ctrl+K. Enter opens a command group; choose its Run entry to execute the parent. Use `/help <path>` for a command, including grouped paths. Suggestions use local session state and recent commands.\n");
    for category in category_order() {
        output.push_str(&format!(
            "\n## {}\n\n| Command | Usage | Description |\n|---|---|---|\n",
            category.label()
        ));
        for entry in &entries {
            if entry.category != *category
                || matches!(entry.lifecycle, CommandLifecycle::Removed { .. })
            {
                continue;
            }
            output.push_str(&format!(
                "| `/{}` | `{}` | {} |\n",
                entry.path,
                entry.usage.replace('|', "\\|"),
                entry.summary.replace('|', "\\|")
            ));
        }
    }
    output.push_str("\n## Removed paths\n\nThese paths no longer execute. Their replacement remains available.\n\n| Removed path | Use instead |\n|---|---|\n");
    for entry in &entries {
        if let CommandLifecycle::Removed { replacement } = &entry.lifecycle {
            for path in std::iter::once(&entry.path).chain(entry.aliases.iter()) {
                output.push_str(&format!("| `/{path}` | `/{replacement}` |\n"));
            }
        }
    }
    output
}

fn child(
    parent: &str,
    name: &str,
    summary: &str,
    category: CommandCategory,
    keywords: &[&str],
) -> CommandSpec {
    let arguments = match (parent, name) {
        ("account", "login") => " [--console] [--codex] [--label <name>]",
        ("account", "switch") => " [name]",
        ("config", "theme") => " [name]",
        ("config", "splash") => " [show|hide|toggle]",
        ("config", "output-style") => " [style]",
        ("config", "advisor") => " [model|off]",
        ("effort", "fast") => " [on|off]",
        ("session", "rename") => " <name>",
        ("session", "resume") => " [id]",
        ("session", "fork") => " [message_index]",
        ("session", "add-dir") => " <path>",
        ("session", "branch" | "tag") => " [arguments]",
        ("familiar", "handoff") => " <name>",
        ("familiar", "whisper") | ("thinking", "steer") => " <message>",
        ("familiar", "incant" | "managed") => " [arguments]",
        ("coven", "goal") => " [objective|status|pause|resume|clear|complete]",
        ("review", "security" | "ultra") => " [path]",
        ("rewind", "diff") => " [checkpoint]",
        ("help", "feedback") => " [report]",
        ("stash", "link" | "attach") | ("integrations", "mcp") => " [arguments]",
        _ => "",
    };
    CommandSpec::active(
        &format!("{parent} {name}"),
        summary,
        &format!("/{parent} {name}{arguments}"),
        category,
        keywords,
    )
}

/// Return pure built-in catalog metadata with fallback command summaries.
pub(crate) fn builtin_catalog_template() -> Vec<CommandSpec> {
    use CommandCategory::{
        AccountModels, Conversation, CovenAdvanced, Customize, Extend, StartNavigate, WorkReview,
    };
    use CommandContextSignal::{
        CovenDaemonAvailable, EmptySession, FamiliarAvailable, GitDirty, HasBackgroundTasks,
        HasCompletedEdits, HighContextUsage, MissingAgentsFile, RunningTurn, Unauthenticated,
        UpdateAvailable,
    };

    #[cfg(feature = "steer")]
    let thinking_usage = "/thinking <steer|back>";
    #[cfg(not(feature = "steer"))]
    let thinking_usage = "/thinking <back>";
    #[cfg(feature = "steer")]
    let thinking_keywords = &["thinking", "steer", "reasoning"];
    #[cfg(not(feature = "steer"))]
    let thinking_keywords = &["thinking", "reasoning"];
    #[cfg(feature = "steer")]
    let thinking_children = vec![
        child(
            "thinking",
            "steer",
            "Steer a running turn",
            Conversation,
            &["steer"],
        )
        .with_suggestions(&[RunningTurn]),
        child(
            "thinking",
            "back",
            "View earlier thinking traces",
            Conversation,
            &["back"],
        ),
    ];
    #[cfg(not(feature = "steer"))]
    let thinking_children = vec![child(
        "thinking",
        "back",
        "View earlier thinking traces",
        Conversation,
        &["back"],
    )];

    let catalog = vec![
        CommandSpec::advanced(
            "find",
            "Search the current transcript",
            "/find",
            Conversation,
            &["transcript", "search"],
        ),
        CommandSpec::advanced(
            "vim",
            "Toggle Vim input mode",
            "/vim",
            Customize,
            &["editing", "vim"],
        ),
        CommandSpec::advanced(
            "voice",
            "Toggle voice input",
            "/voice",
            Customize,
            &["voice", "dictation"],
        ),
        CommandSpec::removed("cost", "usage cost", AccountModels),
        CommandSpec::removed("context", "usage context", AccountModels),
        CommandSpec::removed("stats", "usage stats", AccountModels),
        CommandSpec::removed("rename", "session rename", StartNavigate),
        CommandSpec::removed("changes", "diff", WorkReview),
        CommandSpec::active(
            "account",
            "Manage accounts and provider connections",
            "/account <login|logout|list|dedupe|providers|connect|switch|refresh>",
            AccountModels,
            &["account", "login", "provider", "authentication"],
        )
        .with_children(vec![
            child(
                "account",
                "login",
                "Sign in to a provider",
                AccountModels,
                &["sign in", "auth"],
            )
            .with_suggestions(&[Unauthenticated]),
            child(
                "account",
                "logout",
                "Sign out of a provider",
                AccountModels,
                &["sign out"],
            ),
            child(
                "account",
                "list",
                "List configured accounts",
                AccountModels,
                &["accounts"],
            ),
            child(
                "account",
                "dedupe",
                "Remove duplicate credentials",
                AccountModels,
                &["credentials"],
            ),
            child(
                "account",
                "providers",
                "List available providers",
                AccountModels,
                &["providers"],
            ),
            child(
                "account",
                "connect",
                "Connect a provider account",
                AccountModels,
                &["connect", "auth"],
            )
            .with_suggestions(&[Unauthenticated, EmptySession, MissingAgentsFile]),
            child(
                "account",
                "switch",
                "Switch the active account",
                AccountModels,
                &["switch"],
            ),
            child(
                "account",
                "refresh",
                "Refresh provider authentication",
                AccountModels,
                &["refresh"],
            ),
        ]),
        CommandSpec::active(
            "clear",
            "Clear the conversation history",
            "/clear",
            StartNavigate,
            &["clear", "reset", "new session"],
        )
        .with_aliases(&["c", "reset", "new"]),
        CommandSpec::active(
            "commit",
            "Create a git commit for completed work",
            "/commit [message]",
            WorkReview,
            &["commit", "git", "changes"],
        )
        .with_suggestions(&[GitDirty, HasCompletedEdits]),
        CommandSpec::active(
            "compact",
            "Compact conversation history to reduce context usage",
            "/compact",
            Conversation,
            &["compact", "context", "tokens"],
        )
        .with_suggestions(&[HighContextUsage]),
        CommandSpec::active(
            "config",
            "Configure Coven behavior and appearance",
            "/config [show|get|set|unset|<area>] ...",
            Customize,
            &["config", "settings", "customize"],
        )
        .with_aliases(&["settings"])
        .with_children(vec![
            child(
                "config",
                "splash",
                "Configure the empty-session splash screen",
                Customize,
                &["splash"],
            ),
            child(
                "config",
                "theme",
                "Select the display theme",
                Customize,
                &["theme", "change theme"],
            ),
            child(
                "config",
                "output-style",
                "Select the response output style",
                Customize,
                &["output"],
            ),
            child(
                "config",
                "keybindings",
                "Open keybinding configuration",
                Customize,
                &["keys"],
            ),
            child(
                "config",
                "import",
                "Import configuration from another tool",
                Customize,
                &["import"],
            ),
            child(
                "config",
                "advisor",
                "Configure the advisor model",
                Customize,
                &["advisor"],
            ),
        ]),
        CommandSpec::active(
            "coven",
            "Control Coven daemon features",
            "/coven [status|sessions|familiars|log|run|daemon|goal|help] ...",
            CovenAdvanced,
            &["coven", "daemon", "advanced"],
        )
        .with_suggestions(&[CovenDaemonAvailable])
        .with_children(vec![child(
            "coven",
            "goal",
            "Manage durable session goals",
            CovenAdvanced,
            &["goal", "objective"],
        )]),
        CommandSpec::active(
            "diff",
            "Show uncommitted changes",
            "/diff",
            WorkReview,
            &["diff", "git", "changes"],
        )
        .with_suggestions(&[GitDirty, HasCompletedEdits]),
        CommandSpec::active(
            "effort",
            "Configure model reasoning effort",
            "/effort [low|normal|high|fast [on|off]]",
            Customize,
            &["effort", "reasoning", "speed"],
        )
        .with_children(vec![child(
            "effort",
            "fast",
            "Enable fast response mode",
            Customize,
            &["fast", "speed"],
        )]),
        CommandSpec::active(
            "exit",
            "Exit Coven",
            "/exit",
            StartNavigate,
            &["exit", "quit"],
        )
        .with_aliases(&["quit", "q"]),
        CommandSpec::active(
            "export",
            "Export or share the current conversation",
            "/export [copy [n]|share] [--format markdown|json] [--output <file>]",
            Conversation,
            &["export", "share", "copy"],
        )
        .with_children(vec![
            child(
                "export",
                "share",
                "Create a shareable session export",
                Conversation,
                &["share"],
            ),
            child(
                "export",
                "copy",
                "Copy the session export to the clipboard",
                Conversation,
                &["copy"],
            ),
        ]),
        CommandSpec::active(
            "familiar",
            "Work with Coven familiars",
            "/familiar <incant|whisper|handoff|list|managed>",
            CovenAdvanced,
            &["familiar", "agent", "subagent"],
        )
        .with_aliases(&["familiars", "agent"])
        .with_suggestions(&[EmptySession, FamiliarAvailable])
        .with_children(vec![
            child(
                "familiar",
                "incant",
                "Run a familiar incantation",
                CovenAdvanced,
                &["incant"],
            ),
            child(
                "familiar",
                "whisper",
                "Ask a familiar side question",
                CovenAdvanced,
                &["whisper"],
            )
            .with_suggestions(&[FamiliarAvailable]),
            child(
                "familiar",
                "handoff",
                "Hand work to a familiar",
                CovenAdvanced,
                &["handoff"],
            )
            .with_suggestions(&[FamiliarAvailable, CovenDaemonAvailable]),
            child(
                "familiar",
                "list",
                "List available familiars",
                CovenAdvanced,
                &["list", "agents"],
            ),
            child(
                "familiar",
                "managed",
                "Manage familiar execution",
                CovenAdvanced,
                &["managed"],
            ),
        ]),
        CommandSpec::active(
            "help",
            "Show available commands and usage information",
            "/help [command]",
            StartNavigate,
            &["help", "commands", "usage"],
        )
        .with_aliases(&["h", "?"])
        .with_suggestions(&[Unauthenticated, EmptySession])
        .with_children(vec![child(
            "help",
            "feedback",
            "Submit feedback about Coven",
            Extend,
            &["feedback", "bug", "survey"],
        )]),
        CommandSpec::active(
            "init",
            "Initialize project instructions",
            "/init",
            StartNavigate,
            &["init", "agents", "project"],
        )
        .with_suggestions(&[EmptySession, MissingAgentsFile]),
        CommandSpec::active(
            "integrations",
            "Configure external integrations",
            "/integrations <hooks|mcp|chrome>",
            Extend,
            &["integrations", "mcp", "hooks"],
        )
        .with_children(vec![
            child(
                "integrations",
                "hooks",
                "Configure lifecycle hooks",
                Extend,
                &["hooks"],
            ),
            child(
                "integrations",
                "mcp",
                "Manage MCP servers",
                Extend,
                &["mcp"],
            ),
            child(
                "integrations",
                "chrome",
                "Connect Chrome integration",
                Extend,
                &["chrome"],
            ),
        ]),
        CommandSpec::active(
            "learn",
            "Learn how to use Coven",
            "/learn",
            Extend,
            &["learn", "guide", "tutorial"],
        )
        .with_aliases(&["scribe"]),
        CommandSpec::active(
            "memory",
            "Manage project instruction memory",
            "/memory [edit|clear]",
            WorkReview,
            &["memory", "agents", "instructions"],
        ),
        CommandSpec::active(
            "model",
            "Select the active AI model",
            "/model [name]",
            AccountModels,
            &["model", "provider", "ai"],
        )
        .with_suggestions(&[EmptySession]),
        CommandSpec::active(
            "permissions",
            "Configure command permissions",
            "/permissions [sandbox]",
            AccountModels,
            &["permissions", "sandbox", "security"],
        )
        .with_children(vec![child(
            "permissions",
            "sandbox",
            "Configure sandbox permissions",
            AccountModels,
            &["sandbox"],
        )]),
        CommandSpec::active(
            "plan",
            "Create and manage implementation plans",
            "/plan [prompt]",
            Conversation,
            &["plan", "planning"],
        ),
        CommandSpec::active(
            "plugin",
            "Manage installed plugins",
            "/plugin [reload]",
            Extend,
            &["plugin", "extension"],
        )
        .with_aliases(&["plugins"])
        .with_children(vec![child(
            "plugin",
            "reload",
            "Reload installed plugins",
            Extend,
            &["reload", "plugin"],
        )
        .with_aliases(&[
            "reload",
            "plugin refresh",
            "plugins reload",
            "plugins refresh",
        ])]),
        CommandSpec::active(
            "review",
            "Review changes in the current project",
            "/review [pr-comments]",
            WorkReview,
            &["review", "code", "git"],
        )
        .with_suggestions(&[GitDirty, HasCompletedEdits])
        .with_children(vec![
            child(
                "review",
                "pr-comments",
                "Read pull request comments",
                WorkReview,
                &["pull request", "comments"],
            ),
            child(
                "review",
                "security",
                "Run a security-focused review",
                WorkReview,
                &["security", "vulnerabilities"],
            ),
            child(
                "review",
                "ultra",
                "Run an exhaustive review",
                WorkReview,
                &["exhaustive", "review"],
            ),
        ]),
        CommandSpec::active(
            "rewind",
            "Rewind or inspect recent changes",
            "/rewind [list|diff|last]",
            WorkReview,
            &["rewind", "undo", "changes", "go back"],
        )
        .with_suggestions(&[GitDirty, HasCompletedEdits])
        .with_children(vec![
            child(
                "rewind",
                "list",
                "List recorded file-change checkpoints",
                WorkReview,
                &["checkpoints", "changes"],
            ),
            child(
                "rewind",
                "diff",
                "Preview a recorded file-change diff",
                WorkReview,
                &["snapshot", "diff"],
            ),
        ]),
        CommandSpec::active(
            "search",
            "Search saved sessions",
            "/search <query>",
            StartNavigate,
            &["search", "sessions", "history"],
        ),
        CommandSpec::active(
            "session",
            "Manage the current session",
            "/session <resume|fork|branch|tag|add-dir>",
            StartNavigate,
            &["session", "resume", "branch"],
        )
        .with_aliases(&["remote"])
        .with_children(vec![
            child(
                "session",
                "rename",
                "Rename the current session",
                StartNavigate,
                &["rename", "title"],
            ),
            child(
                "session",
                "resume",
                "Resume a saved session",
                StartNavigate,
                &["resume"],
            )
            .with_suggestions(&[EmptySession]),
            child(
                "session",
                "fork",
                "Fork the current session",
                StartNavigate,
                &["fork"],
            ),
            child(
                "session",
                "branch",
                "Create a session branch",
                StartNavigate,
                &["branch"],
            ),
            child(
                "session",
                "tag",
                "Tag the current session",
                StartNavigate,
                &["tag"],
            ),
            child(
                "session",
                "add-dir",
                "Add an allowed workspace directory",
                StartNavigate,
                &["directory"],
            ),
        ]),
        CommandSpec::active(
            "skills",
            "Discover and run skills",
            "/skills",
            Extend,
            &["skills", "commands", "extensions"],
        )
        .with_aliases(&["skill"]),
        CommandSpec::active(
            "stash",
            "Store useful links and attachments",
            "/stash <link|attach>",
            CovenAdvanced,
            &["stash", "link", "attachment"],
        )
        .with_children(vec![
            child(
                "stash",
                "link",
                "Store and manage links",
                CovenAdvanced,
                &["link"],
            ),
            child(
                "stash",
                "attach",
                "Store and manage attachments",
                CovenAdvanced,
                &["attach"],
            ),
        ]),
        CommandSpec::active(
            "status",
            "Show Coven and project status",
            "/status [doctor]",
            CovenAdvanced,
            &["status", "diagnostics", "doctor"],
        )
        .with_children(vec![child(
            "status",
            "doctor",
            "Run diagnostic checks",
            CovenAdvanced,
            &["doctor", "diagnostics"],
        )]),
        CommandSpec::active(
            "tasks",
            "Manage background tasks",
            "/tasks",
            Conversation,
            &["tasks", "background"],
        )
        .with_aliases(&["bashes"])
        .with_suggestions(&[RunningTurn, HasBackgroundTasks]),
        CommandSpec::active(
            "thinking",
            "Configure extended thinking or inspect traces",
            thinking_usage,
            Conversation,
            thinking_keywords,
        )
        .with_aliases(&["think"])
        .with_suggestions(&[RunningTurn])
        .with_children(thinking_children),
        CommandSpec::active(
            "update",
            "Check for and install Coven updates",
            "/update <version|notes>",
            CovenAdvanced,
            &["update", "upgrade", "version"],
        )
        .with_aliases(&["upgrade"])
        .with_suggestions(&[UpdateAvailable])
        .with_children(vec![
            child(
                "update",
                "version",
                "Show the installed Coven version",
                CovenAdvanced,
                &["version"],
            ),
            child(
                "update",
                "notes",
                "Show recent Coven release notes",
                CovenAdvanced,
                &["notes"],
            ),
        ]),
        CommandSpec::active(
            "usage",
            "Show token, cost, and context usage",
            "/usage [cost|context|stats]",
            AccountModels,
            &["usage", "tokens", "cost", "context"],
        )
        .with_children(vec![
            child(
                "usage",
                "cost",
                "Show session cost details",
                AccountModels,
                &["cost", "pricing"],
            ),
            child(
                "usage",
                "context",
                "Show context window usage",
                AccountModels,
                &["context", "tokens"],
            )
            .with_suggestions(&[HighContextUsage]),
            child(
                "usage",
                "stats",
                "Show detailed session statistics",
                AccountModels,
                &["statistics", "tokens"],
            ),
        ]),
        CommandSpec::removed_with_details(
            "version",
            "update version",
            "Show the installed Coven version",
            "/version",
            CovenAdvanced,
            &["version", "update"],
        )
        .with_aliases(&["v"]),
        CommandSpec::removed_with_details(
            "release-notes",
            "update notes",
            "Show recent Coven highlights",
            "/release-notes",
            CovenAdvanced,
            &["release notes", "changelog"],
        )
        .with_aliases(&["whats-new", "changelog"]),
        CommandSpec::removed_with_details(
            "resume",
            "session resume",
            "Resume a saved session",
            "/resume",
            StartNavigate,
            &["resume", "continue"],
        )
        .with_aliases(&["r", "continue"]),
        CommandSpec::removed("reload-plugins", "plugin reload", Extend),
        CommandSpec::removed("doctor", "status doctor", CovenAdvanced),
        CommandSpec::removed_with_details(
            "login",
            "account login",
            "Sign in to a provider",
            "/login",
            AccountModels,
            &["login"],
        ),
        CommandSpec::removed_with_details(
            "logout",
            "account logout",
            "Sign out of a provider",
            "/logout",
            AccountModels,
            &["logout"],
        ),
        CommandSpec::removed_with_details(
            "accounts",
            "account list",
            "List configured accounts",
            "/accounts",
            AccountModels,
            &["accounts"],
        )
        .with_children(vec![CommandSpec::removed_with_details(
            "accounts dedupe",
            "account dedupe",
            "Remove duplicate account profiles",
            "/accounts dedupe",
            AccountModels,
            &["accounts", "dedupe"],
        )]),
        CommandSpec::removed("switch", "account switch", AccountModels),
        CommandSpec::removed("refresh", "account refresh", AccountModels),
        CommandSpec::removed_with_details(
            "incant",
            "familiar incant",
            "Run a familiar incantation",
            "/incant",
            CovenAdvanced,
            &["incant"],
        ),
        CommandSpec::removed_with_details(
            "handoff",
            "familiar handoff",
            "Hand work to a familiar",
            "/handoff",
            CovenAdvanced,
            &["handoff", "familiar"],
        ),
        CommandSpec::removed_with_details(
            "hooks",
            "integrations hooks",
            "Configure lifecycle hooks",
            "/hooks",
            Extend,
            &["hooks"],
        ),
        CommandSpec::removed("import-config", "config import", Customize),
        CommandSpec::removed_with_details(
            "mcp",
            "integrations mcp",
            "Manage MCP servers",
            "/mcp",
            Extend,
            &["mcp"],
        ),
        CommandSpec::removed("fork", "session fork", StartNavigate),
        CommandSpec::removed("theme", "config theme", Customize),
        CommandSpec::removed("output-style", "config output-style", Customize),
        CommandSpec::removed("keybindings", "config keybindings", Customize),
        CommandSpec::removed_with_details(
            "splash",
            "config splash",
            "Configure splash screen",
            "/splash",
            Customize,
            &["splash"],
        )
        .with_aliases(&["welcome-screen"]),
        CommandSpec::removed("share", "export share", Conversation),
        CommandSpec::removed("add-dir", "session add-dir", StartNavigate),
        CommandSpec::removed("agents", "familiar list", CovenAdvanced),
        CommandSpec::removed("branch", "session branch", StartNavigate),
        CommandSpec::removed("tag", "session tag", StartNavigate),
        CommandSpec::removed("pr-comments", "review pr-comments", WorkReview),
        CommandSpec::removed("copy", "export copy", Conversation),
        CommandSpec::removed_with_details(
            "chrome",
            "integrations chrome",
            "Connect Chrome integration",
            "/chrome",
            Extend,
            &["chrome"],
        ),
        CommandSpec::removed("fast", "effort fast", Customize).with_aliases(&["speed"]),
        CommandSpec::removed("think-back", "thinking back", Conversation)
            .with_aliases(&["thinkback"]),
        #[cfg(feature = "steer")]
        CommandSpec::removed_with_details(
            "steer",
            "thinking steer",
            "Steer a running turn",
            "/steer",
            Conversation,
            &["steer", "thinking"],
        ),
        CommandSpec::removed_with_details(
            "whisper",
            "familiar whisper",
            "Ask a familiar side question",
            "/whisper",
            CovenAdvanced,
            &["whisper"],
        )
        .with_aliases(&["btw"]),
        CommandSpec::removed_with_details(
            "sandbox",
            "permissions sandbox",
            "Configure sandbox permissions",
            "/sandbox",
            AccountModels,
            &["sandbox"],
        )
        .with_aliases(&["sandbox-toggle"]),
        CommandSpec::removed("advisor", "config advisor", Customize),
        CommandSpec::removed_with_details(
            "providers",
            "account providers",
            "List available providers",
            "/providers",
            AccountModels,
            &["providers"],
        ),
        CommandSpec::removed_with_details(
            "connect",
            "account connect",
            "Connect a provider account",
            "/connect",
            AccountModels,
            &["connect"],
        ),
        CommandSpec::removed("managed-agents", "familiar managed", CovenAdvanced),
        CommandSpec::removed("goal", "coven goal", CovenAdvanced),
        CommandSpec::removed_with_details(
            "link",
            "stash link",
            "Store and manage links",
            "/link",
            CovenAdvanced,
            &["link"],
        )
        .with_aliases(&["links"]),
        CommandSpec::removed_with_details(
            "attach",
            "stash attach",
            "Store and manage attachments",
            "/attach",
            CovenAdvanced,
            &["attach"],
        )
        .with_aliases(&["attachments"]),
        CommandSpec::removed_with_details(
            "feedback",
            "help feedback",
            "Submit feedback about Coven",
            "/feedback [report]",
            Extend,
            &["feedback", "bug"],
        )
        .with_aliases(&["bug", "survey"]),
    ];

    catalog
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashSet};

    #[test]
    fn active_canonical_paths_are_unique_and_fully_described() {
        let catalog = builtin_catalog_template();
        let primary_paths = catalog
            .iter()
            .filter(|spec| matches!(spec.lifecycle, CommandLifecycle::Active))
            .map(|spec| spec.path.as_str())
            .collect::<HashSet<_>>();
        let expected_primaries = [
            "account",
            "clear",
            "commit",
            "compact",
            "config",
            "coven",
            "diff",
            "effort",
            "exit",
            "export",
            "familiar",
            "help",
            "init",
            "integrations",
            "learn",
            "memory",
            "model",
            "permissions",
            "plan",
            "plugin",
            "review",
            "rewind",
            "search",
            "session",
            "skills",
            "stash",
            "status",
            "tasks",
            "thinking",
            "update",
            "usage",
        ]
        .into_iter()
        .collect::<HashSet<_>>();
        assert_eq!(primary_paths.len(), 31);
        assert_eq!(primary_paths, expected_primaries);

        for spec in flatten_catalog(&catalog)
            .into_iter()
            .filter(|spec| matches!(spec.lifecycle, CommandLifecycle::Active))
        {
            assert!(!spec.summary.is_empty(), "{} has no summary", spec.path);
            assert!(!spec.usage.is_empty(), "{} has no usage", spec.path);
            assert!(!spec.keywords.is_empty(), "{} has no keywords", spec.path);
            assert_ne!(spec.category, CommandCategory::Other);
        }
    }

    #[test]
    fn removed_replacements_resolve_to_live_paths_including_children() {
        let catalog = builtin_catalog_template();
        let flattened = flatten_catalog(&catalog);
        let live_paths = flattened
            .iter()
            .filter(|spec| {
                matches!(
                    spec.lifecycle,
                    CommandLifecycle::Active | CommandLifecycle::Advanced
                )
            })
            .map(|spec| spec.path.as_str())
            .collect::<HashSet<_>>();

        for spec in &flattened {
            let replacement = match &spec.lifecycle {
                CommandLifecycle::Removed { replacement } => replacement,
                CommandLifecycle::Active | CommandLifecycle::Advanced => continue,
            };
            assert!(
                live_paths.contains(replacement.as_str()),
                "{} replacement {replacement} is not live",
                spec.path
            );
        }
    }

    #[test]
    fn category_order_is_stable() {
        assert_eq!(
            CommandCategory::ordered(),
            &[
                CommandCategory::StartNavigate,
                CommandCategory::WorkReview,
                CommandCategory::Conversation,
                CommandCategory::AccountModels,
                CommandCategory::Customize,
                CommandCategory::Extend,
                CommandCategory::CovenAdvanced,
            ]
        );
        assert_eq!(
            CommandCategory::ordered()
                .iter()
                .map(|category| category.label())
                .collect::<Vec<_>>(),
            vec![
                "Start & Navigate",
                "Work & Review",
                "Conversation",
                "Account & Models",
                "Customize",
                "Extend",
                "Coven & Advanced",
            ]
        );
    }

    #[test]
    fn flattened_canonical_paths_are_unique() {
        let catalog = builtin_catalog_template();
        let mut paths = HashSet::new();

        for spec in flatten_catalog(&catalog) {
            assert!(
                paths.insert(spec.path.as_str()),
                "duplicate flattened path: {}",
                spec.path
            );
        }
    }

    #[test]
    fn compose_catalogs_preserves_execution_precedence() {
        let builtin = vec![CommandSpec::active(
            "review",
            "Review changes",
            "/review",
            CommandCategory::WorkReview,
            &["review"],
        )];
        let user = vec![CommandSpec::active(
            "deploy",
            "Deploy from a user template",
            "/deploy",
            CommandCategory::Extend,
            &["deploy"],
        )
        .with_source(CommandSource::User)];
        let skill = vec![
            CommandSpec::active(
                "deploy",
                "Deploy from a discovered skill",
                "/deploy",
                CommandCategory::Extend,
                &["deploy"],
            )
            .with_source(CommandSource::Skill),
            CommandSpec::active(
                "audit",
                "Audit from a discovered skill",
                "/audit",
                CommandCategory::Extend,
                &["audit"],
            )
            .with_source(CommandSource::Skill),
        ];
        let plugin = vec![
            CommandSpec::active(
                "audit",
                "Audit from a plugin",
                "/audit",
                CommandCategory::Extend,
                &["audit"],
            )
            .with_source(CommandSource::Plugin),
            CommandSpec::active(
                "plugin-inspect",
                "Inspect from a plugin",
                "/plugin-inspect",
                CommandCategory::Extend,
                &["inspect"],
            )
            .with_aliases(&["review"])
            .with_source(CommandSource::Plugin),
            CommandSpec::active(
                "release",
                "Release from a plugin",
                "/release",
                CommandCategory::Extend,
                &["release"],
            )
            .with_source(CommandSource::Plugin),
        ];

        let composed = compose_catalogs(builtin, user, skill, plugin);
        let sources = composed
            .iter()
            .map(|spec| (spec.path.as_str(), spec.source))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(sources.get("review"), Some(&CommandSource::BuiltIn));
        assert_eq!(sources.get("deploy"), Some(&CommandSource::User));
        assert_eq!(sources.get("audit"), Some(&CommandSource::Skill));
        assert_eq!(sources.get("release"), Some(&CommandSource::Plugin));
        assert!(!sources.contains_key("plugin-inspect"));
    }

    #[test]
    fn visible_registered_commands_have_metadata() {
        let catalog = crate::builtin_catalog();
        let flattened = flatten_catalog(&catalog);

        for command in crate::all_commands()
            .iter()
            .filter(|command| !command.hidden())
        {
            let spec = flattened
                .iter()
                .find(|spec| spec.path == command.name())
                .unwrap_or_else(|| {
                    panic!(
                        "visible command /{} has no catalog metadata",
                        command.name()
                    )
                });
            assert_eq!(
                spec.summary,
                crate::SlashCommand::description(command.as_ref()),
                "visible command /{} summary does not match its description",
                command.name()
            );
        }
    }

    #[test]
    fn context_suggestions_cover_the_expected_command_paths() {
        use claurst_core::CommandContextSignal::*;

        let catalog = builtin_catalog_template();
        let suggested_paths = |signal| {
            flatten_catalog(&catalog)
                .into_iter()
                .filter(|spec| spec.suggest_when.contains(&signal))
                .map(|spec| spec.path.as_str())
                .collect::<HashSet<_>>()
        };

        assert_eq!(
            suggested_paths(Unauthenticated),
            HashSet::from(["account connect", "account login", "help"])
        );
        assert_eq!(
            suggested_paths(EmptySession),
            HashSet::from([
                "account connect",
                "session resume",
                "init",
                "model",
                "familiar",
                "help",
            ])
        );
        assert_eq!(
            suggested_paths(MissingAgentsFile),
            HashSet::from(["account connect", "init"])
        );
        assert_eq!(
            suggested_paths(GitDirty),
            HashSet::from(["diff", "review", "commit", "rewind"])
        );
        assert_eq!(
            suggested_paths(HasCompletedEdits),
            HashSet::from(["review", "diff", "commit", "rewind"])
        );
        #[cfg(feature = "steer")]
        let running_turn_paths = HashSet::from(["thinking steer", "tasks", "thinking"]);
        #[cfg(not(feature = "steer"))]
        let running_turn_paths = HashSet::from(["tasks", "thinking"]);
        assert_eq!(suggested_paths(RunningTurn), running_turn_paths);
        assert_eq!(
            suggested_paths(HighContextUsage),
            HashSet::from(["usage context", "compact"])
        );
        assert_eq!(
            suggested_paths(HasBackgroundTasks),
            HashSet::from(["tasks"])
        );
        assert_eq!(suggested_paths(UpdateAvailable), HashSet::from(["update"]));
        assert_eq!(
            suggested_paths(FamiliarAvailable),
            HashSet::from(["familiar", "familiar handoff", "familiar whisper"])
        );
        assert_eq!(
            suggested_paths(CovenDaemonAvailable),
            HashSet::from(["coven", "familiar handoff"])
        );
    }
}
