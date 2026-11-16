//! Distinguish frame rendering from the separately sanitized terminal title.
use claurst_core::{config::Config, cost::CostTracker, types::Message};
use claurst_tui::application::UiRenderer;
use claurst_tui::dialogs::PermissionRequest;
use claurst_tui::notifications::NotificationKind;
use claurst_tui::{
    app::{App, ToolStatus, ToolUseBlock},
    render::render_app,
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal, TerminalOptions, Viewport};

const HOSTILE: &str = "FRAME-BEGIN\x07\x1b]52;c;RlJBTUUtQ0FOQVJZ\x07\u{9d}\u{9c}\u{202e}FRAME-END";

fn assert_frame_inert(app: &App) {
    for (width, height) in [(40, 12), (80, 24), (120, 40)] {
        let mut bytes = Vec::new();
        let leaks;
        {
            let backend = CrosstermBackend::new(&mut bytes);
            let options = TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, width, height)),
            };
            let mut terminal = Terminal::with_options(backend, options).unwrap();
            let completed = terminal
                .draw(|frame| {
                    if app.renderer == UiRenderer::Next {
                        claurst_tui::next::render_next(frame, app)
                    } else {
                        render_app(frame, app)
                    }
                })
                .unwrap();
            leaks = completed
                .buffer
                .content
                .iter()
                .enumerate()
                .filter(|(_, cell)| {
                    cell.symbol().chars().any(|ch| {
                        ch.is_control()
                            || matches!(ch,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
                    })
                })
                .map(|(index, cell)| (index, cell.symbol().to_string()))
                .collect::<Vec<_>>();
        }
        assert!(
            leaks.is_empty(),
            "untrusted control characters in frame cells: {leaks:?}"
        );
        let command = b"\x1b]52;c;RlJBTUUtQ0FOQVJZ";
        assert!(
            !bytes.windows(command.len()).any(|window| window == command),
            "Classic frame emitted a raw OSC52 clipboard command"
        );
    }
}

#[test]
fn classic_prompt_is_inert_on_the_wire() {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.set_prompt_text(HOSTILE.into());
    assert_frame_inert(&app);
}

#[test]
fn classic_user_transcript_is_inert_on_the_wire() {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.push_message(Message::user(HOSTILE));
    assert_frame_inert(&app);
}

#[test]
fn classic_status_is_inert_on_the_wire() {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.status_message = Some(HOSTILE.into());
    assert_frame_inert(&app);
}

#[test]
fn classic_live_text_and_tool_output_are_inert() {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.push_message(Message::assistant(HOSTILE));
    app.is_streaming = true;
    app.streaming_text = HOSTILE.into();
    app.tool_use_blocks.push(ToolUseBlock {
        id: "hostile-tool".into(),
        name: HOSTILE.into(),
        turn_index: None,
        status: ToolStatus::Done,
        output_preview: Some(HOSTILE.into()),
        input_json: "{}".into(),
    });
    assert_frame_inert(&app);
    assert_eq!(
        app.streaming_text, HOSTILE,
        "display guard leaves engine data intact"
    );
}

#[test]
fn classic_error_modal_and_security_prompt_are_inert() {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.notifications
        .push(NotificationKind::Error, HOSTILE.into(), None);
    assert_frame_inert(&app);
    app.permission_request = Some(
        PermissionRequest::bash(
            "tool".into(),
            "Bash".into(),
            HOSTILE.into(),
            HOSTILE.into(),
            None,
        )
        .guarded(),
    );
    assert_frame_inert(&app);
}

#[test]
fn next_delegated_frame_is_inert() {
    let mut app = App::new(Config::default(), CostTracker::new());
    app.renderer = UiRenderer::Next;
    app.push_message(Message::user(HOSTILE));
    app.set_prompt_text(HOSTILE.into());
    app.help_overlay.visible = true;
    assert!(claurst_tui::next::delegates_to_classic(&app));
    assert_frame_inert(&app);
}

#[test]
fn ordinary_unicode_and_source_text_survive_rendering() {
    let text = "plain café 漢字 e\u{301} 👩\u{200d}💻";
    let mut app = App::new(Config::default(), CostTracker::new());
    app.set_prompt_text(text.into());
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(Vec::new()),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 120, 40)),
        },
    )
    .unwrap();
    let frame = terminal.draw(|frame| render_app(frame, &app)).unwrap();
    let rendered: String = frame
        .buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("plain café"));
    assert!(rendered.contains('漢') && rendered.contains('字'));
    assert!(rendered.contains("e\u{301}") && rendered.contains("👩\u{200d}💻"));
    assert_eq!(app.prompt_input.text, text);
}

/// Every row of a Classic frame, as text.
fn classic_rows(app: &App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(Vec::new()),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, width, height)),
        },
    )
    .unwrap();
    let completed = terminal.draw(|frame| render_app(frame, app)).unwrap();
    completed
        .buffer
        .content
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

#[test]
fn classic_draws_a_tab_as_a_space() {
    // A tab is ordinary text in code, diffs and command output. It must not
    // reach the terminal raw, and it is not a character to flag as hostile.
    let text = "TAB-BEGIN\tTAB-END";
    type Show = fn(&mut App, &str);
    let surfaces: [(&str, Show); 4] = [
        ("prompt", |app, text| app.set_prompt_text(text.into())),
        ("user message", |app, text| {
            app.push_message(Message::user(text))
        }),
        ("assistant message", |app, text| {
            app.push_message(Message::assistant(text))
        }),
        ("status", |app, text| app.status_message = Some(text.into())),
    ];
    let mut wrong = Vec::new();
    for (surface, show) in surfaces {
        let mut app = App::new(Config::default(), CostTracker::new());
        show(&mut app, text);
        let frame = classic_rows(&app, 120, 40).join("\n");
        if frame.contains('\t') {
            wrong.push(format!("{surface}: a raw tab reached a cell"));
        }
        if frame.contains('\u{fffd}') {
            wrong.push(format!("{surface}: a tab was drawn as U+FFFD"));
        }
        if !frame.contains("TAB-BEGIN TAB-END") {
            wrong.push(format!("{surface}: the tab is not one space"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
