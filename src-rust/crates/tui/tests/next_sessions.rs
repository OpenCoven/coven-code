//! T5 (#412), safe navigation slice: per-session drafts and reading
//! position, browser resume, stale-scope isolation and honest ownership.
//!
//! No producer binds familiar identity or daemon control yet, so these
//! fixtures check that the shell never claims either.

use claurst_api::streaming::ContentDelta;
use claurst_api::AnthropicStreamEvent;
use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::types::Message;
use claurst_query::QueryEvent;
use claurst_tui::app::App;
use claurst_tui::application::{ScopedQueryEvent, SessionOwnership, SessionScope, UiRenderer};
use claurst_tui::next::style::{Appearance, ColorMode, Glyphs};
use claurst_tui::next::{InspectorPage, NextPresentation};
use claurst_tui::session_browser::SessionEntry;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn next_app() -> App {
    let config = Config {
        model: Some("claude-session-model".to_string()),
        project_dir: Some("/work/acme-api".into()),
        familiar: Some("nyx".to_string()),
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
    app.enter_session_scope("session-a");
    app
}

fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| claurst_tui::next::render_next(frame, app))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let width = usize::from(buffer.area.width.max(1));
    buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn type_text(app: &mut App, text: &str) {
    for ch in text.chars() {
        app.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

fn entry(id: &str) -> SessionEntry {
    SessionEntry {
        id: id.to_string(),
        title: format!("title of {id}"),
        last_updated: "now".to_string(),
        message_count: 1,
        cost_usd: 0.0,
    }
}

#[test]
fn drafts_and_reading_position_are_kept_per_session() {
    let mut app = next_app();
    type_text(&mut app, "half-written plan");
    app.auto_scroll = false;
    app.scroll_offset = 5;

    app.enter_session_scope("session-b");
    assert_eq!(app.prompt_input.text, "", "a draft must not follow into B");
    assert!(app.auto_scroll);
    assert_eq!(app.scroll_offset, 0);
    type_text(&mut app, "b draft");

    app.enter_session_scope("session-a");
    assert_eq!(app.prompt_input.text, "half-written plan");
    assert!(!app.auto_scroll);
    assert_eq!(app.scroll_offset, 5);
    // Restoring a draft shows it; it is never sent.
    assert!(!app.pending_auto_submit);

    app.enter_session_scope("session-b");
    assert_eq!(app.prompt_input.text, "b draft");
}

#[test]
fn the_session_browser_resumes_the_selected_session_and_keeps_the_draft() {
    let mut app = next_app();
    type_text(&mut app, "unsent idea");
    app.session_browser
        .open(vec![entry("session-a"), entry("session-b")]);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);

    assert!(!app.session_browser.visible);
    assert_eq!(app.take_pending_resume().as_deref(), Some("session-b"));
    // The loop resumes through `/session resume`; the draft is kept for A.
    app.enter_session_scope("session-b");
    assert_eq!(app.prompt_input.text, "");
    app.enter_session_scope("session-a");
    assert_eq!(app.prompt_input.text, "unsent idea");
}

#[test]
fn a_failed_browser_resume_gives_the_draft_back() {
    let mut app = next_app();
    type_text(&mut app, "keep me");
    app.session_browser.open(vec![entry("session-gone")]);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_pending_resume().as_deref(), Some("session-gone"));
    // `/session resume` failed; the loop settles without a session switch.
    app.settle_pending_resume();
    assert_eq!(app.session_id.as_deref(), Some("session-a"));
    assert_eq!(app.prompt_input.text, "keep me");
}

#[test]
fn the_browser_refuses_to_resume_during_a_turn_or_into_the_same_session() {
    let mut app = next_app();
    type_text(&mut app, "draft");
    app.session_browser.open(vec![entry("session-a")]);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_pending_resume(), None);
    assert_eq!(app.prompt_input.text, "draft");

    app.is_streaming = true;
    app.session_browser.open(vec![entry("session-b")]);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_pending_resume(), None);
    assert_eq!(app.prompt_input.text, "draft");
}

#[test]
fn switching_sessions_drops_views_that_belonged_to_the_old_one() {
    let mut app = next_app();
    let scope = SessionScope::default();
    scope.apply(
        &mut app,
        ScopedQueryEvent {
            generation: scope.current(),
            event: QueryEvent::ToolStart {
                tool_name: "Read".to_string(),
                tool_id: "toolu-a".to_string(),
                input_json: r#"{"file_path":"secret-a.md"}"#.to_string(),
            },
        },
    );
    app.record_permission_decision("toolu-a", true);
    app.selection_anchor = Some((1, 1));
    app.selection_focus = Some((5, 1));
    app.next_shell.inspector = Some(InspectorPage::Activity);

    app.enter_session_scope("session-b");
    assert!(app.tool_use_blocks.is_empty());
    assert!(app.permission_decisions.is_empty());
    assert_eq!(app.selection_anchor, None);
    let frame = draw(&app, 120, 40);
    assert!(!frame.contains("secret-a.md"), "{frame}");
}

#[test]
fn late_events_from_the_old_session_cannot_change_the_new_view() {
    let mut app = next_app();
    let mut scope = SessionScope::default();
    let old = scope.current();
    app.push_message(Message::user("question for A".to_string()));

    // Resume: the loop advances the scope, then binds the new session.
    scope.advance();
    app.enter_session_scope("session-b");
    app.replace_messages(Vec::new());

    for event in [
        QueryEvent::Stream(AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta {
                text: "late answer for A".to_string(),
            },
        }),
        QueryEvent::ToolStart {
            tool_name: "Bash".to_string(),
            tool_id: "toolu-late".to_string(),
            input_json: r#"{"command":"ls"}"#.to_string(),
        },
        QueryEvent::Status("A is thinking".to_string()),
    ] {
        scope.apply(
            &mut app,
            ScopedQueryEvent {
                generation: old,
                event,
            },
        );
    }
    assert_eq!(app.streaming_text, "");
    assert!(app.tool_use_blocks.is_empty());
    let frame = draw(&app, 120, 40);
    assert!(!frame.contains("late answer for A"), "{frame}");
    assert!(!frame.contains("A is thinking"), "{frame}");
}

#[test]
fn ownership_is_engine_local_and_a_registration_covers_only_its_session() {
    let mut app = next_app();
    let header = |app: &App| draw(app, 160, 30).lines().next().unwrap_or("").to_string();

    app.ownership = SessionOwnership::EngineLocal;
    assert!(header(&app).contains("engine-local"), "{}", header(&app));

    app.ownership = SessionOwnership::LedgerPending {
        session_id: "session-a".to_string(),
    };
    assert!(header(&app).contains("engine-local, registering"));

    app.ownership = SessionOwnership::LedgerRegistered {
        session_id: "session-a".to_string(),
    };
    assert!(header(&app).contains("engine-local, ledger-registered"));

    app.ownership = SessionOwnership::LedgerFailed {
        session_id: "session-a".to_string(),
    };
    assert!(header(&app).contains("engine-local, ledger unavailable"));

    // The registration was for A; after resuming B it says nothing about B.
    app.ownership = SessionOwnership::LedgerRegistered {
        session_id: "session-a".to_string(),
    };
    app.enter_session_scope("session-b");
    let shown = header(&app);
    assert!(shown.contains("engine-local"), "{shown}");
    assert!(!shown.contains("ledger-registered"), "{shown}");

    // Nothing ever claims daemon control.
    app.next_shell.inspector = Some(InspectorPage::Session);
    let frame = draw(&app, 160, 40);
    assert!(!frame.contains("daemon-managed"), "{frame}");
    assert!(frame.contains("different session"), "{frame}");
}

#[test]
fn a_familiar_name_is_never_presented_as_verified_identity() {
    let mut app = next_app();
    app.next_shell.inspector = Some(InspectorPage::Session);
    let frame = draw(&app, 160, 40);
    assert!(frame.contains("nyx (unverified)"), "{frame}");
    assert!(frame.contains("display name only"), "{frame}");
    // The same display name in another session is still only a name.
    app.enter_session_scope("session-b");
    let frame = draw(&app, 160, 40);
    assert!(frame.contains("nyx (unverified)"), "{frame}");
}

#[test]
fn a_hostile_session_title_cannot_reach_the_terminal() {
    let mut app = next_app();
    app.session_title = Some("plan\u{1b}]52;c;ZXZpbA==\u{7}".to_string());
    let frame = draw(&app, 160, 30);
    assert!(!frame.contains('\u{1b}'));
    assert!(frame.contains("plan^[]52"), "{frame}");
}

#[test]
fn a_kept_draft_keeps_its_pasted_blocks() {
    let mut app = next_app();
    let log: String = (1..=40).map(|n| format!("log line {n}\n")).collect();
    app.handle_paste_data(log.clone());
    let placeholder = app.prompt_input.text.clone();
    assert!(placeholder.contains("Pasted"), "{placeholder}");

    app.session_browser.open(vec![entry("session-b")]);
    press(&mut app, KeyCode::Enter);
    assert!(app.take_pending_resume().is_some());
    // The loop submits `/session resume`, which clears the composer.
    app.set_prompt_text("/session resume session-b".to_string());
    let _ = app.take_input();
    app.enter_session_scope("session-b");
    app.enter_session_scope("session-a");

    assert_eq!(app.prompt_input.text, placeholder);
    let expanded = app.take_input();
    assert!(expanded.contains("log line 40"), "{expanded}");
    // The synthetic resume is not left in prompt history.
    assert!(!app
        .prompt_input
        .history
        .iter()
        .any(|entry| entry.starts_with("/session resume")));
}

#[test]
fn the_browser_refuses_to_resume_until_a_cancelled_turn_settles() {
    let mut app = next_app();
    app.cancellation = claurst_tui::application::TurnCancellation::AwaitingRuntime;
    type_text(&mut app, "draft");
    app.session_browser.open(vec![entry("session-b")]);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_pending_resume(), None);
    assert_eq!(app.prompt_input.text, "draft");
}

#[test]
fn renaming_another_session_does_not_retitle_the_shown_one() {
    let mut app = next_app();
    app.session_title = Some("alpha".to_string());
    app.session_browser.open(vec![entry("session-b")]);
    press(&mut app, KeyCode::Char('r'));
    type_text(&mut app, "-renamed");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.session_title.as_deref(), Some("alpha"));
}

#[test]
fn notes_and_turn_metadata_do_not_follow_into_another_session() {
    let mut app = next_app();
    app.push_message(Message::user("question for A".to_string()));
    app.push_system_message(
        "note only about A".to_string(),
        claurst_tui::app::SystemMessageStyle::Info,
    );
    app.enter_session_scope("session-b");
    app.replace_messages(vec![Message::user("question for B".to_string())]);
    assert!(app.system_annotations.is_empty());
    let frame = draw(&app, 120, 40);
    assert!(!frame.contains("note only about A"), "{frame}");
}

#[test]
fn classic_session_behavior_is_unchanged() {
    let mut app = next_app();
    app.renderer = UiRenderer::Classic;
    app.push_system_message(
        "note".to_string(),
        claurst_tui::app::SystemMessageStyle::Info,
    );
    type_text(&mut app, "draft");
    app.session_browser.open(vec![entry("session-b")]);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_pending_resume(),
        None,
        "Classic Enter does not resume"
    );
    assert!(app.session_browser.visible);
    press(&mut app, KeyCode::Esc);

    app.enter_session_scope("session-b");
    assert_eq!(app.prompt_input.text, "draft", "Classic keeps its composer");
    assert!(
        !app.system_annotations.is_empty(),
        "Classic keeps its notes"
    );
}
