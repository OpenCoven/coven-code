#![cfg(unix)]

use claurst_commands::{
    named_commands::{NamedCommand, PrCommentsCommand},
    CommandContext, CommandResult,
};
use claurst_core::{config::Config, cost::CostTracker};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn pr_comments_uses_context_directory_for_both_calls() {
    run_case("success");
}

#[test]
fn pr_comments_errors_are_hermetic() {
    for case in [
        "missing",
        "pr-failure",
        "pr-json",
        "pr-zero",
        "comments-failure",
        "comments-json",
        "comments-empty",
    ] {
        run_case(case);
    }
}

// Only shell builtins are used; PATH contains this fixture directory alone.
const FAKE_GH: &str = r#"#!/bin/sh
printf 'cwd=%s\n' "$PWD" >> "$GH_FIXTURE_LOG"
for arg in "$@"; do printf 'arg=%s\n' "$arg" >> "$GH_FIXTURE_LOG"; done
printf 'end\n' >> "$GH_FIXTURE_LOG"
case "$1" in
  pr)
    case "$GH_FIXTURE_CASE" in
      pr-failure) printf 'fixture no PR\n' >&2; exit 1 ;;
      pr-json) printf 'not JSON'; exit 0 ;;
      pr-zero) printf '{"number":0}'; exit 0 ;;
    esac
    printf '{"number":42,"url":"https://example.invalid/pull/42","headRefName":"feature","baseRefName":"main"}'
    ;;
  api)
    case "$GH_FIXTURE_CASE" in
      comments-failure) printf 'fixture comments failure\n' >&2; exit 1 ;;
      comments-json) printf 'not JSON'; exit 0 ;;
      comments-empty) printf '[]'; exit 0 ;;
    esac
    printf '[{"path":"src/fixture.rs","line":7,"user":{"login":"fixture-user"},"body":"  review text 界  "}]'
    ;;
  *) printf 'unexpected invocation\n' >&2; exit 99 ;;
esac
"#;

fn run_case(case: &str) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let home = root.join("home");
    let process_dir = root.join("process");
    let context_dir = root.join("context");
    let bin = root.join("bin");
    for path in [&home, &process_dir, &context_dir, &bin] {
        std::fs::create_dir(path).unwrap();
    }
    if case != "missing" {
        let script = bin.join("gh");
        std::fs::write(&script, FAKE_GH).unwrap();
        std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let stderr = root.join("stderr");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "pr_comments_worker", "--ignored", "--nocapture"])
        .env_clear()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("COVEN_CODE_HOME", &home)
        .env("PATH", &bin)
        .env("GH_FIXTURE_CASE", case)
        .env("GH_FIXTURE_LOG", root.join("calls"))
        .env("GH_FIXTURE_CONTEXT", &context_dir)
        .current_dir(&process_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&stderr).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{case}: {result:?}");
            }
        }
    };
    assert!(
        status.success(),
        "{case}: {}",
        std::fs::read_to_string(stderr).unwrap()
    );
    assert_eq!(std::fs::read(home.join("done")).unwrap(), b"checked");
    assert_eq!(std::fs::read_dir(context_dir).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(process_dir).unwrap().count(), 0);
}

#[test]
#[ignore = "isolated executable search path and directories"]
fn pr_comments_worker() {
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(claurst_core::Settings::config_dir(), home);
    let working_dir = std::path::PathBuf::from(std::env::var_os("GH_FIXTURE_CONTEXT").unwrap());
    let process_dir = std::env::current_dir().unwrap();
    assert_ne!(working_dir, process_dir);
    let case = std::env::var("GH_FIXTURE_CASE").unwrap();
    let log = std::path::PathBuf::from(std::env::var_os("GH_FIXTURE_LOG").unwrap());
    let context = CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: vec![],
        working_dir: working_dir.clone(),
        session_id: "fixture-session".into(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    let result = PrCommentsCommand.execute_named(&[], &context);
    let header = "PR #42 — https://example.invalid/pull/42\n\n";
    let (is_error, expected) = match case.as_str() {
        "missing" => (true, "GitHub CLI (gh) not found. Install from https://cli.github.com".into()),
        "pr-failure" => (true, "No open PR found: fixture no PR".into()),
        "pr-json" => (true, "Failed to parse gh output".into()),
        "pr-zero" => (true, "Could not determine PR number.".into()),
        "comments-failure" => (false, format!("{header}Could not fetch review comments (check gh auth).\n")),
        "comments-json" => (false, format!("{header}Could not parse review comments.\n")),
        "comments-empty" => (false, format!("{header}No review comments found.\n")),
        "success" => (false, format!("{header}Review comments (1):\n  src/fixture.rs:7 by @fixture-user:\n    review text 界\n\n")),
        _ => panic!("unknown case"),
    };
    match result {
        CommandResult::Error(text) if is_error => assert_eq!(text, expected),
        CommandResult::Message(text) if !is_error => assert_eq!(text, expected),
        other => panic!("unexpected result: {other:?}"),
    }
    if case == "missing" {
        assert!(!log.exists());
    } else {
        let calls = std::fs::read_to_string(log).unwrap();
        let records: Vec<_> = calls
            .split("end\n")
            .filter(|record| !record.is_empty())
            .collect();
        let expected_count = if is_error { 1 } else { 2 };
        assert_eq!(records.len(), expected_count);
        let mut directories = Vec::new();
        for (index, record) in records.iter().enumerate() {
            let (cwd, args) = record.split_once('\n').unwrap();
            directories.push(cwd);
            let expected_args = if index == 0 {
                "arg=pr\narg=view\narg=--json\narg=number,url,headRefName,baseRefName\n"
            } else {
                "arg=api\narg=repos/{owner}/{repo}/pulls/42/comments\n"
            };
            assert_eq!(args, expected_args);
        }
        if case == "success" {
            assert_eq!(
                directories,
                vec![format!("cwd={}", working_dir.display()); 2]
            );
        }
    }
    assert_eq!(std::env::current_dir().unwrap(), process_dir);
    assert_eq!(context.working_dir, working_dir);
    std::fs::write(home.join("done"), b"checked").unwrap();
}
