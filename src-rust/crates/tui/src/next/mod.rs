//! Next renderer (opt-in preview, `COVEN_CODE_UI=next`).
//!
//! Renders the shared [`TranscriptView`] projection: one transcript, one
//! composer and a status row. It reads the same [`App`] state that Classic
//! reads and receives input through the same handlers, so selecting it never
//! changes what the runtime executes, persists or authorizes.
//!
//! Next draws only the plain transcript/composer shell. Until #410/#411
//! deliver Next surfaces for them, any frame in which a surface owns input
//! (permission request, overlays, pickers, dialogs, the familiar switcher,
//! slash suggestions) or an error modal is showing is drawn entirely by
//! Classic, so no choice that holds input focus is ever invisible or
//! partially drawn.

pub mod primitives;
pub mod style;

use crate::app::App;
use crate::application::view::{EntryRole, ToolActivityState, TranscriptView};
use claurst_core::appearance::Density;
use claurst_core::keybindings::{display_binding_for, KeybindingResolver, UserKeybindings};
use primitives::Status;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use style::{Appearance, Role};

/// Header text after the identity mark; the canonical mark is SVG-only, so
/// cells use plain text.
pub const PREVIEW_LABEL: &str = "Coven · preview";

/// Content of the optional key-hint row, resolved through the configurable
/// keybinding system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyHints {
    /// `<keys> <label>` parts, joined with the appearance's separator.
    Hints(Vec<String>),
    /// User bindings leave essential actions without any key.
    Unreachable(Vec<&'static str>),
    /// A user binding changed what an already-bound chord does.
    Conflict {
        chord: String,
        replaced: String,
        by: Option<String>,
    },
}

impl KeyHints {
    pub fn resolve(user: &UserKeybindings) -> Self {
        let resolver = KeybindingResolver::new(user);
        let unreachable = resolver.unreachable_essential_actions();
        if !unreachable.is_empty() {
            return Self::Unreachable(unreachable);
        }
        if let Some(conflict) = resolver.conflicts(user).into_iter().next() {
            return Self::Conflict {
                chord: conflict.chord,
                replaced: conflict.replaced,
                by: conflict.by,
            };
        }
        let hint = |action: &str, label: &str| {
            display_binding_for(user, action).map(|keys| format!("{keys} {label}"))
        };
        let parts: Vec<String> = [
            hint("submit", "send"),
            hint("openCommandPalette", "commands"),
            hint("openHelp", "help"),
        ]
        .into_iter()
        .flatten()
        .collect();
        Self::Hints(parts)
    }
}

/// Presentation-only preferences for Next, resolved once at startup.
#[derive(Debug, Clone)]
pub struct NextPresentation {
    pub appearance: Appearance,
    pub density: Density,
    /// `None` when the hint row is turned off.
    pub hints: Option<KeyHints>,
}

impl Default for NextPresentation {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            density: Density::default(),
            hints: None,
        }
    }
}

impl NextPresentation {
    pub fn from_settings(
        settings: &claurst_core::config::Settings,
        user_keybindings: &UserKeybindings,
        env: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let appearance_settings = &settings.next_appearance;
        Self {
            appearance: Appearance::resolve(appearance_settings, settings.reduce_motion, env),
            density: appearance_settings.density,
            hints: appearance_settings
                .hints
                .then(|| KeyHints::resolve(user_keybindings)),
        }
    }
}

/// Whether this frame must be drawn by Classic because a surface Next does
/// not render yet holds input focus or demands attention.
///
/// `any_modal_open` covers every input-owning surface in
/// `App::handle_key_event` except the familiar switcher and effort picker,
/// which are checked here without changing the list Classic's Enter gating
/// relies on.
pub fn delegates_to_classic(app: &App) -> bool {
    app.any_modal_open()
        || app.familiar_switcher_open
        || app.effort_picker.visible
        || app.notifications.current_is_error()
        || !app.prompt_input.suggestions.is_empty()
}

pub fn render_next(frame: &mut Frame, app: &App) {
    if delegates_to_classic(app) {
        crate::render::render_app(frame, app);
        return;
    }
    // Next does not register mouse hit-test geometry yet. Clear what an
    // earlier Classic frame recorded so selection, copy and context-menu
    // hit-tests never target Classic's stale layout.
    app.last_msg_area.set(Rect::default());
    app.last_selectable_area.set(Rect::default());
    app.last_input_area.set(Rect::default());
    app.footer_right_column_area.set(Rect::default());
    app.thinking_row_map.borrow_mut().clear();
    app.message_row_map.borrow_mut().clear();
    app.last_row_text.borrow_mut().clear();

    let presentation = &app.next_presentation;
    let appearance = &presentation.appearance;
    let view = TranscriptView::project(app);
    let area = frame.area();
    let hint_rows = u16::from(presentation.hints.is_some());
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(hint_rows),
            Constraint::Length(2),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(primitives::header(appearance, "preview")),
        rows[0],
    );
    render_transcript(frame, presentation, &view, rows[1]);
    frame.render_widget(
        Paragraph::new(status_line(appearance, &view, app.frame_count)),
        rows[2],
    );
    if let Some(hints) = &presentation.hints {
        frame.render_widget(Paragraph::new(hints_line(appearance, hints)), rows[3]);
    }
    frame.render_widget(
        Paragraph::new(vec![
            primitives::rule(appearance, rows[4].width),
            primitives::composer(appearance, &view.draft),
        ]),
        rows[4],
    );

    if !app.notifications.is_empty() {
        crate::notifications::render_notification_banner(frame, &app.notifications, area);
    }
}

fn transcript_lines(presentation: &NextPresentation, view: &TranscriptView) -> Vec<Line<'static>> {
    let appearance = &presentation.appearance;
    let text = |content: &str| {
        Line::from(Span::styled(
            content.to_string(),
            appearance.style(Role::Text),
        ))
    };
    let mut lines = Vec::new();
    for entry in &view.entries {
        lines.push(primitives::transcript_label(
            appearance,
            match entry.role {
                EntryRole::User => "you",
                EntryRole::Assistant => "coven",
            },
        ));
        lines.extend(entry.text.lines().map(text));
        if presentation.density == Density::Comfortable {
            lines.push(Line::default());
        }
    }
    for tool in &view.tools {
        // Completion is not verification: a finished tool is neutral evidence.
        let (status, word) = match tool.state {
            ToolActivityState::Running => (Status::Pending, "running"),
            ToolActivityState::Done => (Status::Evidence, "done"),
            ToolActivityState::Error => (Status::Blocked, "error"),
        };
        lines.push(primitives::tool_row(
            appearance, &tool.name, status, word, false,
        ));
    }
    if !view.live_text.is_empty() {
        lines.push(primitives::transcript_label(appearance, "coven"));
        lines.extend(view.live_text.lines().map(text));
    }
    lines
}

/// Split `text` into chunks of at most `width` display cells. A character
/// wider than the whole width still gets its own chunk.
fn wrap_cells(text: &str, width: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let cells = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cells > width && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(ch);
        used += cells;
    }
    chunks.push(current);
    chunks
}

/// Wrap a line to `width` cells. Lines that fit keep their spans; longer
/// lines are body text and wrap with the style of their first span.
fn wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if line.width() <= width {
        return vec![line];
    }
    let style = line
        .spans
        .first()
        .map(|span| span.style)
        .unwrap_or_default();
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    wrap_cells(&text, width)
        .into_iter()
        .map(|chunk| Line::from(Span::styled(chunk, style)))
        .collect()
}

fn render_transcript(
    frame: &mut Frame,
    presentation: &NextPresentation,
    view: &TranscriptView,
    area: Rect,
) {
    let width = usize::from(area.width.max(1));
    let mut lines: Vec<Line<'static>> = transcript_lines(presentation, view)
        .into_iter()
        .flat_map(|line| wrap_line(line, width))
        .collect();
    // Keep the newest content in view: drop the oldest rows that do not fit.
    let overflow = lines.len().saturating_sub(usize::from(area.height));
    lines.drain(..overflow);
    frame.render_widget(Paragraph::new(lines), area);
}

fn status_line(appearance: &Appearance, view: &TranscriptView, frame_count: u64) -> Line<'static> {
    let mut spans = if view.permission.is_some() {
        primitives::status_label(appearance, Status::Pending, "awaiting permission")
    } else if view.is_streaming {
        vec![Span::styled(
            format!("{} working", primitives::activity(appearance, frame_count)),
            appearance.style(Role::Presence),
        )]
    } else {
        vec![Span::styled("ready", appearance.style(Role::TextMuted))]
    };
    if let Some(status) = &view.status {
        spans.push(Span::styled(
            format!(" · {status}"),
            appearance.style(Role::TextMuted),
        ));
    }
    Line::from(spans)
}

fn hints_line(appearance: &Appearance, hints: &KeyHints) -> Line<'static> {
    match hints {
        KeyHints::Hints(parts) => Line::from(Span::styled(
            parts.join(&format!(
                " {} ",
                primitives::Mark::Separator.glyph(appearance.glyphs)
            )),
            appearance.style(Role::TextSubtle),
        )),
        KeyHints::Unreachable(actions) => primitives::status_message(
            appearance,
            Status::Blocked,
            "unreachable",
            &format!("no key reaches {}", actions.join(", ")),
        ),
        KeyHints::Conflict {
            chord,
            replaced,
            by,
        } => primitives::status_message(
            appearance,
            Status::Pending,
            "key conflict",
            &format!(
                "{chord} now {} (was {replaced})",
                by.as_deref().unwrap_or("unbound")
            ),
        ),
    }
}
