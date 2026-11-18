//! A start without `--permission-mode` runs in the saved mode, so the settings
//! screen must not save the mode a session got from a flag or a keystroke when
//! it saves another field. Each case runs in a child process with its own
//! engine home, because the settings file is found through process-wide
//! environment.
use claurst_core::config::{Config, PermissionMode, Settings};
use claurst_tui::settings_screen::{handle_settings_key, SettingsScreen};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
#[ignore = "run by isolated parent with temporary engine home"]
fn settings_permission_mode_child() {
    // Only the parent below may run this: it rewrites the settings file of
    // whatever engine home the environment names.
    assert_eq!(
        std::env::var("COVEN_SETTINGS_PERMISSION_MODE_FIXTURE").as_deref(),
        Ok("1"),
        "run through the parent test"
    );
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let file = home.join("settings.json");
    let save = |mode: &str| {
        std::fs::write(
            &file,
            format!(r#"{{"config": {{"permission_mode": "{mode}"}}}}"#),
        )
        .unwrap()
    };
    // (saved before the screen opens, saved while it is open, the session's
    // mode, what must be on disk afterwards)
    let cases = [
        (
            None,
            None,
            PermissionMode::AcceptEdits,
            PermissionMode::Default,
        ),
        (
            None,
            None,
            PermissionMode::BypassPermissions,
            PermissionMode::Default,
        ),
        (
            Some("plan"),
            None,
            PermissionMode::BypassPermissions,
            PermissionMode::Plan,
        ),
        (
            Some("acceptEdits"),
            None,
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
        ),
        (
            Some("acceptEdits"),
            Some("default"),
            PermissionMode::AcceptEdits,
            PermissionMode::Default,
        ),
    ];
    for (max_tokens, (saved, saved_while_open, session, expected)) in (4000..).zip(cases) {
        match saved {
            Some(mode) => save(mode),
            None => {
                let _ = std::fs::remove_file(&file);
            }
        }
        // The session's mode, as `--permission-mode` or the mode key set it.
        let mut config = Config {
            permission_mode: session.clone(),
            ..Config::default()
        };
        let mut screen = SettingsScreen::new();
        screen.open();
        if let Some(mode) = saved_while_open {
            save(mode);
        }
        screen.start_edit("max_tokens", "");
        for c in max_tokens.to_string().chars() {
            handle_settings_key(
                &mut screen,
                &mut config,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            );
        }
        handle_settings_key(
            &mut screen,
            &mut config,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        let on_disk = Settings::load_sync().unwrap().config;
        assert_eq!(
            on_disk.max_tokens,
            Some(max_tokens),
            "the edited field is saved"
        );
        assert_eq!(
            on_disk.permission_mode, expected,
            "saved {saved:?}, then {saved_while_open:?}, session {session:?}"
        );
        assert_eq!(
            config.permission_mode, session,
            "the session keeps its mode"
        );
    }
    println!("settings-permission-mode-fixture-complete");
}

#[test]
fn saving_a_field_in_the_settings_screen_does_not_save_the_sessions_permission_mode() {
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "settings_permission_mode_child",
            "--ignored",
            "--nocapture",
        ])
        .env("COVEN_CODE_HOME", home.path())
        .env_remove("COVEN_CODE_TEST_HOME")
        .env("COVEN_SETTINGS_PERMISSION_MODE_FIXTURE", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("settings-permission-mode-fixture-complete"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
