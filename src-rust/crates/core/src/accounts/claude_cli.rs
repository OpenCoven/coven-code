//! Named subscription profiles keep their credential storage inside Claude Code.
use super::{account_dir, PROVIDER_ANTHROPIC};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Env var overriding the `claude` binary location.
pub const CLAUDE_BIN_ENV: &str = "COVEN_CODE_CLAUDE_BIN";

/// Locate the `claude` binary: explicit env override first, then PATH.
pub fn resolve_claude_binary() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var(CLAUDE_BIN_ENV) {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    let path = std::env::var_os("PATH")?;
    let names: &[&str] = if cfg!(windows) {
        &["claude.cmd", "claude.exe", "claude"]
    } else {
        &["claude"]
    };
    for dir in std::env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn config_dir(id: &str) -> PathBuf {
    account_dir(PROVIDER_ANTHROPIC, id).join("claude-config")
}

pub fn configure(command: &mut Command, directory: &Path) {
    command.env("CLAUDE_CONFIG_DIR", directory);
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "CLAUDE_CODE_USE_GATEWAY",
    ] {
        command.env_remove(key);
    }
}

pub async fn signed_in(directory: &Path) -> bool {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Status {
        logged_in: bool,
        auth_method: String,
    }
    if !directory.is_dir() {
        return false;
    }
    let Some(binary) = resolve_claude_binary() else {
        return false;
    };
    let mut cmd = Command::new(binary);
    configure(&mut cmd, directory);
    cmd.args(["auth", "status", "--json"]).kill_on_drop(true);
    let Ok(Ok(output)) =
        tokio::time::timeout(std::time::Duration::from_secs(10), cmd.output()).await
    else {
        return false;
    };
    output.status.success()
        && serde_json::from_slice::<Status>(&output.stdout)
            .is_ok_and(|s| s.logged_in && s.auth_method == "claude.ai")
}
