//! T7 (#414), read-only slice: the Next Research page.
//!
//! The engine has no research job, source store or citation producer. The
//! page lists this session's web search and fetch calls from the transcript,
//! says plainly that they are tool activity rather than a research job, shows
//! unrecorded provenance as unknown (#433), never shows URL credentials or
//! query strings, and makes no claim that anything was verified.

use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::types::{ContentBlock, Message, ToolResultContent};
use claurst_tui::app::App;
use claurst_tui::application::UiRenderer;
use claurst_tui::next::style::{Appearance, ColorMode, Glyphs};
use claurst_tui::next::{InspectorPage, NextPresentation};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use serde_json::json;

fn next_app() -> App {
    let config = Config {
        model: Some("claude-shell-model".to_string()),
        provider: Some("anthropic".to_string()),
        project_dir: Some("/work/acme-api".into()),
        ..Config::default()
    };
    let mut app = App::new(config, CostTracker::new());
    app.has_credentials = true;
    app.renderer = UiRenderer::Next;
    app.next_presentation = NextPresentation {
        appearance: Appearance {
            color: ColorMode::NoColor,
            glyphs: Glyphs::Ascii,
            ..Appearance::default()
        },
        ..NextPresentation::default()
    };
    app.turn_start = None;
    app
}

fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| claurst_tui::next::render_next(frame, app))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width.max(1));
    buffer
        .content
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn call(id: &str, name: &str, input: serde_json::Value) -> Message {
    Message::assistant_blocks(vec![ContentBlock::ToolUse {
        id: id.to_string(),
        name: name.to_string(),
        input,
    }])
}

fn result(id: &str, text: &str, is_error: bool) -> Message {
    Message::user_blocks(vec![ContentBlock::ToolResult {
        tool_use_id: id.to_string(),
        content: ToolResultContent::Text(text.to_string()),
        is_error: Some(is_error),
    }])
}

fn research_app() -> App {
    let mut app = next_app();
    app.messages = vec![
        call(
            "toolu_search",
            "WebSearch",
            json!({"query": "ratatui wrapping"}),
        ),
        result(
            "toolu_search",
            "1. **Ratatui docs**\n   URL: https://ratatui.rs/concepts\n   snippet\n\n",
            false,
        ),
        call(
            "toolu_fetch",
            "WebFetch",
            json!({"url": "https://user:secret@api.example.com/v1?token=abc", "prompt": "x"}),
        ),
        result("toolu_fetch", "page text", false),
        call(
            "toolu_gone",
            "WebFetch",
            json!({"url": "https://example.com/gone"}),
        ),
        result(
            "toolu_gone",
            "HTTP 404 Not Found when fetching https://example.com/gone",
            true,
        ),
    ];
    app.next_shell.inspector = Some(InspectorPage::Research);
    app
}

#[test]
fn tab_reaches_research_after_work() {
    let mut app = next_app();
    app.handle_key_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::ALT));
    for _ in 0..3 {
        app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    assert_eq!(app.next_shell.inspector, Some(InspectorPage::Research));
}

#[test]
fn each_call_shows_its_tool_reference_outcome_and_unknown_provenance() {
    let text = draw(&research_app(), 80, 50);
    for expected in [
        "[search] ratatui wrapping",
        "Ratatui docs",
        "https://ratatui.rs/concepts",
        "parsed, unverified",
        "[fetch] https://api.example.com/v1?…",
        "toolu_fetch",
        "returned",
        "HTTP 404",
        "unknown",
    ] {
        assert!(text.contains(expected), "missing {expected:?}:\n{text}");
    }
}

#[test]
fn credentials_and_query_strings_never_reach_the_screen() {
    let text = draw(&research_app(), 120, 50);
    assert!(!text.contains("secret"), "{text}");
    assert!(!text.contains("token=abc"), "{text}");
}

#[test]
fn the_page_says_what_it_is_and_never_claims_verification() {
    let mut app = research_app();
    draw(&app, 80, 24);
    for _ in 0..20 {
        app.next_shell.apply_action("scrollInspectorDown");
    }
    let text = draw(&app, 80, 24);
    assert!(text.contains("not a research job"), "{text}");
    assert!(text.contains("#433"), "{text}");
    let full = draw(&research_app(), 80, 50).to_lowercase();
    assert!(
        !full.replace("unverified", "").contains("verified"),
        "no verified claim:\n{full}"
    );
}

#[test]
fn an_empty_session_says_so() {
    let mut app = next_app();
    app.next_shell.inspector = Some(InspectorPage::Research);
    let text = draw(&app, 80, 24);
    assert!(text.contains("No web search or fetch"), "{text}");
}

#[test]
fn an_interrupted_call_is_not_shown_as_pending() {
    let mut app = next_app();
    app.messages = vec![call(
        "toolu_cut",
        "WebFetch",
        json!({"url": "https://example.com/slow"}),
    )];
    app.next_shell.inspector = Some(InspectorPage::Research);
    assert!(draw(&app, 80, 24).contains("no result yet"));
    app.interrupted_tools.push(claurst_tui::app::ToolUseBlock {
        id: "toolu_cut".to_string(),
        name: "WebFetch".to_string(),
        turn_index: None,
        status: claurst_tui::app::ToolStatus::Error,
        output_preview: None,
        input_json: String::new(),
    });
    let text = draw(&app, 80, 24);
    assert!(text.contains("interrupted"), "{text}");
}

#[test]
fn the_views_page_points_research_at_the_local_page() {
    let mut app = next_app();
    app.next_shell.inspector = Some(InspectorPage::Views);
    let text = draw(&app, 80, 24);
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("Research "))
        .unwrap_or_else(|| panic!("Research missing:\n{text}"));
    assert!(line.contains("local only"), "{line}");
    assert!(line.contains("#414"), "{line}");
}

#[test]
fn the_research_page_renders_at_small_fixtures() {
    let app = research_app();
    let medium = draw(&app, 40, 12);
    assert!(medium.contains("Esc"), "{medium}");
    for (width, height) in [(20, 6), (1, 1)] {
        assert!(!draw(&app, width, height).is_empty());
    }
}

#[test]
fn a_failed_fetch_never_shows_credentials_or_query() {
    let mut app = next_app();
    app.messages = vec![
        call(
            "toolu_401",
            "WebFetch",
            json!({"url": "https://user:secret@api.example.com/v1?token=abc"}),
        ),
        result(
            "toolu_401",
            "HTTP 401 Unauthorized when fetching https://user:secret@api.example.com/v1?token=abc",
            true,
        ),
    ];
    app.next_shell.inspector = Some(InspectorPage::Research);
    let text = draw(&app, 160, 30);
    assert!(text.contains("HTTP 401"), "{text}");
    assert!(!text.contains("secret"), "{text}");
    assert!(!text.contains("token=abc"), "{text}");
}

#[test]
fn a_search_whose_output_is_not_parsed_says_so() {
    let mut app = next_app();
    app.messages = vec![
        call("toolu_ddg", "WebSearch", json!({"query": "ratatui"})),
        result(
            "toolu_ddg",
            "**Wikipedia**\nRatatui is a library\nURL: https://example.com/\n\n- related\n  https://example.com/r\n",
            false,
        ),
    ];
    app.next_shell.inspector = Some(InspectorPage::Research);
    let text = draw(&app, 100, 30);
    assert!(text.contains("none parsed"), "{text}");
}

#[test]
fn a_call_without_a_result_after_the_next_prompt_is_not_pending() {
    let mut app = next_app();
    app.messages = vec![
        call(
            "toolu_cut",
            "WebFetch",
            json!({"url": "https://example.com/slow"}),
        ),
        claurst_core::types::Message::user("next prompt"),
    ];
    app.next_shell.inspector = Some(InspectorPage::Research);
    let text = draw(&app, 80, 24);
    assert!(!text.contains("no result yet"), "{text}");
    assert!(text.contains("no result recorded"), "{text}");
}

#[test]
fn at_40x12_the_scope_note_and_the_first_row_stay_visible() {
    let text = draw(&research_app(), 40, 12);
    assert!(text.contains("not a research job"), "{text}");
    assert!(text.contains("[search]"), "{text}");
}
