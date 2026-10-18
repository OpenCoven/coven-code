#![cfg(unix)]

use claurst_commands::{
    named_commands::{AddDirCommand, NamedCommand},
    CommandContext, CommandResult,
};
use claurst_core::{config::Config, cost::CostTracker, Settings};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_case(case: &str) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let target = temp.path().join("target");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&target).unwrap();
    let mut settings = Settings::default();
    if case == "limited" {
        settings.config.model = Some("x".repeat(16_384));
    }
    let original = if case == "read-error" {
        vec![0xff, 0xfe]
    } else {
        serde_json::to_vec_pretty(&settings).unwrap()
    };
    std::fs::write(home.join("settings.json"), &original).unwrap();
    let mut command = if case == "limited" {
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' XFSZ; ulimit -f 1 || exit 90; exec \"$@\"",
                "fixture",
            ])
            .arg(std::env::current_exe().unwrap());
        command
    } else {
        Command::new(std::env::current_exe().unwrap())
    };
    let mut child = command
        .args(["--exact", "add_dir_worker", "--ignored", "--nocapture"])
        .env_clear()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("COVEN_CODE_HOME", &home)
        .env("PATH", &home)
        .env("ADD_DIR_CASE", case)
        .env("ADD_DIR_TARGET", &target)
        .current_dir(&home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Drain a pipe: regular-file diagnostics would also be subject to RLIMIT_FSIZE.
    let mut stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut output = String::new();
        stderr.read_to_string(&mut output).unwrap();
        output
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                panic!("{case}: {other:?}");
            }
        }
    };
    let output = reader.join().unwrap();
    assert!(status.success(), "{case}: {status}: {output}");
    assert_eq!(std::fs::read(home.join("completed")).unwrap(), b"checked");
    if case == "read-error" {
        assert_eq!(std::fs::read(home.join("settings.json")).unwrap(), original);
    }
    assert_eq!(std::fs::read_dir(target).unwrap().count(), 0);
}

#[test]
fn failed_save_does_not_claim_session_access() {
    run_case("limited");
}
#[test]
fn successful_save_and_repeat_preserve_behavior() {
    run_case("normal");
}
#[test]
fn load_error_preserves_settings() {
    run_case("read-error");
}

#[test]
#[ignore = "owned settings process with optional file-size limit"]
fn add_dir_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(Settings::config_dir(), home);
    let target = std::fs::canonicalize(std::env::var_os("ADD_DIR_TARGET").unwrap()).unwrap();
    let case = std::env::var("ADD_DIR_CASE").unwrap();
    let ctx = CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: vec![],
        working_dir: home.clone(),
        session_id: "pre-session".into(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    let config = serde_json::to_vec(&ctx.config).unwrap();
    if case == "read-error" {
        assert_eq!(
            Settings::load_sync()
                .unwrap_err()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
    } else {
        let settings = Settings::load_sync().unwrap();
        assert!(!settings.config.workspace_paths.contains(&target));
        if case == "limited" {
            assert_eq!(settings.config.model.as_deref().unwrap().len(), 16_384);
            let probe = home.join("limit-probe");
            let error = std::fs::write(&probe, vec![0u8; 16_384]).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::FileTooLarge);
            std::fs::remove_file(probe).unwrap();
        }
    }
    let result = AddDirCommand.execute_named(&[target.to_str().unwrap()], &ctx);
    assert_eq!(serde_json::to_vec(&ctx.config).unwrap(), config);
    assert_eq!(ctx.session_id, "pre-session");
    assert!(ctx.messages.is_empty());
    match case.as_str() {
        "limited" => {
            let CommandResult::Error(message) = result else {
                panic!("{result:?}");
            };
            assert!(message.contains(&target.display().to_string()), "{message}");
            assert!(
                message.to_lowercase().contains("file too large"),
                "{message}"
            );
            assert!(
                !message.contains("Added") && !message.contains("this session"),
                "{message}"
            );
            assert!(
                message.contains("Failed to save workspace path"),
                "{message}"
            );
        }
        "read-error" => {
            let CommandResult::Error(message) = result else {
                panic!("{result:?}");
            };
            assert!(
                message.starts_with("Failed to load settings before updating workspace paths:"),
                "{message}"
            );
        }
        "normal" => {
            let expected = format!("Added {} to allowed workspace paths.", target.display());
            let CommandResult::Message(message) = result else {
                panic!("{result:?}");
            };
            assert_eq!(message, expected);
            assert_eq!(
                Settings::load_sync().unwrap().config.workspace_paths,
                std::slice::from_ref(&target)
            );
            let saved = std::fs::read(home.join("settings.json")).unwrap();
            let CommandResult::Message(repeated) =
                AddDirCommand.execute_named(&[target.to_str().unwrap()], &ctx)
            else {
                panic!("repeat failed");
            };
            assert_eq!(repeated, expected);
            assert_eq!(std::fs::read(home.join("settings.json")).unwrap(), saved);
            assert_eq!(serde_json::to_vec(&ctx.config).unwrap(), config);
        }
        _ => unreachable!(),
    }
    std::fs::write(home.join("completed"), b"checked").unwrap();
}
