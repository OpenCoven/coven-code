//! Classic/Next parity over the shared presentation seam.
//!
//! One synthetic runtime trace is replayed through the shared reducer into
//! one App per renderer. Both must reach identical semantic state, and both
//! renderers must show the semantic content of that state.

use claurst_api::streaming::ContentDelta;
use claurst_api::AnthropicStreamEvent;
use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::types::Message;
use claurst_query::QueryEvent;
use claurst_tui::app::App;
use claurst_tui::application::view::TranscriptView;
use claurst_tui::application::{PresentationEffect, ScopedQueryEvent, SessionScope, UiRenderer};
use claurst_tui::dialogs::{handle_permission_key, PermissionRequest};
use claurst_tui::NotificationKind;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn fixture_app() -> App {
    let config = Config {
        model: Some("claude-baseline-model".to_string()),
        provider: Some("anthropic".to_string()),
        ..Config::default()
    };
    let mut app = App::new(config, CostTracker::new());
    app.has_credentials = true;
    app
}

fn trace() -> Vec<QueryEvent> {
    let delta = |text: &str| {
        QueryEvent::Stream(AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta {
                text: text.to_string(),
            },
        })
    };
    vec![
        delta("Reading the contract "),
        QueryEvent::ToolStart {
            tool_name: "Read".to_string(),
            tool_id: "toolu-parity-1".to_string(),
            input_json: r#"{"file_path":"ENGINE-CONTRACT.md"}"#.to_string(),
        },
        QueryEvent::ToolEnd {
            tool_name: "Read".to_string(),
            tool_id: "toolu-parity-1".to_string(),
            result: "contract text".to_string(),
            is_error: false,
        },
        delta("now."),
        QueryEvent::Status("Thinking about the reply".to_string()),
    ]
}

/// Replay the trace into a fresh App; returns the App and all effects.
fn replay(events: Vec<QueryEvent>) -> (App, Vec<PresentationEffect>) {
    let mut app = fixture_app();
    app.push_message(Message::user("Summarize the engine contract.".to_string()));
    app.turn_start = None;
    let scope = SessionScope::default();
    let mut effects = Vec::new();
    for event in events {
        effects.extend(scope.apply(
            &mut app,
            ScopedQueryEvent {
                generation: scope.current(),
                event,
            },
        ));
    }
    (app, effects)
}

fn draw(app: &App, renderer: UiRenderer, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| match renderer {
            UiRenderer::Classic => claurst_tui::render::render_app(frame, app),
            UiRenderer::Next => claurst_tui::next::render_next(frame, app),
        })
        .expect("draw frame");
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn replay_through_the_shared_reducer_is_deterministic() {
    let (classic, classic_effects) = replay(trace());
    let (next, next_effects) = replay(trace());
    assert_eq!(classic_effects, next_effects);
    let projected = TranscriptView::project(&classic);
    assert_eq!(projected, TranscriptView::project(&next));
    assert_eq!(projected.live_text, "Reading the contract now.");
    assert_eq!(projected.tools.len(), 1);
    assert_eq!(
        projected.status.as_deref(),
        Some("Thinking about the reply")
    );
}

#[test]
fn both_renderers_show_the_projected_transcript_and_live_output() {
    let (app, _) = replay(trace());
    for renderer in [UiRenderer::Classic, UiRenderer::Next] {
        let frame = draw(&app, renderer, 120, 40);
        assert!(
            frame.contains("Summarize the engine contract."),
            "{renderer:?} lost the user turn:\n{frame}"
        );
        assert!(
            frame.contains("Reading the contract now."),
            "{renderer:?} lost live output:\n{frame}"
        );
    }
    let next = draw(&app, UiRenderer::Next, 120, 40);
    assert!(next.contains(claurst_tui::next::PREVIEW_LABEL), "{next}");
    assert!(next.contains("▸ Read ○ done"), "{next}");
}

fn bash_request(id: &str) -> PermissionRequest {
    PermissionRequest::bash(
        id.to_string(),
        "Bash".to_string(),
        "Runs a shell command in the project".to_string(),
        "cargo test --workspace".to_string(),
        Some("cargo test".to_string()),
    )
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// What the interactive loop would submit or decide after a key sequence.
#[derive(Debug, PartialEq, Eq)]
struct Outgoing {
    submitted: String,
    permission: Option<(String, usize, bool)>,
    view: TranscriptView,
}

/// Drive one App through typing, a permission decision and a second draft,
/// drawing with `renderer` between every key the way the interactive loop
/// does, and record the operations the loop would send.
fn drive(renderer: UiRenderer) -> Outgoing {
    let (mut app, _) = replay(trace());
    for ch in "run the tests".chars() {
        draw(&app, renderer, 120, 40);
        app.handle_key_event(key(KeyCode::Char(ch)));
    }
    draw(&app, renderer, 120, 40);
    let submitted = app.take_input();

    app.permission_request = Some(bash_request("toolu-parity-2"));
    let mut confirmed = false;
    for code in [KeyCode::Down, KeyCode::Down, KeyCode::Up, KeyCode::Enter] {
        draw(&app, renderer, 120, 40);
        let request = app.permission_request.as_mut().expect("pending request");
        confirmed = handle_permission_key(request, key(code));
    }
    let permission = app
        .permission_request
        .take()
        .map(|request| (request.tool_use_id, request.selected_option, confirmed));

    for ch in "next draft".chars() {
        draw(&app, renderer, 120, 40);
        app.handle_key_event(key(KeyCode::Char(ch)));
    }
    draw(&app, renderer, 120, 40);
    Outgoing {
        submitted,
        permission,
        view: TranscriptView::project(&app),
    }
}

#[test]
fn renderers_produce_identical_outgoing_operations_for_the_same_input() {
    let classic = drive(UiRenderer::Classic);
    let next = drive(UiRenderer::Next);
    assert_eq!(classic, next);
    assert_eq!(classic.submitted, "run the tests");
    assert_eq!(
        classic.permission,
        Some(("toolu-parity-2".to_string(), 1, true))
    );
    assert_eq!(classic.view.draft, "next draft");
}

#[test]
fn next_draws_permission_requests_through_classic() {
    let (mut app, _) = replay(trace());
    app.permission_request = Some(bash_request("toolu-parity-3"));
    assert!(claurst_tui::next::delegates_to_classic(&app));
    let next = draw(&app, UiRenderer::Next, 120, 40);
    assert_eq!(next, draw(&app, UiRenderer::Classic, 120, 40));
    let request = app.permission_request.as_ref().expect("request");
    for option in &request.options {
        assert!(
            next.contains(&option.label),
            "next lacks {:?}:\n{next}",
            option.label
        );
    }
}

/// Opens one input-owning surface on an App.
type OpenSurface = fn(&mut App);

#[test]
fn next_hands_every_input_owning_surface_to_classic() {
    let cases: Vec<(&str, OpenSurface)> = vec![
        ("help overlay", |app| app.help_overlay.visible = true),
        ("familiar switcher", |app| app.familiar_switcher_open = true),
        ("effort picker", |app| app.effort_picker.visible = true),
        ("response reader", |app| app.response_reader.visible = true),
        ("error modal", |app| {
            app.push_notification(
                NotificationKind::Error,
                "synthetic failure".to_string(),
                None,
            )
        }),
    ];
    for (name, open) in cases {
        let (mut app, _) = replay(trace());
        assert!(
            !claurst_tui::next::delegates_to_classic(&app),
            "{name}: plain shell"
        );
        open(&mut app);
        assert!(claurst_tui::next::delegates_to_classic(&app), "{name}");
        assert_eq!(
            draw(&app, UiRenderer::Next, 120, 40),
            draw(&app, UiRenderer::Classic, 120, 40),
            "{name}"
        );
    }
}

#[test]
fn next_clears_classic_mouse_geometry() {
    let (app, _) = replay(trace());
    draw(&app, UiRenderer::Classic, 120, 40);
    assert!(
        app.last_input_area.get().area() > 0,
        "classic records geometry"
    );
    draw(&app, UiRenderer::Next, 120, 40);
    assert!(app.message_row_map.borrow().is_empty());
    assert!(app.last_row_text.borrow().is_empty());
    assert_eq!(app.last_input_area.get().area(), 0);
    assert_eq!(app.last_selectable_area.get().area(), 0);
    assert_eq!(app.last_msg_area.get().area(), 0);
}

#[test]
fn next_keeps_newest_output_visible_at_small_sizes_without_panicking() {
    let mut long = trace();
    long.push(QueryEvent::Stream(
        AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta {
                text: "\n".to_string() + &"wide 漢字 text ".repeat(40) + "\nnewest line",
            },
        },
    ));
    let (app, _) = replay(long);
    for (width, height) in [(40, 12), (80, 24), (1, 4)] {
        let frame = draw(&app, UiRenderer::Next, width, height);
        if width >= 40 {
            assert!(frame.contains("newest line"), "{width}x{height}:\n{frame}");
        }
    }
}
