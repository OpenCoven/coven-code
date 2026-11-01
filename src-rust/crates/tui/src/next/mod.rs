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

use crate::app::App;
use crate::application::view::{EntryRole, ToolActivityState, TranscriptView};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// Header label; the canonical mark is SVG-only, so cells use plain text.
pub const PREVIEW_LABEL: &str = "Coven · preview";

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

    let view = TranscriptView::project(app);
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            PREVIEW_LABEL,
            Style::default().add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );
    render_transcript(frame, &view, rows[1]);
    frame.render_widget(Paragraph::new(status_line(&view)), rows[2]);
    render_composer(frame, &view, rows[3]);

    if !app.notifications.is_empty() {
        crate::notifications::render_notification_banner(frame, &app.notifications, area);
    }
}

/// One logical transcript row before wrapping.
struct Row {
    text: String,
    style: Style,
}

fn transcript_rows(view: &TranscriptView) -> Vec<Row> {
    let label = |text: &str| Row {
        text: text.to_string(),
        style: Style::default().add_modifier(Modifier::BOLD),
    };
    let plain = |text: &str| Row {
        text: text.to_string(),
        style: Style::default(),
    };
    let mut rows = Vec::new();
    for entry in &view.entries {
        rows.push(label(match entry.role {
            EntryRole::User => "you",
            EntryRole::Assistant => "coven",
        }));
        rows.extend(entry.text.lines().map(plain));
        rows.push(plain(""));
    }
    for tool in &view.tools {
        let state = match tool.state {
            ToolActivityState::Running => "running",
            ToolActivityState::Done => "done",
            ToolActivityState::Error => "error",
        };
        rows.push(plain(&format!("[tool {state}] {}", tool.name)));
    }
    if !view.live_text.is_empty() {
        rows.push(label("coven"));
        rows.extend(view.live_text.lines().map(plain));
    }
    rows
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

fn render_transcript(frame: &mut Frame, view: &TranscriptView, area: Rect) {
    let width = usize::from(area.width.max(1));
    let mut lines: Vec<Line<'static>> = transcript_rows(view)
        .into_iter()
        .flat_map(|row| {
            wrap_cells(&row.text, width)
                .into_iter()
                .map(move |chunk| Line::from(Span::styled(chunk, row.style)))
        })
        .collect();
    // Keep the newest content in view: drop the oldest rows that do not fit.
    let overflow = lines.len().saturating_sub(usize::from(area.height));
    lines.drain(..overflow);
    frame.render_widget(Paragraph::new(lines), area);
}

fn status_line(view: &TranscriptView) -> Line<'static> {
    let activity = if view.permission.is_some() {
        "awaiting permission"
    } else if view.is_streaming {
        "working"
    } else {
        "ready"
    };
    let mut text = activity.to_string();
    if let Some(status) = &view.status {
        text.push_str(" · ");
        text.push_str(status);
    }
    Line::from(Span::styled(
        text,
        Style::default().add_modifier(Modifier::DIM),
    ))
}

fn render_composer(frame: &mut Frame, view: &TranscriptView, area: Rect) {
    let mut lines = vec![Line::from("─".repeat(usize::from(area.width)))];
    lines.push(Line::from(format!("> {}", view.draft)));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}
