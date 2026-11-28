#![cfg(unix)]

use claurst_core::accounts::AccountRegistry;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn named_logins_stay_isolated_and_only_success_registers_a_profile() {
    let home = tempfile::tempdir().unwrap();
    let cli = home.path().join("claude");
    fs::write(
        &cli,
        r#"#!/bin/sh
set -eu
test -z "${ANTHROPIC_API_KEY:-}"
case "$*" in
  'auth login --claudeai')
    test -z "${CANCEL:-}" || exit 130
    touch "$CLAUDE_CONFIG_DIR/signed-in" ;;
  'auth status --json')
    test -f "$CLAUDE_CONFIG_DIR/signed-in"
    echo '{"loggedIn":true,"authMethod":"claude.ai"}' ;;
  *) exit 1 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    let login = || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_coven-code"));
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.path())
            .env("COVEN_CODE_HOME", home.path())
            .env("COVEN_CODE_CLAUDE_BIN", &cli)
            .env("ANTHROPIC_API_KEY", "fixture")
            .args(["auth", "login"]);
        cmd
    };
    assert!(login().arg("--help").output().unwrap().status.success());
    assert!(!home.path().join("accounts").exists());
    for label in ["work", "personal"] {
        let result = login().args(["--label", label]).output().unwrap();
        assert!(result.status.success(), "{:?}", result.stderr);
        assert!(home
            .path()
            .join("accounts/anthropic")
            .join(label)
            .join("claude-config/signed-in")
            .is_file());
    }
    let result = login().env("CANCEL", "1").output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("canceled"));
    let registry: AccountRegistry =
        serde_json::from_slice(&fs::read(home.path().join("accounts.json")).unwrap()).unwrap();
    let accounts = &registry.providers["anthropic"];
    assert!(accounts.active.is_none());
    assert_eq!(accounts.profiles.len(), 2);
    assert!(accounts.profiles.values().all(|profile| profile.claude_cli));
}
