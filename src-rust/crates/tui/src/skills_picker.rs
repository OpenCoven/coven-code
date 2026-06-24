// skills_picker.rs — Interactive `/skills` overlay.
//
// Lists every skill coven-code can see — bundled (in-binary) plus skills
// inherited from other environments (coven, Claude `~/.claude/skills` and
// plugins, Codex `~/.codex/prompts`) — with a search box, a per-skill on/off
// toggle, scope label, and an estimated always-on token cost.
//
// Toggling persists to `Settings.disabled_skills`, which the skill index and
// the model-facing skill list both honour, so disabling a skill reclaims the
// context tokens it would otherwise spend.

use std::path::Path;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::overlays::{
    begin_modal_frame, modal_header_line_area, modal_search_line, render_modal_title_frame,
    COVEN_CODE_ACCENT, COVEN_CODE_MUTED, COVEN_CODE_TEXT,
};

/// One row in the skills picker.
#[derive(Debug, Clone)]
pub struct SkillRow {
    pub name: String,
    pub scope_label: &'static str,
    pub est_tokens: usize,
    pub enabled: bool,
}

/// State for the `/skills` picker overlay.
#[derive(Debug, Default)]
pub struct SkillsPickerState {
    pub visible: bool,
    /// Index into the *filtered* row list.
    pub selected: usize,
    pub filter: String,
    pub rows: Vec<SkillRow>,
}

impl SkillsPickerState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the picker with the given rows, optionally pre-filtered.
    pub fn open(&mut self, rows: Vec<SkillRow>, filter: &str) {
        self.rows = rows;
        self.filter = filter.to_string();
        self.selected = 0;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.filter.clear();
    }

    /// Indices into `self.rows` matching the current filter (name or scope).
    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.rows.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.name.to_lowercase().contains(needle.as_str())
                    || r.scope_label.contains(needle.as_str())
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
    }

    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    pub fn select_prev(&mut self) {
        let count = self.filtered_indices().len();
        if count == 0 {
            return;
        }
        self.selected = if self.selected == 0 {
            count - 1
        } else {
            self.selected - 1
        };
    }

    pub fn select_next(&mut self) {
        let count = self.filtered_indices().len();
        if count == 0 {
            return;
        }
        self.selected = (self.selected + 1) % count;
    }

    /// Toggle the enabled state of the selected row.
    ///
    /// Returns `(name, now_enabled)` so the caller can persist the change.
    pub fn toggle_selected(&mut self) -> Option<(String, bool)> {
        let filtered = self.filtered_indices();
        let &row_idx = filtered.get(self.selected)?;
        let row = self.rows.get_mut(row_idx)?;
        row.enabled = !row.enabled;
        Some((row.name.clone(), row.enabled))
    }
}

/// Assemble the full skill list from every visible source.
///
/// Bundled (in-binary) skills take precedence over same-named disk skills,
/// mirroring `SkillTool`'s resolution order.
pub fn build_skill_rows(cwd: &Path, settings: &claurst_core::config::Settings) -> Vec<SkillRow> {
    let mut rows: Vec<SkillRow> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Bundled, user-invocable skills.
    for skill in claurst_tools::bundled_skills::BUNDLED_SKILLS {
        if !skill.user_invocable || !seen.insert(skill.name.to_string()) {
            continue;
        }
        let meta = format!(
            "{} {} {}",
            skill.name,
            skill.description,
            skill.when_to_use.unwrap_or("")
        );
        rows.push(SkillRow {
            name: skill.name.to_string(),
            scope_label: "builtin",
            est_tokens: claurst_core::estimate_tokens(&meta),
            enabled: settings.is_skill_enabled(skill.name),
        });
    }

    // 2. Skills inherited from disk / other environments.
    let discovered = claurst_core::discover_skills(cwd, &settings.skills);
    let mut disk: Vec<_> = discovered.into_values().collect();
    disk.sort_by(|a, b| a.name.cmp(&b.name));
    for skill in disk {
        if !seen.insert(skill.name.clone()) {
            continue;
        }
        rows.push(SkillRow {
            name: skill.name.clone(),
            scope_label: skill.scope.label(),
            est_tokens: skill.est_tokens,
            enabled: settings.is_skill_enabled(&skill.name),
        });
    }

    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

pub fn render_skills_picker(frame: &mut Frame, state: &SkillsPickerState, area: Rect) {
    if !state.visible {
        return;
    }

    let width = 76u16.min(area.width.saturating_sub(4));
    let height = ((state.rows.len() as u16) + 7)
        .min((area.height as f32 * 0.8) as u16)
        .max(10);
    let layout = begin_modal_frame(frame, area, width, height, 3, 2);
    render_modal_title_frame(frame, layout.header_area, "Skills", "esc");

    let search = modal_search_line(
        &state.filter,
        "Search skills…",
        COVEN_CODE_MUTED,
        COVEN_CODE_TEXT,
    );
    if let Some(search_area) = modal_header_line_area(layout.header_area, 2) {
        frame.render_widget(Paragraph::new(search), search_area);
    }

    let body = layout.body_area;
    if body.height == 0 {
        return;
    }

    let filtered = state.filtered_indices();
    let total = filtered.len();
    let view_h = body.height as usize;

    let mut lines: Vec<Line> = Vec::new();

    if total == 0 {
        lines.push(Line::from(Span::styled(
            "  No matching skills",
            Style::default().fg(COVEN_CODE_MUTED),
        )));
    }

    // Window the filtered rows so the selection stays visible.
    let sel = state.selected.min(total.saturating_sub(1));
    let start = if total <= view_h {
        0
    } else {
        sel.saturating_sub(view_h / 2).min(total - view_h)
    };
    let end = (start + view_h).min(total);

    for (vis_i, &row_idx) in filtered[start..end].iter().enumerate() {
        let is_selected = start + vis_i == sel;
        let row = &state.rows[row_idx];

        let cursor = if is_selected { "›" } else { " " };
        let (state_glyph, state_color) = if row.enabled {
            ("✓ on ", Color::Green)
        } else {
            ("✗ off", COVEN_CODE_MUTED)
        };
        let name_style = if is_selected {
            Style::default()
                .fg(COVEN_CODE_ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(COVEN_CODE_TEXT)
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", cursor),
                Style::default().fg(COVEN_CODE_ACCENT),
            ),
            Span::styled(format!("{}  ", state_glyph), Style::default().fg(state_color)),
            Span::styled(row.name.clone(), name_style),
            Span::styled(
                format!("  · {} · ~{} tok", row.scope_label, row.est_tokens),
                Style::default().fg(COVEN_CODE_MUTED),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), body);

    // Footer: scroll status (line 0) + keybinds (line 1).
    let above = start;
    let below = total.saturating_sub(end);
    let mut scroll_parts: Vec<String> = Vec::new();
    if above > 0 {
        scroll_parts.push(format!("↑ {} above", above));
    }
    if below > 0 {
        scroll_parts.push(format!("↓ {} more below", below));
    }
    let footer_lines = vec![
        Line::from(Span::styled(
            format!(" {}", scroll_parts.join("   ")),
            Style::default().fg(COVEN_CODE_MUTED),
        )),
        Line::from(Span::styled(
            " ↑↓ navigate · space/enter toggle · type to filter · esc close",
            Style::default()
                .fg(COVEN_CODE_MUTED)
                .add_modifier(Modifier::ITALIC),
        )),
    ];
    frame.render_widget(Paragraph::new(footer_lines), layout.footer_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<SkillRow> {
        vec![
            SkillRow {
                name: "brainstorming".into(),
                scope_label: "user",
                est_tokens: 70,
                enabled: true,
            },
            SkillRow {
                name: "debug".into(),
                scope_label: "builtin",
                est_tokens: 40,
                enabled: true,
            },
            SkillRow {
                name: "higgsfield-generate".into(),
                scope_label: "user",
                est_tokens: 330,
                enabled: false,
            },
        ]
    }

    #[test]
    fn filter_matches_name_and_scope() {
        let mut s = SkillsPickerState::new();
        s.open(rows(), "");
        assert_eq!(s.filtered_indices().len(), 3);
        for c in "higgs".chars() {
            s.push_filter_char(c);
        }
        assert_eq!(s.filtered_indices().len(), 1);
        s.filter.clear();
        for c in "builtin".chars() {
            s.push_filter_char(c);
        }
        assert_eq!(s.filtered_indices().len(), 1);
    }

    #[test]
    fn navigation_wraps_over_filtered() {
        let mut s = SkillsPickerState::new();
        s.open(rows(), "");
        s.selected = 0;
        s.select_prev();
        assert_eq!(s.selected, 2);
        s.select_next();
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn toggle_flips_and_reports() {
        let mut s = SkillsPickerState::new();
        s.open(rows(), "");
        s.selected = 0; // brainstorming, currently enabled
        let res = s.toggle_selected();
        assert_eq!(res, Some(("brainstorming".to_string(), false)));
        assert!(!s.rows[0].enabled);
    }

    #[test]
    fn render_matches_screenshot_layout() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut s = SkillsPickerState::new();
        s.open(rows(), "");
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_skills_picker(f, &s, f.area())).unwrap();

        let rendered = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();

        for needle in [
            "Skills",
            "Search skills",
            "brainstorming",
            "on",
            "off",
            "user",
            "builtin",
            "tok",
            "toggle",
        ] {
            assert!(rendered.contains(needle), "render missing {needle:?}");
        }
    }
}
