//! T10 (#417), large-content fixtures for the Next renderer: correctness
//! only. A 100,000-line answer, a 10,000-line diff-shaped tool result and
//! binary tool output must render at every size without panicking, keep
//! cells inert, keep the composer and the way back visible, and scroll to
//! both ends. Rows are kept between frames (#439), so the same tests check a
//! kept row never outlives the content it was built from. Timing targets
//! belong to the performance harness (#425); the elapsed time printed here
//! is an observation, not a gate.

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
                content: ToolResultContent::Text(diff),
                is_error: Some(false),
            },
            ContentBlock::ToolResult {
                tool_use_id: "toolu_bin".to_string(),
                content: ToolResultContent::Text(binary),
                is_error: Some(false),
            },
        ]),
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
    assert!(
        away.contains("answer line 0"),
        "reading the answer:\n{away}"
    );
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

#[test]
fn jump_to_start_reaches_the_first_row_and_keeps_scrolling_working() {
    let mut app = workload_app();
    draw(&app, 80, 24);
    let alt = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT);
    app.handle_key_event(alt('g'));
    let top = text_of(&draw(&app, 80, 24));
    app.settle_scroll_anchor();
    assert!(top.contains("first question"), "start:\n{top}");
    assert!(top.contains("answer line 000001"), "{top}");
    assert!(!app.auto_scroll);

    app.handle_key_event(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    let lower = text_of(&draw(&app, 80, 24));
    app.settle_scroll_anchor();
    assert!(
        !lower.contains("first question"),
        "PageDown moves at once:\n{lower}"
    );

    app.handle_key_event(alt('l'));
    let live = text_of(&draw(&app, 80, 24));
    assert!(live.contains("final words"), "live:\n{live}");
}

/// A frame drawn after any change must match one drawn from scratch.
#[test]
fn kept_rows_never_outlive_what_they_were_built_from() {
    fn small() -> App {
        let mut app = workload_app();
        app.messages = vec![
            Message::user("alpha question"),
            Message::assistant("bravo answer"),
            Message::assistant("charlie notes"),
        ];
        app
    }
    fn same_as_fresh(app: &App, messages: &[Message], width: u16, step: &str) {
        let mut fresh = small();
        fresh.messages = messages.to_vec();
        fresh.next_presentation = app.next_presentation.clone();
        assert_eq!(
            text_of(&draw(app, width, 24)),
            text_of(&draw(&fresh, width, 24)),
            "{step}"
        );
    }

    let mut app = small();
    draw(&app, 80, 24);

    // Same length and position, different text.
    app.messages[1] = Message::assistant("bravo ANSWER");
    let frame = text_of(&draw(&app, 80, 24));
    assert!(frame.contains("bravo ANSWER"), "{frame}");
    assert!(!frame.contains("bravo answer"), "{frame}");
    same_as_fresh(&app, &app.messages.clone(), 80, "replaced");

    app.messages.truncate(1);
    let frame = text_of(&draw(&app, 80, 24));
    assert!(!frame.contains("bravo"), "truncated:\n{frame}");
    same_as_fresh(&app, &app.messages.clone(), 80, "truncated");

    app.messages.clear();
    app.messages.push(Message::user("delta question"));
    let frame = text_of(&draw(&app, 80, 24));
    assert!(!frame.contains("alpha"), "cleared:\n{frame}");
    same_as_fresh(&app, &app.messages.clone(), 80, "cleared");

    // A narrower pane rewraps kept rows.
    app.messages.push(Message::assistant(
        "echo ".repeat(30).trim_end().to_string(),
    ));
    draw(&app, 80, 24);
    same_as_fresh(&app, &app.messages.clone(), 30, "rewrapped");

    // A density change alters the rows themselves.
    let before = text_of(&draw(&app, 80, 24));
    app.next_presentation.density = match app.next_presentation.density {
        claurst_core::appearance::Density::Comfortable => {
            claurst_core::appearance::Density::Compact
        }
        _ => claurst_core::appearance::Density::Comfortable,
    };
    assert_ne!(before, text_of(&draw(&app, 80, 24)), "density");
    same_as_fresh(&app, &app.messages.clone(), 80, "density");
}

/// Going to the start holds through whatever moves the bottom-relative offset,
/// until the reader scrolls down.
#[test]
fn the_start_stays_the_start_through_layout_changes_and_new_output() {
    let alt = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT);
    let at_start = |app: &mut App, width, height, step: &str| {
        let frame = text_of(&draw(app, width, height));
        app.settle_scroll_anchor();
        assert!(frame.contains("first question"), "{step}:\n{frame}");
    };

    let mut app = workload_app();
    draw(&app, 160, 40);
    app.handle_key_event(alt('g'));
    at_start(&mut app, 160, 40, "jump");
    at_start(&mut app, 160, 20, "shorter");
    at_start(&mut app, 80, 40, "narrower");
    at_start(&mut app, 80, 24, "resized again");

    app.streaming_text.push_str("arriving\n");
    at_start(&mut app, 80, 24, "new output");
    let frame = text_of(&draw(&app, 80, 24));
    assert!(frame.contains("new activity below"), "{frame}");

    app.handle_key_event(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    let frame = text_of(&draw(&app, 80, 24));
    app.settle_scroll_anchor();
    assert!(!frame.contains("first question"), "released:\n{frame}");
    // Far enough that a taller pane does not reach the first row by itself.
    for _ in 0..5 {
        app.handle_key_event(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        draw(&app, 80, 24);
        app.settle_scroll_anchor();
    }
    let frame = text_of(&draw(&app, 80, 40));
    assert!(
        !frame.contains("first question"),
        "stays released:\n{frame}"
    );

    // With the transcript hidden after returning to live, nothing is measured.
    let mut app = workload_app();
    app.next_shell.inspector = Some(InspectorPage::Activity);
    draw(&app, 80, 24);
    app.handle_key_event(alt('l'));
    app.handle_key_event(alt('g'));
    draw(&app, 80, 24);
    app.next_shell.inspector = None;
    at_start(&mut app, 80, 24, "nothing measured");
}

/// A long streaming answer costs about the same as a long committed one.
#[test]
fn observation_frame_cost_while_streaming() {
    let mut app = workload_app();
    app.messages.truncate(1);
    let mut times = Vec::new();
    for lines in [1_000, 100_000] {
        app.streaming_text = (1..=lines).map(|n| format!("live line {n:06}\n")).collect();
        draw(&app, 80, 24);
        let started = std::time::Instant::now();
        for n in 0..3 {
            app.streaming_text.push_str(&format!("more {n}\npartial"));
            draw(&app, 80, 24);
            app.streaming_text.push('\n');
        }
        times.push((lines, started.elapsed() / 3));
    }
    eprintln!("observation: streaming frame cost by answer length {times:?}");
    let frame = text_of(&draw(&app, 80, 24));
    assert!(frame.contains("more 2"), "{frame}");
}
