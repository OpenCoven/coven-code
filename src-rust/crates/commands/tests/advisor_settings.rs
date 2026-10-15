use claurst_commands::{AdvisorCommand, CommandContext, CommandResult, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn advisor_invalid_model_preserves_settings() {
    run_cases(&["invalid-set"]);
}

#[test]
fn advisor_settings_read_errors() {
    run_cases(&["utf8-set", "utf8-off", "syntax-set", "syntax-off"]);
}
#[test]
fn advisor_settings_shape_controls() {
    run_cases(&[
        "array-set",
        "array-off",
        "string-set",
        "string-off",
        "number-set",
        "number-off",
        "bool-set",
        "bool-off",
        "null-set",
        "null-off",
    ]);
}
#[test]
fn advisor_settings_valid_and_missing_controls() {
    run_cases(&[
        "valid-set",
        "valid-off",
        "missing-set",
        "missing-off",
        "parent-set",
        "parent-off",
    ]);
}
#[cfg(unix)]
#[test]
fn advisor_settings_save_errors() {
    run_cases(&["save-set", "save-off"]);
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
            .args([
                "--exact",
                "advisor_settings_worker",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("COVEN_CODE_HOME", &home)
            .env("ADVISOR_FIXTURE_PROJECT", &project)
            .env("ADVISOR_FIXTURE_CASE", case)
            .env("ADVISOR_FIXTURE_DONE", temp.path().join("done"))
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
fn advisor_settings_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let project = std::path::PathBuf::from(std::env::var_os("ADVISOR_FIXTURE_PROJECT").unwrap());
    let case = std::env::var("ADVISOR_FIXTURE_CASE").unwrap();
    let (kind, action) = case.split_once('-').unwrap();
    assert_eq!(claurst_core::config::config_home(), home);
    let path = home.join("settings.json");
    let bytes: Option<&[u8]> = match kind {
        "utf8" => Some(b"\xffprivate-fixture-value"),
        "syntax" | "invalid" => Some(b"{\"private-fixture-value\":"),
        "array" => Some(b"[1,2]"),
        "string" => Some(br#""private-fixture-value""#),
        "number" => Some(b"123"),
        "bool" => Some(b"true"),
        "null" => Some(b"null"),
        "valid" => Some(br#"{"advisorModel":"claude-old","extra":{"nested":[1,true,"retained"]}}"#),
        "missing" | "parent" | "save" => None,
        _ => panic!("unknown case"),
    };
    if let Some(bytes) = bytes {
        std::fs::write(&path, bytes).unwrap();
    } else if kind == "parent" {
        std::fs::remove_dir(&home).unwrap();
    } else if kind == "save" {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(home.join("absent/settings.json"), &path).unwrap();
        }
        #[cfg(not(unix))]
        panic!("Unix symlink fixture");
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
    let display = runtime.block_on(AdvisorCommand.execute("", &mut context));
    assert!(
        matches!(display, CommandResult::Message(_)),
        "display fallback changed: {display:?}"
    );
    if let Some(bytes) = bytes {
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    if kind == "parent" {
        assert!(!home.exists());
    }
    let arg = if kind == "invalid" {
        "not-a-model"
    } else if action == "set" {
        "claude-fixture"
    } else {
        "off"
    };
    let result = runtime.block_on(AdvisorCommand.execute(arg, &mut context));
    if kind == "invalid" {
        assert!(
            matches!(&result, CommandResult::Message(message) if message.contains("Unknown model")),
            "{result:?}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes.unwrap());
        std::fs::write(
            std::env::var_os("ADVISOR_FIXTURE_DONE").unwrap(),
            b"checked",
        )
        .unwrap();
        return;
    }
    let bad_shape = matches!(kind, "array" | "string" | "number" | "bool");
    let must_fail = matches!(kind, "utf8" | "syntax" | "save") || bad_shape && action == "set";
    if must_fail {
        let preserved = if let Some(bytes) = bytes {
            std::fs::read(&path).unwrap() == bytes
        } else {
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
                && !home.join("absent").exists()
        };
        assert!(
            matches!(result, CommandResult::Error(_)) && preserved,
            "expected preserved file and error: {result:?}, preserved={preserved}"
        );
        if let CommandResult::Error(message) = result {
            assert!(message.contains("settings.json"), "missing path: {message}");
            assert!(!message.contains("private-fixture-value"));
        }
    } else {
        assert!(matches!(result, CommandResult::Message(_)), "{result:?}");
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        if bad_shape || kind == "null" && action == "off" {
            assert_eq!(
                saved,
                serde_json::from_slice::<serde_json::Value>(bytes.unwrap()).unwrap()
            );
        } else {
            assert!(saved.is_object());
            if action == "set" {
                assert_eq!(saved["advisorModel"], "claude-fixture");
            } else {
                assert!(saved.get("advisorModel").is_none());
            }
            if kind == "valid" {
                assert_eq!(
                    saved["extra"],
                    serde_json::json!({"nested":[1,true,"retained"]})
                );
            }
        }
    }
    std::fs::write(
        std::env::var_os("ADVISOR_FIXTURE_DONE").unwrap(),
        b"checked",
    )
    .unwrap();
}
