//! T4 (#411), engine-local slice: tool rows, tool detail, approval safety and
//! hostile text.
//!
//! These fixtures exercise the engine-local permission path only. Protected
//! (daemon-enforced) approvals have no qualified producer contract yet and
//! must render as unavailable, never as a local grant.

use claurst_api::streaming::ContentDelta;
use claurst_api::AnthropicStreamEvent;
use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::types::Message;
use claurst_query::QueryEvent;
use claurst_tui::app::App;
use claurst_tui::application::{ScopedQueryEvent, SessionScope, UiRenderer};
use claurst_tui::dialogs::{handle_permission_key_at, PermissionRequest, PERMISSION_ARM_DELAY};
use claurst_tui::next::style::{Appearance, ColorMode, Glyphs};
use claurst_tui::next::{InspectorPage, NextPresentation};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use std::time::Duration;

fn next_app() -> App {
    let config = Config {
        model: Some("claude-tools-model".to_string()),
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

fn tool_start(app: &mut App, id: &str, name: &str, input: &str) {
    apply(
        app,
        QueryEvent::ToolStart {
            tool_name: name.to_string(),
            tool_id: id.to_string(),
            input_json: input.to_string(),
        },
    );
}

fn tool_end(app: &mut App, id: &str, name: &str, result: &str, is_error: bool) {
    apply(
        app,
        QueryEvent::ToolEnd {
            tool_name: name.to_string(),
            tool_id: id.to_string(),
            result: result.to_string(),
            is_error,
        },
    );
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn bash_request() -> PermissionRequest {
    PermissionRequest::bash(
        "toolu-rm".to_string(),
        "Bash".to_string(),
        "Delete the build directory".to_string(),
        "rm -rf build".to_string(),
        None,
    )
}

fn deny_index(request: &PermissionRequest) -> usize {
    request
        .options
        .iter()
        .position(|option| option.key == 'n')
        .expect("every request has a deny option")
}

#[test]
fn a_new_permission_request_defaults_to_deny_and_ignores_keys_until_armed() {
    let mut request = bash_request();
    assert_eq!(
        request.selected_option,
        deny_index(&request),
        "default focus is deny"
    );
    let shown = request.quiet_since;

    // Keys typed for the composer that land as the prompt appears decide nothing.
    for early in [
        key(KeyCode::Enter),
        key(KeyCode::Char('y')),
        key(KeyCode::Char('1')),
    ] {
        assert!(
            !handle_permission_key_at(&mut request, early, shown + Duration::from_millis(50)),
            "{early:?} decided an unarmed request"
        );
        assert_eq!(request.selected_option, deny_index(&request));
    }

    // Each ignored key restarts the wait.
    let armed = shown + Duration::from_millis(50) + PERMISSION_ARM_DELAY;
    // Enter on the default choice denies.
    let mut enter = request.clone();
    assert!(handle_permission_key_at(
        &mut enter,
        key(KeyCode::Enter),
        armed
    ));
    assert_eq!(enter.selected_option, deny_index(&enter));
    // An explicit allow still works once armed.
    assert!(handle_permission_key_at(
        &mut request,
        key(KeyCode::Char('y')),
        armed
    ));
    assert_eq!(request.options[request.selected_option].key, 'y');
}

#[test]
fn continuous_typing_or_a_held_key_never_decides_a_request() {
    // A user still typing, or key auto-repeat, keeps the request unarmed:
    // a decision needs a quiet gap after the last ignored key.
    for held in [KeyCode::Char('y'), KeyCode::Enter, KeyCode::Char('p')] {
        let mut request = bash_request();
        let shown = request.quiet_since;
        for step in 1..=40u64 {
            let at = shown + Duration::from_millis(step * 50);
            assert!(
                !handle_permission_key_at(&mut request, key(held), at),
                "{held:?} decided the request {}ms in",
                step * 50
            );
        }
        assert_eq!(request.selected_option, deny_index(&request));
        let quiet = shown + Duration::from_millis(40 * 50) + PERMISSION_ARM_DELAY;
        assert!(handle_permission_key_at(
            &mut request,
            key(KeyCode::Esc),
            quiet
        ));
    }
}

#[test]
fn escape_denies_even_before_the_request_is_armed() {
    let mut request = bash_request();
    request.selected_option = 0;
    let shown = request.quiet_since;
    assert!(handle_permission_key_at(
        &mut request,
        key(KeyCode::Esc),
        shown + Duration::from_millis(10)
    ));
    assert_eq!(request.selected_option, deny_index(&request));
}

#[test]
fn hostile_control_sequences_never_reach_the_terminal() {
    let mut app = next_app();
    let osc52 = "\u{1b}]52;c;ZXZpbA==\u{7}";
    let clear = "\u{1b}[2J\u{1b}[H";
    let link = "\u{1b}]8;;https://evil.example\u{1b}\\click\u{1b}]8;;\u{1b}\\";
    app.push_message(Message::assistant(format!(
        "before {osc52}{clear}{link} after"
    )));
    tool_start(
        &mut app,
        "toolu-x",
        "Bash",
        &format!(r#"{{"command":"echo {clear}"}}"#),
    );
    tool_end(
        &mut app,
        "toolu-x",
        "Bash",
        &format!("out {osc52} done"),
        false,
    );
    for (inspector, width, height) in [
        (None, 80, 24),
        (Some(InspectorPage::Activity), 80, 24),
        (Some(InspectorPage::Activity), 120, 40),
    ] {
        app.next_shell.inspector = inspector;
        let buffer = draw(&app, width, height);
        let bad: Vec<&str> = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .filter(|symbol| symbol.chars().any(char::is_control))
            .collect();
        assert!(bad.is_empty(), "{width}x{height}: control symbols {bad:?}");
        let text = text_of(&buffer);
        if inspector.is_none() || width >= 120 {
            let joined = text.replace('\n', "");
            assert!(
                joined.contains("before") && joined.contains("after"),
                "{text}"
            );
        }
        assert!(
            text.contains("^["),
            "escapes are shown, not executed:\n{text}"
        );
    }
}

#[test]
fn tool_rows_show_a_safe_target_and_an_honest_state() {
    let mut app = next_app();
    app.push_message(Message::user("run the tests".to_string()));
    tool_start(
        &mut app,
        "toolu-1",
        "Bash",
        r#"{"command":"cargo test -p core\nrm -rf /"}"#,
    );
    tool_start(
        &mut app,
        "toolu-2",
        "Read",
        r#"{"file_path":"/work/acme-api/src/lib.rs"}"#,
    );
    let text = text_of(&draw(&app, 100, 30));
    let bash = text.lines().find(|l| l.contains("Bash")).expect("bash row");
    assert!(bash.contains("cargo test -p core"), "{bash}");
    assert!(
        !bash.contains("rm -rf"),
        "only the first line is the target: {bash}"
    );
    assert!(bash.contains("running"), "{bash}");
    let read = text.lines().find(|l| l.contains("Read")).expect("read row");
    assert!(read.contains("src/lib.rs"), "{read}");

    tool_end(&mut app, "toolu-2", "Read", "contents", false);
    tool_end(&mut app, "toolu-1", "Bash", "1 failed", true);
    let text = text_of(&draw(&app, 100, 30));
    assert!(
        text.lines()
            .any(|l| l.contains("Bash") && l.contains("error")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.contains("Read") && l.contains("done")),
        "{text}"
    );
    assert!(
        !text
            .lines()
            .any(|l| l.contains("Read") && l.contains("verified")),
        "completion is not verification:\n{text}"
    );
}

#[test]
fn running_tools_show_cancel_requested_until_the_runtime_settles() {
    let mut app = next_app();
    app.push_message(Message::user("go".to_string()));
    apply(
        &mut app,
        QueryEvent::Stream(AnthropicStreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta {
                text: "working".to_string(),
            },
        }),
    );
    tool_start(&mut app, "toolu-1", "Bash", r#"{"command":"sleep 100"}"#);
    app.handle_key_event(key(KeyCode::Esc));
    assert!(app.take_cancellation_request());
    // Classic clears its tool rows on cancel; Next keeps the evidence.
    let text = text_of(&draw(&app, 100, 30));
    assert!(
        text.lines()
            .any(|l| l.contains("Bash") && l.contains("cancel requested")),
        "{text}"
    );
    app.settle_cancellation();
    let text = text_of(&draw(&app, 100, 30));
    assert!(!text.contains("cancel requested"), "{text}");
}

#[test]
fn the_activity_inspector_shows_sanitized_arguments_output_and_provenance() {
    let mut app = next_app();
    tool_start(
        &mut app,
        "toolu-1",
        "Bash",
        r#"{"command":"ls -la","description":"List files"}"#,
    );
    tool_end(&mut app, "toolu-1", "Bash", "total 0\nREADME.md", false);
    app.next_shell.inspector = Some(InspectorPage::Activity);
    let text = text_of(&draw(&app, 80, 30));
    assert!(text.contains("ls -la"), "{text}");
    assert!(text.contains("README.md"), "output preview shown:\n{text}");
    assert!(
        text.contains("engine-local"),
        "execution owner shown:\n{text}"
    );
}

#[test]
fn next_draws_its_own_approval_with_owner_scope_and_consequences() {
    let mut app = next_app();
    app.set_prompt_text("unsent draft".to_string());
    app.permission_request = Some(bash_request());
    assert!(
        !claurst_tui::next::delegates_to_classic(&app),
        "Next owns the approval surface"
    );
    for (width, height) in [(80, 24), (40, 12), (120, 40)] {
        let text = text_of(&draw(&app, width, height));
        assert!(
            text.contains("rm -rf build"),
            "{width}x{height} exact operation:\n{text}"
        );
        assert!(
            text.contains("engine-local"),
            "{width}x{height} enforcement owner:\n{text}"
        );
        // The selected (default) option is deny and is marked without color.
        assert!(
            text.lines()
                .any(|l| l.starts_with('>') && l.contains("deny")),
            "{width}x{height} deny is focused:\n{text}"
        );
    }
    let text = text_of(&draw(&app, 120, 40));
    assert!(text.contains("acme-api"), "project shown:\n{text}");
    assert!(
        text.contains("persistent"),
        "consequence of always-allow shown:\n{text}"
    );
    assert!(
        text.contains("unsent draft"),
        "draft stays visible:\n{text}"
    );
}

#[test]
fn an_authorization_is_not_shown_as_a_successful_execution() {
    let mut app = next_app();
    tool_start(
        &mut app,
        "toolu-rm",
        "Bash",
        r#"{"command":"rm -rf build"}"#,
    );
    app.record_permission_decision("toolu-rm", true);
    let text = text_of(&draw(&app, 100, 30));
    let row = text.lines().find(|l| l.contains("Bash")).expect("row");
    assert!(row.contains("allowed") && row.contains("running"), "{row}");
    tool_end(
        &mut app,
        "toolu-rm",
        "Bash",
        "permission ok but command failed",
        true,
    );
    let text = text_of(&draw(&app, 100, 30));
    let row = text.lines().find(|l| l.contains("Bash")).expect("row");
    assert!(row.contains("error"), "the execution result wins: {row}");

    let mut denied = next_app();
    tool_start(&mut denied, "toolu-2", "Write", r#"{"file_path":"a.txt"}"#);
    denied.record_permission_decision("toolu-2", false);
    let text = text_of(&draw(&denied, 100, 30));
    assert!(
        text.lines()
            .any(|l| l.contains("Write") && l.contains("denied")),
        "{text}"
    );
}

#[test]
fn a_cancelled_call_closing_result_does_not_add_a_second_failed_row() {
    use claurst_core::types::{ContentBlock, ToolResultContent};
    let mut app = next_app();
    app.push_message(Message::user("go".to_string()));
    let mut asked = Message::assistant_blocks(vec![ContentBlock::ToolUse {
        id: "toolu-1".to_string(),
        name: "Bash".to_string(),
        input: serde_json::json!({"command": "sleep 100"}),
    }]);
    asked.uuid = Some("asked-1".to_string());
    apply(&mut app, QueryEvent::DurableMessage { message: asked });
    tool_start(&mut app, "toolu-1", "Bash", r#"{"command":"sleep 100"}"#);
    app.handle_key_event(key(KeyCode::Esc));
    assert!(app.take_cancellation_request());
    // The engine closes every cancelled tool use with one durable error.
    let mut closing = Message::user_blocks(vec![ContentBlock::ToolResult {
        tool_use_id: "toolu-1".to_string(),
        content: ToolResultContent::Text("cancelled".to_string()),
        is_error: Some(true),
    }]);
    closing.uuid = Some("closing-1".to_string());
    apply(&mut app, QueryEvent::DurableMessage { message: closing });
    app.settle_cancellation();
    let text = text_of(&draw(&app, 100, 30));
    let rows: Vec<&str> = text.lines().filter(|l| l.contains("sleep 100")).collect();
    assert_eq!(rows.len(), 1, "{text}");
    assert!(rows[0].contains("cancelled"), "{text}");
}

fn multiline_request(lines: usize) -> PermissionRequest {
    let mut command: Vec<String> = (1..lines).map(|n| format!("echo step {n}")).collect();
    command.push("rm -rf ~/precious".to_string());
    PermissionRequest::bash(
        "toolu-multi".to_string(),
        "Bash".to_string(),
        "Run a script".to_string(),
        command.join("\n"),
        None,
    )
}

#[test]
fn the_approval_shows_the_whole_command_or_says_what_is_hidden() {
    // Room for everything: every line, including the last, is visible.
    let mut app = next_app();
    app.permission_request = Some(multiline_request(6));
    let text = text_of(&draw(&app, 120, 40));
    assert!(text.contains("echo step 3"), "{text}");
    assert!(text.contains("rm -rf ~/precious"), "{text}");

    // Too long to fit: the hidden part is announced and no option is lost.
    app.permission_request = Some(multiline_request(60));
    for (width, height) in [(80, 24), (120, 40), (40, 12)] {
        let text = text_of(&draw(&app, width, height));
        if !text.contains("rm -rf ~/precious") {
            assert!(
                text.contains("more input rows hidden"),
                "{width}x{height}:\n{text}"
            );
        }
        let request = app.permission_request.as_ref().expect("request");
        let deny = &request.options[deny_index(request)].label;
        assert!(
            text.contains(deny.as_str()),
            "{width}x{height} lost deny:\n{text}"
        );
        if height >= 24 {
            for option in &request.options {
                assert!(
                    text.contains(option.label.as_str()),
                    "{width}x{height} lost {:?}:\n{text}",
                    option.label
                );
            }
        }
    }
}
