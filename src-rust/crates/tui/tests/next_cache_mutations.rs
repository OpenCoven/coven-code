//! Cache correctness against a fresh renderer after every mutable-state change.
use claurst_core::{
    config::Config,
    cost::CostTracker,
    types::{Message, MessageContent},
};
use claurst_tui::{
    app::{App, ToolStatus, ToolUseBlock},
    application::{TurnCancellation, UiRenderer},
    next::{style::ColorMode, InspectorPage},
};
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

fn app() -> App {
    let mut app = App::new(
        Config {
            project_dir: Some("/synthetic/project".into()),
            ..Config::default()
        },
        CostTracker::new(),
    );
    app.has_credentials = true;
    app.renderer = UiRenderer::Next;
    app.turn_start = None;
    app
}
fn frame(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| claurst_tui::next::render_next(f, app))
        .unwrap();
    terminal.backend().buffer().clone()
}
fn matches_fresh(current: &App, width: u16, height: u16) {
    let mut fresh = app();
    fresh.config = current.config.clone();
    fresh.messages = current.messages.clone().into();
    fresh.streaming_text = current.streaming_text.clone().into();
    fresh.tool_use_blocks = current.tool_use_blocks.clone().into();
    fresh.interrupted_tools = current.interrupted_tools.clone().into();
    fresh.permission_decisions = current.permission_decisions.clone().into();
    fresh.is_streaming = current.is_streaming;
    fresh.cancellation = current.cancellation;
    fresh.next_presentation = current.next_presentation.clone();
    fresh.next_shell.inspector = current.next_shell.inspector;
    assert_eq!(frame(current, width, height), frame(&fresh, width, height));
}

#[test]
fn indexed_text_edits_and_vector_borrows_never_leave_stale_rows() {
    let mut app = app();
    app.messages.push(Message::user("question"));
    app.messages.push(Message::assistant("old text"));
    matches_fresh(&app, 80, 24);
    if let MessageContent::Text(text) = &mut app.messages[1].content {
        text.replace_range(.., "NEW TEXT");
    }
    matches_fresh(&app, 80, 24);
    let messages: &mut Vec<Message> = &mut app.messages;
    messages[0] = Message::user("QUESTION");
    matches_fresh(&app, 80, 24);
    app.messages = vec![Message::assistant("same size, new container")].into();
    matches_fresh(&app, 80, 24);
    app.messages.clear();
    matches_fresh(&app, 80, 24);
}

#[test]
fn streaming_chunks_replacements_and_layout_changes_match_fresh_rows() {
    let mut app = app();
    app.is_streaming = true;
    let text = "alpha\u{1b}]52;c;test\u{7}\r\n\n漢字 ".to_string()
        + &"wide 漢字 ".repeat(30)
        + "\nlast line\n\ntail \u{9b}31m end";
    for ch in text.chars() {
        app.streaming_text.push(ch);
        matches_fresh(&app, 40, 12);
    }
    app.streaming_text
        .replace_range(.., "same byte length is irrelevant\nretry");
    matches_fresh(&app, 40, 12);
    app.streaming_text.truncate(4);
    matches_fresh(&app, 40, 12);
    app.streaming_text.clear();
    app.streaming_text.push_str("a new response\npartial");
    matches_fresh(&app, 80, 24);
    app.next_presentation.appearance.color = ColorMode::NoColor;
    matches_fresh(&app, 30, 12);
    matches_fresh(&app, 120, 40);
}

#[test]
fn tool_rows_and_running_count_follow_direct_edits_decisions_and_cancellation() {
    let mut app = app();
    app.tool_use_blocks.push(ToolUseBlock {
        id: "tool".into(),
        name: "Read".into(),
        turn_index: None,
        status: ToolStatus::Running,
        output_preview: None,
        input_json: "{\"path\":\"/synthetic/project/file\"}".into(),
        run: Default::default(),
    });
    matches_fresh(&app, 80, 24);
    app.tool_use_blocks[0].input_json = "{\"path\":\"/synthetic/project/EDIT\"}".into();
    app.permission_decisions.insert("tool".into(), false);
    matches_fresh(&app, 80, 24);
    app.permission_decisions.insert("tool".into(), true);
    app.tool_use_blocks[0].status = ToolStatus::Done;
    app.tool_use_blocks[0].output_preview = Some("output\u{1b}[2J".into());
    matches_fresh(&app, 80, 24);
    app.interrupted_tools.push(app.tool_use_blocks[0].clone());
    app.tool_use_blocks.clear();
    app.cancellation = TurnCancellation::Requested;
    matches_fresh(&app, 80, 24);
    app.cancellation = TurnCancellation::Idle;
    app.config.project_dir = Some("/different".into());
    matches_fresh(&app, 80, 24);
    app.next_shell.inspector = Some(InspectorPage::Activity);
    matches_fresh(&app, 120, 40);
    matches_fresh(&app, 40, 12);
}
