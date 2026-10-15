use claurst_commands::{CommandContext, CommandResult, SlashCommand, VimCommand, VoiceCommand};
use claurst_core::{config::Config, cost::CostTracker};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn ui_settings_mutation_preserves_failed_reads() {
    run_cases(&["utf8", "syntax", "type", "directory"]);
}

#[test]
fn ui_settings_mutation_missing_and_valid_controls() {
    run_cases(&["missing", "valid"]);
}

fn run_cases(cases: &[&str]) {
    let mut failures = Vec::new();
    for case in cases {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&project).unwrap();
        let log = temp.path().join("stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ui_settings_worker", "--ignored", "--nocapture"])
            .env_clear()
            .env("COVEN_CODE_HOME", &home)
            .env("UI_FIXTURE_PROJECT", &project)
            .env("UI_FIXTURE_CASE", case)
            .env("UI_FIXTURE_DONE", temp.path().join("done"))
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
                    panic!("{case}: child wait failed: {result:?}");
                }
            }
        };
        if outcome.success() {
            assert_eq!(std::fs::read(temp.path().join("done")).unwrap(), b"checked");
        } else {
            failures.push(format!("{case}: {}", std::fs::read_to_string(log).unwrap()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "owned process configuration"]
fn ui_settings_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("UI_FIXTURE_PROJECT").unwrap());
    let case = std::env::var("UI_FIXTURE_CASE").unwrap();
    assert_eq!(claurst_core::config::config_home(), home);
    let path = claurst_core::Settings::config_dir().join("ui-settings.json");
    assert_eq!(path, home.join("ui-settings.json"));
    let bytes: Option<&[u8]> = match case.as_str() {
        "utf8" => Some(b"\xffprivate-fixture-value"),
        "syntax" => Some(b"{\"voice_enabled\":true,"),
        "type" => Some(br#"{"voice_enabled":"private-fixture-value"}"#),
        "valid" => Some(br#"{"editor_mode":"normal","voice_enabled":true,"sandbox_excluded_commands":["fixture-only"],"statusline_show_cost":false}"#),
        "missing" | "directory" => None,
        _ => panic!("unknown case"),
    };
    if let Some(bytes) = bytes {
        std::fs::write(&path, bytes).unwrap();
    } else if case == "directory" {
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("sentinel"), b"original").unwrap();
    }
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
    let display = runtime.block_on(VoiceCommand.execute("status", &mut context));
    assert!(
        matches!(display, CommandResult::Message(_)),
        "display fallback changed: {display:?}"
    );
    if let Some(bytes) = bytes {
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    } else if case == "missing" {
        assert!(!path.exists());
    }
    let result = runtime.block_on(VimCommand.execute("on", &mut context));
    if matches!(case.as_str(), "missing" | "valid") {
        assert!(matches!(result, CommandResult::Message(_)), "{result:?}");
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["editor_mode"], "vim");
        if case == "valid" {
            assert_eq!(saved["voice_enabled"], true);
            assert_eq!(saved["statusline_show_cost"], false);
            assert_eq!(
                saved["sandbox_excluded_commands"],
                serde_json::json!(["fixture-only"])
            );
        }
    } else {
        let preserved = if let Some(bytes) = bytes {
            std::fs::read(&path).unwrap() == bytes
        } else {
            std::fs::read(path.join("sentinel")).unwrap() == b"original"
        };
        assert!(
            matches!(result, CommandResult::Error(_)) && preserved,
            "failed read must prevent mutation: result={result:?}, preserved={preserved}"
        );
        if let CommandResult::Error(message) = result {
            assert!(!message.contains("private-fixture-value"));
            assert!(
                message.contains("ui-settings.json"),
                "missing file context: {message}"
            );
        }
    }
    std::fs::write(std::env::var_os("UI_FIXTURE_DONE").unwrap(), b"checked").unwrap();
}
