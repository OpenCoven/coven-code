//! T6 (#413), read-only slice: the Next Work page.
//!
//! Only engine-local stores exist today (background registry, model task
//! list, session goal). The page labels each row with its origin and full
//! identifier, never merges rows that share a title, never infers that a
//! blocked task may run, and says plainly that canonical orchestration is not
//! available. It offers no action that would start, stop or retry work.

use chrono::{TimeZone, Utc};
use claurst_core::config::Config;
use claurst_core::cost::CostTracker;
use claurst_core::goal::{Goal, GoalStatus};
use claurst_core::tasks::{BackgroundTask, TaskStatus as BackgroundStatus};
use claurst_tools::{Task, TaskStatus};
use claurst_tui::app::App;
use claurst_tui::application::work::WorkSources;
use claurst_tui::application::UiRenderer;
use claurst_tui::next::style::{Appearance, ColorMode, Glyphs};
use claurst_tui::next::{InspectorPage, NextPresentation};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

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
    text_of(terminal.backend().buffer())
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

fn model_task(id: &str, subject: &str, status: TaskStatus, blocked_by: &[&str]) -> Task {
    let at = Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
    Task {
        id: id.to_string(),
        subject: subject.to_string(),
        description: String::new(),
        status,
        owner: Some("planner".to_string()),
        blocks: Vec::new(),
        blocked_by: blocked_by.iter().map(|id| id.to_string()).collect(),
        metadata: None,
        output: None,
        created_at: at,
        updated_at: at,
    }
}

fn sample_sources() -> WorkSources {
    let mut build = BackgroundTask::new("cargo build --workspace");
    build.status = BackgroundStatus::Cancelled;
    WorkSources {
        background: vec![build],
        model_tasks: vec![model_task(
            "task-7f3a",
            "write the failing tests",
            TaskStatus::Pending,
            &["task-ghost"],
        )],
        goal: Some(Ok(Goal {
            id: "goal-91c2".to_string(),
            session_id: "session-1".to_string(),
            objective: "Ship the Work view".to_string(),
            status: GoalStatus::Paused,
            token_budget: Some(50_000),
            tokens_used: 12_000,
            time_used_secs: 90,
            turns_used: 3,
            created_at_ms: 0,
            updated_at_ms: 0,
        })),
    }
}

#[test]
fn tab_reaches_the_work_page_after_session() {
    let mut app = next_app();
    app.handle_key_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::ALT));
    app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.next_shell.inspector, Some(InspectorPage::Session));
    app.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.next_shell.inspector, Some(InspectorPage::Work));
}

#[test]
fn every_row_names_its_store_and_full_id_and_the_page_says_it_is_local() {
    let mut app = next_app();
    app.work_sources = sample_sources();
    let background_id = app.work_sources.background[0].id.clone();
    app.next_shell.inspector = Some(InspectorPage::Work);
    let text = draw(&app, 80, 40);

    assert!(text.contains("engine-local"), "{text}");
    assert!(text.contains("Psyche #11/#12"), "{text}");
    for expected in [
        "[background] cargo build --workspace",
        "[model task] write the failing tests",
        "[session goal] Ship the Work view",
        "task-7f3a",
        "goal-91c2",
        background_id.as_str(),
        "cancel signalled; exit not confirmed",
        "12000 of 50000 tokens",
    ] {
        assert!(text.contains(expected), "missing {expected:?}:\n{text}");
    }
    assert!(text.contains("Blocked by  task-ghost"), "{text}");
    let missing = text
        .lines()
        .find(|line| line.contains("missing"))
        .unwrap_or_else(|| panic!("unresolved blocker not flagged:\n{text}"));
    assert!(missing.contains("task-ghost"), "{missing}");
}

#[test]
fn the_source_note_stays_visible_at_80x24() {
    let mut app = next_app();
    app.work_sources = sample_sources();
    app.next_shell.inspector = Some(InspectorPage::Work);
    let text = draw(&app, 80, 24);
    assert!(text.contains("engine-local"), "{text}");
    assert!(text.contains("[background]"), "{text}");
}

#[test]
fn an_empty_page_and_an_unreadable_goal_say_so() {
    let mut app = next_app();
    app.next_shell.inspector = Some(InspectorPage::Work);
    let empty = draw(&app, 80, 24);
    assert!(
        empty.contains("no engine-local work in this process"),
        "{empty}"
    );
    assert!(
        empty.contains("not just this session"),
        "says the in-memory stores are process-wide:\n{empty}"
    );

    app.work_sources.goal = Some(Err("database is locked".to_string()));
    let failed = draw(&app, 80, 24);
    assert!(failed.contains("unreadable"), "{failed}");
    assert!(failed.contains("database is locked"), "{failed}");
}

#[test]
fn the_views_page_points_work_at_the_local_page() {
    let mut app = next_app();
    app.next_shell.inspector = Some(InspectorPage::Views);
    let text = draw(&app, 80, 24);
    let work = text
        .lines()
        .find(|line| line.trim_start().starts_with("Work "))
        .unwrap_or_else(|| panic!("Work missing:\n{text}"));
    assert!(work.contains("local only"), "{work}");
    assert!(work.contains("#413"), "{work}");
}

#[test]
fn the_work_page_offers_no_action_on_the_work() {
    let mut app = next_app();
    app.work_sources = sample_sources();
    app.next_shell.inspector = Some(InspectorPage::Work);
    let tasks_before = claurst_tools::TASK_STORE.len();
    let background_before = claurst_core::tasks::global_registry().list().len();
    for code in [KeyCode::Enter, KeyCode::Delete, KeyCode::Backspace] {
        app.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
    }
    assert_eq!(claurst_tools::TASK_STORE.len(), tasks_before);
    assert_eq!(
        claurst_core::tasks::global_registry().list().len(),
        background_before
    );
    assert!(
        app.take_input().is_empty(),
        "no text or command would be submitted"
    );
    let text = draw(&app, 80, 40).to_lowercase();
    for verb in ["retry", "resume", "start "] {
        assert!(!text.contains(verb), "offers {verb:?}:\n{text}");
    }
}

#[test]
fn the_scope_notes_stay_visible_when_the_page_scrolls() {
    let mut app = next_app();
    app.work_sources = sample_sources();
    app.next_shell.inspector = Some(InspectorPage::Work);
    draw(&app, 80, 24);
    for _ in 0..20 {
        app.next_shell.apply_action("scrollInspectorDown");
    }
    let text = draw(&app, 80, 24);
    assert!(text.contains("engine-local"), "{text}");
    assert!(text.contains("Psyche #11/#12"), "{text}");
}

#[test]
fn the_work_page_renders_at_small_fixtures_without_losing_the_way_back() {
    let mut app = next_app();
    app.work_sources = sample_sources();
    app.next_shell.inspector = Some(InspectorPage::Work);
    let medium = draw(&app, 40, 12);
    assert!(medium.contains("engine-local"), "{medium}");
    assert!(medium.contains("Esc"), "{medium}");
    for (width, height) in [(20, 6), (1, 1)] {
        let text = draw(&app, width, height);
        assert!(!text.is_empty(), "{width}x{height}");
    }
}

#[test]
fn a_budget_limited_goal_is_not_shown_as_a_failure() {
    let mut app = next_app();
    let mut sources = sample_sources();
    if let Some(Ok(goal)) = sources.goal.as_mut() {
        goal.status = GoalStatus::BudgetLimited;
    }
    app.work_sources = sources;
    app.next_presentation.appearance.color = ColorMode::TrueColor;
    app.next_shell.inspector = Some(InspectorPage::Work);
    let mut terminal = Terminal::new(TestBackend::new(80, 40)).expect("terminal");
    terminal
        .draw(|frame| claurst_tui::next::render_next(frame, &app))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let text = text_of(&buffer);
    let row = text
        .lines()
        .position(|line| line.contains("budget limited"))
        .unwrap_or_else(|| panic!("{text}"));
    let column = text
        .lines()
        .nth(row)
        .unwrap()
        .find("budget limited")
        .unwrap();
    let limited = buffer[(column as u16, row as u16)].fg;
    let failed_row = text
        .lines()
        .position(|line| line.contains("unavailable"))
        .unwrap();
    let failed_column = text
        .lines()
        .nth(failed_row)
        .unwrap()
        .find("unavailable")
        .unwrap();
    let blocked = buffer[(failed_column as u16, failed_row as u16)].fg;
    assert_ne!(
        limited, blocked,
        "budget limited uses the pending color, not blocked"
    );
}
