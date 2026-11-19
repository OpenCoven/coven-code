//! `/import-config` reloads the saved settings into the running session. The
//! saved permission mode must reach it under the same rule as a start: a mode
//! chosen at launch stays, and a saved bypass mode is never restored. Each case
//! runs in a child process with its own home and engine home, because both are
//! found through process-wide environment. Unix only: on Windows the import
//! finds `~/.claude` through the profile folder, which no variable redirects.
#![cfg(unix)]
use claurst_core::config::{Config, PermissionMode};
use claurst_core::cost::CostTracker;
use claurst_tui::app::App;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const FIXTURE_ENV: &str = "COVEN_IMPORT_CONFIG_PERMISSION_MODE_FIXTURE";
const COMPLETE: &str = "import-config-permission-mode-fixture-complete";

fn press(app: &mut App, code: KeyCode) {
    app.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
#[ignore = "run by isolated parent with temporary home and engine home"]
fn import_config_permission_mode_child() {
    // Only the parent below may run this: it rewrites the settings files of
    // whatever homes the environment names.
    assert_eq!(
        std::env::var(FIXTURE_ENV).as_deref(),
        Ok("1"),
        "run through the parent test"
    );
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let engine_home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    // The file `/import-config` reads. It names no permission mode: the mode
    // comes from the engine settings the import reloads.
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(home.join(".claude/settings.json"), r#"{"theme":"dark"}"#).unwrap();
    std::fs::create_dir_all(&engine_home).unwrap();

    let bypass_refused = Some("Saved bypass mode not restored");
    // (the session's mode, whether a launch flag chose it, the saved mode,
    // the session's mode after the import, the notice it shows)
    let cases = [
        // No flag: a saved bypass is not applied, and the session says so.
        (
            PermissionMode::Default,
            false,
            "bypassPermissions",
            PermissionMode::Default,
            bypass_refused,
        ),
        // A refused bypass leaves a mode the session chose as it is.
        (
            PermissionMode::AcceptEdits,
            false,
            "bypassPermissions",
            PermissionMode::AcceptEdits,
            bypass_refused,
        ),
        (
            PermissionMode::Plan,
            false,
            "bypassPermissions",
            PermissionMode::Plan,
            bypass_refused,
        ),
        // A session that chose bypass itself keeps it; nothing was refused.
        (
            PermissionMode::BypassPermissions,
            false,
            "bypassPermissions",
            PermissionMode::BypassPermissions,
            None,
        ),
        // A saved default is indistinguishable from nothing saved, so it
        // neither loosens a read-only session nor resets accept-edits.
        (
            PermissionMode::Plan,
            false,
            "default",
            PermissionMode::Plan,
            None,
        ),
        (
            PermissionMode::AcceptEdits,
            false,
            "default",
            PermissionMode::AcceptEdits,
            None,
        ),
        // No flag: another saved mode applies, as at a start, and says so
        // when it changes the session.
        (
            PermissionMode::Default,
            false,
            "acceptEdits",
            PermissionMode::AcceptEdits,
            Some("Saved accept-edits mode restored"),
        ),
        (
            PermissionMode::Plan,
            false,
            "plan",
            PermissionMode::Plan,
            None,
        ),
        // A mode chosen at launch survives the reload, whatever is saved.
        (
            PermissionMode::AcceptEdits,
            true,
            "bypassPermissions",
            PermissionMode::AcceptEdits,
            None,
        ),
        (
            PermissionMode::Plan,
            true,
            "default",
            PermissionMode::Plan,
            None,
        ),
        (
            PermissionMode::Default,
            true,
            "acceptEdits",
            PermissionMode::Default,
            None,
        ),
        (
            PermissionMode::BypassPermissions,
            true,
            "default",
            PermissionMode::BypassPermissions,
            None,
        ),
    ];
    let mut failures = Vec::new();
    for (session, from_launch, saved, expected, notice) in cases {
        std::fs::write(
            engine_home.join("settings.json"),
            format!(r#"{{"config":{{"permission_mode":"{saved}"}}}}"#),
        )
        .unwrap();
        let context = format!("session {session:?}, from launch {from_launch}, saved {saved}");
        let mut app = App::new(
            Config {
                permission_mode: session.clone(),
                ..Config::default()
            },
            CostTracker::new(),
        );
        app.permission_mode_from_launch = from_launch;
        app.plan_mode = session == PermissionMode::Plan;

        // /import-config, then "settings.json", then confirm the preview.
        app.open_import_config_picker();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert!(app.import_config_dialog.visible, "{context}: preview open");
        press(&mut app, KeyCode::Enter);
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Config import completed")),
            "{context}: {:?}",
            app.status_message
        );

        if app.config.permission_mode != expected {
            failures.push(format!(
                "{context}: mode {:?}, expected {expected:?}",
                app.config.permission_mode
            ));
        }
        if app.plan_mode != (expected == PermissionMode::Plan) {
            failures.push(format!("{context}: plan indicator {}", app.plan_mode));
        }
        let mode_notices: Vec<&str> = app
            .notifications
            .notifications
            .iter()
            .map(|n| n.message.as_str())
            .filter(|message| message.contains("mode"))
            .collect();
        if mode_notices != notice.into_iter().collect::<Vec<_>>() {
            failures.push(format!(
                "{context}: notices {mode_notices:?}, expected {notice:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    println!("{COMPLETE}");
}

#[test]
fn import_config_applies_the_saved_permission_mode_rule_to_the_running_session() {
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "import_config_permission_mode_child",
            "--ignored",
            "--nocapture",
        ])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("COVEN_CODE_HOME", home.path().join("code"))
        .env("COVEN_HOME", home.path().join("coven"))
        .env_remove("COVEN_CODE_TEST_HOME")
        .env_remove("COVEN_PARENT")
        .env(FIXTURE_ENV, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains(COMPLETE),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
