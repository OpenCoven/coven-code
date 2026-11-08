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
    app.handle_key_event(alt('g'));
    assert_eq!(app.next_shell.inspector, None);
    assert!(app.auto_scroll);
    // Same as before #410: the chords fall through to text input
    // (verified against the #409 head a65df294).
    assert_eq!(app.prompt_input.text, "abcilg");
}

#[test]
fn escape_and_interrupt_request_runtime_cancellation_and_next_shows_it_pending() {
    for (renderer, cancel_key) in [
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
    // Classic keeps its existing cancel handling: no runtime request, and
    // nothing for the loop to settle.
    for cancel_key in [esc(), key(KeyCode::Char('c'), KeyModifiers::CONTROL)] {
        let mut classic = next_app();
        classic.renderer = UiRenderer::Classic;
        classic.push_message(Message::user("run it".to_string()));
        apply(&mut classic, delta("working on it"));
        classic.handle_key_event(cancel_key);
        assert!(!classic.is_streaming);
        assert!(!classic.take_cancellation_request(), "{cancel_key:?}");
        assert_eq!(classic.cancellation, TurnCancellation::Idle);
    }
    // Esc while idle does not invent a cancellation.
    let mut idle = next_app();
    idle.handle_key_event(esc());
    assert!(!idle.take_cancellation_request());
}

#[test]
fn deltas_still_queued_when_a_turn_is_cancelled_cannot_restart_it() {
    let interrupt = key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    for cancel_key in [esc(), interrupt] {
        let mut app = next_app();
        app.push_message(Message::user("run it".to_string()));
        apply(&mut app, delta("delivered in time"));
        app.handle_key_event(cancel_key);
        assert!(app.take_cancellation_request());
        assert!(!app.is_streaming);

        // The worker's backlog keeps arriving until the runtime settles.
        apply(&mut app, delta("too late"));
        assert!(!app.is_streaming, "{cancel_key:?} restarted the turn");
        assert!(app.streaming_text.is_empty(), "{cancel_key:?}");
        assert!(text_of(&draw(&app, 80, 24).0).contains("cancelling"));

        // Once settled, the next turn streams as usual.
        app.settle_cancellation();
        apply(&mut app, delta("next turn"));
        assert!(app.is_streaming, "{cancel_key:?}");
        assert_eq!(app.streaming_text, "next turn", "{cancel_key:?}");
    }

    // Ctrl+C reaches the exit sequence while the backlog is still arriving
    // instead of cancelling the same turn again on every press.
    let mut app = next_app();
    app.push_message(Message::user("run it".to_string()));
    apply(&mut app, delta("delivered in time"));
    app.handle_key_event(interrupt);
    assert!(app.take_cancellation_request());
    for press in 0..2 {
        assert!(!app.should_exit, "exited before press {press}");
        apply(&mut app, delta("too late"));
        app.handle_key_event(interrupt);
    }
    assert!(app.should_exit);

    // Classic keeps its existing handling of late deltas.
    let mut classic = next_app();
    classic.renderer = UiRenderer::Classic;
    classic.push_message(Message::user("run it".to_string()));
    apply(&mut classic, delta("delivered in time"));
    classic.handle_key_event(esc());
    assert!(!classic.is_streaming);
    apply(&mut classic, delta("too late"));
    assert!(classic.is_streaming);
    assert_eq!(classic.streaming_text, "too late");
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
fn views_name_their_limits_instead_of_faking_integrations() {
    let mut app = next_app();
    app.next_shell.inspector = Some(InspectorPage::Views);
    let text = text_of(&draw(&app, 80, 24).0);
    // Work, Research and Memory have limited pages; see next_work.rs,
    // next_research.rs and next_memory.rs. None claims to be complete.
    for (name, issue) in [("Work", "#413"), ("Research", "#414"), ("Memory", "#415")] {
        let line = text
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{name} ")))
            .unwrap_or_else(|| panic!("{name} missing:\n{text}"));
        assert!(line.contains(issue), "{line}");
        assert!(!line.contains("current"), "{line}");
    }
    assert!(
        text.contains("Ctrl+K"),
        "commands stay discoverable:\n{text}"
    );
}

#[test]
fn wide_terminals_show_navigation_only_on_request_and_beside_the_conversation() {
    let mut app = next_app();
    with_history(&mut app, 40);
    app.set_prompt_text("unsent draft".to_string());

    // Every inspector page draws a footer naming its page key, so a frame
    // without it shows no page, whichever one could have opened.
    let footer = "next page";
    for _ in 0..2 {
        let closed = text_of(&draw(&app, 180, 50).0);
        assert!(closed.contains("answer 039"), "{closed}");
        assert!(
            !closed.contains(footer) && !closed.contains("local only"),
            "nothing opens by itself:\n{closed}"
        );
        assert_eq!(app.next_shell.inspector, None);
    }
    let area = Rect::new(0, 0, 180, 50);
    assert!(plan(area, false, 0, 2).inspector.is_none());

    app.handle_key_event(alt('i'));
    let _ = draw(&app, 180, 50);
    app.handle_key_event(key(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.next_shell.inspector, Some(InspectorPage::Views));

    let open = text_of(&draw(&app, 180, 50).0);
    assert!(
        open.contains(footer),
        "the footer marks an open page:\n{open}"
    );
    for destination in ["Chat", "Work", "Research", "Memory", "Ctrl+K"] {
        assert!(open.contains(destination), "{destination}:\n{open}");
    }
    let row = open
        .lines()
        .find(|line| line.contains("answer 039"))
        .unwrap_or_else(|| panic!("the conversation stays beside it:\n{open}"));
    assert!(row.chars().count() > 72, "{row}");
    assert!(open.contains("unsent draft"), "{open}");

    let layout = plan(area, true, 0, 2);
    let (transcript, inspector) = (layout.transcript.unwrap(), layout.inspector.unwrap());
    assert_eq!(transcript.width + 1 + inspector.width, 180);
    assert!(transcript.width >= 72 && inspector.width >= 40);

    // Closing returns to the same conversation and the same draft.
    app.handle_key_event(esc());
    assert_eq!(app.next_shell.inspector, None);
    let back = text_of(&draw(&app, 180, 50).0);
    assert!(back.contains("answer 039"), "{back}");
    assert!(back.contains("unsent draft"), "{back}");
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

const SELECTED: ratatui::style::Color = ratatui::style::Color::Rgb(200, 200, 200);

fn highlighted(buffer: &Buffer) -> String {
    buffer
        .content
        .iter()
        .filter(|cell| cell.bg == SELECTED)
        .map(|cell| cell.symbol())
        .collect()
}

/// Select `needle` the way a drag does, from the cells of the last frame.
fn select(app: &mut App, needle: &str) {
    let area = app.last_selectable_area.get();
    let (row, column) = {
        let rows = app.last_row_text.borrow();
        let (row, text) = rows
            .iter()
            .find(|(_, text)| text.contains(needle))
            .expect("the text is on screen");
        (*row, text.find(needle).unwrap())
    };
    let start = area.x + u16::try_from(column).unwrap();
    let end = start + u16::try_from(needle.len()).unwrap() - 1;
    app.selection_anchor = Some((start, row));
    app.selection_focus = Some((end, row));
}

fn selected_text(app: &App) -> String {
    app.selection_text.borrow().clone()
}

#[test]
fn a_selection_stays_on_its_text_while_nothing_moves() {
    let mut app = next_app();
    with_history(&mut app, 40);
    let _ = draw(&app, 80, 24);
    select(&mut app, "answer 039");
    for _ in 0..3 {
        let (buffer, _) = draw(&app, 80, 24);
        assert_eq!(selected_text(&app), "answer 039");
        assert_eq!(highlighted(&buffer), "answer 039");
        assert!(!text_of(&buffer).contains("selection kept"));
    }
}

#[test]
fn a_resize_keeps_the_selected_text_and_marks_nothing_else() {
    for (width, height) in [(100, 30), (180, 50), (60, 16), (40, 12), (20, 6)] {
        let mut app = next_app();
        with_history(&mut app, 40);
        let _ = draw(&app, 80, 24);
        select(&mut app, "answer 039");
        let _ = draw(&app, 80, 24);

        let (buffer, _) = draw(&app, width, height);
        assert_eq!(
            selected_text(&app),
            "answer 039",
            "{width}x{height}: copy still gives what was selected"
        );
        assert_eq!(highlighted(&buffer), "", "{width}x{height}: no wrong mark");

        // Going back does not guess that the cells mean the same thing again.
        let (buffer, _) = draw(&app, 80, 24);
        assert_eq!(selected_text(&app), "answer 039");
        assert_eq!(highlighted(&buffer), "");
        assert!(
            text_of(&buffer).contains("selection kept"),
            "the kept selection is visible:\n{}",
            text_of(&buffer)
        );
    }
}

#[test]
fn text_moving_under_a_selection_does_not_change_what_it_copies() {
    let mut app = next_app();
    with_history(&mut app, 40);
    let _ = draw(&app, 80, 24);
    select(&mut app, "answer 039");
    let _ = draw(&app, 80, 24);

    apply(&mut app, delta(&"streamed line\n".repeat(30)));
    let (buffer, _) = draw(&app, 80, 24);
    assert_eq!(selected_text(&app), "answer 039");
    assert_eq!(highlighted(&buffer), "");
    assert!(text_of(&buffer).contains("selection kept"));
}

#[test]
fn rows_arriving_elsewhere_leave_a_selection_alone() {
    let mut app = next_app();
    with_history(&mut app, 40);
    app.handle_key_event(key(KeyCode::PageUp, KeyModifiers::NONE));
    let first = text_of(&draw(&app, 80, 24).0);
    app.settle_scroll_anchor();
    let line = first
        .lines()
        .find(|line| line.starts_with("answer "))
        .expect("history visible")
        .trim()
        .to_string();
    select(&mut app, &line);
    let _ = draw(&app, 80, 24);

    apply(&mut app, delta(&"streamed line\n".repeat(30)));
    let (buffer, _) = draw(&app, 80, 24);
    assert_eq!(selected_text(&app), line);
    assert_eq!(highlighted(&buffer), line);
}

#[test]
fn a_new_selection_replaces_a_kept_one() {
    let mut app = next_app();
    with_history(&mut app, 40);
    let _ = draw(&app, 80, 24);
    select(&mut app, "answer 039");
    let _ = draw(&app, 80, 24);
    let _ = draw(&app, 60, 16);
    assert_eq!(selected_text(&app), "answer 039");

    select(&mut app, "answer 039");
    let (buffer, _) = draw(&app, 60, 16);
    assert_eq!(selected_text(&app), "answer 039");
    assert_eq!(highlighted(&buffer), "answer 039");
    assert!(!text_of(&buffer).contains("selection kept"));
}

#[test]
fn any_other_key_lets_go_of_a_kept_selection() {
    let mut app = next_app();
    with_history(&mut app, 40);
    let _ = draw(&app, 80, 24);
    select(&mut app, "answer 039");
    let _ = draw(&app, 80, 24);
    let _ = draw(&app, 60, 16);

    app.handle_key_event(key(KeyCode::Char('x'), KeyModifiers::NONE));
    let (buffer, _) = draw(&app, 60, 16);
    assert_eq!(selected_text(&app), "");
    assert_eq!(highlighted(&buffer), "");
    assert!(!text_of(&buffer).contains("selection kept"));
}

#[test]
fn pasting_or_fast_typing_lets_go_of_a_selection_only_in_next() {
    let mut app = next_app();
    with_history(&mut app, 40);
    let _ = draw(&app, 80, 24);
    select(&mut app, "answer 039");
    let _ = draw(&app, 80, 24);
    let _ = draw(&app, 60, 16);

    // Fast typing reaches the app as a paste, not as keys.
    app.handle_paste_data("q7z".to_string());
    let (buffer, _) = draw(&app, 60, 16);
    assert_eq!(selected_text(&app), "");
    assert!(!text_of(&buffer).contains("selection kept"));

    let mut classic = next_app();
    classic.renderer = UiRenderer::Classic;
    classic.selection_anchor = Some((1, 1));
    classic.selection_focus = Some((4, 1));
    classic.handle_paste_data("q7z".to_string());
    assert_eq!(classic.selection_anchor, Some((1, 1)));
}

#[test]
fn a_bound_key_keeps_a_selection_so_scrolling_can_come_before_copying() {
    let mut app = next_app();
    with_history(&mut app, 40);
    let _ = draw(&app, 80, 24);
    select(&mut app, "answer 039");
    let _ = draw(&app, 80, 24);

    app.handle_key_event(key(KeyCode::PageUp, KeyModifiers::NONE));
    let (buffer, _) = draw(&app, 80, 24);
    assert_eq!(selected_text(&app), "answer 039");
    assert_eq!(highlighted(&buffer), "");
    assert!(text_of(&buffer).contains("selection kept"));
}

#[test]
fn escape_still_cancels_a_turn_while_a_selection_is_kept() {
    let mut app = next_app();
    with_history(&mut app, 40);
    let _ = draw(&app, 80, 24);
    select(&mut app, "answer 039");
    let _ = draw(&app, 80, 24);
    app.is_streaming = true;
    let _ = draw(&app, 60, 16);

    app.handle_key_event(esc());
    assert_ne!(app.cancellation, TurnCancellation::Idle);
}

#[test]
fn a_kept_selection_does_not_offer_ctrl_c_which_still_interrupts() {
    let mut app = next_app();
    with_history(&mut app, 40);
    let _ = draw(&app, 80, 24);
    select(&mut app, "answer 039");
    let _ = draw(&app, 80, 24);
    app.is_streaming = true;
    let (buffer, _) = draw(&app, 60, 16);
    let text = text_of(&buffer);
    assert!(text.contains("selection kept"), "{text}");
    assert!(
        !text.contains("Ctrl+C"),
        "the hint must not offer a key that cancels the turn:\n{text}"
    );

    // Ctrl+Shift+C, where the terminal reports it in either form, is not the
    // interrupt key.
    for c in ['c', 'C'] {
        assert_eq!(
            app.handle_control_key(key(
                KeyCode::Char(c),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            None,
            "{c}"
        );
        assert_eq!(app.cancellation, TurnCancellation::Idle);
    }

    // The CLI routes Ctrl+C here before any other key handling.
    assert_eq!(
        app.handle_control_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Some(true),
        "Ctrl+C interrupts"
    );
    assert_ne!(app.cancellation, TurnCancellation::Idle);
}

fn row_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|line| line.contains(needle))
}

#[test]
fn the_reading_row_stays_fixed_while_rows_arrive_and_disappear_below() {
    let mut app = next_app();
    with_history(&mut app, 40);
    app.handle_key_event(key(KeyCode::PageUp, KeyModifiers::NONE));
    let first = text_of(&draw(&app, 80, 24).0);
    app.settle_scroll_anchor();
    let anchor = first
        .lines()
        .find(|line| line.starts_with("answer "))
        .expect("history visible")
        .to_string();
    let at = row_of(&first, &anchor);

    apply(&mut app, delta(&"streamed line\n".repeat(30)));
    let grown = text_of(&draw(&app, 80, 24).0);
    app.settle_scroll_anchor();
    assert_eq!(
        row_of(&grown, &anchor),
        at,
        "growth moved the reader:\n{grown}"
    );

    // Cancelling clears the live text: rows vanish below the reader.
    app.streaming_text.clear();
    let shrunk = text_of(&draw(&app, 80, 24).0);
    app.settle_scroll_anchor();
    assert_eq!(
        row_of(&shrunk, &anchor),
        at,
        "shrinking moved the reader:\n{shrunk}"
    );
}

#[test]
fn rewrapping_keeps_the_reader_near_their_text_and_over_scroll_is_clamped() {
    let mut app = next_app();
    with_history(&mut app, 60);
    app.scroll_offset = 40;
    app.auto_scroll = false;
    let wide = text_of(&draw(&app, 120, 40).0);
    app.settle_scroll_anchor();
    let anchor = wide
        .lines()
        .find(|line| line.starts_with("answer "))
        .expect("history")
        .to_string();
    // Opening the inspector in Split narrows the transcript; closing it
    // widens it again. The reader stays on the same stretch of history.
    app.handle_key_event(alt('i'));
    let narrow = text_of(&draw(&app, 120, 40).0);
    app.settle_scroll_anchor();
    assert!(narrow.contains(&anchor), "{narrow}");
    app.handle_key_event(alt('i'));
    let back = text_of(&draw(&app, 120, 40).0);
    app.settle_scroll_anchor();
    assert!(back.contains(&anchor), "{back}");

    // An offset past the top is clamped, so the next step down moves.
    app.scroll_offset = 100_000;
    let top = text_of(&draw(&app, 80, 24).0);
    app.settle_scroll_anchor();
    assert!(
        app.scroll_offset < 100_000,
        "over-scroll settled to the range"
    );
    app.handle_key_event(key(KeyCode::PageDown, KeyModifiers::NONE));
    let moved = text_of(&draw(&app, 80, 24).0);
    assert_ne!(
        top, moved,
        "the first PageDown after over-scroll takes effect"
    );
}

#[test]
fn the_cursor_row_stays_visible_and_exact_in_long_wrapped_and_wide_drafts() {
    let mut app = next_app();
    let lines: Vec<String> = (0..12).map(|n| format!("line-{n:02}")).collect();
    app.set_prompt_text(lines.join("\n"));
    // Cursor at the end of line-02.
    app.prompt_input.cursor = lines[..3].iter().map(|l| l.len() + 1).sum::<usize>() - 1;
    let (buffer, cursor) = draw(&app, 80, 24);
    let (x, y) = cursor.expect("cursor visible in a draft taller than the composer");
    let row = text_of(&buffer)
        .lines()
        .nth(usize::from(y))
        .unwrap_or_default()
        .to_string();
    assert!(
        row.contains("line-02"),
        "cursor row is the edited line: {row}"
    );
    assert_eq!(x, 2 + 7);

    // A wide character that would cross the edge wraps early; the cursor
    // on the next logical line must account for that extra row.
    app.set_prompt_text(format!("a{}b\nxyz", "漢".repeat(78)));
    let (buffer, cursor) = draw(&app, 80, 24);
    let (x, y) = cursor.expect("cursor");
    let row = text_of(&buffer)
        .lines()
        .nth(usize::from(y))
        .unwrap_or_default()
        .to_string();
    assert!(row.contains("xyz"), "cursor row: {row}");
    assert_eq!(x, 2 + 3);
}

#[test]
fn a_settled_cancellation_reports_its_outcome_and_remote_cancels_wait_too() {
    let mut app = next_app();
    apply(&mut app, delta("working"));
    app.handle_key_event(esc());
    assert!(app.take_cancellation_request());
    // The loop clears the status when the worker finishes, then settles.
    app.status_message = None;
    app.settle_cancellation();
    let text = text_of(&draw(&app, 80, 24).0);
    assert!(text.contains("Cancelled."), "{text}");

    let mut remote = next_app();
    apply(&mut remote, delta("working"));
    remote.note_cancellation_sent();
    let text = text_of(&draw(&remote, 80, 24).0);
    assert!(text.contains("cancelling"), "{text}");
}

#[test]
fn inspector_scrolling_is_bounded_and_voice_recording_is_visible() {
    let mut app = next_app();
    app.handle_key_event(alt('i'));
    let _ = draw(&app, 120, 40);
    for _ in 0..10 {
        app.handle_key_event(key(KeyCode::PageDown, KeyModifiers::NONE));
    }
    assert_eq!(
        app.next_shell.inspector_scroll, 0,
        "a short page does not scroll"
    );

    let mut voice = next_app();
    voice.voice_recording = true;
    let text = text_of(&draw(&voice, 80, 24).0);
    assert!(text.contains("recording voice"), "{text}");
}

#[test]
fn switching_sessions_drops_work_read_for_the_old_one() {
    let mut app = next_app();
    app.enter_session_scope("session-a");
    app.work_sources.goal = Some(Err("read for session-a".into()));
    app.enter_session_scope("session-a");
    assert!(app.work_sources.goal.is_some(), "same session keeps it");
    app.enter_session_scope("session-b");
    assert!(app.work_sources.goal.is_none());
}
