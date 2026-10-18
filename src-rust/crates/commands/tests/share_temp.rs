#![cfg(unix)]

use base64::{engine::general_purpose::STANDARD, Engine};
use claurst_commands::{CommandContext, CommandResult, ShareCommand, SlashCommand};
use claurst_core::{config::Config, cost::CostTracker, types::Message};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn share_preserves_existing_regular_temp_file() {
    run_case("regular");
}
#[test]
fn share_preserves_existing_symlink_and_target() {
    run_case("symlink");
}
#[test]
fn share_temp_cleanup_and_filename_controls() {
    run_case("ordinary");
    run_case("fallback");
}

const FAKE_GH: &str = r#"#!/bin/sh
for arg in "$@"; do printf 'arg=%s\n' "$arg" >> "$SHARE_LOG"; done
printf 'end\n' >> "$SHARE_LOG"
case "$1" in
  auth) [ "$#" -eq 2 ] && [ "$2" = status ] || exit 91; exit 0 ;;
  gist)
    [ "$#" -eq 4 ] && [ "$2" = create ] && [ "$3" = --public=false ] || exit 92
    [ -f "$4" ] || exit 93
    printf '%s' "$4" > "$SHARE_PATH"
    while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "$4" > "$SHARE_HTML"
    printf 'fixture rejected upload\n' >&2
    exit 1 ;;
  *) exit 94 ;;
esac
"#;

fn run_case(case: &str) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let home = root.join("home");
    let staging = root.join("staging");
    let bin = root.join("bin");
    for path in [&home, &staging, &bin] {
        std::fs::create_dir(path).unwrap();
    }
    let gh = bin.join("gh");
    std::fs::write(&gh, FAKE_GH).unwrap();
    std::fs::set_permissions(gh, std::fs::Permissions::from_mode(0o700)).unwrap();
    let stderr = root.join("stderr");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "share_worker", "--ignored", "--nocapture"])
        .env_clear()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("COVEN_CODE_HOME", &home)
        .env("COVEN_HOME", root.join("coven"))
        .env("COVEN_CAVE_HOME", root.join("cave"))
        .env("PATH", &bin)
        .env("TMPDIR", &staging)
        .env("TMP", &staging)
        .env("TEMP", &staging)
        .env("COVEN_CODE_SHARE_NO_OPEN", "1")
        .env("SHARE_CASE", case)
        .env("SHARE_LOG", root.join("calls"))
        .env("SHARE_PATH", root.join("uploaded-path"))
        .env("SHARE_HTML", root.join("rendered-html"))
        .current_dir(&home)
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
}

#[test]
#[ignore = "owned temporary directory and fake gh executable"]
fn share_worker() {
    let home = PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    let staging = PathBuf::from(std::env::var_os("TMPDIR").unwrap());
    assert_eq!(claurst_core::Settings::config_dir(), home);
    assert_eq!(std::env::temp_dir().canonicalize().unwrap(), staging);
    let case = std::env::var("SHARE_CASE").unwrap();
    let (id, basename) = if case == "fallback" {
        ("界/ !", "claurst-session-session.html")
    } else {
        (
            "fixture/session_1!",
            "claurst-session-fixturesession_1.html",
        )
    };
    let predictable = staging.join(basename);
    let victim = home.join("victim");
    if case == "regular" {
        std::fs::write(&predictable, b"existing temp file").unwrap();
    }
    if case == "symlink" {
        std::fs::write(&victim, b"owned victim").unwrap();
        symlink(&victim, &predictable).unwrap();
    }
    let mut context = CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: vec![Message::user("fixture conversation 界")],
        working_dir: home.clone(),
        session_id: id.into(),
        session_title: Some("fixture title".into()),
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    let original_messages = serde_json::to_value(&context.messages).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(ShareCommand.execute("", &mut context));
    let CommandResult::Error(error) = result else {
        panic!("{result:?}");
    };
    assert_eq!(error, "gh gist create failed: fixture rejected upload");
    let upload =
        PathBuf::from(std::fs::read_to_string(std::env::var_os("SHARE_PATH").unwrap()).unwrap());
    assert!(upload.starts_with(&staging));
    assert_eq!(upload.file_name().unwrap(), basename);
    let calls = std::fs::read_to_string(std::env::var_os("SHARE_LOG").unwrap()).unwrap();
    assert_eq!(
        calls,
        format!(
            "arg=auth\narg=status\nend\narg=gist\narg=create\narg=--public=false\narg={}\nend\n",
            upload.display()
        )
    );
    let html = std::fs::read_to_string(std::env::var_os("SHARE_HTML").unwrap()).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<title>fixture title</title>"));
    let payload = html
        .split("id=\"session-data\"")
        .nth(1)
        .unwrap()
        .split_once('>')
        .unwrap()
        .1
        .split('<')
        .next()
        .unwrap();
    let decoded: serde_json::Value =
        serde_json::from_slice(&STANDARD.decode(payload.trim()).unwrap()).unwrap();
    assert_eq!(decoded["messages"], original_messages);
    assert_eq!(decoded["meta"]["session_id"], id);
    assert_eq!(decoded["meta"]["title"], "fixture title");
    assert_eq!(decoded["meta"]["working_dir"], home.to_str().unwrap());
    assert!(!upload.exists(), "owned rendered file was not cleaned up");
    match case.as_str() {
        "regular" => assert_eq!(std::fs::read(&predictable).unwrap(), b"existing temp file"),
        "symlink" => {
            let victim_bytes = std::fs::read(&victim).unwrap();
            assert!(
                victim_bytes == b"owned victim",
                "victim contents changed ({} bytes)",
                victim_bytes.len()
            );
            assert!(std::fs::symlink_metadata(&predictable)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(std::fs::read_link(&predictable).unwrap(), victim);
        }
        "ordinary" | "fallback" => {}
        _ => panic!("unknown case"),
    }
    let entries: Vec<_> = std::fs::read_dir(&staging)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    if matches!(case.as_str(), "regular" | "symlink") {
        assert_eq!(entries, vec![predictable]);
    } else {
        assert!(entries.is_empty());
    }
    assert_eq!(context.session_id, id);
    assert_eq!(context.session_title.as_deref(), Some("fixture title"));
    assert_eq!(
        serde_json::to_value(&context.messages).unwrap(),
        original_messages
    );
    std::fs::write(home.join("done"), b"checked").unwrap();
}
