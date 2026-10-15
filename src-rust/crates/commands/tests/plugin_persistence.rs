use claurst_commands::{CommandContext, CommandResult, PluginCommand, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker, Settings};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn plugin_settings_read_errors_are_reported() {
    run_cases("read");
}

#[cfg(unix)]
#[test]
fn plugin_settings_save_errors_are_reported() {
    run_cases("save");
}

#[test]
fn plugin_settings_success_preserves_other_values() {
    run_cases("success");
}

fn run_cases(case: &str) {
    let mut failures = Vec::new();
    for action in ["enable", "disable"] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&project).unwrap();
        let log = temp.path().join("stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "plugin_persistence_worker",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("COVEN_CODE_HOME", &home)
            .env("PLUGIN_FIXTURE_PROJECT", &project)
            .env("PLUGIN_FIXTURE_CASE", case)
            .env("PLUGIN_FIXTURE_ACTION", action)
            .env("PLUGIN_FIXTURE_DONE", temp.path().join("done"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{case}/{action}: child wait failed: {result:?}");
                }
            }
        };
        if !outcome.success() {
            failures.push(format!(
                "{case}/{action}: {}",
                std::fs::read_to_string(log).unwrap()
            ));
        } else {
            assert_eq!(std::fs::read(temp.path().join("done")).unwrap(), b"checked");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "owned configuration and process globals"]
fn plugin_persistence_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("PLUGIN_FIXTURE_PROJECT").unwrap());
    let case = std::env::var("PLUGIN_FIXTURE_CASE").unwrap();
    let action = std::env::var("PLUGIN_FIXTURE_ACTION").unwrap();
    assert_eq!(claurst_core::config::config_home(), home);
    let settings_path = home.join("settings.json");
    let invalid = b"\xff original settings must survive";
    match case.as_str() {
        "read" => {
            std::fs::write(&settings_path, invalid).unwrap();
            assert!(Settings::load_sync().is_err());
        }
        "save" => {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(home.join("missing/settings.json"), &settings_path)
                    .unwrap();
                assert!(Settings::load_sync().is_ok());
            }
            #[cfg(not(unix))]
            panic!("save fixture requires Unix symlinks");
        }
        "success" => {
            let mut settings = Settings::default();
            settings.enabled_plugins.insert("other-enabled".into());
            settings.disabled_plugins.insert("other-disabled".into());
            settings.enabled_plugins.insert("fixture".into());
            settings.disabled_plugins.insert("fixture".into());
            settings.save_sync().unwrap();
        }
        _ => panic!("unknown case"),
    }
    // Install the fixture as a user plugin in this child's own engine home,
    // so /plugin loads it the same way it loads any installed plugin.
    let plugin_dir = home.join("plugins").join("fixture");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.json"),
        br#"{"name":"fixture","version":"1.0.0"}"#,
    )
    .unwrap();
    let mut context = CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: vec![],
        working_dir: project,
        session_id: "fixture-session".into(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // A failed command must leave the loaded enabled state unchanged.
    let enabled_before = runtime
        .block_on(claurst_plugins::load_plugins(&context.working_dir, &[]))
        .is_enabled("fixture");
    let result =
        runtime.block_on(PluginCommand.execute(&format!("{action} fixture"), &mut context));
    match case.as_str() {
        "read" => {
            let preserved = std::fs::read(&settings_path).unwrap() == invalid;
            assert!(matches!(result, CommandResult::Error(_)) && preserved,
                "read error must be reported and bytes preserved; result={result:?}, preserved={preserved}");
        }
        "save" => {
            assert!(std::fs::symlink_metadata(&settings_path)
                .unwrap()
                .file_type()
                .is_symlink());
            assert!(!home.join("missing").exists());
            assert!(
                matches!(result, CommandResult::Error(_)),
                "save error reported success: {result:?}"
            );
        }
        "success" => {
            assert!(matches!(result, CommandResult::Message(_)), "{result:?}");
            let settings = Settings::load_sync().unwrap();
            assert!(settings.enabled_plugins.contains("other-enabled"));
            assert!(settings.disabled_plugins.contains("other-disabled"));
            assert_eq!(
                settings.enabled_plugins.contains("fixture"),
                action == "enable"
            );
            assert_eq!(
                settings.disabled_plugins.contains("fixture"),
                action == "disable"
            );
        }
        _ => unreachable!(),
    }
    if let CommandResult::Error(message) = &result {
        assert!(
            message.contains("fixture"),
            "missing plugin context: {message}"
        );
        assert!(
            message.contains("Failed"),
            "missing failure context: {message}"
        );
        assert!(!message.contains("original settings must survive"));
    }
    let enabled_after = claurst_plugins::global_plugin_registry()
        .unwrap()
        .is_enabled("fixture");
    if case == "success" {
        assert_eq!(enabled_after, action == "enable");
    } else {
        assert_eq!(enabled_after, enabled_before);
    }
    std::fs::write(std::env::var_os("PLUGIN_FIXTURE_DONE").unwrap(), b"checked").unwrap();
}
