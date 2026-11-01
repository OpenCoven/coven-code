//! T3 (#410): the Next transcript/composer shell.
//!
//! Layout rules at the accepted fixture sizes, one inspector that owns focus
//! and returns predictably, reading position under streaming, cancellation
//! that stays pending until the runtime settles it, paste safety, workspace
//! context and text fallbacks. Classic must not observe any of it.

use claurst_api::streaming::ContentDelta;
use claurst_api::AnthropicStreamEvent;
use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::types::{ContentBlock, ImageSource, Message};
use claurst_query::QueryEvent;
use claurst_tui::app::App;
use claurst_tui::application::{ScopedQueryEvent, SessionScope, TurnCancellation, UiRenderer};
use claurst_tui::image_paste::PastedImage;
use claurst_tui::next::layout::{plan, SizeClass};
use claurst_tui::next::style::{Appearance, ColorMode, Glyphs};
use claurst_tui::next::{InspectorPage, NextPresentation};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::{Backend, TestBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Terminal;

const FIXTURES: [(u16, u16); 5] = [(20, 6), (40, 12), (80, 24), (120, 40), (180, 50)];

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

fn draw(app: &App, width: u16, height: u16) -> (Buffer, Option<(u16, u16)>) {
    // A frame that places no cursor leaves this sentinel untouched.
    const UNSET: (u16, u16) = (u16::MAX, u16::MAX);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .backend_mut()
        .set_cursor_position(UNSET)
        .expect("cursor");
    terminal
        .draw(|frame| claurst_tui::next::render_next(frame, app))
        .expect("draw");
    let cursor = terminal
        .backend_mut()
        .get_cursor_position()
        .ok()
        .map(|p| (p.x, p.y))
        .filter(|&position| position != UNSET);
    (terminal.backend().buffer().clone(), cursor)
}

fn text_of(buffer: &Buffer) -> String {
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

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn alt(c: char) -> KeyEvent {
    key(KeyCode::Char(c), KeyModifiers::ALT)
}

fn esc() -> KeyEvent {
    key(KeyCode::Esc, KeyModifiers::NONE)
}

fn delta(text: &str) -> QueryEvent {
    QueryEvent::Stream(AnthropicStreamEvent::ContentBlockDelta {
        index: 0,
        delta: ContentDelta::TextDelta {
            text: text.to_string(),
        },
    })
}

fn apply(app: &mut App, event: QueryEvent) {
    let scope = SessionScope::default();
    scope.apply(
        app,
        ScopedQueryEvent {
            generation: scope.current(),
            event,
        },
    );
}

fn with_history(app: &mut App, count: usize) {
    for index in 0..count {
        app.push_message(Message::user(format!("question {index:03}")));
        app.push_message(Message::assistant(format!("answer {index:03}")));
    }
}

#[test]
fn layout_follows_the_accepted_size_rules() {
    for (width, height) in FIXTURES {
        let area = Rect::new(0, 0, width, height);
        let closed = plan(area, false, 0, 2);
        assert!(closed.inspector.is_none(), "{width}x{height}");
        assert!(closed.transcript.is_some(), "{width}x{height}");
        assert!(closed.composer.height >= 1, "{width}x{height}");

        let open = plan(area, true, 0, 2);
        let inspector = open.inspector.expect("requested inspector is shown");
        match open.size_class {
            SizeClass::Split => {
                let transcript = open.transcript.expect("split keeps the transcript");
                assert!(transcript.width >= 72, "{width}x{height}: {transcript:?}");
                assert!(inspector.width >= 40, "{width}x{height}: {inspector:?}");
                assert!(transcript.x + transcript.width < inspector.x, "separator");
            }
            SizeClass::Focus | SizeClass::Single => {
                assert!(
                    open.transcript.is_none(),
                    "inspector replaces the main pane"
                );
                assert_eq!(inspector.width, width);
            }
        }
        // Every region stays inside the terminal and the composer survives.
        for rect in [
            Some(open.header),
            open.transcript,
            Some(inspector),
            Some(open.composer),
        ]
        .into_iter()
        .flatten()
        {
            assert!(rect.x + rect.width <= width && rect.y + rect.height <= height);
        }
        assert!(open.composer.height >= 1);
    }
    let class = |w, h| plan(Rect::new(0, 0, w, h), false, 0, 2).size_class;
    assert_eq!(class(79, 40), SizeClass::Focus);
    assert_eq!(class(120, 23), SizeClass::Focus);
    assert_eq!(class(80, 24), SizeClass::Single);
    assert_eq!(class(119, 40), SizeClass::Single);
    assert_eq!(class(120, 31), SizeClass::Single);
    assert_eq!(class(120, 32), SizeClass::Split);
    assert_eq!(class(180, 50), SizeClass::Split);
}

#[test]
fn every_fixture_size_draws_without_underflow_and_keeps_the_draft_visible() {
    let mut app = next_app();
    with_history(&mut app, 30);
    app.set_prompt_text("draft-x".to_string());
    for inspector in [None, Some(InspectorPage::Activity)] {
        app.next_shell.inspector = inspector;
        for (width, height) in FIXTURES
            .into_iter()
            .chain([(0, 0), (1, 1), (8, 2), (20, 3)])
        {
            let (buffer, _) = draw(&app, width, height);
            if width >= 20 && height >= 3 {
                let text = text_of(&buffer);
                assert!(text.contains("draft-x"), "{width}x{height}:\n{text}");
            }
        }
    }
}

#[test]
fn inspector_opens_beside_the_transcript_only_when_both_fit() {
    let mut app = next_app();
    with_history(&mut app, 3);
    assert!(!app.handle_key_event(alt('i')));
    assert_eq!(app.next_shell.inspector, Some(InspectorPage::Activity));

    let (wide, _) = draw(&app, 120, 40);
    let wide = text_of(&wide);
    assert!(
        wide.contains("answer 002"),
        "transcript stays at 120x40:\n{wide}"
    );
    assert!(
        wide.contains("Activity"),
        "inspector shown at 120x40:\n{wide}"
    );

    let (narrow, _) = draw(&app, 80, 24);
    let narrow = text_of(&narrow);
    assert!(narrow.contains("Activity"), "{narrow}");
    assert!(
        !narrow.contains("answer 002"),
        "inspector replaces main at 80x24:\n{narrow}"
    );
    assert!(narrow.contains("Esc"), "the way back is shown:\n{narrow}");
}

#[test]
fn escape_closes_the_inspector_first_and_preserves_draft_cursor_and_position() {
    let mut app = next_app();
    with_history(&mut app, 20);
    app.set_prompt_text("half written".to_string());
    app.prompt_input.cursor = 4;
    app.scroll_offset = 7;
    app.auto_scroll = false;
    app.is_streaming = true;

    app.handle_key_event(alt('i'));
    app.handle_key_event(key(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.next_shell.inspector, Some(InspectorPage::Session));
    app.handle_key_event(esc());
    assert_eq!(app.next_shell.inspector, None, "Esc closes the inspector");
    assert!(
        app.is_streaming,
        "Esc that closes the inspector does not cancel"
    );
    assert_eq!(app.cancellation, TurnCancellation::Idle);
    assert_eq!(app.prompt_input.text, "half written");
    assert_eq!(app.prompt_input.cursor, 4);
    assert_eq!((app.scroll_offset, app.auto_scroll), (7, false));

    // With nothing open, Esc asks the runtime to cancel the turn.
    app.handle_key_event(esc());
    assert_eq!(app.cancellation, TurnCancellation::Requested);
}

#[test]
fn classic_does_not_observe_next_only_keys() {
    let mut app = next_app();
    app.renderer = UiRenderer::Classic;
    app.set_prompt_text("abc".to_string());
    app.handle_key_event(alt('i'));
    app.handle_key_event(alt('l'));
    assert_eq!(app.next_shell.inspector, None);
    assert!(app.auto_scroll);
    // Same as before #410: the chords fall through to text input
    // (verified against the #409 head a65df294).
    assert_eq!(app.prompt_input.text, "abcil");
}

#[test]
fn escape_and_interrupt_request_runtime_cancellation_and_next_shows_it_pending() {
    for (renderer, cancel_key) in [
        (UiRenderer::Classic, esc()),
        (UiRenderer::Next, esc()),
        (
            UiRenderer::Next,
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ),
    ] {
        let mut app = next_app();
        app.renderer = renderer;
        app.push_message(Message::user("run it".to_string()));
        apply(&mut app, delta("working on it"));
        assert!(app.is_streaming);
        app.handle_key_event(cancel_key);
        assert!(
            app.take_cancellation_request(),
            "{renderer:?} {cancel_key:?}"
        );
        assert!(!app.take_cancellation_request(), "a request is taken once");
        assert_eq!(app.cancellation, TurnCancellation::AwaitingRuntime);
        if renderer == UiRenderer::Next {
            let text = text_of(&draw(&app, 80, 24).0);
            assert!(text.contains("cancelling"), "{text}");
            assert!(!text.contains("Cancelled."), "not confirmed yet:\n{text}");
        }
        app.settle_cancellation();
        assert_eq!(app.cancellation, TurnCancellation::Idle);
        if renderer == UiRenderer::Next {
            let text = text_of(&draw(&app, 80, 24).0);
            assert!(!text.contains("cancelling"), "{text}");
        }
    }
    // Esc while idle does not invent a cancellation.
    let mut idle = next_app();
    idle.handle_key_event(esc());
    assert!(!idle.take_cancellation_request());
}

#[test]
fn a_scrolled_reader_keeps_position_while_output_streams_and_can_jump_to_live() {
    let mut app = next_app();
    with_history(&mut app, 40);
    app.handle_key_event(key(KeyCode::PageUp, KeyModifiers::NONE));
    assert!(!app.auto_scroll);
    let (before, _) = draw(&app, 80, 24);
    let before = text_of(&before);
    let anchor_line = before
        .lines()
        .find(|line| line.starts_with("answer "))
        .expect("history visible")
        .to_string();
    assert!(!before.contains("new activity"), "{before}");

    app.push_message(Message::user("follow-up".to_string()));
    apply(&mut app, delta(&"streamed line\n".repeat(30)));
    apply(
        &mut app,
        QueryEvent::ToolStart {
            tool_name: "Read".to_string(),
            tool_id: "toolu-1".to_string(),
            input_json: "{}".to_string(),
        },
    );
    for _ in 0..3 {
        let (after, _) = draw(&app, 80, 24);
        app.settle_scroll_anchor();
        let after = text_of(&after);
        assert!(
            after.contains(&anchor_line),
            "reading position kept:\n{after}"
        );
        assert!(after.contains("new activity"), "{after}");
        assert!(after.contains("Alt+L"), "jump key shown:\n{after}");
        assert!(!after.contains("streamed line"), "{after}");
    }
    assert_eq!(
        app.next_shell.inspector, None,
        "a tool start never steals focus"
    );

    app.handle_key_event(alt('l'));
    assert!(app.auto_scroll);
    let (live, _) = draw(&app, 80, 24);
    let live = text_of(&live);
    assert!(live.contains("streamed line"), "{live}");
    assert!(!live.contains("new activity"), "{live}");
}

#[test]
fn pasted_newlines_and_commands_stay_an_unsent_draft() {
    let mut app = next_app();
    app.handle_paste_data("echo one\n/clear".to_string());
    assert!(app.messages.is_empty(), "paste never submits");
    assert!(app.prompt_input.text.contains("/clear"));
    let text = text_of(&draw(&app, 80, 24).0);
    assert!(text.contains("/clear"), "{text}");
    assert!(
        text.contains("not sent"),
        "draft is visibly unsent:\n{text}"
    );

    let mut large = next_app();
    large.handle_paste_data((0..200).map(|n| format!("line {n}\n")).collect());
    assert!(large.messages.is_empty());
    let text = text_of(&draw(&large, 80, 24).0);
    assert!(
        text.contains("[Pasted ~200 lines #1]"),
        "large paste is summarized:\n{text}"
    );
}

#[test]
fn workspace_line_names_project_session_and_model_without_claiming_identity() {
    let mut app = next_app();
    app.session_title = Some("Fix billing export".to_string());
    app.model_name = "claude-shell-model".to_string();
    app.config.familiar = Some("nyx".to_string());
    let text = text_of(&draw(&app, 120, 40).0);
    let header = text.lines().next().unwrap_or_default();
    assert!(header.contains("acme-api"), "{header}");
    assert!(header.contains("Fix billing export"), "{header}");
    assert!(header.contains("claude-shell-model"), "{header}");
    assert!(header.contains("nyx"), "{header}");
    assert!(
        header.contains("unverified"),
        "display name is not identity: {header}"
    );

    // Narrow terminals keep the project and drop optional context first.
    let narrow = text_of(&draw(&app, 40, 12).0);
    let header = narrow.lines().next().unwrap_or_default();
    assert!(header.contains("acme-api"), "{header}");
}

#[test]
fn the_terminal_cursor_sits_in_the_composer_for_ime_and_unicode_input() {
    let mut app = next_app();
    app.set_prompt_text("héllo 漢".to_string());
    let (buffer, cursor) = draw(&app, 80, 24);
    let (x, y) = cursor.expect("composer shows the terminal cursor");
    let row = text_of(&buffer)
        .lines()
        .nth(usize::from(y))
        .unwrap_or_default()
        .to_string();
    assert!(
        row.contains("héllo 漢"),
        "cursor row is the draft row: {row}"
    );
    // "> " prefix is 2 cells, then 6 + 2 cells of text.
    assert_eq!(x, 2 + 6 + 2);

    app.prompt_input.cursor = 1;
    let (_, cursor) = draw(&app, 80, 24);
    assert_eq!(cursor.map(|c| c.0), Some(3));
}

#[test]
fn images_fall_back_to_filename_and_type_text() {
    let mut app = next_app();
    app.prompt_input.add_image(PastedImage {
        path: "/tmp/shot.png".into(),
        label: "shot.png".to_string(),
        dimensions: Some((640, 480)),
    });
    app.push_message(Message::user_blocks(vec![
        ContentBlock::Text {
            text: "see attached".to_string(),
        },
        ContentBlock::Image {
            source: ImageSource {
                source_type: "base64".to_string(),
                media_type: Some("image/png".to_string()),
                data: Some("AAAA".to_string()),
                url: None,
            },
        },
    ]));
    let text = text_of(&draw(&app, 80, 24).0);
    assert!(text.contains("[image shot.png 640x480]"), "{text}");
    assert!(text.contains("[image image/png]"), "{text}");
}

#[test]
fn unavailable_views_explain_their_blocker_instead_of_faking_integrations() {
    let mut app = next_app();
    app.next_shell.inspector = Some(InspectorPage::Views);
    let text = text_of(&draw(&app, 80, 24).0);
    for (view, issue) in [("Work", "#413"), ("Research", "#414"), ("Memory", "#415")] {
        let line = text
            .lines()
            .find(|line| line.contains(view))
            .unwrap_or_else(|| panic!("{view} missing:\n{text}"));
        assert!(line.contains("unavailable"), "{line}");
        assert!(line.contains(issue), "{line}");
    }
    assert!(
        text.contains("Ctrl+K"),
        "commands stay discoverable:\n{text}"
    );
}

#[test]
fn transcript_text_is_selectable_in_next() {
    let mut app = next_app();
    with_history(&mut app, 2);
    let _ = draw(&app, 80, 24);
    let area = app.last_selectable_area.get();
    assert!(area.width > 0 && area.height > 0, "selection has a target");
    let rows = app.last_row_text.borrow();
    assert!(rows.values().any(|row| row.contains("answer 001")));
}
