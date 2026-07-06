// render.rs — All ratatui rendering logic.

use std::cell::RefCell;

use crate::agents_view::render_agents_menu;
use crate::app::{
    readable_fg_on, App, ContextMenuKind, SystemAnnotation, SystemMessageStyle, ToolStatus,
};
use crate::ask_user_dialog::render_ask_user_dialog;
use crate::bypass_permissions_dialog::render_bypass_permissions_dialog;
use crate::context_viz::render_context_viz;
use crate::device_auth_dialog::render_device_auth_dialog;
use crate::dialog_select::render_dialog_select;
use crate::dialogs::{render_mcp_approval_dialog, render_permission_dialog};
use crate::diff_viewer::render_diff_dialog;
use crate::elicitation_dialog::render_elicitation_dialog;
use crate::export_dialog::render_export_dialog;
use crate::familiar_card;
use crate::familiar_image;
use crate::familiar_theme;
use crate::feedback_survey::render_feedback_survey;
use crate::figures;
use crate::file_injection_dialog::render_file_injection_dialog;
use crate::hooks_config_menu::render_hooks_config_menu;
use crate::import_config_dialog::render_import_config_dialog;
use crate::invalid_config_dialog::render_invalid_config_dialog;
use crate::key_input_dialog::render_key_input_dialog;
use crate::mcp_view::render_mcp_view;
use crate::memory_file_selector::render_memory_file_selector;
use crate::memory_update_notification::render_memory_update_notification;
use crate::messages::{
    render_thinking_live_content, render_transcript_assistant_message_tagged,
    render_transcript_assistant_meta, render_transcript_live_text, render_transcript_user_message,
    RenderContext,
};
use crate::model_picker::render_model_picker;
use crate::notifications::{render_notification_banner, Notification, NotificationKind};
use crate::onboarding_dialog::render_onboarding_dialog;
use crate::overage_upsell::render_overage_upsell;
use crate::overlays::{
    render_global_search, render_help_overlay, render_history_search_overlay, render_rewind_flow,
    COVEN_CODE_ACCENT, COVEN_CODE_APP_BG, COVEN_CODE_MUTED, COVEN_CODE_TEXT,
};
use crate::plugin_views::render_plugin_hints;
use crate::prompt_input::{input_height, render_prompt_input, InputMode, TypeaheadSource, VimMode};
use crate::session_branching::render_session_branching;
use crate::session_browser::render_session_browser;
use crate::settings_screen::render_settings_screen;
use crate::stats_dialog::render_stats_dialog;
use crate::tasks_overlay::render_tasks_overlay;
use crate::theme_screen::render_theme_screen;
use crate::transcript_turn::{build_transcript_turns, TranscriptTurn};
use crate::virtual_list::{VirtualItem, VirtualList};
use crate::voice_mode_notice::render_voice_mode_notice;
use claurst_core::constants::APP_VERSION;
use claurst_core::types::Role;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

// Spinner frames matching the TypeScript SpinnerGlyph: platform-specific base
// characters mirrored (forward + reverse) for a smooth pulse effect.
// Windows uses '*' instead of '✳'/'✽' for better font coverage.
#[cfg(target_os = "windows")]
const SPINNER: &[char] = &[
    '\u{00b7}', '\u{2722}', '*', '\u{2736}', '\u{273b}', '\u{273d}', '\u{273d}', '\u{273b}',
    '\u{2736}', '*', '\u{2722}', '\u{00b7}',
];
#[cfg(not(target_os = "windows"))]
const SPINNER: &[char] = &[
    '\u{00b7}', '\u{2722}', '\u{2733}', '\u{2736}', '\u{273b}', '\u{273d}', '\u{273d}', '\u{273b}',
    '\u{2736}', '\u{2733}', '\u{2722}', '\u{00b7}',
];
const COVEN_VIOLET: Color = Color::Rgb(184, 175, 220);
const SPINNER_FRAME_DIVISOR: u64 = 2;
// 13 rows leave an 11-row interior. Left column: Welcome (1) + blank (1) +
// avatar (4) + blank (1) + metadata (3) + optional Goal (1). Right column:
// Tips header (1) + tip (1-2) + rule (1) + What's new header (1) + 3 highlights
// + "/release-notes" footer (1). Both fit cleanly inside the interior.
const WELCOME_BOX_HEIGHT: u16 = 13;
// Cap the welcome box width on large terminals so it doesn't stretch
// edge-to-edge; everything inside fits comfortably within 120 columns.
const WELCOME_BOX_MAX_WIDTH: u16 = 120;
const STATUS_THINKING: &str = "thinking";
const STATUS_THINKING_ELLIPSIS: &str = "thinking\u{2026}";
const STREAM_STALL_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(3);
const STREAM_WAITING_LABEL: &str = "Waiting on network";
const STREAM_STALLED_LABEL: &str = "Stalled — check connection, Ctrl+C to interrupt";

fn spinner_char(frame_count: u64) -> char {
    SPINNER[((frame_count / SPINNER_FRAME_DIVISOR) as usize) % SPINNER.len()]
}

/// Returns the colour to use for the streaming spinner.
/// Turns red when no stream data has arrived for more than 3 seconds.
fn spinner_color(app: &App) -> Color {
    if stream_is_stalled(app) {
        return Color::Red;
    }
    Color::Yellow
}

fn stream_is_stalled(app: &App) -> bool {
    app.stall_start
        .is_some_and(|start| start.elapsed() > STREAM_STALL_THRESHOLD)
}

fn is_modal_open(app: &App) -> bool {
    app.any_modal_open()
}

// -----------------------------------------------------------------------
// Error modal rendering
// -----------------------------------------------------------------------

/// Render an error modal dialog with wrapped content.
fn render_error_modal(
    frame: &mut Frame,
    area: Rect,
    notification: &Notification,
    _scroll_offset: usize,
    footer_area: Rect,
    is_welcome_screen: bool,
) {
    // When the footer anchor is inside the welcome box (y < WELCOME_BOX_HEIGHT), or explicitly on
    // the welcome screen, center the modal so it doesn't awkwardly overlap the welcome box.
    let anchored_in_welcome_box = footer_area.width > 0 && footer_area.y < WELCOME_BOX_HEIGHT;
    let modal_area = if is_welcome_screen || anchored_in_welcome_box {
        let modal_width = (area.width * 2 / 3).max(40).min(area.width);
        let modal_height = (area.height / 3).max(8).min(area.height.saturating_sub(2));
        Rect {
            x: area.x + (area.width.saturating_sub(modal_width)) / 2,
            y: area.y + (area.height.saturating_sub(modal_height)) / 2,
            width: modal_width,
            height: modal_height,
        }
    } else if footer_area.width > 0 {
        let desired_height = (area.height / 3)
            .max(8)
            .min(area.height.saturating_sub(footer_area.y));
        Rect {
            x: footer_area.x,
            y: footer_area.y,
            width: footer_area.width,
            height: desired_height,
        }
    } else {
        let modal_width = area.width / 2;
        let modal_height = area.height.saturating_sub(4);
        Rect {
            x: area.x + modal_width,
            y: area.y,
            width: modal_width,
            height: modal_height,
        }
    };

    frame.render_widget(Clear, modal_area);

    let modal_block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().fg(Color::Red));
    frame.render_widget(modal_block, modal_area);

    let header_bg_area = Rect {
        x: modal_area.x + 1,
        y: modal_area.y + 1,
        width: modal_area.width.saturating_sub(2),
        height: 1,
    };
    let header_style = Style::default().bg(Color::Rgb(60, 15, 15)).fg(Color::Red);
    let header_para =
        Paragraph::new("  ⚠ Error  ").style(header_style.add_modifier(Modifier::BOLD));
    frame.render_widget(header_para, header_bg_area);

    let sep_area = Rect {
        x: modal_area.x + 1,
        y: modal_area.y + 2,
        width: modal_area.width.saturating_sub(2),
        height: 1,
    };
    let sep_line = Paragraph::new(Line::from(Span::styled(
        "─".repeat(sep_area.width as usize),
        Style::default().fg(Color::Rgb(80, 20, 20)),
    )));
    frame.render_widget(sep_line, sep_area);

    // Chrome: border(1) + header(1) + sep(1) + blank(1) + border(1) = 5 rows
    let body_start_y = modal_area.y + 4;
    let body_height = modal_area.height.saturating_sub(5).max(1);
    let body_area = Rect {
        x: modal_area.x + 2,
        y: body_start_y,
        width: modal_area.width.saturating_sub(4),
        height: body_height,
    };

    let body_para = Paragraph::new(notification.message.as_str())
        .style(Style::default().fg(Color::Rgb(220, 220, 220)))
        .wrap(Wrap { trim: true });
    frame.render_widget(body_para, body_area);
}

// -----------------------------------------------------------------------
// Text truncation helpers
// -----------------------------------------------------------------------

fn truncate_end(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return "\u{2026}".to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
        if width + ch_width >= max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push('\u{2026}');
    out
}

fn truncate_middle(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return truncate_end(text, max_width);
    }
    let keep_each_side = (max_width.saturating_sub(1)) / 2;
    let left: String = text.chars().take(keep_each_side).collect();
    let right: String = text
        .chars()
        .rev()
        .take(keep_each_side)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{left}\u{2026}{right}")
}

fn truncate_text(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let next = format!("{out}{ch}");
        if next.width() > max_width {
            if max_width > 1 && out.width() < max_width {
                out.push('\u{2026}');
            }
            break;
        }
        out.push(ch);
    }
    out
}

// -----------------------------------------------------------------------
// Startup notice helpers
// -----------------------------------------------------------------------

fn startup_notice_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let max_width = width.saturating_sub(10) as usize;

    if let Some(summary) = app.away_summary.as_deref() {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", crate::figures::REFERENCE_MARK),
                Style::default()
                    .fg(COVEN_VIOLET)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_end(summary, max_width),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    match &app.bridge_state {
        crate::bridge_state::BridgeConnectionState::Connected { peer_count, .. } => {
            let label = if *peer_count > 0 {
                format!(
                    "Remote session active \u{00b7} {} peer{}",
                    peer_count,
                    if *peer_count == 1 { "" } else { "s" }
                )
            } else {
                "Remote session active".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(" remote ", Style::default().fg(COVEN_VIOLET)),
                Span::styled(label, Style::default().fg(Color::DarkGray)),
            ]));
        }
        crate::bridge_state::BridgeConnectionState::Reconnecting { attempt } => {
            lines.push(Line::from(vec![
                Span::styled(" remote ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("Reconnecting remote session (attempt #{attempt})"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        crate::bridge_state::BridgeConnectionState::Failed { reason } => {
            lines.push(Line::from(vec![
                Span::styled(" remote ", Style::default().fg(Color::Red)),
                Span::styled(
                    truncate_end(reason, max_width),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        _ => {}
    }

    if let Some(url) = app.remote_session_url.as_deref() {
        lines.push(Line::from(vec![
            Span::styled(" link ", Style::default().fg(COVEN_VIOLET)),
            Span::styled(
                truncate_end(url, max_width),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    // Additional directories (from --add-dir)
    for dir in &app.config.additional_dirs {
        lines.push(Line::from(vec![
            Span::styled(" +dir ", Style::default().fg(Color::Cyan)),
            Span::styled(
                truncate_end(&dir.display().to_string(), max_width),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines
}

fn render_startup_notices(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let lines = startup_notice_lines(app, area.width);
    if lines.is_empty() {
        return;
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

#[derive(Clone)]
struct RenderedLineItem {
    line: Line<'static>,
    search_text: String,
    is_header: bool,
    message_index: Option<usize>,
    /// If this line is the clickable header of a thinking block, its hash.
    thinking_hash: Option<u64>,
}

impl VirtualItem for RenderedLineItem {
    fn measure_height(&self, _width: u16) -> u16 {
        1
    }

    fn render(&self, area: Rect, buf: &mut Buffer, _selected: bool) {
        Paragraph::new(vec![self.line.clone()]).render(area, buf);
    }

    fn search_text(&self) -> String {
        self.search_text.clone()
    }

    fn is_section_header(&self) -> bool {
        self.is_header
    }
}

fn flatten_line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.to_string())
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MessageLinesCacheKey {
    width: u16,
    transcript_version: u64,
    messages_ptr: usize,
    messages_len: usize,
    annotations_ptr: usize,
    annotations_len: usize,
    thinking_expanded_len: usize,
}

#[derive(Clone)]
struct MessageLinesCache {
    key: MessageLinesCacheKey,
    lines: Vec<RenderedLineItem>,
}

/// Cache key for completed messages only (no ptr — len change = new message).
#[derive(Clone, Copy, PartialEq, Eq)]
struct CompletedMsgCacheKey {
    width: u16,
    transcript_version: u64,
    messages_len: usize,
    annotations_len: usize,
    thinking_expanded_len: usize,
}

#[derive(Clone)]
struct CompletedMsgCache {
    key: CompletedMsgCacheKey,
    lines: Vec<RenderedLineItem>,
}

thread_local! {
    static MESSAGE_LINES_CACHE: RefCell<Option<MessageLinesCache>> = const { RefCell::new(None) };
    /// Stores rendered lines for committed messages only; valid even during streaming.
    static COMPLETED_MSG_CACHE: RefCell<Option<CompletedMsgCache>> = const { RefCell::new(None) };
}

// -----------------------------------------------------------------------
// Top-level layout
// -----------------------------------------------------------------------

/// Render the entire application into the current frame.
pub fn render_app(frame: &mut Frame, app: &App) {
    let size = frame.area();
    app.last_selectable_area.set(size);

    // Fill the entire frame with a true RGB app background so terminal theme
    // palette overrides for ANSI black do not bleed through the TUI shell.
    frame.render_widget(
        Block::default().style(Style::default().bg(COVEN_CODE_APP_BG).fg(COVEN_CODE_TEXT)),
        size,
    );

    let prompt_focused = app.permission_request.is_none() && !app.history_search_overlay.visible;
    // Suggestions popup tracks whether the prompt accepts input, not whether
    // it is the focused widget. Text entry is allowed during streaming so the
    // user can queue the next message, so the typeahead popup must follow
    // that same affordance.
    let suggestions_visible =
        app.permission_request.is_none() && !app.history_search_overlay.visible;
    let status_visible = should_render_status_row(app);
    // One blank separator row above the status/input area when status is active,
    // matching the visual breathing room in the TS layout.
    let separator_height: u16 = if status_visible { 1 } else { 0 };
    let status_height: u16 = if status_visible {
        if app.is_streaming {
            // The spinner row is always a short single line.
            1
        } else if let Some(text) = app.status_message.as_deref() {
            // Measure how many terminal rows the message needs so that long
            // error strings (e.g. "Error: overloaded_error (529): …") wrap
            // instead of overflowing the input area.  Cap at 3 lines.
            let usable_width = size.width.max(1) as usize;
            let char_count = text.chars().count();
            char_count.div_ceil(usable_width).clamp(1, 3) as u16
        } else {
            1
        }
    } else {
        0
    };
    let suggestions_height = if suggestions_visible && !app.prompt_input.suggestions.is_empty() {
        app.prompt_input.suggestions.len().min(5) as u16
    } else {
        0
    };
    // The prompt body width is the terminal width minus the prompt prefix
    // ("> ") and the right-margin padding used inside `render_prompt_input`.
    // Keep this in sync with prefix_width=2 + right_pad=2 there.
    let prompt_text_width = size.width.saturating_sub(4);
    let prompt_height = input_height(&app.prompt_input, prompt_text_width) + 1; // +1 for model/mode status line

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(separator_height),
            Constraint::Length(status_height),
            Constraint::Length(prompt_height),
            Constraint::Length(suggestions_height),
            Constraint::Length(2),
        ])
        .split(size);

    render_messages(frame, app, chunks[0]);
    // chunks[1] is the blank separator — intentionally left empty
    if status_height > 0 {
        render_status_row(frame, app, chunks[2]);
    }
    render_input(frame, app, chunks[3], prompt_focused);
    app.last_input_area.set(chunks[3]);
    if suggestions_height > 0 {
        render_prompt_suggestions(frame, app, chunks[4]);
    }
    render_footer(frame, app, chunks[5]);

    // Overlays (rendered on top in Z-order)

    // Rewind flow (takes over screen)
    if app.rewind_flow.visible {
        render_rewind_flow(frame, &app.rewind_flow, size);
    }

    // Tasks overlay (Ctrl+T)
    if app.tasks_overlay.visible {
        render_tasks_overlay(frame, &app.tasks_overlay, size);
    }

    // Help overlay
    if app.help_overlay.visible {
        render_help_overlay(frame, &app.help_overlay, size);
    }

    // History search overlay
    if app.history_search_overlay.visible {
        render_history_search_overlay(
            frame,
            &app.history_search_overlay,
            &app.prompt_input.history,
            size,
        );
    }

    // Settings screen (highest-priority full-screen overlay)
    if app.settings_screen.visible {
        render_settings_screen(frame, &app.settings_screen, size);
    }

    // Theme picker overlay
    if app.theme_screen.visible {
        render_theme_screen(frame, &app.theme_screen, size);
    }

    if app.stats_dialog.visible {
        render_stats_dialog(&app.stats_dialog, size, frame.buffer_mut());
    }

    if app.mcp_view.visible {
        render_mcp_view(&app.mcp_view, size, frame.buffer_mut());
    }

    if app.agents_menu.visible {
        render_agents_menu(&app.agents_menu, size, frame.buffer_mut());
    }

    if app.diff_viewer.visible {
        let mut state = app.diff_viewer.clone();
        render_diff_dialog(&mut state, size, frame.buffer_mut());
    }

    if app.global_search.visible {
        render_global_search(&app.global_search, size, frame.buffer_mut());
    }

    // Familiar switcher popup (F2)
    if app.familiar_switcher_open {
        render_familiar_switcher(frame, app, size);
    }

    if app.feedback_survey.visible {
        render_feedback_survey(&app.feedback_survey, size, frame.buffer_mut());
    }

    if app.memory_file_selector.visible {
        render_memory_file_selector(&app.memory_file_selector, size, frame.buffer_mut());
    }

    if app.hooks_config_menu.visible {
        render_hooks_config_menu(&app.hooks_config_menu, size, frame.buffer_mut());
    }

    // Overage credit upsell banner
    if app.overage_upsell.visible {
        let banner_h = app.overage_upsell.height();
        if size.height > banner_h + 4 {
            let banner_area = Rect {
                x: size.x,
                y: size.y,
                width: size.width,
                height: banner_h,
            };
            render_overage_upsell(&app.overage_upsell, banner_area, frame.buffer_mut());
        }
    }

    // Voice mode availability notice
    if app.voice_mode_notice.visible {
        let notice_h = app.voice_mode_notice.height();
        if size.height > notice_h + 4 {
            let notice_area = Rect {
                x: size.x,
                y: size.y,
                width: size.width,
                height: notice_h,
            };
            render_voice_mode_notice(&app.voice_mode_notice, notice_area, frame.buffer_mut());
        }
    }

    // Memory update notification banner (bottom of message area)
    if app.memory_update_notification.visible {
        let notif_h = app.memory_update_notification.height();
        if size.height > notif_h + 4 {
            // Place at the bottom of the screen, just above the prompt bar area
            let notif_y = size.y + size.height.saturating_sub(notif_h + 4);
            let notif_area = Rect {
                x: size.x,
                y: notif_y,
                width: size.width,
                height: notif_h,
            };
            render_memory_update_notification(
                &app.memory_update_notification,
                notif_area,
                frame.buffer_mut(),
            );
        }
    }

    // Import-config preview dialog
    if app.import_config_dialog.visible {
        render_import_config_dialog(frame, &app.import_config_dialog, size);
    }

    // Invalid config/settings dialog (shown when settings.json or AGENTS.md is malformed)
    if app.invalid_config_dialog.visible {
        render_invalid_config_dialog(frame, &app.invalid_config_dialog, size);
    }

    // Bypass-permissions confirmation dialog (topmost — rendered last so it sits above all)
    if app.bypass_permissions_dialog.visible {
        render_bypass_permissions_dialog(frame, &app.bypass_permissions_dialog, size);
    }

    // File injection warning dialog (shown when oversized/binary files detected)
    if app.file_injection_dialog.visible {
        render_file_injection_dialog(frame, &app.file_injection_dialog, size);
    }

    // AskUserQuestion dialog — renders above bypass-permissions so the model's
    // question is never obscured by the startup confirmation prompt.
    if app.ask_user_dialog.visible {
        render_ask_user_dialog(&app.ask_user_dialog, size, frame.buffer_mut());
    }

    // First-launch onboarding dialog (shown after bypass dialog, below elicitation)
    if app.onboarding_dialog.visible {
        render_onboarding_dialog(frame, &app.onboarding_dialog, size);
    }

    // /effort picker
    if app.effort_picker.visible {
        crate::effort_picker::render_effort_picker(frame, &app.effort_picker, size);
    }

    // /skills picker
    if app.skills_picker.visible {
        crate::skills_picker::render_skills_picker(frame, &app.skills_picker, size);
    }

    // Import-config source picker
    if app.import_config_picker.visible {
        render_dialog_select(frame, &app.import_config_picker, size);
    }

    // Connect-a-provider dialog (/connect command)
    if app.connect_dialog.visible {
        render_dialog_select(frame, &app.connect_dialog, size);
    }

    // API key input dialog (opened from /connect for key-based providers)
    if app.key_input_dialog.visible {
        render_key_input_dialog(frame, &app.key_input_dialog, size);
    }

    // Device code / browser auth dialog (Claude OAuth, Codex login)
    if app.device_auth_dialog.visible {
        render_device_auth_dialog(frame, &app.device_auth_dialog, size);
    }

    // Ctrl+K command palette
    if app.command_palette.visible {
        render_dialog_select(frame, &app.command_palette, size);
    }

    // MCP elicitation dialog (highest priority modal — rendered last to sit on top)
    if app.elicitation.visible {
        render_elicitation_dialog(&app.elicitation, size, frame.buffer_mut());
    }

    // Model picker overlay
    if app.model_picker.visible {
        render_model_picker(&app.model_picker, size, frame.buffer_mut());
    }

    // Session browser overlay
    if app.session_browser.visible {
        render_session_browser(&app.session_browser, size, frame.buffer_mut());
    }

    // Session branching overlay
    if app.session_branching.visible {
        render_session_branching(&app.session_branching, size, frame.buffer_mut());
    }

    // Export format picker dialog
    if app.export_dialog.visible {
        render_export_dialog(frame, &app.export_dialog, size);
    }

    // Context visualization overlay
    if app.context_viz.visible {
        render_context_viz(
            frame,
            &app.context_viz,
            size,
            app.context_used_tokens,
            app.context_window_size,
            app.rate_limit_5h_pct,
            app.rate_limit_7day_pct,
            app.cost_usd,
        );
    }

    // MCP approval dialog
    if app.mcp_approval.visible {
        render_mcp_approval_dialog(&app.mcp_approval, size, frame.buffer_mut());
    }

    // Error modals sit above non-security overlays. If a permission request is
    // also pending, render the permission dialog after the error modal so the
    // security-sensitive prompt remains visible for the input it owns.
    if let Some(notif) = app.notifications.current() {
        if notif.kind == NotificationKind::Error {
            let is_welcome_screen = app.messages.is_empty()
                && app.streaming_text.is_empty()
                && app.streaming_thinking.is_empty()
                && app.tool_use_blocks.is_empty();
            render_error_modal(
                frame,
                size,
                notif,
                app.error_modal_scroll_offset,
                app.footer_right_column_area.get(),
                is_welcome_screen,
            );
            if let Some(ref pr) = app.permission_request {
                render_permission_dialog(frame, pr, size);
            }
            return; // Don't render other overlays/notifications when error modal is showing
        }
    }

    let modal_active = is_modal_open(app);

    // Render non-error notifications as toast banners (unless another modal is open)
    if !modal_active && app.notifications.current().is_some() {
        render_notification_banner(frame, &app.notifications, size);
    }

    // ---- Text selection highlight (topmost post-pass) ---------------------
    apply_selection_highlight(frame, app);
    cache_selectable_row_text(frame, app);
    render_context_menu(frame, app);

    // Permission dialog is rendered last so the visible security prompt matches
    // the key handler priority in the interactive event loop.
    if let Some(ref pr) = app.permission_request {
        render_permission_dialog(frame, pr, size);
    }
}

/// Snapshot the rendered text of every row inside the selectable area into
/// `app.last_row_text` so that subsequent double/triple-clicks can locate
/// word and paragraph boundaries (issue #149 follow-up).
fn cache_selectable_row_text(frame: &mut Frame, app: &App) {
    let selectable_area = app.last_selectable_area.get();
    if selectable_area.width == 0 || selectable_area.height == 0 {
        app.last_row_text.borrow_mut().clear();
        return;
    }
    let buf = frame.buffer_mut();
    let max_row = selectable_area
        .y
        .saturating_add(selectable_area.height)
        .saturating_sub(1);
    let max_col = selectable_area
        .x
        .saturating_add(selectable_area.width)
        .saturating_sub(1);
    let mut cache = app.last_row_text.borrow_mut();
    cache.clear();
    for row in selectable_area.y..=max_row {
        let mut s = String::new();
        for col in selectable_area.x..=max_col {
            if let Some(cell) = buf.cell_mut((col, row)) {
                let sym = cell.symbol();
                if sym.is_empty() || sym == "\0" {
                    s.push(' ');
                } else {
                    s.push_str(sym);
                }
            }
        }
        cache.insert(row, s);
    }
}

/// Post-render pass: invert colours on selected cells and extract the
/// selection text into `app.selection_text`.
fn apply_selection_highlight(frame: &mut Frame, app: &App) {
    let (anchor, focus) = match (app.selection_anchor, app.selection_focus) {
        (Some(a), Some(f)) => (a, f),
        _ => return,
    };
    if anchor == focus {
        return;
    }

    let selectable_area = app.last_selectable_area.get();
    if selectable_area.width == 0 || selectable_area.height == 0 {
        return;
    }

    // Validate selection is within selectable bounds
    if anchor.0 < selectable_area.x
        || anchor.0 >= selectable_area.x.saturating_add(selectable_area.width)
        || anchor.1 < selectable_area.y
        || anchor.1 >= selectable_area.y.saturating_add(selectable_area.height)
    {
        return;
    }

    let max_row = selectable_area
        .y
        .saturating_add(selectable_area.height)
        .saturating_sub(1);
    let max_col = selectable_area
        .x
        .saturating_add(selectable_area.width)
        .saturating_sub(1);

    // Clamp anchor and focus to selectable bounds
    let anchor = (
        anchor.0.clamp(selectable_area.x, max_col),
        anchor.1.clamp(selectable_area.y, max_row),
    );
    let focus = (
        focus.0.clamp(selectable_area.x, max_col),
        focus.1.clamp(selectable_area.y, max_row),
    );

    // Normalise so start ≤ end (row-major order).
    let (start, end) = if (anchor.1, anchor.0) <= (focus.1, focus.0) {
        (anchor, focus)
    } else {
        (focus, anchor)
    };

    let buf = frame.buffer_mut();
    let mut text = String::new();
    let last_row = end.1.min(max_row);
    for row in start.1..=last_row {
        let col_from = if row == start.1 {
            start.0
        } else {
            selectable_area.x
        };
        let col_to = if row == end.1 { end.0 } else { max_col };
        for col in col_from..=col_to {
            if let Some(cell) = buf.cell_mut((col, row)) {
                let sym = cell.symbol().to_owned();
                text.push_str(if sym.is_empty() || sym == "\0" {
                    " "
                } else {
                    &sym
                });
                // Highlight: white background, black foreground
                let new_style = Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(200, 200, 200));
                cell.set_style(new_style);
            }
        }
        if row < last_row {
            // Trim trailing spaces from line before newline
            while text.ends_with(' ') {
                text.pop();
            }
            text.push('\n');
        }
    }
    while text.ends_with(|c: char| c.is_whitespace()) {
        text.pop();
    }
    *app.selection_text.borrow_mut() = text;
}

/// Render a right-click context menu at the specified position.
fn render_context_menu(frame: &mut Frame, app: &App) {
    if let Some(menu) = app.context_menu_state {
        let selection_present = !app.selection_text.borrow().trim().is_empty();
        let items: Vec<(&str, bool)> = match menu.kind {
            ContextMenuKind::Message { message_index } => vec![
                ("Copy", app.messages.get(message_index).is_some()),
                ("Fork new chat", app.messages.get(message_index).is_some()),
            ],
            ContextMenuKind::Selection => vec![("Copy", selection_present)],
        };

        let menu_height = (items.len() as u16).saturating_add(2);
        let menu_width = items
            .iter()
            .map(|(label, _)| label.len())
            .max()
            .unwrap_or(4)
            .saturating_add(4) as u16;

        // Clamp menu position to screen bounds
        let screen = frame.area();
        let menu_x = menu.x.min(screen.width.saturating_sub(menu_width + 1));
        let menu_y = menu.y.min(screen.height.saturating_sub(menu_height + 1));

        let menu_area = Rect {
            x: menu_x,
            y: menu_y,
            width: menu_width,
            height: menu_height,
        };

        // Draw menu background with border
        let menu_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .style(Style::default().fg(Color::White).bg(Color::Rgb(24, 24, 30)))
            .border_style(Style::default().fg(COVEN_CODE_ACCENT));
        menu_block.render(menu_area, frame.buffer_mut());

        // Render menu items
        let inner = Rect {
            x: menu_area.x + 1,
            y: menu_area.y + 1,
            width: menu_area.width.saturating_sub(2),
            height: menu_area.height.saturating_sub(2),
        };

        for (idx, (label, enabled)) in items.iter().enumerate() {
            if idx >= inner.height as usize {
                break;
            }

            let y = inner.y + idx as u16;
            let is_selected = idx == menu.selected_index;

            let fg_color = if *enabled {
                if is_selected {
                    Color::Black
                } else {
                    Color::White
                }
            } else {
                Color::DarkGray
            };

            let bg_color = if is_selected {
                if *enabled {
                    COVEN_CODE_ACCENT
                } else {
                    Color::Rgb(24, 24, 30)
                }
            } else {
                Color::Rgb(24, 24, 30)
            };

            let style = Style::default().fg(fg_color).bg(bg_color);
            let padded_label = format!(
                " {:<width$} ",
                label,
                width = menu_width.saturating_sub(2) as usize
            );

            if let Some(cell) = frame.buffer_mut().cell_mut((inner.x, y)) {
                cell.set_symbol(&padded_label[0..1.min(padded_label.len())]);
                cell.set_style(style);
            }

            for (col_offset, ch) in padded_label.chars().enumerate() {
                if col_offset >= inner.width as usize {
                    break;
                }
                if let Some(cell) = frame
                    .buffer_mut()
                    .cell_mut((inner.x + col_offset as u16, y))
                {
                    cell.set_symbol(&ch.to_string());
                    cell.set_style(style);
                }
            }
        }
    }
}

// -----------------------------------------------------------------------
// Messages pane
// -----------------------------------------------------------------------

fn render_messages(frame: &mut Frame, app: &App, area: Rect) {
    // Reserve space at the top for plugin hint banners
    let hint_height = if app.plugin_hints.iter().any(|h| h.is_visible()) {
        3u16
    } else {
        0
    };

    let (hint_area, content_area) = if hint_height > 0 && area.height > hint_height + 2 {
        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(hint_height), Constraint::Min(1)])
            .split(area);
        (Some(splits[0]), splits[1])
    } else {
        (None, area)
    };

    // Render plugin hint banner if there is one
    if let Some(ha) = hint_area {
        render_plugin_hints(frame, &app.plugin_hints, ha);
    }

    let notice_lines = startup_notice_lines(app, content_area.width);
    let header_height = WELCOME_BOX_HEIGHT + notice_lines.len() as u16;
    let show_splash = app.config.show_splash_enabled();
    let show_logo_header =
        show_splash && content_area.height >= header_height + 3 && content_area.width >= 60;
    let (logo_area, notices_area, msg_area) = if show_logo_header {
        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(1)])
            .split(content_area);
        if notice_lines.is_empty() {
            (Some(splits[0]), None, splits[1])
        } else {
            let header_splits = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(WELCOME_BOX_HEIGHT),
                    Constraint::Length(notice_lines.len() as u16),
                ])
                .split(splits[0]);
            (Some(header_splits[0]), Some(header_splits[1]), splits[1])
        }
    } else {
        (None, None, content_area)
    };

    if let Some(la) = logo_area {
        render_welcome_box(frame, app, la);
        if let Some(na) = notices_area {
            render_startup_notices(frame, app, na);
        }
    } else if show_splash
        && app.messages.is_empty()
        && app.streaming_text.is_empty()
        && app.streaming_thinking.is_empty()
        && app.tool_use_blocks.is_empty()
    {
        app.last_msg_area.set(Rect::default());
        app.message_row_map.borrow_mut().clear();
        app.thinking_row_map.borrow_mut().clear();
        render_welcome_box(frame, app, content_area);
        return;
    }

    // Store the actual message pane bounds for mouse event handling (text selection, scrolling).
    app.last_msg_area.set(msg_area);

    let lines = render_message_items(app, msg_area.width);

    // Highlight search matches in transcript when global search is active
    let lines = if app.global_search.visible && !app.global_search.query.is_empty() {
        let query_lc = app.global_search.query.to_lowercase();
        lines
            .into_iter()
            .map(|mut item| {
                if item.search_text.to_lowercase().contains(query_lc.as_str()) {
                    // Re-render the line with yellow highlight on matching spans
                    let highlighted_spans: Vec<Span<'static>> = item
                        .line
                        .spans
                        .into_iter()
                        .map(|span| {
                            if span.content.to_lowercase().contains(query_lc.as_str()) {
                                Span::styled(
                                    span.content,
                                    span.style.bg(Color::Rgb(60, 50, 0)).fg(Color::Yellow),
                                )
                            } else {
                                span
                            }
                        })
                        .collect();
                    item.line = ratatui::text::Line::from(highlighted_spans);
                }
                item
            })
            .collect()
    } else {
        lines
    };

    // Compute total virtual height and apply scroll clamping.
    // When auto_scroll is on we always show the tail; otherwise we respect
    // the user's scroll_offset.
    let content_height = lines.len() as u16;
    let visible_height = msg_area.height; // no borders, full height available
    let max_scroll = content_height.saturating_sub(visible_height) as usize;
    // scroll_offset counts lines above the bottom (0 = at bottom).
    // ratatui scroll() takes an absolute top-row index, so convert:
    //   top_row = max_scroll - scroll_offset  (clamped to [0, max_scroll])
    let scroll = if app.auto_scroll {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.scroll_offset)
    };

    let mut visible_rows: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
    let mut thinking_rows: std::collections::HashMap<u16, u64> = std::collections::HashMap::new();
    for (idx, item) in lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(msg_area.height as usize)
    {
        let screen_row = msg_area
            .y
            .saturating_add((idx.saturating_sub(scroll)) as u16);
        if let Some(message_index) = item.message_index {
            visible_rows.insert(screen_row, message_index);
        }
        if let Some(hash) = item.thinking_hash {
            thinking_rows.insert(screen_row, hash);
        }
    }
    *app.message_row_map.borrow_mut() = visible_rows;
    *app.thinking_row_map.borrow_mut() = thinking_rows;

    // No border — messages render directly into the area.
    let mut list = VirtualList::new();
    list.viewport_height = msg_area.height;
    list.sticky_bottom = app.auto_scroll;
    list.set_items(lines);
    list.scroll_offset = scroll as u16;

    // Track scroll offset for selection validation
    app.last_render_scroll_offset.set(scroll as u16);

    list.render(msg_area, frame.buffer_mut());

    // Scrollbar: thin vertical strip flush with the right edge — no arrow
    // caps, no visible track, muted thumb color. Mirrors Windows Terminal /
    // most modern terminal scrollbars rather than ratatui's chunky default.
    if content_height > visible_height {
        use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

        // ratatui 0.29's Scrollbar maps `position` over `content_length - 1`,
        // not over a 0..=max_scroll range. Passing `content_height` directly
        // makes the thumb top out at `content / (content + viewport)` of the
        // track when fully scrolled — i.e. it never reaches the bottom.
        // Fix: tell ratatui the content length is the number of distinct
        // scroll positions (`max_scroll + 1`), keeping `viewport_content_length`
        // for the proportional thumb size.
        let content_len = max_scroll + 1;
        let mut scrollbar_state = ScrollbarState::new(content_len)
            .position(scroll.min(max_scroll))
            .viewport_content_length(visible_height as usize);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None)
            .thumb_symbol("\u{2590}") // ▐ right half block — thin vertical strip
            .thumb_style(Style::default().fg(Color::Rgb(110, 110, 130)));

        frame.render_stateful_widget(scrollbar, msg_area, &mut scrollbar_state);
    }

    // “â†” N new messages” indicator when scrolled up and new messages arrived.
    if app.new_messages_while_scrolled > 0 && msg_area.height > 4 && msg_area.width > 20 {
        let indicator = format!(
            " \u{2193} {} new message{} ",
            app.new_messages_while_scrolled,
            if app.new_messages_while_scrolled == 1 {
                ""
            } else {
                "s"
            }
        );
        let ind_len = indicator.len() as u16;
        let ind_x = msg_area
            .x
            .saturating_add(msg_area.width.saturating_sub(ind_len + 2));
        let ind_y = msg_area.y + msg_area.height.saturating_sub(1);
        let ind_area = Rect {
            x: ind_x,
            y: ind_y,
            width: ind_len.min(msg_area.width.saturating_sub(2)),
            height: 1,
        };
        let ind_line = Line::from(vec![Span::styled(
            indicator,
            Style::default()
                .fg(Color::Black)
                .bg(COVEN_VIOLET)
                .add_modifier(Modifier::BOLD),
        )]);
        frame.render_widget(Paragraph::new(vec![ind_line]), ind_area);
    }
}

fn push_rendered_items(
    items: &mut Vec<RenderedLineItem>,
    lines: Vec<Line<'static>>,
    message_index: Option<usize>,
    mark_first_header: bool,
) {
    for (index, line) in lines.into_iter().enumerate() {
        items.push(RenderedLineItem {
            search_text: flatten_line_text(&line),
            is_header: mark_first_header && index == 0,
            message_index,
            thinking_hash: None,
            line,
        });
    }
}

/// Push tagged lines from `render_transcript_assistant_message_tagged`.
/// Lines with `Some(hash)` become clickable thinking headers.
fn push_rendered_items_tagged(
    items: &mut Vec<RenderedLineItem>,
    tagged: Vec<(Line<'static>, Option<u64>)>,
    message_index: Option<usize>,
) {
    for (line, thinking_hash) in tagged {
        items.push(RenderedLineItem {
            search_text: flatten_line_text(&line),
            is_header: false,
            message_index,
            thinking_hash,
            line,
        });
    }
}

fn push_blank_item(items: &mut Vec<RenderedLineItem>) {
    push_rendered_items(items, vec![Line::from("")], None, false);
}

fn render_live_thinking_lines(
    turn: &TranscriptTurn<'_>,
    frame_count: u64,
    width: u16,
) -> Vec<Line<'static>> {
    let mut header_spans = vec![Span::raw("  ▼ ")];
    header_spans.extend(shimmer_spans("Thinking", frame_count));
    if let Some(heading) = turn.reasoning_heading() {
        header_spans.push(Span::styled(
            format!(": {}", heading),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ));
    }
    let mut lines = vec![Line::from(header_spans)];
    if let Some(text) = turn.live_thinking {
        lines.extend(render_thinking_live_content(text, width));
    }
    lines
}

fn append_turn_items(
    items: &mut Vec<RenderedLineItem>,
    turn: &TranscriptTurn<'_>,
    width: u16,
    tool_names: &std::collections::HashMap<String, String>,
    expanded_thinking: &std::collections::HashSet<u64>,
    frame_count: u64,
    accent: Color,
) {
    push_rendered_items(
        items,
        render_transcript_user_message(turn.user_message, turn.metadata, width),
        Some(turn.user_index),
        true,
    );

    enum SectionContent {
        Plain(Vec<Line<'static>>),
        Tagged(Vec<(Line<'static>, Option<u64>)>),
    }

    let mut sections: Vec<(SectionContent, Option<usize>)> = Vec::new();
    for (message_index, message) in &turn.assistant_messages {
        let tagged = render_transcript_assistant_message_tagged(
            message,
            &RenderContext {
                width,
                highlight: true,
                show_thinking: false,
                tool_names: tool_names.clone(),
                expanded_thinking: expanded_thinking.clone(),
            },
        );
        if !tagged.is_empty() {
            sections.push((SectionContent::Tagged(tagged), Some(*message_index)));
        }
    }

    for block in &turn.tool_blocks {
        let mut lines = Vec::new();
        render_tool_block_lines(&mut lines, block, frame_count);
        if !lines.is_empty() {
            sections.push((
                SectionContent::Plain(lines),
                Some(turn.primary_message_index()),
            ));
        }
    }

    if turn.active && turn.live_thinking.is_some() {
        sections.push((
            SectionContent::Plain(render_live_thinking_lines(turn, frame_count, width)),
            Some(turn.primary_message_index()),
        ));
    }

    // Show a "Thinking" shimmer when the turn is active but no text or
    // thinking content has arrived yet — gives visual feedback that the
    // model is working (especially for providers without thinking support).
    if turn.active
        && turn.live_text.is_none()
        && turn.live_thinking.is_none()
        && turn
            .tool_blocks
            .iter()
            .all(|b| b.status != ToolStatus::Running)
    {
        let mut spans = vec![Span::raw("  ")];
        spans.extend(shimmer_spans("Thinking", frame_count));
        sections.push((
            SectionContent::Plain(vec![Line::from(spans)]),
            Some(turn.primary_message_index()),
        ));
    }

    if let Some(text) = turn.live_text {
        let lines = render_transcript_live_text(text, width);
        if !lines.is_empty() {
            sections.push((
                SectionContent::Plain(lines),
                Some(turn.primary_message_index()),
            ));
        }
    }

    if !turn.active {
        if let Some(meta_line) = render_transcript_assistant_meta(turn.metadata, accent) {
            if turn.has_visible_assistant_content() {
                sections.push((
                    SectionContent::Plain(vec![meta_line]),
                    Some(turn.primary_message_index()),
                ));
            }
        }
    }

    if !sections.is_empty() {
        push_blank_item(items);
        let total_sections = sections.len();
        for (index, (content, message_index)) in sections.into_iter().enumerate() {
            match content {
                SectionContent::Plain(lines) => {
                    push_rendered_items(items, lines, message_index, false)
                }
                SectionContent::Tagged(tagged) => {
                    push_rendered_items_tagged(items, tagged, message_index)
                }
            }
            if index + 1 < total_sections {
                push_blank_item(items);
            }
        }
    }

    push_blank_item(items);
}

fn render_message_items(app: &App, width: u16) -> Vec<RenderedLineItem> {
    let streaming =
        app.is_streaming || !app.streaming_text.is_empty() || !app.streaming_thinking.is_empty();
    let has_running_tool_blocks = app
        .tool_use_blocks
        .iter()
        .any(|block| block.status == ToolStatus::Running);
    let cacheable = !streaming && !has_running_tool_blocks;

    // Fast path: nothing live — use the full-result cache (ptr-stable check).
    let full_key = MessageLinesCacheKey {
        width,
        transcript_version: app.transcript_version.get(),
        messages_ptr: app.messages.as_ptr() as usize,
        messages_len: app.messages.len(),
        annotations_ptr: app.system_annotations.as_ptr() as usize,
        annotations_len: app.system_annotations.len(),
        thinking_expanded_len: app.thinking_expanded.len(),
    };
    if cacheable {
        if let Some(lines) = MESSAGE_LINES_CACHE.with(|cache| {
            cache
                .borrow()
                .as_ref()
                .filter(|c| c.key == full_key)
                .map(|c| c.lines.clone())
        }) {
            return lines;
        }
    }

    let completed_key = CompletedMsgCacheKey {
        width,
        transcript_version: app.transcript_version.get(),
        messages_len: app.messages.len(),
        annotations_len: app.system_annotations.len(),
        thinking_expanded_len: app.thinking_expanded.len(),
    };
    let build_items = || {
        let tool_names = build_tool_names(&app.messages);
        let turns = build_transcript_turns(app);
        let mut turn_map = std::collections::HashMap::new();
        for turn in &turns {
            turn_map.insert(turn.user_index, turn);
        }

        let mut items = Vec::new();
        let total = app.messages.len();
        let mut index = 0usize;
        while index <= total {
            for ann in app
                .system_annotations
                .iter()
                .filter(|ann| ann.after_index == index)
            {
                let mut lines = Vec::new();
                render_system_annotation_lines(&mut lines, ann, width as usize);
                push_rendered_items(&mut items, lines, None, false);
            }

            if index >= total {
                break;
            }

            let message = &app.messages[index];
            if message.role == Role::User {
                if let Some(&turn) = turn_map.get(&index) {
                    append_turn_items(
                        &mut items,
                        turn,
                        width,
                        &tool_names,
                        &app.thinking_expanded,
                        app.frame_count,
                        app.accent_color,
                    );
                    index = turn.end_message_index + 1;
                    continue;
                }
            }

            let tagged = render_transcript_assistant_message_tagged(
                message,
                &RenderContext {
                    width,
                    highlight: true,
                    show_thinking: false,
                    tool_names: tool_names.clone(),
                    expanded_thinking: app.thinking_expanded.clone(),
                },
            );
            push_rendered_items_tagged(&mut items, tagged, Some(index));
            push_blank_item(&mut items);
            index += 1;
        }

        if total == 0 && !app.tool_use_blocks.is_empty() {
            for block in &app.tool_use_blocks {
                let mut lines = Vec::new();
                render_tool_block_lines(&mut lines, block, app.frame_count);
                push_rendered_items(&mut items, lines, None, false);
                push_blank_item(&mut items);
            }
        }

        items
    };
    let completed_lines: Vec<RenderedLineItem> = if cacheable {
        if let Some(lines) = COMPLETED_MSG_CACHE.with(|cache| {
            cache
                .borrow()
                .as_ref()
                .filter(|c| c.key == completed_key)
                .map(|c| c.lines.clone())
        }) {
            lines
        } else {
            let items = build_items();
            COMPLETED_MSG_CACHE.with(|cache| {
                *cache.borrow_mut() = Some(CompletedMsgCache {
                    key: completed_key,
                    lines: items.clone(),
                });
            });
            items
        }
    } else {
        build_items()
    };

    // If there is no live content, store in the full cache and return.
    if cacheable {
        MESSAGE_LINES_CACHE.with(|cache| {
            *cache.borrow_mut() = Some(MessageLinesCache {
                key: full_key,
                lines: completed_lines.clone(),
            });
        });
        return completed_lines;
    }

    completed_lines
}

// ── Welcome / startup screen ─────────────────────────────────────────────────

/// Render the two-column orange round-bordered welcome box (matches TS LogoV2).
/// Short, user-facing label for the active model. Falls back to the
/// configured default when no override is set.
fn welcome_model_label(app: &App) -> String {
    app.config
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .map(|m| m.to_string())
        .unwrap_or_else(|| app.config.effective_model().to_string())
}

/// Short label for the active provider id (or "default" when no override).
fn welcome_provider_label(app: &App) -> String {
    app.config
        .provider
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .unwrap_or_else(|| "anthropic".to_string())
}

/// One-glance daemon status: "Daemon: online" / "Daemon: offline".
///
/// Uses the cached, non-blocking probe in [`crate::coven_status`]: the render
/// path must never wait on a socket round-trip. The underlying background
/// probe keeps the 2 s budget so a busy daemon doesn't flip the welcome panel
/// to "offline" mid-load — see issue #50.
fn welcome_daemon_label() -> String {
    if crate::coven_status::daemon_looks_online() {
        "Daemon: online".to_string()
    } else {
        "Daemon: offline".to_string()
    }
}

/// Familiar display name for the welcome panel.
fn welcome_familiar_label(app: &App) -> String {
    visible_familiar(app)
        .map(|familiar| format!("Familiar: {}", familiar.id))
        .unwrap_or_else(|| "Familiar: none".to_string())
}

fn visible_familiar(app: &App) -> Option<claurst_core::coven_shared::CovenFamiliar> {
    let id = app.config.familiar.as_deref()?;
    crate::coven_status::cached_familiars()
        .into_iter()
        .find(|familiar| familiar.id == id)
}

/// Friendly, human-facing model name for the welcome panel: turns a raw id
/// like `claude-opus-4-8` into `Opus 4.8`, and appends `(1M context)` when the
/// id carries the long-context `[1m]` marker. Unknown ids pass through.
fn friendly_model_label(app: &App) -> String {
    friendly_model_from_id(&welcome_model_label(app))
}

/// Pure mapping from a raw model id to a friendly label (see
/// [`friendly_model_label`]). Split out so it can be tested without an `App`.
fn friendly_model_from_id(raw: &str) -> String {
    let lc = raw.to_lowercase();
    let one_m = lc.contains("[1m]") || lc.contains("-1m") || lc.ends_with("1m");
    for (key, disp) in [
        ("opus", "Opus"),
        ("sonnet", "Sonnet"),
        ("haiku", "Haiku"),
        ("fable", "Fable"),
    ] {
        if let Some(idx) = lc.find(key) {
            let rest = &lc[idx + key.len()..];
            let ver: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            let ver = ver.trim_matches('-').replace('-', ".");
            let base = if ver.is_empty() {
                disp.to_string()
            } else {
                format!("{disp} {ver}")
            };
            return if one_m {
                format!("{base} (1M context)")
            } else {
                base
            };
        }
    }
    raw.to_string()
}

/// The current working directory with `$HOME` abbreviated to `~`, shown as the
/// workspace line on the welcome panel.
fn welcome_cwd_label() -> String {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .unwrap_or_default();
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() && cwd.starts_with(&home) {
            return format!("~{}", &cwd[home.len()..]);
        }
    }
    cwd
}

/// Capitalize the first character (e.g. `anthropic` → `Anthropic`).
fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Truncate `s` to at most `max` display cells, appending `…` when cut.
fn truncate_meta(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let take = max.saturating_sub(1).max(1);
        let mut out: String = s.chars().take(take).collect();
        out.push('\u{2026}');
        out
    }
}

fn render_welcome_box(frame: &mut Frame, app: &App, area: Rect) {
    // --- Box dimensions ---
    // Fixed height; width capped so the box doesn't stretch edge-to-edge on
    // very wide terminals.
    let box_width = area.width.min(WELCOME_BOX_MAX_WIDTH);
    let box_height: u16 = WELCOME_BOX_HEIGHT;
    if area.height < box_height || box_width < 30 {
        // Too small: collapse to a single-line status that still surfaces
        // the model, daemon, and familiar so a user on a 24×9 terminal
        // doesn't see a content-less "Coven Code v0.0.13" header.
        let model = welcome_model_label(app);
        let daemon = welcome_daemon_label();
        let familiar = welcome_familiar_label(app);
        let line = Line::from(vec![
            Span::styled(
                "Coven Code ",
                Style::default()
                    .fg(COVEN_VIOLET)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("v{}", APP_VERSION),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!(" · {model}"), Style::default().fg(Color::DarkGray)),
            Span::styled(format!(" · {daemon}"), Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(" · {familiar}"),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(vec![line]), area);
        return;
    }
    let box_area = Rect {
        x: area.x,
        y: area.y,
        width: box_width,
        height: box_height,
    };

    // Outer border with title "Coven Code vX.Y"
    let accent = app.accent_color;
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .title(Line::from(vec![
            Span::styled(
                " Coven Code ",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("v{} ", APP_VERSION),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    frame.render_widget(outer_block, box_area);

    // Inner area (inside the border)
    let inner = Rect {
        x: box_area.x + 1,
        y: box_area.y + 1,
        width: box_area.width.saturating_sub(2),
        height: box_area.height.saturating_sub(2),
    };

    // Split inner into left | divider(1) | right
    // Left width: ~28 chars or half the inner width, whichever is smaller
    let left_w = (inner.width / 2)
        .clamp(22, 32)
        .min(inner.width.saturating_sub(3));
    let right_w = inner.width.saturating_sub(left_w + 1);
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(left_w),
            Constraint::Length(1),
            Constraint::Length(right_w),
        ])
        .split(inner);

    // Store the right column area for error modal positioning
    app.footer_right_column_area.set(h_chunks[2]);

    // Draw vertical divider in accent color
    let divider_lines: Vec<Line> = (0..inner.height)
        .map(|_| Line::from(Span::styled("\u{2502}", Style::default().fg(accent))))
        .collect();
    frame.render_widget(Paragraph::new(divider_lines), h_chunks[1]);

    // --- Left column: centered identity, 8-bit avatar, and metadata ---
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|u| !u.is_empty());
    let welcome_msg = if let Some(ref name) = username {
        format!("Welcome back {}!", name)
    } else {
        "Welcome back!".to_string()
    };
    let daemon_familiars = crate::coven_status::cached_familiars();
    let left_w_usize = left_w as usize;

    let mut left_lines: Vec<Line> = Vec::new();
    left_lines.push(Line::from(Span::styled(
        welcome_msg,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    left_lines.push(Line::from(""));
    if let Some(familiar) = visible_familiar(app) {
        let familiar_name = familiar.id.as_str();
        if let Some(seq) = familiar_image::render_familiar_image(familiar_name, 11, 5) {
            left_lines.push(Line::from(Span::raw(seq)));
        } else {
            let theme = familiar_theme::resolve(familiar_name, &daemon_familiars);
            left_lines.extend(familiar_card::render_avatar(
                &theme,
                &app.companion_current_pose,
            ));
        }
        left_lines.push(Line::from(""));
    }
    // Gray metadata block mirroring the reference layout: model, provider ·
    // daemon state, and the working directory.
    let meta_style = Style::default().fg(Color::Gray);
    left_lines.push(Line::from(Span::styled(
        truncate_meta(&friendly_model_label(app), left_w_usize),
        meta_style,
    )));
    let daemon = welcome_daemon_label();
    let provider_line = format!(
        "{} \u{00b7} {}",
        title_case(&welcome_provider_label(app)),
        if daemon.contains("online") {
            "online"
        } else {
            "offline"
        }
    );
    left_lines.push(Line::from(Span::styled(
        truncate_meta(&provider_line, left_w_usize),
        meta_style,
    )));
    left_lines.push(Line::from(Span::styled(
        truncate_meta(&welcome_cwd_label(), left_w_usize),
        Style::default().fg(Color::DarkGray),
    )));
    if let Some(goal) = app.active_goal_badge.as_deref().filter(|s| !s.is_empty()) {
        left_lines.push(Line::from(Span::styled(
            truncate_meta(&format!("Goal: {goal}"), left_w_usize),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )));
    }
    frame.render_widget(
        Paragraph::new(left_lines).alignment(Alignment::Center),
        h_chunks[0],
    );

    // --- Right column: Tips + What's new, left-aligned like the image ---
    let right_w_usize = right_w.saturating_sub(1).max(1) as usize;
    // The tip for a session is stable; selecting it re-reads tip history from
    // disk, so do that once per process instead of on every frame.
    static WELCOME_TIP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let tip_text = WELCOME_TIP.get_or_init(|| {
        claurst_core::tips::select_tip(0)
            .map(|t| t.content.to_string())
            .unwrap_or_else(|| "Edit AGENTS.md to add instructions for Coven Code".to_string())
    });

    let mut right_lines: Vec<Line> = Vec::new();
    // When unauthenticated, lead the right column with a prominent /connect
    // call-to-action so a brand-new user has an obvious next step. The tips and
    // what's-new sections still render below (kept for the startup chrome).
    if !app.has_credentials {
        let pink = crate::overlays::COVEN_CODE_ACCENT;
        right_lines.push(Line::from(vec![
            Span::styled(
                " /connect ",
                Style::default()
                    .fg(readable_fg_on(pink))
                    .bg(pink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " connect a provider to begin",
                Style::default().fg(pink).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    right_lines.push(Line::from(Span::styled(
        "Tips for getting started",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )));
    // Word-wrap the tip on word boundaries; when the CTA is also shown, keep it
    // to a single line so the fixed-height box doesn't clip What's new.
    let tip_lines = crate::dialogs::word_wrap(tip_text, right_w_usize);
    let tip_take = if app.has_credentials {
        tip_lines.len()
    } else {
        1
    };
    for line in tip_lines.into_iter().take(tip_take) {
        right_lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(Color::Gray),
        )));
    }
    // Divider rule between the two sections (matches the reference image).
    right_lines.push(Line::from(Span::styled(
        "\u{2500}".repeat(right_w_usize),
        Style::default().fg(accent),
    )));
    right_lines.push(Line::from(Span::styled(
        "What's new",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )));
    // Trim the changelog by one when the CTA line is present so the section
    // stays inside the fixed box height.
    let whats_new_take = if app.has_credentials { 3 } else { 2 };
    for item in claurst_core::constants::WHATS_NEW
        .iter()
        .take(whats_new_take)
    {
        right_lines.push(Line::from(Span::styled(
            truncate_meta(item, right_w_usize),
            Style::default().fg(Color::Gray),
        )));
    }
    right_lines.push(Line::from(Span::styled(
        "/release-notes for more",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    frame.render_widget(
        Paragraph::new(right_lines).wrap(Wrap { trim: false }),
        h_chunks[2],
    );
}

// ── Per-message rendering ─────────────────────────────────────────────────────

/// Build a tool_use_id → tool_name lookup from all messages in the transcript.
/// This allows ToolResult blocks to dispatch to tool-specific renderers.
fn build_tool_names(
    messages: &[claurst_core::types::Message],
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for msg in messages {
        for block in msg.content_blocks() {
            if let claurst_core::types::ContentBlock::ToolUse { id, name, .. } = block {
                map.insert(id.clone(), name.clone());
            }
        }
    }
    map
}

// ── System annotation (compact boundary, info notices) ───────────────────────

fn render_system_annotation_lines(
    lines: &mut Vec<Line<'static>>,
    ann: &SystemAnnotation,
    width: usize,
) {
    // Compact boundary: show ✻ prefix with dimmed text
    if ann.style == SystemMessageStyle::Compact {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", figures::TEARDROP_ASTERISK),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                ann.text.clone(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
        ]));
        lines.push(Line::from(""));
        return;
    }

    let (text_color, border_color) = match ann.style {
        SystemMessageStyle::Info => (Color::DarkGray, Color::DarkGray),
        SystemMessageStyle::Warning => (Color::Yellow, Color::Yellow),
        SystemMessageStyle::Compact => unreachable!(),
    };

    // Centred, padded rule: "─── text ───"
    let text = ann.text.as_str();
    let inner_width = width.saturating_sub(4);
    let text_len = text.len();
    let dashes = inner_width.saturating_sub(text_len + 2);
    let left = dashes / 2;
    let right = dashes - left;

    lines.push(Line::from(vec![
        Span::styled(
            format!("  {}", "\u{2500}".repeat(left)),
            Style::default().fg(border_color),
        ),
        Span::styled(
            format!("\u{2500} {} \u{2500}", text),
            Style::default().fg(text_color).add_modifier(Modifier::DIM),
        ),
        Span::styled("\u{2500}".repeat(right), Style::default().fg(border_color)),
    ]));
    lines.push(Line::from(""));
}

// ── Tool use block ────────────────────────────────────────────────────────────

fn render_tool_block_lines(
    lines: &mut Vec<Line<'static>>,
    block: &crate::app::ToolUseBlock,
    frame_count: u64,
) {
    let input_val: serde_json::Value =
        serde_json::from_str(&block.input_json).unwrap_or(serde_json::Value::Null);
    let normalized = block.name.to_ascii_lowercase();
    let running = block.status == ToolStatus::Running;
    let mut summary = crate::messages::extract_tool_summary(&block.name, &input_val);
    let title = if normalized == "task" || normalized == "agent" {
        if let Some(description) = input_val
            .get("description")
            .and_then(|value| value.as_str())
        {
            summary = description.to_string();
        }
        crate::messages::subagent_title(&input_val)
    } else {
        match (normalized.as_str(), running) {
            ("bash" | "powershell", true) => "Running command".to_string(),
            ("bash" | "powershell", false) => "Ran command".to_string(),
            ("read", true) => "Reading file".to_string(),
            ("read", false) => "Read file".to_string(),
            ("write" | "apply_patch", true) => "Writing file".to_string(),
            ("write" | "apply_patch", false) => "Wrote file".to_string(),
            ("edit", true) => "Editing file".to_string(),
            ("edit", false) => "Edited file".to_string(),
            ("glob" | "list", true) => "Listing files".to_string(),
            ("glob" | "list", false) => "Listed files".to_string(),
            ("grep" | "codesearch", true) => "Searching code".to_string(),
            ("grep" | "codesearch", false) => "Searched code".to_string(),
            ("webfetch", true) => "Fetching page".to_string(),
            ("webfetch", false) => "Fetched page".to_string(),
            ("websearch", true) => "Searching web".to_string(),
            ("websearch", false) => "Searched web".to_string(),
            _ => block.name.clone(),
        }
    };

    let accent = if block.status == ToolStatus::Error {
        Color::Rgb(255, 140, 0)
    } else {
        COVEN_VIOLET
    };
    let mut header_spans = vec![Span::styled(
        "   ~ ".to_string(),
        Style::default().fg(accent),
    )];
    if running {
        header_spans.extend(shimmer_spans(&title, frame_count));
    } else {
        header_spans.push(Span::styled(
            title,
            Style::default()
                .fg(if block.status == ToolStatus::Error {
                    accent
                } else {
                    Color::White
                })
                .add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(header_spans));

    if !summary.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("     "),
            Span::styled(summary, Style::default().fg(Color::DarkGray)),
        ]));
    }

    if normalized == "bash" || normalized == "powershell" {
        let command = input_val
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        for (i, cmd_line) in command.lines().enumerate() {
            if i >= 2 {
                break;
            }
            let display: String = cmd_line.chars().take(160).collect();
            let display = if cmd_line.chars().count() > 160 {
                format!("{}\u{2026}", display)
            } else {
                display
            };
            lines.push(Line::from(vec![
                Span::styled("     $ ".to_string(), Style::default().fg(Color::Green)),
                Span::styled(display, Style::default().fg(Color::White)),
            ]));
        }
    }

    // Output preview (done/error state)
    if let Some(ref preview) = block.output_preview {
        let preview_style = match block.status {
            ToolStatus::Error => Style::default().fg(Color::Rgb(255, 140, 0)),
            _ => Style::default().fg(Color::DarkGray),
        };
        for line_text in preview.lines() {
            if line_text.starts_with('\u{2026}') {
                lines.push(Line::from(vec![
                    Span::raw("     "),
                    Span::styled(
                        line_text.to_string(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    ),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw("     "),
                    Span::styled(line_text.to_string(), preview_style),
                ]));
            }
        }
    }
}

// -----------------------------------------------------------------------
// Input pane
// -----------------------------------------------------------------------

fn render_input(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    // Split: 1-row model/mode status line + remaining rows for the prompt input.
    let (status_area, input_area) = if area.height > 2 {
        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);
        (Some(splits[0]), splits[1])
    } else {
        // Not enough room for the extra line — skip the status row.
        (None, area)
    };

    // Render model + familiar mode status line above the prompt.
    if let Some(status_area) = status_area {
        let agent_mode = match app.agent_mode.as_deref() {
            Some(m) => m,
            None if app.plan_mode => "plan",
            _ => "build",
        };

        let pink = app.accent_color;
        let dim = COVEN_CODE_MUTED;
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(status_area.width.min(50)),
            ])
            .split(status_area);

        let left_line = if app.has_credentials {
            let (provider, model_short) =
                if let Some((provider, model)) = app.model_name.split_once('/') {
                    (provider.to_string(), model.to_string())
                } else {
                    ("local".to_string(), app.model_name.clone())
                };
            let mut spans = vec![
                Span::styled(
                    format!(" {} ", agent_mode.to_uppercase()),
                    Style::default()
                        .fg(readable_fg_on(pink))
                        .bg(pink)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    model_short,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            spans.push(Span::styled(
                format!(" · {}", provider),
                Style::default().fg(dim),
            ));
            if let Some(ref badge) = app.agent_type_badge {
                spans.push(Span::styled(
                    format!(" · {}", badge),
                    Style::default().fg(dim),
                ));
            }
            Line::from(spans)
        } else {
            Line::from(vec![
                Span::styled(
                    " /connect ",
                    Style::default()
                        .fg(readable_fg_on(pink))
                        .bg(pink)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" connect a provider", Style::default().fg(dim)),
            ])
        };

        // `?` opens the shortcuts overlay which already lists Ctrl+A / Ctrl+K
        // and friends — surfacing them again here is redundant clutter.
        let right_hint = if app.has_credentials && app.prompt_input.is_empty() {
            Line::from(vec![Span::styled("? shortcuts", Style::default().fg(dim))])
        } else {
            Line::from(Vec::<Span>::new())
        };

        let left_padded = Rect {
            x: chunks[0].x + 1,
            y: chunks[0].y,
            width: chunks[0].width.saturating_sub(1),
            height: chunks[0].height,
        };
        let right_padded = Rect {
            x: chunks[1].x,
            y: chunks[1].y,
            width: chunks[1].width.saturating_sub(1),
            height: chunks[1].height,
        };
        frame.render_widget(Paragraph::new(vec![left_line]), left_padded);
        frame.render_widget(
            Paragraph::new(vec![right_hint]).alignment(Alignment::Right),
            right_padded,
        );
    }

    render_prompt_input(
        &app.prompt_input,
        input_area,
        frame.buffer_mut(),
        focused,
        if app.is_streaming {
            InputMode::Readonly
        } else if app.plan_mode {
            InputMode::Plan
        } else {
            InputMode::Default
        },
        app.accent_color,
        app.settings_screen.cursor_blink_enabled,
    );
}

/// Pick a streaming-state label that distinguishes tool calls from
/// reasoning from text generation. Returns an owned String because the
/// most informative label may interpolate the active tool's name.
///
/// Precedence:
///   1. Adapter-set `status_message` (when it's not the generic
///      "thinking" placeholder)
///   2. Running tool call — "Running <tool>"
///   3. Active streaming text — "Generating"
///   4. Active streaming thinking — "Reasoning"
///   5. App-level `spinner_verb` override
///   6. Default — "Waiting on network"
fn streaming_status_label(app: &App) -> String {
    if app.is_streaming && stream_is_stalled(app) {
        return STREAM_STALLED_LABEL.to_string();
    }
    if app.is_streaming && app.permission_request.is_some() {
        return "Permission pending".to_string();
    }
    // 1. adapter-set message wins (unless it's the placeholder)
    if let Some(custom) = app.status_message.as_deref() {
        let trimmed = custom.trim();
        if !trimmed.is_empty()
            && !trimmed.eq_ignore_ascii_case(STATUS_THINKING)
            && !trimmed.eq_ignore_ascii_case(STATUS_THINKING_ELLIPSIS)
        {
            return trimmed.to_string();
        }
    }
    // 2. tool call in progress — show the most-recent active tool name
    if let Some(running) = app
        .tool_use_blocks
        .iter()
        .rev()
        .find(|b| matches!(b.status, crate::app::ToolStatus::Running))
    {
        return format!("Running {}", running.name);
    }
    // 3. streaming text
    if !app.streaming_text.is_empty() {
        return "Generating".to_string();
    }
    // 4. streaming reasoning
    if !app.streaming_thinking.is_empty() {
        return "Reasoning".to_string();
    }
    // 5. spinner_verb override (sampled at turn start)
    if let Some(v) = app.spinner_verb.as_deref().filter(|s| !s.is_empty()) {
        return v.to_string();
    }
    // 6. default
    STREAM_WAITING_LABEL.to_string()
}

fn should_render_status_row(app: &App) -> bool {
    let interesting_stream_status = app
        .status_message
        .as_deref()
        .map(|status| {
            let trimmed = status.trim();
            !trimmed.is_empty()
                && !trimmed.eq_ignore_ascii_case(STATUS_THINKING)
                && !trimmed.eq_ignore_ascii_case(STATUS_THINKING_ELLIPSIS)
        })
        .unwrap_or(false);

    app.voice_recording
        || app.last_turn_elapsed.is_some()
        || app.is_streaming
        || app.status_message.is_some()
        || interesting_stream_status
}

fn render_status_row(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let spans = if app.voice_recording {
        vec![Span::styled(
            format!(
                "{} Recording... press Alt+V to transcribe",
                figures::black_circle()
            ),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]
    } else if app.is_streaming {
        // Pick a streaming status label. We try to differentiate the three
        // distinguishable streaming modes so the spinner means something:
        //
        //   1. tool call in progress  →  "Running <tool>…"
        //   2. streaming text         →  "Generating…"
        //   3. streaming thinking     →  "Reasoning…"
        //   4. fallback               →  custom status / spinner_verb / waiting status
        //
        // A custom `status_message` set by the model adapter (e.g. extended
        // thinking budgets) still wins so adapters can override.
        let raw_label = streaming_status_label(app);

        let mut s = vec![Span::styled(
            spinner_char(app.frame_count).to_string(),
            Style::default()
                .fg(spinner_color(app))
                .add_modifier(Modifier::BOLD),
        )];
        let label = format!("{}…", raw_label.trim_end_matches('…'));

        s.push(Span::raw(" "));
        s.extend(shimmer_spans(&label, app.frame_count));
        s
    } else if let (Some(verb), Some(elapsed)) =
        (app.last_turn_verb, app.last_turn_elapsed.as_deref())
    {
        // "✓ Worked for 2m 5s · done" — turn-complete idle marker. Used to be
        // DIM DarkGray; bumped to a bright check mark + softer label so the
        // user can tell at a glance that the model is finished and waiting.
        let accent = Color::Rgb(80, 200, 120); // matches NotificationKind::Success
        vec![
            Span::styled(
                "✓ ".to_string(),
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} for {}", verb, elapsed),
                Style::default().fg(Color::Gray),
            ),
            Span::styled(" · done".to_string(), Style::default().fg(Color::DarkGray)),
        ]
    } else if let Some(status) = app.status_message.as_deref() {
        vec![Span::styled(
            status.to_string(),
            Style::default().fg(Color::DarkGray),
        )]
    } else {
        Vec::new()
    };

    if spans.is_empty() {
        return;
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

/// Build spans for a text string with a right-to-left glimmer sweep, matching
/// the TS `GlimmerMessage` behaviour (glimmerSpeed=200ms, 3-char shimmer window).
///
/// At ~50ms per frame a 4-frame step ≈ 200ms, giving the same cadence as TS.
fn shimmer_spans(text: &str, frame_count: u64) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len == 0 {
        return Vec::new();
    }

    // Cycle length = text_len + 20 (10 off-screen on each side)
    let cycle_len = len + 20;
    // One step every 4 frames (~200ms at 50ms/frame)
    let cycle_pos = (frame_count as usize / 4) % cycle_len;
    // Glimmer sweeps right→left: starts at len+10 (off right), ends at -10 (off left)
    let glimmer_center = (len + 10).saturating_sub(cycle_pos) as isize;

    let base = Style::default().fg(Color::DarkGray);
    let bright = Style::default().fg(Color::White);

    // Accumulate runs of same style to minimise span count
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_bright = false;

    for (i, &ch) in chars.iter().enumerate() {
        let is_bright = (i as isize - glimmer_center).abs() <= 1
            && glimmer_center >= 0
            && glimmer_center < len as isize;

        if is_bright != run_bright && !run.is_empty() {
            spans.push(Span::styled(
                run.clone(),
                if run_bright { bright } else { base },
            ));
            run.clear();
        }
        run_bright = is_bright;
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_bright { bright } else { base }));
    }
    spans
}
// Keybinding hints footer
// -----------------------------------------------------------------------

/// Single footer line matching the TS contract more closely:
/// - `? for shortcuts` is suppressed once the prompt becomes non-empty
/// - the right side shows comprehensive status info and notifications
fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }

    // Use only the first line of the footer area, leaving bottom padding
    let footer_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };

    // Left side: ordered pills — voice > PR badge > background task > vim > hint
    let left_spans: Vec<Span> = if app.voice_recording {
        vec![Span::styled(
            format!(" {} REC — speak now", figures::black_circle()),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]
    } else {
        let mut spans: Vec<Span> = Vec::new();

        // Daemon online/offline indicator
        {
            let (label, color) = if app.daemon_online {
                ("\u{2726} coven", crate::overlays::COVEN_CODE_ACCENT)
            } else {
                ("\u{25cb} coven", Color::DarkGray)
            };
            spans.push(Span::styled(label, Style::default().fg(color)));
            spans.push(Span::raw("  "));
        }

        // Current familiar emoji + name, only when it still exists in the
        // user's saved daemon roster. Reset must not leave stale/default
        // familiars visible in the footer.
        if let Some(familiar) = visible_familiar(app) {
            let emoji = familiar.emoji.as_deref().unwrap_or("\u{2b50}");
            spans.push(Span::styled(
                format!("{} {}  ", emoji, familiar.id),
                Style::default().fg(Color::DarkGray),
            ));
        }

        if app.prompt_input.text.is_empty() && !app.is_streaming {
            spans.push(Span::styled(
                "F2 familiar  Alt+H help  Ctrl+B branch  Tab mode",
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Agent type badge (shown when running as subagent / coordinator)
        if let Some(ref badge) = app.agent_type_badge {
            spans.push(Span::styled(
                format!("\u{2699} {}", badge),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // PR badge — shows "PR #<n>" in cyan, with optional state in brackets.
        // State color: approved=green, changes_requested=red,
        //              review_required=yellow, else=gray.
        if let Some(pr_num) = app.pr_number {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            let pr_label = match &app.pr_state {
                Some(state) => format!("PR #{} [{}]", pr_num, state),
                None => format!("PR #{}", pr_num),
            };
            // Colors mirror TS PrBadge getPrStatusColor + TS ink color names:
            //   approved → Green, changes_requested → Red (error),
            //   pending / review_required → Yellow (warning), merged → Magenta.
            let pr_color = match app.pr_state.as_deref() {
                Some("approved") => Color::Green,
                Some("changes_requested") => Color::Red,
                Some("merged") => Color::Magenta,
                Some("pending") | Some("review_required") => Color::Yellow,
                Some(_) => Color::Gray,
                None => Color::Cyan,
            };
            spans.push(Span::styled(
                pr_label,
                Style::default().fg(pr_color).add_modifier(Modifier::BOLD),
            ));
        }

        // Background task status pill — shows "⟳ N tasks" when count > 0.
        // Falls back to background_task_status pre-formatted string if set.
        if app.background_task_count > 0 {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            let label = if app.background_task_count == 1 {
                "\u{27f3} 1 task".to_string()
            } else {
                format!("\u{27f3} {} tasks", app.background_task_count)
            };
            spans.push(Span::styled(label, Style::default().fg(Color::Yellow)));
        } else if let Some(ref task_status) = app.background_task_status {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                format!("\u{27f3} {}", task_status),
                Style::default().fg(Color::Yellow),
            ));
        }

        // Vim mode indicator — shown for all modes using neovim "-- MODE --" convention.
        // INSERT is dim (common, low-noise); other modes use bright colour.
        if app.prompt_input.vim_enabled {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            let (label, style) = match app.prompt_input.vim_mode {
                VimMode::Insert => ("-- INSERT --", Style::default().fg(Color::DarkGray)),
                VimMode::Normal => (
                    "-- NORMAL --",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::Visual => (
                    "-- VISUAL --",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::VisualLine => (
                    "-- VISUAL LINE --",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::VisualBlock => (
                    "-- VISUAL BLOCK --",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::Command => (
                    "-- COMMAND --",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::Search => (
                    "-- SEARCH --",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            };
            spans.push(Span::styled(label, style));
        }

        // Bash prefix indicator — shown when prompt starts with '!'
        if app.prompt_input.text.starts_with('!') {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                "[BASH]",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // Permission mode badge (left side, mirrors TS bottom-left indicator).
        // Default mode is silent; non-default modes show a badge.
        {
            use claurst_core::config::PermissionMode;
            match &app.config.permission_mode {
                PermissionMode::BypassPermissions => {
                    if !spans.is_empty() {
                        spans.push(Span::raw("  "));
                    }
                    spans.push(Span::styled(
                        "\u{23f5}\u{23f5} bypass",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ));
                }
                PermissionMode::AcceptEdits => {
                    if !spans.is_empty() {
                        spans.push(Span::raw("  "));
                    }
                    spans.push(Span::styled(
                        "accept-edits",
                        Style::default().fg(Color::Yellow),
                    ));
                }
                PermissionMode::Plan => {
                    if !spans.is_empty() {
                        spans.push(Span::raw("  "));
                    }
                    spans.push(Span::styled("plan", Style::default().fg(Color::Blue)));
                }
                PermissionMode::Default => {}
            }
        }

        // During streaming show "esc to interrupt". The "? shortcuts" hint is
        // rendered in the top-right status bar (see render_prompt area), so do
        // not duplicate it here (issue #149 follow-up).
        if spans.is_empty() && app.is_streaming {
            spans.push(Span::styled(
                "esc interrupt",
                Style::default().fg(Color::DarkGray),
            ));
        }

        spans
    };

    // Right side: status metrics and lightweight badges.
    let right_spans: Vec<Span> = {
        let mut parts: Vec<Span> = Vec::new();

        // 1. Context window usage — show "N% until auto-compact" mirroring TS TokenWarning.
        //    When an update is available and context is below 85%, show the update notification
        //    instead to keep the status bar uncluttered.
        if app.context_window_size > 0 {
            let used_pct =
                (app.context_used_tokens as f64 / app.context_window_size as f64 * 100.0) as u64;
            let left_pct = 100u64.saturating_sub(used_pct);

            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }

            if used_pct >= 85 {
                // High usage — always show context window info regardless of update status.
                if used_pct >= 95 {
                    parts.push(Span::styled(
                        format!("{}% context used — /compact now", used_pct),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ));
                } else {
                    parts.push(Span::styled(
                        format!("{}% until auto-compact", left_pct),
                        Style::default().fg(Color::Yellow),
                    ));
                }
            } else if let Some(ref version) = app.update_available {
                // Update available and context is fine — show update nudge in bottom-right.
                parts.push(Span::styled(
                    format!("⬆ v{} available  Run: /update", version),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            } else if used_pct >= 70 {
                // 70–84%: mild warning.
                parts.push(Span::styled(
                    format!("{}% until auto-compact", left_pct),
                    Style::default().fg(Color::Yellow),
                ));
            } else {
                // Normal: dim display.
                let used_k = app.context_used_tokens / 1000;
                let total_k = app.context_window_size / 1000;
                parts.push(Span::styled(
                    format!("{}k/{}k", used_k, total_k),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        // 3. Cost — mirrors TS formatCost: 4 decimal places for costs < $0.50, else 2.
        // Display cost if it's >= 0.0, so free models show $0.00
        if app.cost_usd >= 0.0 {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            let cost_str = if app.cost_usd < 0.5 {
                format!("${:.4}", app.cost_usd)
            } else {
                format!("${:.2}", app.cost_usd)
            };
            parts.push(Span::styled(cost_str, Style::default().fg(Color::DarkGray)));
        }

        // 3b. Token budget (feature-gated)
        #[cfg(feature = "token_budget")]
        if let Some(max_tokens) = app.token_budget {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            let used = app.token_count as u64;
            let max = max_tokens as u64;
            let pct = if max > 0 {
                (used as f64 / max as f64 * 100.0) as u32
            } else {
                0
            };
            let color = if pct >= 90 {
                Color::Red
            } else if pct >= 75 {
                Color::Yellow
            } else {
                Color::DarkGray
            };
            parts.push(Span::styled(
                format!("Tokens: {}/{} ({}%)", used, max, pct),
                Style::default().fg(color),
            ));
        }

        // 4. Rate limits
        if let Some(pct) = app.rate_limit_5h_pct {
            if pct > 0.0 {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                let color = if pct >= 90.0 {
                    Color::Red
                } else {
                    Color::Yellow
                };
                parts.push(Span::styled(
                    format!("5h:{:.0}%", pct),
                    Style::default().fg(color),
                ));
            }
        }
        if let Some(pct) = app.rate_limit_7day_pct {
            if pct > 0.0 {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                let color = if pct >= 90.0 {
                    Color::Red
                } else {
                    Color::Yellow
                };
                parts.push(Span::styled(
                    format!("7d:{:.0}%", pct),
                    Style::default().fg(color),
                ));
            }
        }

        // 5. Vim mode — displayed on the left side as "-- MODE --"; nothing extra on right.

        // 5b. Goal badge — shown when a goal is active for this session.
        if let Some(ref badge) = app.active_goal_badge {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                format!("[goal: {}]", badge),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // 6. Agent type badge
        if let Some(ref badge) = app.agent_type_badge {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                format!("[{}]", badge),
                Style::default().fg(COVEN_CODE_ACCENT),
            ));
        }

        // 7. Worktree branch
        if let Some(ref branch) = app.worktree_branch {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                format!("[{}]", branch),
                Style::default().fg(Color::Green),
            ));
        }

        // Git branch (if settings enabled)
        if app.settings_screen.show_git_branch {
            if let Some(ref branch) = app.git_branch {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                parts.push(Span::styled(
                    format!("⎇ {}", branch),
                    Style::default().fg(Color::Cyan),
                ));
            }
        }

        // Current directory (if settings enabled)
        if app.settings_screen.show_cwd {
            if let Some(ref dir) = app.current_dir {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                // Use dirs::home_dir() so this works on Windows (where $HOME
                // is unset and the home is $USERPROFILE). Guard against an
                // empty home string: `str::replace("", "~")` inserts "~"
                // between every character, producing the infamous
                // `~X~:~\~B~i~g~g~e~r~…` output.
                let home = dirs::home_dir()
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty());
                let display_dir = match home {
                    Some(h) if dir.starts_with(&h) => dir.replacen(&h, "~", 1),
                    _ => dir.clone(),
                };
                parts.push(Span::styled(
                    display_dir,
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        // Output style indicator (only when non-default)
        if app.output_style != "auto" {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                format!("[{}]", app.output_style),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // External status line override
        if let Some(ref override_text) = app.status_line_override {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            // Strip any ANSI escapes for terminal rendering (plain text)
            let clean: String = override_text
                .chars()
                .filter(|c| c.is_ascii_graphic() || *c == ' ')
                .collect();
            parts.push(Span::styled(clean, Style::default().fg(Color::DarkGray)));
        }

        // 8. Bridge badge
        if let Some(badge) = app.bridge_state.status_badge(app.frame_count) {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(badge);
        } else if app.pending_mcp_reconnect {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                "MCP reconnecting",
                Style::default().fg(Color::Yellow),
            ));
        }

        parts
    };

    // Gap fill
    let left_len: usize = left_spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let right_len: usize = right_spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let gap = (footer_area.width.saturating_sub(2) as usize).saturating_sub(left_len + right_len);

    let mut spans = left_spans;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right_spans);

    // Add padding: 1 char on each side
    let padded_area = Rect {
        x: footer_area.x + 1,
        y: footer_area.y,
        width: footer_area.width.saturating_sub(2),
        height: footer_area.height,
    };
    frame.render_widget(Paragraph::new(vec![Line::from(spans)]), padded_area);
}

fn render_prompt_suggestions(frame: &mut Frame, app: &App, area: Rect) {
    let suggestions = &app.prompt_input.suggestions;
    if suggestions.is_empty() || area.height == 0 {
        return;
    }

    let selected = app.prompt_input.suggestion_index.unwrap_or(0);
    let max_visible = area.height as usize;
    let start = selected
        .saturating_sub(max_visible / 2)
        .min(suggestions.len().saturating_sub(max_visible));
    let end = (start + max_visible).min(suggestions.len());
    let label_width = area.width.saturating_div(3).max(12) as usize;

    for (row, suggestion) in suggestions[start..end].iter().enumerate() {
        let is_selected = start + row == selected;
        let accent_style = if is_selected {
            Style::default()
                .fg(COVEN_VIOLET)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let label_style = if is_selected {
            Style::default()
                .fg(COVEN_VIOLET)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let detail_style = if is_selected {
            Style::default().fg(COVEN_VIOLET)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut spans = vec![Span::styled(
            if is_selected { "\u{203a} " } else { "  " },
            accent_style,
        )];
        match suggestion.source {
            TypeaheadSource::SlashCommand => {
                let display_name = truncate_text(&suggestion.text, label_width);
                spans.push(Span::styled(
                    format!("{display_name:<width$}", width = label_width),
                    label_style,
                ));
                spans.push(Span::styled(
                    " [cmd] ",
                    Style::default().fg(Color::DarkGray),
                ));
                if !suggestion.description.is_empty() {
                    spans.push(Span::styled(
                        truncate_text(
                            &suggestion.description,
                            area.width.saturating_sub(label_width as u16 + 10) as usize,
                        ),
                        detail_style,
                    ));
                }
            }
            TypeaheadSource::FileRef => {
                spans.push(Span::styled("+ ", accent_style));
                spans.push(Span::styled(
                    truncate_middle(&suggestion.text, label_width),
                    label_style,
                ));
                spans.push(Span::styled(
                    " [context] ",
                    Style::default().fg(Color::DarkGray),
                ));
                if !suggestion.description.is_empty() {
                    spans.push(Span::styled(
                        truncate_text(&suggestion.description, area.width as usize / 2),
                        detail_style,
                    ));
                }
            }
            TypeaheadSource::History => {
                let display_name = truncate_text(&suggestion.text, label_width);
                spans.push(Span::styled(
                    format!("{display_name:<width$}", width = label_width),
                    label_style,
                ));
                spans.push(Span::styled(
                    " [history] ",
                    Style::default().fg(Color::DarkGray),
                ));
                if !suggestion.description.is_empty() {
                    spans.push(Span::styled(
                        truncate_text(&suggestion.description, area.width as usize / 2),
                        detail_style,
                    ));
                }
            }
        }

        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x: area.x,
                y: area.y + row as u16,
                width: area.width,
                height: 1,
            },
        );
    }
}

// -----------------------------------------------------------------------
// Complete status line (T2-8)
// -----------------------------------------------------------------------

/// Complete status line data for rendering.
#[derive(Debug, Clone, Default)]
pub struct StatusLineData {
    pub model: String,
    pub tokens_used: u64,
    pub tokens_total: u64,
    pub cost_cents: f64,
    pub compact_warning_pct: Option<f64>, // None = no warning; Some(pct) = show warning
    pub vim_mode: Option<String>,         // None = no vim mode; Some("NORMAL") etc.
    pub bridge_connected: bool,
    pub session_id: Option<String>,
    pub worktree: Option<String>,
    pub agent_badge: Option<String>,
    pub rate_limit_pct_5h: Option<f64>,
    pub rate_limit_pct_7d: Option<f64>,
    /// Goal badge: Some("active · 5m · 3 turns") when a goal is running.
    pub goal_badge: Option<String>,
}

pub fn render_full_status_line(
    data: &StatusLineData,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
) {
    use ratatui::{
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Paragraph, Widget},
    };

    let mut spans = Vec::new();

    // Model name
    if !data.model.is_empty() {
        spans.push(Span::styled(
            format!(" {} ", data.model),
            Style::default().fg(Color::Cyan),
        ));
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
    }

    // Context window
    if data.tokens_total > 0 {
        let pct = data.tokens_used as f64 / data.tokens_total as f64;
        // Convey the fill level by shape as well as color so colorblind and
        // monochrome users can read the danger level, not just the number.
        let (ctx_color, ctx_glyph) = if pct >= 0.95 {
            (Color::Red, "\u{25cf}") // ● critical
        } else if pct >= 0.80 {
            (Color::Yellow, "\u{25d0}") // ◐ warning
        } else {
            (Color::Green, "\u{25cb}") // ○ ok
        };
        let used_k = data.tokens_used / 1000;
        let total_k = data.tokens_total / 1000;
        spans.push(Span::styled(
            format!(
                "{} {}k/{}k ({:.0}%)",
                ctx_glyph,
                used_k,
                total_k,
                pct * 100.0
            ),
            Style::default().fg(ctx_color),
        ));
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
    }

    // Cost
    if data.cost_cents > 0.0 {
        spans.push(Span::styled(
            format!("${:.2}", data.cost_cents / 100.0),
            Style::default().fg(Color::White),
        ));
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
    }

    // Compact warning
    if let Some(pct) = data.compact_warning_pct {
        if pct >= 0.80 {
            let color = if pct >= 0.95 {
                Color::Red
            } else {
                Color::Yellow
            };
            spans.push(Span::styled(
                format!("⚠ ctx {:.0}% ", pct * 100.0),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
        }
    }

    // Vim mode
    if let Some(mode) = &data.vim_mode {
        let color = match mode.as_str() {
            "NORMAL" => Color::Green,
            "INSERT" => Color::Blue,
            "VISUAL" => Color::Magenta,
            _ => Color::White,
        };
        spans.push(Span::styled(
            format!("[{}]", mode),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" ", Style::default()));
    }

    // Agent badge
    if let Some(badge) = &data.agent_badge {
        spans.push(Span::styled(
            format!("[{}]", badge),
            Style::default().fg(Color::Magenta),
        ));
        spans.push(Span::styled(" ", Style::default()));
    }

    // Goal badge
    if let Some(goal) = &data.goal_badge {
        spans.push(Span::styled(
            format!("[goal: {}]", goal),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" ", Style::default()));
    }

    // Bridge connected
    if data.bridge_connected {
        spans.push(Span::styled("🔗 ", Style::default().fg(Color::Green)));
    }

    // Session ID
    if let Some(sid) = &data.session_id {
        let short = &sid[..sid.len().min(8)];
        spans.push(Span::styled(
            format!("[session:{}]", short),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Worktree
    if let Some(wt) = &data.worktree {
        spans.push(Span::styled(
            format!("[worktree:{}]", wt),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let line = Line::from(spans);
    Paragraph::new(line)
        .style(Style::default().bg(COVEN_CODE_APP_BG))
        .render(area, buf);
}

// ---------------------------------------------------------------------------
// Multi-agent UI components
// ---------------------------------------------------------------------------

/// Render a single progress-indicator row for a sub-agent.
///
/// Format: `[agent-<id>]` in cyan dim · space · status in colour · ` · ` · tool in dim gray
///
/// # Arguments
/// * `agent_id`    — short agent identifier (e.g. `"abc123"`)
/// * `status`      — current status string: `"working"`, `"done"`, `"error"`, or other
/// * `current_tool` — tool the agent is currently executing, if any
pub fn render_agent_progress_line(
    agent_id: &str,
    status: &str,
    current_tool: Option<&str>,
) -> Line<'static> {
    let status_color = match status {
        "working" | "running" => Color::Yellow,
        "done" | "complete" | "completed" => Color::Green,
        "error" | "failed" => Color::Red,
        _ => Color::DarkGray,
    };

    let mut spans = vec![
        Span::styled(
            format!("[agent-{}]", agent_id),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        ),
        Span::raw(" "),
        Span::styled(status.to_string(), Style::default().fg(status_color)),
    ];

    if let Some(tool) = current_tool {
        spans.push(Span::styled(
            " · ".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(
            tool.to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
    }

    Line::from(spans)
}

/// Render a multi-line coordinator status block for a multi-agent session.
///
/// Returns a `Vec<Line>` containing:
/// 1. A header: `Coordinator · N agents (M active)` in cyan bold
/// 2. One compact row per entry in `active_agents` using [`render_agent_progress_line`]
///
/// # Arguments
/// * `agent_count`   — total number of sub-agents spawned
/// * `completed`     — number of agents that have finished
/// * `active_agents` — slice of agent ID strings currently running
pub fn render_coordinator_status_lines(
    agent_count: usize,
    completed: usize,
    active_agents: &[&str],
) -> Vec<Line<'static>> {
    let active_count = active_agents.len();

    let header = Line::from(vec![
        Span::styled(
            "Coordinator".to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ".to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(
                "{} agent{}",
                agent_count,
                if agent_count == 1 { "" } else { "s" }
            ),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!(" ({} active)", active_count),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            if completed > 0 {
                format!("  ✔ {} done", completed)
            } else {
                String::new()
            },
            Style::default().fg(Color::Green),
        ),
    ]);

    let mut lines = vec![header];

    for agent_id in active_agents {
        let row = render_agent_progress_line(agent_id, "working", None);
        // Indent agent rows by two spaces
        let mut indented_spans = vec![Span::raw("  ")];
        indented_spans.extend(row.spans);
        lines.push(Line::from(indented_spans));
    }

    lines
}

/// Render a single header line for a teammate's message block.
///
/// Format: `┤ teammate: <id> ├` in magenta, optional `· <session_info>` in dim
///
/// # Arguments
/// * `teammate_id`  — teammate identifier string
/// * `session_info` — optional session info snippet to append
pub fn render_teammate_header(teammate_id: &str, session_info: Option<&str>) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            "┤ teammate: ".to_string(),
            Style::default().fg(Color::Magenta),
        ),
        Span::styled(
            teammate_id.to_string(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ├".to_string(), Style::default().fg(Color::Magenta)),
    ];

    if let Some(info) = session_info {
        spans.push(Span::styled(
            "  · ".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(
            info.to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
    }

    Line::from(spans)
}

// ---------------------------------------------------------------------------
// Familiar switcher popup (F2)
// ---------------------------------------------------------------------------

fn render_familiar_switcher(frame: &mut Frame, app: &App, area: Rect) {
    use crate::app::FamiliarSwitcherEntry;
    use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};

    let entries = app.familiar_switcher_entries();
    // +2 for borders, +1 for the filter/hint footer line inside the block.
    let list_len = entries.len() as u16;
    let popup_h = list_len
        .saturating_add(3)
        .min(area.height.saturating_sub(4))
        .max(4);
    let popup_w = 44u16.min(area.width.saturating_sub(4));
    let popup_x = area.x + area.width.saturating_sub(popup_w) / 2;
    let popup_y = area.y + area.height.saturating_sub(popup_h) / 2;
    let popup_area = Rect {
        x: popup_x,
        y: popup_y,
        width: popup_w,
        height: popup_h,
    };

    frame.render_widget(Clear, popup_area);

    let pink = crate::overlays::COVEN_CODE_ACCENT;
    let daemon_familiars = crate::coven_status::cached_familiars();
    let interior_w = popup_w.saturating_sub(2);

    // Footer shows the active filter (or a hint when empty) so incremental
    // typing is discoverable.
    let footer = if app.familiar_switcher_filter.is_empty() {
        Line::from(Span::styled(
            " type to filter · ↑↓ move · ⏎ select ",
            Style::default().fg(Color::Rgb(120, 120, 130)),
        ))
    } else {
        Line::from(vec![
            Span::styled(" filter: ", Style::default().fg(Color::Rgb(120, 120, 130))),
            Span::styled(
                app.familiar_switcher_filter.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    };

    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let selected = i == app.familiar_switcher_idx;
            match entry {
                FamiliarSwitcherEntry::Clear => {
                    let line = Line::from(Span::styled(
                        "  ✕  none (clear active familiar)",
                        Style::default().fg(Color::Rgb(170, 170, 180)),
                    ));
                    let item = ListItem::new(line);
                    if selected {
                        item.style(
                            Style::default()
                                .bg(Color::Rgb(70, 70, 80))
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        item
                    }
                }
                FamiliarSwitcherEntry::Familiar(id) => {
                    let theme = familiar_theme::resolve(id, &daemon_familiars);
                    let row = familiar_card::render_mini_row(&theme, interior_w);
                    let item = ListItem::new(row);
                    if selected {
                        item.style(
                            Style::default()
                                .bg(theme.palette.primary)
                                .fg(Color::Black)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        item
                    }
                }
            }
        })
        .collect();

    let block = Block::default()
        .title(" \u{2728} Familiar (F2) ")
        .title_bottom(footer)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pink));

    let list = List::new(items).block(block);
    let mut state = ListState::default();
    state.select(Some(app.familiar_switcher_idx));
    frame.render_stateful_widget(list, popup_area, &mut state);
}

#[cfg(test)]
mod welcome_tests {
    use super::*;
    use crate::app::test_env::EnvGuard;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_test_app_with_model_and_familiar(
        model: Option<&str>,
        provider: Option<&str>,
        familiar: Option<&str>,
        goal: Option<&str>,
    ) -> App {
        let config = claurst_core::config::Config {
            model: model.map(str::to_string),
            provider: provider.map(str::to_string),
            familiar: familiar.map(str::to_string),
            ..claurst_core::config::Config::default()
        };
        let mut app = App::new(config, claurst_core::cost::CostTracker::new());
        app.active_goal_badge = goal.map(str::to_string);
        app
    }

    #[test]
    fn welcome_model_label_prefers_config_override() {
        let app = make_test_app_with_model_and_familiar(
            Some("claude-haiku-4-5-20251001"),
            None,
            None,
            None,
        );
        assert_eq!(welcome_model_label(&app), "claude-haiku-4-5-20251001");
    }

    #[test]
    fn welcome_provider_label_falls_back_to_anthropic() {
        let app = make_test_app_with_model_and_familiar(None, None, None, None);
        assert_eq!(welcome_provider_label(&app), "anthropic");
    }

    #[test]
    fn welcome_familiar_label_uses_none_by_default() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let coven_home = temp.path().join("coven");
        std::fs::create_dir_all(&home).expect("home dir");
        std::fs::create_dir_all(&coven_home).expect("coven home dir");
        let _guard = EnvGuard::set(&home, &coven_home);

        let app = make_test_app_with_model_and_familiar(None, None, None, None);
        assert_eq!(welcome_familiar_label(&app), "Familiar: none");
    }

    #[test]
    fn welcome_familiar_label_hides_stale_config_without_roster() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let coven_home = temp.path().join("coven");
        std::fs::create_dir_all(&home).expect("home dir");
        std::fs::create_dir_all(&coven_home).expect("coven home dir");
        let _guard = EnvGuard::set(&home, &coven_home);

        let app = make_test_app_with_model_and_familiar(None, None, Some("wisp"), None);
        assert_eq!(welcome_familiar_label(&app), "Familiar: none");
    }

    #[test]
    fn welcome_familiar_label_reflects_saved_roster_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let coven_home = temp.path().join("coven");
        std::fs::create_dir_all(&home).expect("home dir");
        std::fs::create_dir_all(&coven_home).expect("coven home dir");
        std::fs::write(
            coven_home.join("familiars.toml"),
            "[[familiar]]\nid = \"wisp\"\nemoji = \"🌿\"\n",
        )
        .expect("familiar roster");
        let _guard = EnvGuard::set(&home, &coven_home);

        let app = make_test_app_with_model_and_familiar(None, None, Some("wisp"), None);
        assert_eq!(welcome_familiar_label(&app), "Familiar: wisp");
    }

    #[test]
    fn streaming_status_label_prefers_running_tool_over_default() {
        let mut app = make_test_app_with_model_and_familiar(None, None, None, None);
        app.tool_use_blocks.push(crate::app::ToolUseBlock {
            id: "tu_1".to_string(),
            name: "Bash".to_string(),
            turn_index: None,
            status: crate::app::ToolStatus::Running,
            output_preview: None,
            input_json: String::new(),
        });
        assert_eq!(streaming_status_label(&app), "Running Bash");
    }

    #[test]
    fn streaming_status_label_falls_back_through_text_then_thinking() {
        let mut app = make_test_app_with_model_and_familiar(None, None, None, None);
        app.is_streaming = true;
        assert_eq!(streaming_status_label(&app), "Waiting on network");
        assert!(should_render_status_row(&app));
        app.streaming_thinking = "let me think".to_string();
        assert_eq!(streaming_status_label(&app), "Reasoning");
        app.streaming_text = "let me think".to_string();
        // Text is more user-visible than reasoning, so it wins the race.
        assert_eq!(streaming_status_label(&app), "Generating");
    }

    #[test]
    fn streaming_status_label_adapter_message_overrides_everything() {
        let mut app = make_test_app_with_model_and_familiar(None, None, None, None);
        app.tool_use_blocks.push(crate::app::ToolUseBlock {
            id: "tu_1".to_string(),
            name: "Bash".to_string(),
            turn_index: None,
            status: crate::app::ToolStatus::Running,
            output_preview: None,
            input_json: String::new(),
        });
        // Adapter-set message wins even when a tool is running.
        app.status_message = Some("Compacting context".to_string());
        assert_eq!(streaming_status_label(&app), "Compacting context");
    }

    #[test]
    fn streaming_status_label_ignores_placeholder_thinking_message() {
        let mut app = make_test_app_with_model_and_familiar(None, None, None, None);
        // "thinking" / "thinking…" are placeholders — they should NOT win
        // over more informative state.
        app.status_message = Some("thinking".to_string());
        app.streaming_text = "hello".to_string();
        assert_eq!(streaming_status_label(&app), "Generating");
    }

    #[test]
    fn streaming_status_label_skips_completed_tools() {
        let mut app = make_test_app_with_model_and_familiar(None, None, None, None);
        // A done tool from a previous turn must not hijack the label.
        app.tool_use_blocks.push(crate::app::ToolUseBlock {
            id: "tu_1".to_string(),
            name: "Bash".to_string(),
            turn_index: None,
            status: crate::app::ToolStatus::Done,
            output_preview: None,
            input_json: String::new(),
        });
        assert_eq!(streaming_status_label(&app), "Waiting on network");
    }

    #[test]
    fn streaming_status_label_explains_stalled_streams() {
        let mut app = make_test_app_with_model_and_familiar(None, None, None, None);
        app.is_streaming = true;
        app.stall_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(4));

        assert_eq!(
            streaming_status_label(&app),
            "Stalled — check connection, Ctrl+C to interrupt"
        );
        assert!(should_render_status_row(&app));
        assert_eq!(spinner_color(&app), Color::Red);
    }

    #[test]
    fn spinner_advances_no_faster_than_every_two_render_ticks() {
        // The main render loop ticks at roughly 50ms, so each visible spinner
        // glyph must stay stable across at least two ticks to avoid a 20fps
        // flash cadence.
        assert_eq!(spinner_char(0), spinner_char(1));
        assert_ne!(spinner_char(1), spinner_char(2));
    }

    #[test]
    fn footer_exposes_hidden_keybinding_hints() {
        let mut terminal = Terminal::new(TestBackend::new(180, 1)).expect("terminal");
        let app = make_test_app_with_model_and_familiar(None, None, None, None);
        terminal
            .draw(|frame| render_footer(frame, &app, frame.area()))
            .expect("draw footer");

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        for expected in ["F2", "Alt+H", "Ctrl+B", "Tab"] {
            assert!(
                content.contains(expected),
                "footer should mention {expected}, got {content:?}"
            );
        }
    }

    #[test]
    fn welcome_daemon_label_is_one_of_two_strings() {
        // Either string is acceptable — the test machine may or may not have
        // the daemon socket. The label MUST be a stable, non-empty
        // human-readable hint, not a raw IO error.
        let label = welcome_daemon_label();
        assert!(
            label == "Daemon: online" || label == "Daemon: offline",
            "unexpected daemon label: {label}"
        );
    }

    #[test]
    fn welcome_box_width_is_capped_on_large_terminals() {
        let mut terminal = Terminal::new(TestBackend::new(180, 14)).expect("terminal");
        let app = make_test_app_with_model_and_familiar(None, None, None, None);

        terminal
            .draw(|frame| render_welcome_box(frame, &app, frame.area()))
            .expect("draw welcome");

        let buffer = terminal.backend().buffer();
        for y in 0..WELCOME_BOX_HEIGHT {
            for x in WELCOME_BOX_MAX_WIDTH..180 {
                assert_eq!(
                    buffer[(x, y)].symbol(),
                    " ",
                    "welcome box should not paint past {WELCOME_BOX_MAX_WIDTH} columns at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn welcome_screen_can_be_hidden_by_config() {
        let mut terminal = Terminal::new(TestBackend::new(100, 28)).expect("terminal");
        let mut app = make_test_app_with_model_and_familiar(None, None, None, None);
        app.config.show_splash = Some(false);

        terminal
            .draw(|frame| render_app(frame, &app))
            .expect("draw app");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(!rendered.contains("Welcome back"));
        assert!(!rendered.contains("What's new"));
    }

    #[test]
    fn friendly_model_label_maps_known_families() {
        assert_eq!(friendly_model_from_id("claude-opus-4-8"), "Opus 4.8");
        assert_eq!(friendly_model_from_id("claude-sonnet-4-6"), "Sonnet 4.6");
        assert_eq!(
            friendly_model_from_id("claude-haiku-4-5-20251001"),
            "Haiku 4.5.20251001"
        );
        assert_eq!(friendly_model_from_id("claude-fable-5"), "Fable 5");
        // The 1M long-context marker is surfaced.
        assert_eq!(
            friendly_model_from_id("claude-opus-4-8[1m]"),
            "Opus 4.8 (1M context)"
        );
        // Unknown ids pass through untouched.
        assert_eq!(
            friendly_model_from_id("some-other-model"),
            "some-other-model"
        );
    }

    #[test]
    fn title_case_capitalizes_first_char() {
        assert_eq!(title_case("anthropic"), "Anthropic");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn truncate_meta_appends_ellipsis_when_cut() {
        assert_eq!(truncate_meta("short", 10), "short");
        assert_eq!(truncate_meta("toolongvalue", 5), "tool\u{2026}");
        assert_eq!(truncate_meta("anything", 0), "");
    }

    #[test]
    fn welcome_box_shows_whats_new_and_release_notes() {
        let mut terminal = Terminal::new(TestBackend::new(120, 14)).expect("terminal");
        let app = make_test_app_with_model_and_familiar(None, None, None, None);

        terminal
            .draw(|frame| render_welcome_box(frame, &app, frame.area()))
            .expect("draw welcome");

        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..14 {
            for x in 0..120 {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        assert!(text.contains("Welcome back"), "missing greeting:\n{text}");
        assert!(
            text.contains("Tips for getting started"),
            "missing tips header:\n{text}"
        );
        assert!(text.contains("What's new"), "missing what's new:\n{text}");
        assert!(
            text.contains("/release-notes for more"),
            "missing release-notes footer:\n{text}"
        );
    }

    #[test]
    fn typeahead_suggestions_show_source_badges() {
        let mut terminal = Terminal::new(TestBackend::new(100, 3)).expect("terminal");
        let mut app = make_test_app_with_model_and_familiar(None, None, None, None);
        app.prompt_input.suggestions = vec![
            crate::prompt_input::TypeaheadSuggestion {
                text: "/help".to_string(),
                description: "Show help".to_string(),
                source: TypeaheadSource::SlashCommand,
            },
            crate::prompt_input::TypeaheadSuggestion {
                text: "@src/main.rs".to_string(),
                description: "file".to_string(),
                source: TypeaheadSource::FileRef,
            },
            crate::prompt_input::TypeaheadSuggestion {
                text: "previous prompt".to_string(),
                description: "recent".to_string(),
                source: TypeaheadSource::History,
            },
        ];

        terminal
            .draw(|frame| render_prompt_suggestions(frame, &app, frame.area()))
            .expect("draw suggestions");

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        for expected in ["[cmd]", "[context]", "[history]"] {
            assert!(
                content.contains(expected),
                "suggestion list should include {expected} badge, got {content:?}"
            );
        }
    }
}
