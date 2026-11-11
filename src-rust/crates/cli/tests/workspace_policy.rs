//! Exercise the shipped entrypoint and native sandbox without provider credentials.
#![cfg(target_os = "macos")]

use serde_json::{json, Value};
use std::{fs, process::Command};

#[test]
fn installed_entrypoint_reports_exact_kernel_scope_and_rejects_a_changed_root() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("a-work");
    let project = base.join("b-project");
    let read = base.join("c-read");
    let file = base.join("policy.json");
    for root in [&work, &project, &read] {
        fs::create_dir(root).unwrap();
    }
    let policy = json!({
        "version": 1, "runId": "run-native-test", "access": "workspace-write",
        "primaryRoot": work, "writableRoots": [work, project], "readOnlyRoots": [read],
    });
    fs::write(&file, policy.to_string()).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_coven-code"))
            .arg("--verify-workspace-policy-v1")
            .arg(&file)
            .env("COVEN_HOME", base.join("coven"))
            .env("COVEN_CAVE_HOME", base.join("cave"))
            .current_dir(&work)
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success(), "{:?}", output);
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut expected = policy;
    expected["enforced"] = json!(true);
    assert_eq!(actual, expected);
    fs::rename(&project, base.join("changed")).unwrap();
    let output = run();
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

#[test]
fn scoped_turn_does_not_delegate_to_an_unqualified_claude_cli() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let work = base.join("workspace");
    let file = base.join("policy.json");
    let claude = base.join("claude");
    let marker = base.join("launched");
    fs::create_dir(&work).unwrap();
    fs::write(
        &file,
        json!({"version":1,"runId":"no-delegation","access":"workspace-write",
            "primaryRoot":work,"writableRoots":[work],"readOnlyRoots":[]})
        .to_string(),
    )
    .unwrap();
    fs::write(
        &claude,
        "#!/bin/sh\nprintf started > \"$SCOPED_TEST_MARKER\"\n",
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_coven-code"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &base)
        .env("COVEN_HOME", base.join("coven"))
        .env("COVEN_CAVE_HOME", base.join("cave"))
        .env("COVEN_CODE_HOME", base.join("code"))
        .env("COVEN_CODE_CLAUDE_BIN", &claude)
        .env("SCOPED_TEST_MARKER", &marker)
        .current_dir(&work)
        .args([
            "--print",
            "--output-format",
            "stream-json",
            "--provider",
            "anthropic",
        ])
        .arg("--chat-workspace-policy-v1")
        .arg(&file)
        .args(["--", "Fixture turn"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("requires direct Anthropic credentials"),
        "{output:?}"
    );
    assert!(!marker.exists());
}
