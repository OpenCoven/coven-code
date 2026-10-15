//! Public global dispatch tests run in child processes to isolate the registry.
#![cfg(unix)]

use claurst_plugins::{
    run_global_post_tool_hook, run_global_pre_tool_hook, set_global_hooks, HookOutcome,
    HookRegistry, RegisteredHook,
};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn global_pre_tool_matchers() {
    run_case("pre");
}

#[test]
fn global_post_tool_matchers() {
    run_case("post");
}

fn run_case(mode: &str) {
    let directory = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "global_hook_fixture", "--ignored", "--nocapture"])
        .env("COVEN_HOOK_MATCH_CASE", mode)
        .env("COVEN_HOOK_MATCH_DIR", directory.path())
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{mode} fixture failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &format!("-{}", child.id())])
                .status();
            let _ = child.kill();
            let _ = child.wait();
            panic!("{mode} fixture exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "invoked by the parent tests in an isolated process"]
fn global_hook_fixture() {
    let mode = std::env::var("COVEN_HOOK_MATCH_CASE").unwrap();
    let directory = std::path::PathBuf::from(std::env::var_os("COVEN_HOOK_MATCH_DIR").unwrap());
    let input = serde_json::json!({"path": "fixture"});
    for (matcher, tool, matches) in [
        (None, "FileRead", true),
        (Some(""), "FileRead", true),
        (Some("*"), "FileRead", true),
        (Some("FileRead"), "FileRead", true),
        (Some("File*"), "FileRead", true),
        (Some("*Read"), "FileRead", true),
        (Some("File*"), "File", true),
        (Some("*Read"), "Read", true),
        (Some("\u{e9}*"), "\u{e9}Tool", true),
        (Some("*\u{e9}"), "Tool\u{e9}", true),
        (Some("Write"), "FileRead", false),
        (Some("File*"), "OtherTool", false),
        (Some("*Read"), "FileWrite", false),
        (Some("file*"), "FileRead", false),
    ] {
        let output_path = directory.join("event.json");
        if output_path.exists() {
            std::fs::remove_file(&output_path).unwrap();
        }
        let command = if mode == "pre" {
            "cat > \"$CLAUDE_PLUGIN_ROOT/event.json\"; printf 'fixture denial' >&2; exit 7"
        } else {
            "cat > \"$CLAUDE_PLUGIN_ROOT/event.json\""
        };
        let hook = RegisteredHook {
            command: command.into(),
            matcher: matcher.map(str::to_owned),
            blocking: true,
            plugin_root: directory.to_str().unwrap().into(),
            plugin_name: "matcher-fixture".into(),
            plugin_source: "fixture".into(),
        };
        let event = if mode == "pre" {
            "PreToolUse"
        } else {
            "PostToolUse"
        };
        let mut registry = HookRegistry::new();
        registry.insert(event.into(), vec![hook]);
        set_global_hooks(registry);
        if mode == "pre" {
            match run_global_pre_tool_hook(tool, &input) {
                HookOutcome::Deny(reason) if matches => assert!(reason.contains("fixture denial")),
                HookOutcome::Allow if !matches => {}
                outcome => panic!("unexpected {outcome:?} for {matcher:?} and {tool}"),
            }
        } else {
            run_global_post_tool_hook(tool, &input, "fixture output", true);
        }
        assert_eq!(output_path.exists(), matches, "{mode}: {matcher:?}, {tool}");
        if matches {
            let actual: serde_json::Value =
                serde_json::from_slice(&std::fs::read(output_path).unwrap()).unwrap();
            let expected = if mode == "pre" {
                serde_json::json!({"event": event, "tool_name": tool, "tool_input": input})
            } else {
                serde_json::json!({"event": event, "tool_name": tool, "tool_input": input,
                    "tool_output": "fixture output", "is_error": true})
            };
            assert_eq!(actual, expected);
        }
    }
}
