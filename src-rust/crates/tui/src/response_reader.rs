//! Full-screen reader for completed assistant response text.

use claurst_core::types::{ContentBlock, Message};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::{
    messages::render_markdown,
    overlays::{
        centered_rect, render_dark_overlay, render_dialog_bg, COVEN_CODE_ACCENT, COVEN_CODE_MUTED,
    },
};

/// TUI-local state for a reader opened from the transcript.
#[derive(Debug, Clone, Default)]
pub struct ResponseReaderState {
    pub visible: bool,
    pub message_index: Option<usize>,
    pub scroll_offset: usize,
    pub restore_transcript_offset: usize,
    pub search_query: String,
    pub search_active: bool,
}

/// Reconstruct visible assistant text with the transcript's section boundaries.
/// Non-text blocks are omitted, but text that resumes after one starts on a new line.
pub fn response_reader_text(message: &Message) -> String {
    message
        .content_blocks()
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl ResponseReaderState {
    /// Open the reader for one transcript message, remembering where to return.
    pub fn open(&mut self, message_index: usize, restore_offset: usize) {
        self.visible = true;
        self.message_index = Some(message_index);
        self.scroll_offset = 0;
        self.restore_transcript_offset = restore_offset;
        self.search_query.clear();
        self.search_active = false;
    }

    /// Close the reader and return the transcript offset captured on open.
    pub fn close(&mut self) -> Option<usize> {
        if !self.visible {
            return None;
        }

        self.visible = false;
        self.message_index = None;
        self.scroll_offset = 0;
        let restore_offset = self.restore_transcript_offset;
        self.restore_transcript_offset = 0;
        self.search_query.clear();
        self.search_active = false;
        Some(restore_offset)
    }

    /// Scroll down one viewport without passing the last complete viewport.
    pub fn page_down(&mut self, viewport_height: usize, line_count: usize) {
        let max_offset = line_count.saturating_sub(viewport_height);
        self.scroll_offset = self
            .scroll_offset
            .saturating_add(viewport_height)
            .min(max_offset);
    }

    /// Scroll up one viewport.
    pub fn page_up(&mut self, viewport_height: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(viewport_height);
    }

    /// Scroll to the first rendered line.
    pub fn home(&mut self) {
        self.scroll_offset = 0;
    }

    /// Scroll to the final complete viewport.
    pub fn end(&mut self, viewport_height: usize, line_count: usize) {
        self.scroll_offset = line_count.saturating_sub(viewport_height);
    }
}

/// Render a response reader containing only the message's text content.
pub fn render_response_reader(
    frame: &mut Frame,
    state: &ResponseReaderState,
    message: &Message,
    area: Rect,
) {
    if !state.visible {
        return;
    }

    let dialog_area = centered_rect(
        area.width.saturating_sub(4),
        area.height.saturating_sub(4),
        area,
    );
    render_dark_overlay(frame, area);
    frame.render_widget(Clear, dialog_area);
    render_dialog_bg(frame, dialog_area);

    let inner_area = Rect {
        x: dialog_area.x.saturating_add(1),
        y: dialog_area.y.saturating_add(1),
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };
    let body_area = Rect {
        x: inner_area.x,
        y: inner_area.y.saturating_add(1),
        width: inner_area.width,
        height: inner_area.height.saturating_sub(2),
    };
    let lines = render_markdown(&response_reader_text(message), body_area.width);
    let line_count = lines.len();
    let scroll_offset = state
        .scroll_offset
        .min(line_count.saturating_sub(body_area.height as usize));
    let line_position = if line_count == 0 {
        0
    } else {
        scroll_offset + 1
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COVEN_CODE_ACCENT));
    frame.render_widget(block, dialog_area);
    let mut header = vec![
        Span::styled("Reader", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(" · line {line_position} / {line_count}"),
            Style::default().fg(COVEN_CODE_MUTED),
        ),
    ];
    if !state.search_query.is_empty() {
        header.push(Span::styled(
            format!(" · /{}", state.search_query),
            Style::default().fg(COVEN_CODE_MUTED),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(header)),
        Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width,
            height: 1,
        },
    );
    let visible_lines: Vec<_> = lines
        .into_iter()
        .skip(scroll_offset)
        .take(body_area.height as usize)
        .collect();
    frame.render_widget(Paragraph::new(visible_lines), body_area);
    frame.render_widget(
        Paragraph::new(Line::styled(
            if state.search_active {
                "Type search  Enter done  Esc close"
            } else {
                "PgUp/PgDn  j/k  / search  y copy  Esc close"
            },
            Style::default().fg(COVEN_CODE_MUTED),
        )),
        Rect {
            x: inner_area.x,
            y: inner_area.y + inner_area.height.saturating_sub(1),
            width: inner_area.width,
            height: 1,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_core::types::{ContentBlock, Message};
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn open_resets_reader_state_and_close_restores_transcript_offset() {
        let mut state = ResponseReaderState {
            visible: false,
            message_index: Some(2),
            scroll_offset: 7,
            restore_transcript_offset: 0,
            search_query: "old".to_string(),
            search_active: true,
        };

        state.open(4, 12);

        assert!(state.visible);
        assert_eq!(state.message_index, Some(4));
        assert_eq!(state.scroll_offset, 0);
        assert_eq!(state.restore_transcript_offset, 12);
        assert!(state.search_query.is_empty());
        assert!(!state.search_active);
        assert_eq!(state.close(), Some(12));
        assert!(!state.visible);
        assert_eq!(state.message_index, None);
        assert_eq!(state.scroll_offset, 0);
        assert_eq!(state.restore_transcript_offset, 0);
        assert!(state.search_query.is_empty());
        assert!(!state.search_active);
    }

    #[test]
    fn navigation_stays_within_rendered_line_bounds() {
        let mut state = ResponseReaderState::default();

        state.page_down(4, 10);
        assert_eq!(state.scroll_offset, 4);
        state.page_down(4, 10);
        assert_eq!(state.scroll_offset, 6);
        state.page_up(4);
        assert_eq!(state.scroll_offset, 2);
        state.home();
        assert_eq!(state.scroll_offset, 0);
        state.end(4, 10);
        assert_eq!(state.scroll_offset, 6);
        state.end(8, 3);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn reader_text_separates_text_sections_around_non_text_blocks() {
        let message = Message::assistant_blocks(vec![
            ContentBlock::Text {
                text: "one".to_string(),
            },
            ContentBlock::Thinking {
                thinking: "internal work".to_string(),
                signature: String::new(),
            },
            ContentBlock::Text {
                text: "two".to_string(),
            },
        ]);

        assert_eq!(response_reader_text(&message), "one\ntwo");
    }

    #[test]
    fn render_shows_text_lines_and_omits_tool_blocks() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut state = ResponseReaderState::default();
        state.open(0, 3);
        let message = Message::assistant_blocks(vec![
            ContentBlock::Text {
                text: "# Response\n\nvisible reader text".to_string(),
            },
            ContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "secret_tool".to_string(),
                input: serde_json::json!({}),
            },
        ]);

        terminal
            .draw(|frame| render_response_reader(frame, &state, &message, frame.area()))
            .unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("Reader"));
        assert!(content.contains("line 1 /"));
        assert!(content.contains("visible reader text"));
        assert!(!content.contains("secret_tool"));
        assert!(content.contains("PgUp/PgDn"));
    }

    #[test]
    fn render_keeps_tail_visible_after_more_than_u16_lines() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut state = ResponseReaderState::default();
        state.open(0, 0);
        let message = Message::assistant(format!("{}TAIL MARKER", "line\n".repeat(70_000)));
        state.end(22, 70_001);

        terminal
            .draw(|frame| render_response_reader(frame, &state, &message, frame.area()))
            .unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("TAIL MARKER"));
    }
}
