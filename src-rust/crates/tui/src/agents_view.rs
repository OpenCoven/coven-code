//! Agent / coordinator progress views for the TUI.
//! Mirrors src/components/agents/ (13 files).

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};
use std::path::{Path, PathBuf};

use claurst_core::coven_shared;

use crate::overlays::{
    begin_modal_buf, modal_header_line_area, render_modal_title_buf, COVEN_CODE_ACCENT,
    COVEN_CODE_MUTED, COVEN_CODE_PANEL_BG, COVEN_CODE_TEXT,
};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// The role of an agent in the manager-executor architecture.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentRole {
    Normal,
    Manager,
    Executor { parent_id: String },
}

impl Default for AgentRole {
    fn default() -> Self { AgentRole::Normal }
}

/// The current status of a sub-agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Running,
    WaitingForTool,
    Complete,
    Failed,
}

impl AgentStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::WaitingForTool => "waiting",
            Self::Complete => "done",
            Self::Failed => "failed",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Self::Idle => Color::DarkGray,
            Self::Running => Color::Green,
            Self::WaitingForTool => Color::Yellow,
            Self::Complete => Color::Rgb(167, 139, 250),
            Self::Failed => Color::Red,
        }
    }
}

/// A sub-agent or coordinator instance.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    /// Unique agent ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Current status.
    pub status: AgentStatus,
    /// Current tool being executed (if any).
    pub current_tool: Option<String>,
    /// Number of turns completed.
    pub turns_completed: u32,
    /// Is this the coordinator?
    pub is_coordinator: bool,
    /// Brief description or last output snippet.
    pub last_output: Option<String>,
    /// Role in the managed agent architecture.
    #[allow(dead_code)]
    pub agent_role: AgentRole,
    /// Model name used by this agent.
    pub model_name: Option<String>,
    /// Cost in USD accumulated by this agent.
    pub cost_usd: f64,
}

/// A defined agent (from .coven-code/agents/*.md or plugin).
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    /// Backing markdown file path.
    pub file_path: PathBuf,
    /// Agent name.
    pub name: String,
    /// Source: "user" | "plugin:{name}" | "builtin".
    pub source: String,
    /// Model name.
    pub model: Option<String>,
    /// Memory scope.
    pub memory_scope: Option<String>,
    /// Description.
    pub description: String,
    /// Tool list (empty = all tools).
    pub tools: Vec<String>,
    /// If another agent overrides this one.
    pub shadowed_by: Option<String>,
    /// Markdown body / instructions.
    pub instructions: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEditorField {
    Name,
    Model,
    Memory,
    Tools,
    Description,
    Prompt,
}

impl AgentEditorField {
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Model,
            Self::Model => Self::Memory,
            Self::Memory => Self::Tools,
            Self::Tools => Self::Description,
            Self::Description => Self::Prompt,
            Self::Prompt => Self::Name,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Name => Self::Prompt,
            Self::Model => Self::Name,
            Self::Memory => Self::Model,
            Self::Tools => Self::Memory,
            Self::Description => Self::Tools,
            Self::Prompt => Self::Description,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentEditorState {
    pub original_index: Option<usize>,
    pub name: String,
    pub model: String,
    pub memory_scope: String,
    pub tools: String,
    pub description: String,
    pub prompt: String,
    pub selected_field: AgentEditorField,
    pub error: Option<String>,
    pub saved_message: Option<String>,
}

impl AgentEditorState {
    pub fn new() -> Self {
        Self {
            original_index: None,
            name: String::new(),
            model: "claude-sonnet-4-6".to_string(),
            memory_scope: String::new(),
            tools: String::new(),
            description: String::new(),
            prompt: String::new(),
            selected_field: AgentEditorField::Name,
            error: None,
            saved_message: None,
        }
    }

    pub fn from_definition(def: Option<(usize, &AgentDefinition)>) -> Self {
        match def {
            Some((idx, def)) => Self {
                original_index: Some(idx),
                name: def.name.clone(),
                model: def
                    .model
                    .clone()
                    .unwrap_or_else(|| "claude-sonnet-4-6".to_string()),
                memory_scope: def.memory_scope.clone().unwrap_or_default(),
                tools: def.tools.join(", "),
                description: def.description.clone(),
                prompt: def.instructions.clone(),
                selected_field: AgentEditorField::Name,
                error: None,
                saved_message: None,
            },
            None => Self::new(),
        }
    }

    fn selected_text_mut(&mut self) -> &mut String {
        match self.selected_field {
            AgentEditorField::Name => &mut self.name,
            AgentEditorField::Model => &mut self.model,
            AgentEditorField::Memory => &mut self.memory_scope,
            AgentEditorField::Tools => &mut self.tools,
            AgentEditorField::Description => &mut self.description,
            AgentEditorField::Prompt => &mut self.prompt,
        }
    }
}

// ---------------------------------------------------------------------------
// Screen routes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentsRoute {
    List,
    Detail(usize),        // index into definitions
    Editor(Option<usize>), // None = create new
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Full state for the agents menu overlay.
#[derive(Debug, Clone)]
pub struct AgentsMenuState {
    pub visible: bool,
    pub route: AgentsRoute,
    pub definitions: Vec<AgentDefinition>,
    pub active_agents: Vec<AgentInfo>,
    pub list_scroll: usize,
    pub selected_row: usize,
    pub project_root: Option<PathBuf>,
    pub editor: AgentEditorState,
}

impl AgentsMenuState {
    pub fn new() -> Self {
        Self {
            visible: false,
            route: AgentsRoute::List,
            definitions: Vec::new(),
            active_agents: Vec::new(),
            list_scroll: 0,
            selected_row: 0,
            project_root: None,
            editor: AgentEditorState::new(),
        }
    }

    pub fn open(&mut self, project_root: &std::path::Path) {
        self.definitions = load_agent_definitions(project_root);
        self.selected_row = 0;
        self.list_scroll = 0;
        self.route = AgentsRoute::List;
        self.project_root = Some(project_root.to_path_buf());
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        let row_count = self.definitions.len() + 1;
        if row_count == 0 {
            return;
        }
        if self.selected_row == 0 {
            self.selected_row = row_count - 1;
        } else {
            self.selected_row -= 1;
        }
    }

    pub fn select_next(&mut self) {
        let row_count = self.definitions.len() + 1;
        if row_count == 0 {
            return;
        }
        self.selected_row = (self.selected_row + 1) % row_count;
    }

    pub fn confirm_selection(&mut self) {
        match self.route {
            AgentsRoute::List => {
                if self.selected_row == 0 {
                    self.open_editor(None);
                } else {
                    let idx = self.selected_row - 1;
                    if idx < self.definitions.len() {
                        self.route = AgentsRoute::Detail(idx);
                    }
                }
            }
            AgentsRoute::Detail(idx) => {
                // Coven familiars are read-only — do not open editor.
                let is_familiar = self
                    .definitions
                    .get(idx)
                    .map(|d| d.source.starts_with("coven:familiar"))
                    .unwrap_or(false);
                if !is_familiar {
                    self.open_editor(Some(idx));
                }
            }
            AgentsRoute::Editor(_) => {}
        }
    }

    pub fn go_back(&mut self) {
        match &self.route {
            AgentsRoute::Detail(_) | AgentsRoute::Editor(_) => {
                self.route = AgentsRoute::List;
            }
            AgentsRoute::List => {
                self.close();
            }
        }
    }

    pub fn open_editor(&mut self, idx: Option<usize>) {
        self.editor = AgentEditorState::from_definition(
            idx.and_then(|index| self.definitions.get(index).map(|def| (index, def))),
        );
        self.route = AgentsRoute::Editor(idx);
    }

    pub fn editor_insert_char(&mut self, ch: char) {
        let field = self.editor.selected_text_mut();
        field.push(ch);
        self.editor.error = None;
        self.editor.saved_message = None;
    }

    pub fn editor_backspace(&mut self) {
        self.editor.selected_text_mut().pop();
    }

    pub fn editor_insert_newline(&mut self) {
        match self.editor.selected_field {
            AgentEditorField::Description | AgentEditorField::Prompt => {
                self.editor.selected_text_mut().push('\n');
            }
            _ => self.editor.selected_field = self.editor.selected_field.next(),
        }
    }

    pub fn editor_next_field(&mut self) {
        self.editor.selected_field = self.editor.selected_field.next();
    }

    pub fn editor_prev_field(&mut self) {
        self.editor.selected_field = self.editor.selected_field.prev();
    }

    pub fn save_editor(&mut self) -> Result<String, String> {
        validate_editor(&self.editor)?;
        let root = self
            .project_root
            .clone()
            .ok_or_else(|| "Project root is unavailable.".to_string())?;
        let file_path = self
            .editor
            .original_index
            .and_then(|idx| self.definitions.get(idx).map(|def| def.file_path.clone()))
            .unwrap_or_else(|| {
                root.join(".coven-code")
                    .join("agents")
                    .join(format!("{}.md", slugify_agent_name(&self.editor.name)))
            });

        write_editor_to_disk(&file_path, &self.editor)?;
        self.definitions = load_agent_definitions(&root);

        let saved_idx = self
            .definitions
            .iter()
            .position(|def| def.file_path == file_path)
            .unwrap_or(0);
        self.selected_row = saved_idx + 1;
        self.route = AgentsRoute::Detail(saved_idx);
        let msg = format!("Saved agent to {}", file_path.display());
        self.editor.saved_message = Some(msg.clone());
        self.editor.error = None;
        Ok(msg)
    }
}

impl Default for AgentsMenuState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

/// Load agent definitions from `.coven-code/agents/` in project root and home dir.
pub fn load_agent_definitions(project_root: &std::path::Path) -> Vec<AgentDefinition> {
    let mut defs = Vec::new();
    let dirs = [
        dirs::home_dir().map(|h| h.join(".coven-code").join("agents")),
        Some(project_root.join(".coven-code").join("agents")),
    ];

    for dir_opt in &dirs {
        let Some(dir) = dir_opt else { continue };
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "md") {
                if let Some(def) = parse_agent_def(&path) {
                    defs.push(def);
                }
            }
        }
    }

    // --- Coven daemon familiars as agent definitions ---
    // Each familiar from ~/.coven/familiars.toml is surfaced as an agent
    // that can be selected to give the session a named-familiar persona.
    // Familiar-sourced agents are appended after user agents so user
    // definitions always take precedence for the same name.
    if let Some(familiars) = coven_shared::load_familiars() {
        let familiar_names: std::collections::HashSet<String> = defs
            .iter()
            .map(|d| d.name.to_lowercase())
            .collect();
        for fam in &familiars {
            let display = fam
                .display_name
                .as_deref()
                .unwrap_or(&fam.id)
                .to_string();
            // Skip if user already defined an agent with the same display name.
            if familiar_names.contains(&display.to_lowercase()) {
                continue;
            }
            defs.push(familiar_as_agent_def(fam));
        }
    }

    defs
}

fn parse_agent_def(path: &std::path::Path) -> Option<AgentDefinition> {
    let content = std::fs::read_to_string(path).ok()?;
    let stem = path.file_stem()?.to_string_lossy().to_string();

    let (name, model, memory, description, tools, instructions) = if content.starts_with("---") {
        let end = content[3..].find("\n---")? + 3;
        let front = &content[3..end];
        let body = content[end + 4..].trim().to_string();
        let name = extract_yaml_str(front, "name").unwrap_or_else(|| stem.clone());
        let model = extract_yaml_str(front, "model");
        let memory = extract_yaml_str(front, "memory_scope")
            .or_else(|| extract_yaml_str(front, "memory"));
        let desc = extract_yaml_str(front, "description").unwrap_or_default();
        let tools = extract_yaml_list(front, "tools");
        (name, model, memory, desc, tools, body)
    } else {
        (
            stem,
            None,
            None,
            content.lines().next().unwrap_or("").to_string(),
            vec![],
            content.trim().to_string(),
        )
    };

    Some(AgentDefinition {
        file_path: path.to_path_buf(),
        name,
        source: "user".to_string(),
        model,
        memory_scope: memory,
        description,
        tools,
        shadowed_by: None,
        instructions,
    })
}

/// Build an `AgentDefinition` from a Coven daemon familiar record.
///
/// The resulting definition carries:
/// - `source: "coven:familiar"` — identifies it as daemon-sourced
/// - `file_path` pointing to `~/.coven/familiars.toml` (informational)
/// - a synthesised system-prompt body describing the familiar's role
pub fn familiar_as_agent_def(fam: &coven_shared::CovenFamiliar) -> AgentDefinition {
    let display = fam
        .display_name
        .as_deref()
        .unwrap_or(&fam.id)
        .to_string();
    let emoji = fam.emoji.as_deref().unwrap_or("✨");
    let role = fam.role.as_deref().unwrap_or("Familiar");
    let desc = fam
        .description
        .as_deref()
        .unwrap_or("A Coven familiar available as an agent context.")
        .to_string();
    let pronouns = fam
        .pronouns
        .as_deref()
        .map(|p| format!(" Pronouns: {p}."))
        .unwrap_or_default();

    let instructions = format!(
        "You are {emoji} {display}, a Coven familiar with the role of {role}.{pronouns}\n\n{desc}\n\nOperate with the full context of the current workspace. Respond in character but remain focused on the developer's goals."
    );

    let familiar_toml = dirs::home_dir()
        .map(|h| h.join(".coven").join("familiars.toml"))
        .unwrap_or_else(|| PathBuf::from("~/.coven/familiars.toml"));

    AgentDefinition {
        file_path: familiar_toml,
        name: display,
        source: format!("coven:familiar:{}", fam.id),
        model: None, // use session default; user can override
        memory_scope: Some("workspace".to_string()),
        description: format!("{emoji} {role} — {desc}"),
        tools: Vec::new(),
        shadowed_by: None,
        instructions,
    }
}

fn extract_yaml_str(front: &str, key: &str) -> Option<String> {
    for line in front.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            return Some(
                rest.trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    None
}

fn extract_yaml_list(front: &str, key: &str) -> Vec<String> {
    for line in front.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            let rest = rest.trim().trim_matches('[').trim_matches(']');
            return rest
                .split(',')
                .map(|s| {
                    s.trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string()
                })
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    Vec::new()
}

fn slugify_agent_name(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if matches!(ch, ' ' | '-' | '_' | '.') && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

fn validate_editor(editor: &AgentEditorState) -> Result<(), String> {
    let name = editor.name.trim();
    if name.is_empty() {
        return Err("Familiar name is required.".to_string());
    }
    if slugify_agent_name(name).is_empty() {
        return Err("Familiar name must contain letters or numbers.".to_string());
    }
    if editor.model.trim().is_empty() {
        return Err("Model is required.".to_string());
    }
    if editor.description.trim().is_empty() {
        return Err("Description is required.".to_string());
    }
    if editor.prompt.trim().is_empty() {
        return Err("Prompt body is required.".to_string());
    }
    Ok(())
}

fn serialize_editor(editor: &AgentEditorState) -> String {
    let tools = editor
        .tools
        .split(',')
        .map(|tool| tool.trim())
        .filter(|tool| !tool.is_empty())
        .map(|tool| format!("\"{}\"", tool))
        .collect::<Vec<_>>()
        .join(", ");

    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("name: {}\n", editor.name.trim()));
    out.push_str(&format!("model: {}\n", editor.model.trim()));
    if !editor.memory_scope.trim().is_empty() {
        out.push_str(&format!("memory_scope: {}\n", editor.memory_scope.trim()));
    }
    out.push_str(&format!("description: {}\n", editor.description.trim()));
    if !tools.is_empty() {
        out.push_str(&format!("tools: [{}]\n", tools));
    }
    out.push_str("---\n\n");
    out.push_str(editor.prompt.trim());
    out.push('\n');
    out
}

fn write_editor_to_disk(path: &Path, editor: &AgentEditorState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {}", parent.display(), err))?;
    }
    std::fs::write(path, serialize_editor(editor))
        .map_err(|err| format!("Failed to write {}: {}", path.display(), err))
}

// ---------------------------------------------------------------------------
// Rendering: Agents Menu overlay
// ---------------------------------------------------------------------------

/// Render the agents menu overlay.
pub fn render_agents_menu(state: &AgentsMenuState, area: Rect, buf: &mut Buffer) {
    if !state.visible {
        return;
    }

    let layout = begin_modal_buf(buf, area, 92, 30, 2, 1);
    let (title, subtitle, footer) = match &state.route {
        AgentsRoute::List => (
            "Familiars".to_string(),
            format!(
                " {} active  ·  {} definitions",
                state.active_agents.len(),
                state.definitions.len()
            ),
            " j/k navigate  ·  enter open  ·  esc close".to_string(),
        ),
        AgentsRoute::Detail(idx) => {
            let is_familiar = state
                .definitions
                .get(*idx)
                .map(|def| def.source.starts_with("coven:familiar"))
                .unwrap_or(false);
            (
                state
                    .definitions
                    .get(*idx)
                    .map(|def| def.name.clone())
                    .unwrap_or_else(|| "Familiar".to_string()),
                " Review configuration and prompt details.".to_string(),
                if is_familiar {
                    " read-only  ·  create a workspace override to customise  ·  esc back".to_string()
                } else {
                    " enter edit  ·  esc back".to_string()
                },
            )
        }
        AgentsRoute::Editor(Some(_)) => (
            "Edit familiar".to_string(),
            " Update metadata, tools, and prompt instructions.".to_string(),
            " tab move  ·  ctrl+s save  ·  esc back".to_string(),
        ),
        AgentsRoute::Editor(None) => (
            "Create familiar".to_string(),
            " Define a new reusable familiar for this workspace.".to_string(),
            " tab move  ·  ctrl+s save  ·  esc back".to_string(),
        ),
    };
    render_modal_title_buf(buf, layout.header_area, &title, "esc");
    if let Some(subtitle_area) = modal_header_line_area(layout.header_area, 1) {
        Paragraph::new(Line::from(vec![Span::styled(
            subtitle,
            Style::default().fg(COVEN_CODE_MUTED),
        )]))
        .render(subtitle_area, buf);
    }

    match &state.route {
        AgentsRoute::List => render_agents_list(state, layout.body_area, buf),
        AgentsRoute::Detail(idx) => {
            if let Some(def) = state.definitions.get(*idx) {
                render_agent_detail(def, layout.body_area, buf);
            }
        }
        AgentsRoute::Editor(Some(_idx)) => {
            render_agent_editor(state, layout.body_area, buf);
        }
        AgentsRoute::Editor(None) => {
            render_agent_editor(state, layout.body_area, buf);
        }
    }
    Paragraph::new(Line::from(vec![Span::styled(
        footer,
        Style::default().fg(COVEN_CODE_MUTED).add_modifier(Modifier::ITALIC),
    )]))
    .render(layout.footer_area, buf);
}

fn render_agents_list(state: &AgentsMenuState, area: Rect, buf: &mut Buffer) {
    let mut lines = Vec::new();
    if !state.active_agents.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            " Active now",
            Style::default().fg(COVEN_CODE_ACCENT).add_modifier(Modifier::BOLD),
        )]));
        for agent in state.active_agents.iter().take(3) {
            lines.push(Line::from(vec![
                Span::styled(" ", Style::default().fg(COVEN_CODE_MUTED)),
                Span::styled(agent.name.clone(), Style::default().fg(COVEN_CODE_TEXT)),
                Span::styled(
                    format!("  {}", agent.status.label()),
                    Style::default().fg(agent.status.color()),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }

    let create_selected = state.selected_row == 0;
    lines.push(agent_list_row(
        "[+ Create new agent]".to_string(),
        "Create a reusable workspace agent".to_string(),
        create_selected,
        area.width,
    ));
    lines.push(Line::from(""));

    let max_visible = (area.height as usize).saturating_sub(lines.len() + 1);
    let start = state
        .list_scroll
        .min(state.definitions.len().saturating_sub(max_visible));

    // Separate user-defined and familiar-sourced definitions for display.
    let (familiar_defs, user_defs): (Vec<_>, Vec<_>) = state.definitions[start..]
        .iter()
        .enumerate()
        .partition(|(_, d)| d.source.starts_with("coven:familiar"));

    // User / workspace agents first.
    if !user_defs.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            " Workspace Agents",
            Style::default()
                .fg(COVEN_CODE_MUTED)
                .add_modifier(Modifier::BOLD),
        )]));
    }
    let mut rendered = 0;
    for (abs_rel_idx, def) in &user_defs {
        if rendered >= max_visible { break; }
        let abs_idx = start + abs_rel_idx;
        let selected = state.selected_row == abs_idx + 1;
        let model_str = def.model.as_deref().unwrap_or("default");
        let shadow_suffix = if def.shadowed_by.is_some() { " ⚠" } else { "" };
        lines.push(agent_list_row(
            def.name.clone(),
            format!("{}  ·  {}{}", model_str, def.source, shadow_suffix),
            selected,
            area.width,
        ));
        rendered += 1;
    }

    // Coven familiars section.
    if !familiar_defs.is_empty() {
        if !user_defs.is_empty() { lines.push(Line::from("")); }
        lines.push(Line::from(vec![Span::styled(
            " Coven Familiars",
            Style::default()
                .fg(Color::Rgb(139, 92, 246)) // violet-500
                .add_modifier(Modifier::BOLD),
        )]));
    }
    for (abs_rel_idx, def) in &familiar_defs {
        if rendered >= max_visible { break; }
        let abs_idx = start + abs_rel_idx;
        let selected = state.selected_row == abs_idx + 1;
        // Extract emoji from description prefix if present.
        let desc_short = def
            .description
            .split(" — ")
            .next()
            .unwrap_or(&def.description)
            .trim()
            .to_string();
        lines.push(agent_list_row(
            def.name.clone(),
            desc_short,
            selected,
            area.width,
        ));
        rendered += 1;
    }
    Paragraph::new(lines)
        .style(Style::default().bg(COVEN_CODE_PANEL_BG))
        .render(area, buf);
}

fn render_agent_detail(def: &AgentDefinition, area: Rect, buf: &mut Buffer) {
    let mut lines = Vec::new();
    let is_familiar = def.source.starts_with("coven:familiar");

    // Source badge — colour-coded for familiar vs user.
    let source_style = if is_familiar {
        Style::default().fg(Color::Rgb(139, 92, 246)).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(COVEN_CODE_MUTED)
    };
    let source_label = if is_familiar {
        // e.g. "coven:familiar:nova" -> "Coven Familiar · nova"
        let id = def.source.trim_start_matches("coven:familiar:");
        format!("Coven Familiar · {}", id)
    } else {
        def.source.clone()
    };

    lines.push(Line::from(vec![
        Span::styled(" Name       ", Style::default().fg(COVEN_CODE_MUTED)),
        Span::styled(
            def.name.clone(),
            Style::default()
                .fg(COVEN_CODE_TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ({})", source_label),
            source_style,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(" Model      ", Style::default().fg(COVEN_CODE_MUTED)),
        Span::raw(def.model.as_deref().unwrap_or("default").to_string()),
    ]));
    if let Some(mem) = &def.memory_scope {
        lines.push(Line::from(vec![
            Span::styled(" Memory     ", Style::default().fg(COVEN_CODE_MUTED)),
            Span::raw(mem.clone()),
        ]));
    }
    if !def.tools.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" Tools      ", Style::default().fg(COVEN_CODE_MUTED)),
            Span::raw(def.tools.join(", ")),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" Tools      ", Style::default().fg(COVEN_CODE_MUTED)),
            Span::styled("All tools", Style::default().fg(COVEN_CODE_MUTED)),
        ]));
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![Span::styled(
        " Description",
        Style::default().fg(COVEN_CODE_ACCENT).add_modifier(Modifier::BOLD),
    )]));
    for line in def.description.lines() {
        lines.push(Line::from(vec![Span::raw(format!(" {}", line))]));
    }
    lines.push(Line::default());
    let prompt_label = if is_familiar { " Persona" } else { " Prompt" };
    lines.push(Line::from(vec![Span::styled(
        prompt_label,
        Style::default().fg(COVEN_CODE_ACCENT).add_modifier(Modifier::BOLD),
    )]));
    for line in def.instructions.lines().take(8) {
        lines.push(Line::from(vec![Span::styled(
            format!(" {}", line),
            Style::default().fg(COVEN_CODE_TEXT),
        )]));
    }

    if let Some(shadow) = &def.shadowed_by {
        lines.push(Line::default());
        lines.push(Line::from(vec![Span::styled(
            format!("⚠ Shadowed by: {}", shadow),
            Style::default().fg(Color::Yellow),
        )]));
    }

    if is_familiar {
        lines.push(Line::default());
        lines.push(Line::from(vec![Span::styled(
            " ✨ Coven Familiar — read-only. Create a workspace override to customise this familiar.",
            Style::default().fg(Color::Rgb(139, 92, 246)),
        )]));
    }

    Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .style(Style::default().bg(COVEN_CODE_PANEL_BG))
        .render(area, buf);
}

fn render_agent_editor(state: &AgentsMenuState, area: Rect, buf: &mut Buffer) {
    let editor = &state.editor;
    let selected_style = Style::default()
        .fg(Color::White)
        .bg(COVEN_CODE_ACCENT)
        .add_modifier(Modifier::BOLD);
    let normal_style = Style::default().fg(COVEN_CODE_TEXT);

    let field_style = |field: AgentEditorField| {
        if editor.selected_field == field {
            selected_style
        } else {
            normal_style
        }
    };

    let mut lines = vec![
        render_editor_field("Name", &editor.name, field_style(AgentEditorField::Name)),
        render_editor_field("Model", &editor.model, field_style(AgentEditorField::Model)),
        render_editor_field(
            "Memory",
            &editor.memory_scope,
            field_style(AgentEditorField::Memory),
        ),
        render_editor_field("Tools", &editor.tools, field_style(AgentEditorField::Tools)),
        render_editor_field(
            "Description",
            &editor.description,
            field_style(AgentEditorField::Description),
        ),
        Line::default(),
        Line::from(vec![Span::styled(
            " Prompt",
            Style::default().fg(COVEN_CODE_ACCENT).add_modifier(Modifier::BOLD),
        )]),
    ];

    let prompt_style = field_style(AgentEditorField::Prompt);
    let prompt_lines = if editor.prompt.is_empty() {
        vec![Line::from(vec![Span::styled(
            "(empty)",
            prompt_style.add_modifier(Modifier::ITALIC),
        )])]
    } else {
        editor
            .prompt
            .lines()
            .map(|line| Line::from(vec![Span::styled(line.to_string(), prompt_style)]))
            .collect::<Vec<_>>()
    };
    lines.extend(prompt_lines);
    lines.push(Line::default());

    if let Some(msg) = editor.saved_message.as_ref() {
        lines.push(Line::from(vec![Span::styled(
            msg.clone(),
            Style::default().fg(Color::Green),
        )]));
    }
    if let Some(err) = editor.error.as_ref() {
        lines.push(Line::from(vec![Span::styled(
            err.clone(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]));
    }

    Paragraph::new(lines)
        .style(Style::default().bg(COVEN_CODE_PANEL_BG))
        .render(area, buf);
}

fn render_editor_field(label: &str, value: &str, value_style: Style) -> Line<'static> {
    let display = if value.is_empty() {
        "(empty)".to_string()
    } else {
        value.to_string()
    };
    Line::from(vec![
        Span::styled(
            format!(" {label:<10} "),
            Style::default().fg(COVEN_CODE_MUTED),
        ),
        Span::styled(display, value_style),
    ])
}

fn agent_list_row(title: String, meta: String, selected: bool, width: u16) -> Line<'static> {
    let bg = if selected { COVEN_CODE_ACCENT } else { COVEN_CODE_PANEL_BG };
    let title_style = if selected {
        Style::default().fg(Color::White).bg(bg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(COVEN_CODE_TEXT).bg(bg)
    };
    let meta_style = if selected {
        Style::default().fg(Color::Rgb(248, 220, 236)).bg(bg)
    } else {
        Style::default().fg(COVEN_CODE_MUTED).bg(bg)
    };
    let mut spans = vec![
        Span::styled(" ", Style::default().bg(bg)),
        Span::styled(title, title_style),
        Span::styled(format!("  {}", meta), meta_style),
    ];
    let used: usize = spans.iter().map(|span| span.content.len()).sum();
    let pad = width.saturating_sub(used as u16) as usize;
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
    }
    Line::from(spans)
}

// ---------------------------------------------------------------------------
// Rendering: Coordinator status inline widget
// ---------------------------------------------------------------------------

/// Render an inline coordinator + sub-agent status widget.
///
/// Shows: coordinator status, then each sub-agent with its current tool.
/// Suitable for embedding in the main TUI layout (e.g., below the message list).
pub fn render_coordinator_status(agents: &[AgentInfo], area: Rect, buf: &mut Buffer) {
    if agents.is_empty() {
        return;
    }

    Block::default()
        .title(" Active Agents ")
        .borders(Borders::TOP)
        .style(Style::default().fg(Color::DarkGray))
        .render(area, buf);

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(1),
    };

    for (i, agent) in agents.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let y = inner.y + i as u16;
        let row_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };

        let (prefix, role_badge, role_color, indent) = match &agent.agent_role {
            AgentRole::Manager => ("● ", "[MGR]", Color::Magenta, ""),
            AgentRole::Executor { .. } => ("  ○ ", "[EXE]", Color::Rgb(167, 139, 250), "  "),
            AgentRole::Normal => {
                if agent.is_coordinator {
                    ("● ", "", Color::Green, "")
                } else {
                    ("  ○ ", "", Color::DarkGray, "  ")
                }
            }
        };
        let tool_str = agent
            .current_tool
            .as_deref()
            .map(|t| format!(" → {}", t))
            .unwrap_or_default();
        let model_str = agent.model_name.as_deref()
            .map(|m| format!(" ({})", m))
            .unwrap_or_default();
        let cost_str = if agent.cost_usd > 0.0 {
            format!("  ${:.4}", agent.cost_usd)
        } else {
            String::new()
        };

        let mut spans = vec![
            Span::styled(indent.to_string(), Style::default()),
            Span::styled(prefix, Style::default().fg(agent.status.color())),
        ];
        if !role_badge.is_empty() {
            spans.push(Span::styled(
                format!("{} ", role_badge),
                Style::default().fg(role_color).add_modifier(Modifier::BOLD),
            ));
        }
        spans.extend(vec![
            Span::styled(agent.name.clone(), Style::default().fg(Color::White)),
            Span::styled(model_str, Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(" [{}]", agent.status.label()),
                Style::default().fg(agent.status.color()),
            ),
            Span::styled(
                format!(" {} turns", agent.turns_completed),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(tool_str, Style::default().fg(Color::Yellow)),
            Span::styled(cost_str, Style::default().fg(Color::DarkGray)),
        ]);

        let line = Line::from(spans);
        Paragraph::new(line).render(row_area, buf);
    }
}
