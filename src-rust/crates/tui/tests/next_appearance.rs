//! `/config appearance` under Next: a preview changes what is drawn and
//! nothing else, and dropping it restores the same frame.

use claurst_core::appearance::AppearanceEdits;
use claurst_core::config::Settings;
use claurst_core::keybindings::UserKeybindings;
use claurst_core::types::Message;
use claurst_tui::application::UiRenderer;
use claurst_tui::next::appearance::{
    appearance_args, apply_presentation, parse_appearance_command, run_appearance_command,
    AppearanceCommand,
};
use claurst_tui::next::render_next;
use claurst_tui::next::style::{ColorMode, Glyphs, Motion};
use claurst_tui::next::InspectorPage;
use claurst_tui::App;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

const SPLIT: (u16, u16) = (130, 36);
const NARROW: (u16, u16) = (100, 30);

fn app() -> App {
    let mut app = App::new(
        claurst_core::config::Config::default(),
        claurst_core::cost::CostTracker::new(),
    );
    app.select_renderer(UiRenderer::Next);
    app.push_message(Message::user("first question".to_string()));
    app.push_message(Message::assistant("first answer".to_string()));
    app.push_message(Message::user("second question".to_string()));
    app.push_message(Message::assistant("second answer".to_string()));
    app
}

fn env(name: &str) -> Option<String> {
    match name {
        "COLORTERM" => Some("truecolor".to_string()),
        "COLORFGBG" => Some("15;0".to_string()),
        "LANG" | "LC_ALL" | "LC_CTYPE" => Some("en_US.UTF-8".to_string()),
        _ => None,
    }
}

fn run(app: &mut App, saved: &Settings, args: &str, terminal: (u16, u16)) -> String {
    let command = parse_appearance_command(args).expect("command parses");
    run_appearance_command(
        app,
        command,
        saved,
        None,
        &UserKeybindings::default(),
        env,
        terminal,
    )
}

fn frame(app: &App) -> Buffer {
    let backend = TestBackend::new(SPLIT.0, SPLIT.1);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_next(frame, app))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    buffer
        .content()
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn row_of(buffer: &Buffer, needle: &str) -> usize {
    text(buffer)
        .lines()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle} is on screen"))
}

#[test]
fn only_the_appearance_subcommand_of_config_is_taken() {
    assert_eq!(appearance_args("config", "appearance"), Some(""));
    assert_eq!(
        appearance_args("settings", " appearance save "),
        Some("save")
    );
    assert_eq!(
        appearance_args("config", "appearance density compact"),
        Some("density compact")
    );
    assert_eq!(appearance_args("config", "appearances"), None);
    assert_eq!(appearance_args("config", "theme dark"), None);
    assert_eq!(appearance_args("config", ""), None);
    assert_eq!(appearance_args("model", "appearance"), None);
    assert!(parse_appearance_command("density compact extra").is_err());
    assert_eq!(
        parse_appearance_command("reset"),
        Ok(AppearanceCommand::Reset)
    );
}

#[test]
fn each_setting_shows_at_once_and_a_revert_restores_the_same_frame() {
    let saved = Settings::default();
    let mut app = app();
    apply_presentation(&mut app, &saved, &UserKeybindings::default(), env);
    let before = frame(&app);
    assert_eq!(app.next_presentation.appearance.color, ColorMode::TrueColor);

    // Compact drops the blank row between entries.
    let gap =
        |buffer: &Buffer| row_of(buffer, "second question") - row_of(buffer, "first question");
    let comfortable = gap(&before);
    let reply = run(&mut app, &saved, "density compact", SPLIT);
    assert!(reply.contains("Previewing density = compact"), "{reply}");
    assert!(reply.contains("save"), "{reply}");
    let compact = frame(&app);
    assert!(gap(&compact) < comfortable, "compact is tighter");

    run(&mut app, &saved, "colorMode none", SPLIT);
    assert_eq!(app.next_presentation.appearance.color, ColorMode::NoColor);
    run(&mut app, &saved, "glyphs ascii", SPLIT);
    assert_eq!(app.next_presentation.appearance.glyphs, Glyphs::Ascii);
    run(&mut app, &saved, "reduceMotion true", SPLIT);
    assert_eq!(app.next_presentation.appearance.motion, Motion::Reduced);
    assert!(app.next_presentation.hints.is_none());
    run(&mut app, &saved, "hints true", SPLIT);
    assert!(app.next_presentation.hints.is_some());

    let reply = run(&mut app, &saved, "revert", SPLIT);
    assert!(reply.contains("dropped"), "{reply}");
    assert!(app.next_shell.appearance_edits.is_empty());
    // The two messages above are not part of the conversation being compared.
    assert_eq!(frame(&app), before, "a revert restores every cell");
    let reply = run(&mut app, &saved, "revert", SPLIT);
    assert!(reply.contains("Nothing to revert"), "{reply}");
}

#[test]
fn a_refused_value_changes_nothing_and_says_what_is_accepted() {
    let saved = Settings::default();
    let mut app = app();
    apply_presentation(&mut app, &saved, &UserKeybindings::default(), env);
    let before = frame(&app);
    let reply = run(&mut app, &saved, "density roomy", SPLIT);
    assert!(reply.contains("comfortable, compact"), "{reply}");
    let reply = run(&mut app, &saved, "permissionMode bypass", SPLIT);
    assert!(reply.contains("not an appearance setting"), "{reply}");
    assert!(app.next_shell.appearance_edits.is_empty());
    assert_eq!(frame(&app), before);
}

#[test]
fn a_preview_leaves_the_engines_state_alone() {
    let saved = Settings::default();
    let mut app = app();
    let (config, messages) = (format!("{:?}", app.config), app.messages.len());
    for args in [
        "theme light",
        "colorMode 16",
        "glyphs ascii",
        "density compact",
        "layout work",
        "hints true",
        "reduceMotion true",
        "reset",
        "revert",
    ] {
        run(&mut app, &saved, args, SPLIT);
    }
    assert_eq!(format!("{:?}", app.config), config);
    assert_eq!(app.messages.len(), messages, "the caller posts the reply");
}

#[test]
fn layout_opens_the_inspector_only_when_it_fits_beside_the_transcript() {
    let saved = Settings::default();
    let mut app = app();
    run(&mut app, &saved, "layout work", SPLIT);
    assert_eq!(app.next_shell.inspector, Some(InspectorPage::Work));
    run(&mut app, &saved, "layout inspect", SPLIT);
    assert_eq!(app.next_shell.inspector, Some(InspectorPage::Activity));
    run(&mut app, &saved, "layout focus", SPLIT);
    assert_eq!(app.next_shell.inspector, None);

    let reply = run(&mut app, &saved, "layout work", NARROW);
    assert_eq!(app.next_shell.inspector, None, "the transcript stays");
    assert!(reply.contains("120x32"), "{reply}");
    assert!(reply.contains("100x30"), "{reply}");

    // Changing something else keeps an inspector the user opened.
    app.next_shell.apply_action("toggleInspector");
    run(&mut app, &saved, "density compact", SPLIT);
    assert_eq!(app.next_shell.inspector, Some(InspectorPage::Activity));
    // Dropping a layout preview closes what the preview opened.
    run(&mut app, &saved, "revert", SPLIT);
    run(&mut app, &saved, "layout work", SPLIT);
    run(&mut app, &saved, "revert", SPLIT);
    assert_eq!(app.next_shell.inspector, None);
}

#[test]
fn the_listing_marks_previews_and_values_it_had_to_ignore() {
    let saved: Settings =
        serde_json::from_str(r#"{"nextAppearance":{"theme":"plaid","density":"compact"}}"#)
            .expect("settings");
    let mut app = app();
    let listing = run(&mut app, &saved, "", SPLIT);
    assert!(listing.contains("density"), "{listing}");
    assert!(listing.contains("compact"), "{listing}");
    assert!(listing.contains("reduceMotion"), "{listing}");
    assert!(!listing.contains('*'), "nothing previewed: {listing}");
    assert!(listing.contains("using the default: theme"), "{listing}");

    run(&mut app, &saved, "density comfortable", SPLIT);
    let listing = run(&mut app, &saved, "", SPLIT);
    assert!(listing.contains("comfortable*"), "{listing}");
    assert!(listing.contains("previewing, not saved"), "{listing}");
}

#[test]
fn saving_reports_failure_and_what_a_folder_still_overrides() {
    let saved = Settings::default();
    let mut app = app();
    let reply = run(&mut app, &saved, "save", SPLIT);
    assert!(reply.contains("Nothing to save"), "{reply}");

    run(&mut app, &saved, "density compact", SPLIT);
    let reply = run_appearance_command(
        &mut app,
        AppearanceCommand::Save,
        &saved,
        Some("settings.json is not valid JSON".to_string()),
        &UserKeybindings::default(),
        env,
        SPLIT,
    );
    assert!(reply.contains("Not saved"), "{reply}");
    assert!(reply.contains("is not valid JSON"), "{reply}");
    assert!(
        !app.next_shell.appearance_edits.is_empty(),
        "a failed save keeps the preview"
    );

    // The file now says compact and the theme, but this folder sets a theme.
    let on_disk: Settings = serde_json::from_str(
        r#"{"reduceMotion":true,"nextAppearance":{"density":"compact","theme":"light"}}"#,
    )
    .expect("settings");
    let mut edits = AppearanceEdits::default();
    edits.set("density", "compact").unwrap();
    edits.set("theme", "dark").unwrap();
    edits.set("reduceMotion", "false").unwrap();
    app.next_shell.appearance_edits = edits;
    let reply = run(&mut app, &on_disk, "save", SPLIT);
    assert!(reply.starts_with("Saved, but"), "{reply}");
    assert!(reply.contains("reduceMotion, theme"), "{reply}");
    assert!(!reply.contains("density"), "{reply}");
    assert!(app.next_shell.appearance_edits.is_empty());
    assert_eq!(
        app.next_presentation.appearance.motion,
        Motion::Reduced,
        "the screen shows what the next launch will"
    );

    let mut clean = AppearanceEdits::default();
    clean.set("density", "compact").unwrap();
    app.next_shell.appearance_edits = clean;
    assert_eq!(run(&mut app, &on_disk, "save", SPLIT), "Saved.");
}
