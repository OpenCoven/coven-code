//! Saving a field in the settings screen writes that field to the user's
//! settings file and nothing else from the session: not this start's CLI
//! flags, not a resumed session's model, and not values merged in from shared
//! or project settings. The cases run in a child process with its own engine
//! home, shared home and project folder, because the settings files are found
//! through process-wide environment.
use claurst_core::config::{Config, Settings, Theme};
use claurst_tui::settings_screen::{handle_settings_key, SettingsScreen};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const API_KEY: &str = "sk-cli-600-never-saved";
const CLI_MODEL: &str = "cli-model-600";
const PROJECT_PROMPT: &str = "project prompt 600";

fn press(screen: &mut SettingsScreen, config: &mut Config, code: KeyCode) {
    handle_settings_key(screen, config, KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(screen: &mut SettingsScreen, config: &mut Config, text: &str) {
    for c in text.chars() {
        press(screen, config, KeyCode::Char(c));
    }
}

/// The session's config as `main` builds it: the user, shared and project
/// layers, then `--api-key` and `--model` on top.
fn session_config(project: &std::path::Path) -> Config {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut config = runtime
        .block_on(Settings::load_hierarchical(project))
        .effective_config();
    config.api_key = Some(API_KEY.to_string());
    config.model = Some(CLI_MODEL.to_string());
    config
}

/// Edit Max Tokens through the screen's keys, then toggle Show current
/// directory, which saves again after the text field did.
fn edit_through_screen(config: &mut Config, max_tokens: u32) {
    let mut screen = SettingsScreen::new();
    screen.open();
    type_text(&mut screen, config, "max tokens");
    press(&mut screen, config, KeyCode::Enter);
    assert!(screen.edit_field.is_some(), "Max Tokens opens for editing");
    for _ in 0..screen.edit_value.len() {
        press(&mut screen, config, KeyCode::Backspace);
    }
    type_text(&mut screen, config, &max_tokens.to_string());
    press(&mut screen, config, KeyCode::Enter);
    assert!(screen.edit_field.is_none(), "Enter saves the edit");

    press(&mut screen, config, KeyCode::Esc);
    type_text(&mut screen, config, "show current directory");
    press(&mut screen, config, KeyCode::Enter);
    assert!(screen.show_cwd, "Enter toggles Show current directory");
}

fn assert_no_session_values(raw: &str, saved: &Settings) {
    assert!(
        !raw.contains(API_KEY),
        "the CLI api key is not saved:\n{raw}"
    );
    assert!(
        !raw.contains(CLI_MODEL),
        "the CLI model is not saved:\n{raw}"
    );
    assert!(
        !raw.contains(PROJECT_PROMPT),
        "project settings are not saved:\n{raw}"
    );
    assert_eq!(saved.config.api_key, None);
    assert!(
        matches!(saved.config.theme, Theme::Default),
        "shared settings are not saved:\n{raw}"
    );
}

#[test]
#[ignore = "run by isolated parent with temporary engine home"]
fn settings_save_only_edited_child() {
    // Only the parent below may run this: it rewrites the settings file of
    // whatever engine home the environment names.
    assert_eq!(
        std::env::var("COVEN_SETTINGS_SAVE_ONLY_EDITED_FIXTURE").as_deref(),
        Ok("1"),
        "run through the parent test"
    );
    let path = |name: &str| std::path::PathBuf::from(std::env::var_os(name).unwrap());
    let file = path("COVEN_CODE_HOME").join("settings.json");
    let project = path("COVEN_SETTINGS_TEST_PROJECT");
    assert_eq!(Settings::global_settings_path(), file);
    let project_file = project.join(".coven-code").join("settings.json");
    std::fs::create_dir_all(project_file.parent().unwrap()).unwrap();
    std::fs::write(
        &project_file,
        format!(r#"{{"config": {{"custom_system_prompt": "{PROJECT_PROMPT}"}}}}"#),
    )
    .unwrap();
    std::fs::write(
        path("COVEN_HOME").join("settings.json"),
        r#"{"theme": "light"}"#,
    )
    .unwrap();

    // A folder without project settings, so the shared theme reaches the
    // session: a project file replaces the theme even when it leaves it out.
    let plain = project.parent().unwrap().join("plain");
    std::fs::create_dir_all(&plain).unwrap();

    // No user settings file yet; the session runs in the project folder.
    let _ = std::fs::remove_file(&file);
    let mut config = session_config(&project);
    assert_eq!(config.custom_system_prompt.as_deref(), Some(PROJECT_PROMPT));
    edit_through_screen(&mut config, 4321);
    let raw = std::fs::read_to_string(&file).unwrap();
    let saved = Settings::load_sync().unwrap();
    assert_eq!(saved.config.max_tokens, Some(4321), "the edit is saved");
    assert!(saved.show_cwd, "the toggle is saved");
    assert_eq!(saved.config.model, None);
    assert_eq!(saved.config.custom_system_prompt, None);
    assert_no_session_values(&raw, &saved);
    // The session keeps its own values and sees the edit.
    assert_eq!(config.max_tokens, Some(4321));
    assert_eq!(config.api_key.as_deref(), Some(API_KEY));
    assert_eq!(config.model.as_deref(), Some(CLI_MODEL));
    assert_eq!(config.custom_system_prompt.as_deref(), Some(PROJECT_PROMPT));

    // Values already in the user file stay as they were saved; the session
    // runs outside the project and gets the shared theme.
    std::fs::write(
        &file,
        r#"{"config": {"model": "saved-model", "append_system_prompt": "saved append", "max_tokens": 1000}}"#,
    )
    .unwrap();
    let mut config = session_config(&plain);
    assert!(matches!(config.theme, Theme::Light), "shared theme applies");
    assert_eq!(config.model.as_deref(), Some(CLI_MODEL));
    edit_through_screen(&mut config, 8765);
    let raw = std::fs::read_to_string(&file).unwrap();
    let saved = Settings::load_sync().unwrap();
    assert_eq!(saved.config.max_tokens, Some(8765), "the edit is saved");
    assert!(saved.show_cwd, "the toggle is saved");
    assert_eq!(saved.config.model.as_deref(), Some("saved-model"));
    assert_eq!(
        saved.config.append_system_prompt.as_deref(),
        Some("saved append")
    );
    assert_no_session_values(&raw, &saved);
    // The session keeps the shared theme it started with.
    assert!(matches!(config.theme, Theme::Light));
    println!("settings-save-only-edited-fixture-complete");
}

#[test]
fn saving_a_field_in_the_settings_screen_saves_only_that_field() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = |name: &str| {
        let dir = scratch.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "settings_save_only_edited_child",
            "--ignored",
            "--nocapture",
        ])
        .env("HOME", dir("home"))
        .env("USERPROFILE", dir("home"))
        .env("COVEN_CODE_HOME", dir("engine"))
        .env("COVEN_HOME", dir("coven"))
        .env("COVEN_SETTINGS_TEST_PROJECT", dir("project"))
        .env_remove("COVEN_CODE_TEST_HOME")
        .env_remove("COVEN_PARENT")
        .env("COVEN_SETTINGS_SAVE_ONLY_EDITED_FIXTURE", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("settings-save-only-edited-fixture-complete"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
