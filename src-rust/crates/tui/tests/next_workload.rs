//! T10 (#417), large-content fixtures for the Next renderer: correctness
//! only. A 100,000-line answer, a 10,000-line diff and binary output must
//! render at every size without panicking, keep cells inert, keep the
//! composer and the way back visible, and scroll to both ends. Next does not
//! draw tool results yet, so the diff and the binary output also arrive as
//! assistant text, which it does draw. Timing targets belong to the performance harness (#425); the
//! elapsed time printed here is an observation, not a gate.

use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::types::{ContentBlock, Message, ToolResultContent};
use claurst_tui::app::App;
use claurst_tui::application::UiRenderer;
use claurst_tui::next::style::{Appearance, ColorMode, Glyphs};
use claurst_tui::next::{InspectorPage, NextPresentation};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use serde_json::json;

fn workload_app() -> App {
    let config = Config {
        model: Some("claude-shell-model".to_string()),
        provider: Some("anthropic".to_string()),
        project_dir: Some("/work/acme-api".into()),
        ..Config::default()
    };
    let mut app = App::new(config, CostTracker::new());
    app.has_credentials = true;
    app.renderer = UiRenderer::Next;
    app.turn_start = None;
    app.next_presentation = NextPresentation {
        appearance: Appearance {
            color: ColorMode::NoColor,
            glyphs: Glyphs::Ascii,
            ..Appearance::default()
        },
        ..NextPresentation::default()
    };

    let answer: String = (1..=100_000)
        .map(|n| format!("answer line {n:06}\n"))
        .collect();
    let diff: String = (1..=10_000)
        .map(|n| {
            if n % 2 == 0 {
                format!("+added line {n}\n")
            } else {
                format!("-removed line {n}\n")
            }
        })
        .collect();
    // Bytes 0..=255 repeated, read lossily as a tool would.
    let bytes: Vec<u8> = (0..64 * 1024).map(|n| (n % 256) as u8).collect();
    let binary = String::from_utf8_lossy(&bytes).into_owned();

    app.messages = vec![
        Message::user("first question"),
        Message::assistant(answer),
        Message::assistant_blocks(vec![
            ContentBlock::ToolUse {
                id: "toolu_diff".to_string(),
                name: "Bash".to_string(),
                input: json!({ "command": "git diff" }),
            },
            ContentBlock::ToolUse {
                id: "toolu_bin".to_string(),
                name: "Bash".to_string(),
                input: json!({ "command": "cat blob.bin" }),
            },
        ]),
        Message::user_blocks(vec![
            ContentBlock::ToolResult {
                tool_use_id: "toolu_diff".to_string(),
                content: ToolResultContent::Text(diff.clone()),
                is_error: Some(false),
            },
            ContentBlock::ToolResult {
                tool_use_id: "toolu_bin".to_string(),
                content: ToolResultContent::Text(binary.clone()),
                is_error: Some(false),
            },
        ]),
        Message::assistant(diff),
        Message::assistant(binary),
        Message::assistant("final words"),
    ];
    app
}

fn draw(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| claurst_tui::next::render_next(frame, app))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn text_of(buffer: &Buffer) -> String {
    let width = usize::from(buffer.area.width.max(1));
    buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn inert(buffer: &Buffer) -> bool {
    buffer.content.iter().all(|cell| {
        !cell
            .symbol()
            .chars()
            .any(|ch| matches!(ch, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}'))
    })
}

#[test]
fn large_content_renders_inert_at_every_size_with_the_composer_and_way_back() {
    let mut app = workload_app();
    let started = std::time::Instant::now();
    for (width, height) in [(20, 6), (40, 12), (80, 24), (180, 50)] {
        app.next_shell.inspector = None;
        let buffer = draw(&app, width, height);
        assert!(inert(&buffer), "{width}x{height}");
        if height >= 12 {
            let text = text_of(&buffer);
            assert!(
                text.contains("final words"),
                "{width}x{height} live end:\n{text}"
            );
        }
        app.next_shell.inspector = Some(InspectorPage::Activity);
        let buffer = draw(&app, width, height);
        assert!(inert(&buffer), "{width}x{height} inspector");
        if height >= 12 {
            assert!(text_of(&buffer).contains("Esc"), "{width}x{height}");
        }
    }
    eprintln!("observation: 8 large frames in {:?}", started.elapsed());
}

#[test]
fn a_100k_line_answer_scrolls_away_from_live_and_returns() {
    let mut app = workload_app();
    draw(&app, 80, 24);
    for _ in 0..50 {
        app.handle_key_event(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        draw(&app, 80, 24);
        app.settle_scroll_anchor();
    }
    let away = text_of(&draw(&app, 80, 24));
    assert!(!away.contains("final words"), "scrolled away:\n{away}");
    // The far end: an offset past the first row settles on the first rows.
    app.scroll_offset = 1_000_000;
    app.auto_scroll = false;
    draw(&app, 80, 24);
    app.settle_scroll_anchor();
    let start = text_of(&draw(&app, 80, 24));
    assert!(start.contains("first question"), "the start:\n{start}");
    assert!(start.contains("answer line 000001"), "the start:\n{start}");
    app.handle_key_event(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::ALT));
    app.settle_scroll_anchor();
    let live = text_of(&draw(&app, 80, 24));
    assert!(live.contains("final words"), "back to live:\n{live}");
}

/// Observation for #417's virtualization criterion and #425: one frame with
/// a 1,000-line and a 100,000-line answer. Printed, not asserted.
#[test]
fn observation_frame_cost_against_transcript_length() {
    let mut app = workload_app();
    let mut times = Vec::new();
    for lines in [1_000, 100_000] {
        let answer: String = (1..=lines)
            .map(|n| format!("answer line {n:06}\n"))
            .collect();
        app.messages[1] = Message::assistant(answer);
        draw(&app, 80, 24);
        let started = std::time::Instant::now();
        for _ in 0..3 {
            draw(&app, 80, 24);
        }
        times.push((lines, started.elapsed() / 3));
    }
    eprintln!("observation: frame cost by answer length {times:?}");
}
