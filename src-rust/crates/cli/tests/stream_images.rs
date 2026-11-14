//! Real stream-json binary → query loop → keyless Claude adapter, without inference.
#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQVDIGAACuAGcVHqFfAAAAAElFTkSuQmCC";

fn run_frame(content: Value) -> (String, Vec<Value>) {
    let scratch = tempfile::tempdir().unwrap();
    let fake = scratch.path().join("claude");
    // All arguments are constant; the fixture records exact provider input.
    fs::write(&fake, r#"#!/bin/sh
cat > "$COVEN_HOME/provider-input.json"
printf '%s\n' '{"type":"system","subtype":"init","session_id":"image-fixture"}' '{"type":"assistant","message":{"content":[{"type":"text","text":"fixture accepted"}]}}' '{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":1,"output_tokens":1}}'
"#).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_coven-code"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", scratch.path().join("home"))
        .env("USER", "image-fixture")
        .env("COVEN_HOME", scratch.path().join("coven"))
        .env("COVEN_CODE_CLAUDE_BIN", &fake)
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .current_dir(scratch.path())
        .args([
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        json!({"type":"user","message":{"role":"user","content":content}})
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "engine timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frames = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let provider =
        fs::read_to_string(scratch.path().join("coven/provider-input.json")).unwrap_or_default();
    (provider, frames)
}

#[test]
fn streamed_image_reaches_real_query_and_claude_adapter() {
    let image =
        json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}});
    let (provider, frames) = run_frame(json!([{"type":"text","text":"inspect this pixel"},image]));
    assert!(
        frames
            .iter()
            .any(|frame| frame["type"] == "result" && frame["is_error"] == false),
        "{frames:?}"
    );
    let input: Value = serde_json::from_str(&provider).unwrap();
    let blocks = input["message"]["content"].as_array().unwrap();
    assert!(
        blocks.contains(&image),
        "image bytes lost before provider: {input}"
    );
    assert!(blocks.iter().any(|block| block["text"]
        .as_str()
        .is_some_and(|text| text.contains("inspect this pixel"))));
}

#[test]
fn interleaved_image_captions_reach_provider_in_exact_order() {
    let image =
        json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}});
    let expected =
        json!([{"type":"text","text":"caption A"},image,{"type":"text","text":"caption B"},image]);
    let (provider, _) = run_frame(expected.clone());
    let input: Value = serde_json::from_str(&provider).unwrap();
    assert_eq!(input["message"]["content"], expected);
}

#[test]
fn image_only_frame_executes_a_turn() {
    let (provider, frames) = run_frame(
        json!([{"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}}]),
    );
    assert!(
        !provider.is_empty(),
        "image-only input was ignored: {frames:?}"
    );
}

#[test]
fn malformed_image_refuses_entire_prompt_with_terminal_error() {
    let (provider, frames) = run_frame(
        json!([{"type":"text","text":"must never execute"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"bm90IGFuIGltYWdl"}}]),
    );
    assert!(
        provider.is_empty(),
        "partial prompt was dispatched: {provider}"
    );
    assert!(
        frames
            .iter()
            .any(|frame| frame["type"] == "result" && frame["is_error"] == true),
        "missing refusal receipt: {frames:?}"
    );
}
