// providers/claude_cli_policy.rs — Coven's permission policy for the Claude CLI.
//
// The CLI runs its own tools under its own permission system. Started bare, it
// uses its headless default mode plus whatever the operator's `~/.claude` and
// the workspace's `.claude` files allow, so one chat behaves differently on
// each machine and the engine's own permission never reaches it. Coven hands
// its policy to a headless engine in `COVEN_CLAUDE_POLICY`
// (OpenCoven/coven#1738), and every CLI launch then carries it. Without the
// variable (an interactive or standalone engine, or an older Coven) the CLI
// starts as it always has.

use serde::Deserialize;
use std::path::Path;

/// Set by Coven to a JSON object: `version`, `settingsFile`, `settingSources`,
/// `permissionMode`, `addDirs` and `mcpConfigFile`.
pub const CLAUDE_POLICY_ENV: &str = "COVEN_CLAUDE_POLICY";

const SUPPORTED_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudePolicy {
    /// Claude Code settings that sit above every settings file.
    settings_file: String,
    /// The settings files the CLI still reads: a comma-separated subset of
    /// `user`, `project` and `local`, or nothing.
    setting_sources: String,
    /// The CLI's name for the engine's permission. Absent leaves the CLI in
    /// its headless default, which refuses whatever no rule allows.
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    add_dirs: Vec<String>,
    /// The operator's own MCP servers, which the user source would have read.
    #[serde(default)]
    mcp_config_file: Option<String>,
}

/// The policy in this process's environment, as CLI arguments.
pub(crate) fn args_from_environment() -> Result<Vec<String>, String> {
    match std::env::var(CLAUDE_POLICY_ENV) {
        Ok(value) => policy_args(Some(&value)),
        Err(std::env::VarError::NotPresent) => policy_args(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(unusable("it is not UTF-8")),
    }
}

/// The CLI arguments for the policy in `value`. A policy this engine cannot
/// apply in full is an error: starting the CLI without it would quietly put
/// the operator's own settings back in charge.
fn policy_args(value: Option<&str>) -> Result<Vec<String>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    // The version is read first, so a newer policy is reported as newer
    // rather than as a list of fields this engine does not know.
    let raw: serde_json::Value =
        serde_json::from_str(value).map_err(|error| unusable(&error.to_string()))?;
    match raw.get("version").and_then(serde_json::Value::as_u64) {
        Some(version) if version == u64::from(SUPPORTED_VERSION) => {}
        Some(version) => {
            return Err(unusable(&format!(
                "it is version {version}, and this engine applies version {SUPPORTED_VERSION}"
            )))
        }
        None => return Err(unusable("it has no version")),
    }
    let policy: ClaudePolicy =
        serde_json::from_value(raw).map_err(|error| unusable(&error.to_string()))?;
    if !Path::new(&policy.settings_file).is_absolute()
        || !Path::new(&policy.settings_file).is_file()
    {
        return Err(unusable("its settings file does not exist"));
    }
    if !policy
        .setting_sources
        .chars()
        .all(|c| c.is_ascii_lowercase() || c == ',')
    {
        return Err(unusable("its setting sources are not a list of names"));
    }
    // Each value follows a flag, so none may read as a flag itself.
    let is_value = |value: &str| !value.is_empty() && !value.starts_with('-');
    let mut args = Vec::new();
    for directory in &policy.add_dirs {
        if !is_value(directory) {
            return Err(unusable("it names an unusable directory"));
        }
        args.extend(["--add-dir".to_string(), directory.clone()]);
    }
    if let Some(file) = &policy.mcp_config_file {
        if !Path::new(file).is_absolute() {
            return Err(unusable("its MCP config file is not an absolute path"));
        }
        args.extend(["--mcp-config".to_string(), file.clone()]);
    }
    if let Some(mode) = &policy.permission_mode {
        if mode.is_empty() || !mode.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(unusable("its permission mode is not a mode name"));
        }
        args.extend(["--permission-mode".to_string(), mode.clone()]);
    }
    // `--add-dir` and `--mcp-config` take several values. These two take one
    // each and come last, so nothing after them can be read as a directory.
    args.extend([
        "--setting-sources".to_string(),
        policy.setting_sources,
        "--settings".to_string(),
        policy.settings_file,
    ]);
    Ok(args)
}

fn unusable(reason: &str) -> String {
    format!(
        "Coven's Claude Code permission policy ({CLAUDE_POLICY_ENV}) can't be applied: {reason}. Update Coven and its engine together, then try again."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A scratch directory that is removed again when the test ends.
    struct Scratch(std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn settings_file() -> (Scratch, String) {
        let dir =
            std::env::temp_dir().join(format!("coven-claude-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("claude-settings.json");
        std::fs::write(&file, "{}").unwrap();
        let path = file.to_str().unwrap().to_owned();
        (Scratch(dir), path)
    }

    #[test]
    fn no_policy_leaves_the_cli_launch_as_it_was() {
        assert_eq!(policy_args(None).unwrap(), Vec::<String>::new());
        assert_eq!(policy_args(Some("  ")).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn a_full_policy_becomes_cli_arguments_ending_in_single_value_flags() {
        let (_dir, settings) = settings_file();
        let policy = json!({
            "version": 1,
            "settingsFile": settings,
            "settingSources": "project",
            "permissionMode": "bypassPermissions",
            "addDirs": ["/granted/one", "/granted/two"],
            "mcpConfigFile": "/operator/.claude.json",
        });
        assert_eq!(
            policy_args(Some(&policy.to_string())).unwrap(),
            [
                "--add-dir",
                "/granted/one",
                "--add-dir",
                "/granted/two",
                "--mcp-config",
                "/operator/.claude.json",
                "--permission-mode",
                "bypassPermissions",
                "--setting-sources",
                "project",
                "--settings",
                settings.as_str(),
            ]
        );
    }

    #[test]
    fn optional_parts_may_be_null_or_absent() {
        let (_dir, settings) = settings_file();
        let expected = ["--setting-sources", "", "--settings", settings.as_str()];
        let explicit = json!({
            "version": 1, "settingsFile": settings, "settingSources": "",
            "permissionMode": null, "addDirs": [], "mcpConfigFile": null,
        });
        assert_eq!(policy_args(Some(&explicit.to_string())).unwrap(), expected);
        let minimal = json!({"version": 1, "settingsFile": settings, "settingSources": ""});
        assert_eq!(policy_args(Some(&minimal.to_string())).unwrap(), expected);
    }

    #[test]
    fn a_policy_that_cannot_be_applied_in_full_is_refused() {
        let (_dir, settings) = settings_file();
        let valid = json!({
            "version": 1, "settingsFile": settings, "settingSources": "project",
        });
        let with = |key: &str, value: serde_json::Value| {
            let mut policy = valid.clone();
            policy[key] = value;
            policy.to_string()
        };
        for (policy, reason) in [
            ("not json".to_owned(), "can't be applied"),
            ("{}".to_owned(), "no version"),
            (r#"{"version":1}"#.to_owned(), "missing field"),
            (r#"{"version":2}"#.to_owned(), "version 2"),
            (with("version", json!(2)), "version 2"),
            (
                with("settingsFile", json!("/absent/policy.json")),
                "does not exist",
            ),
            (
                with("settingsFile", json!("relative.json")),
                "does not exist",
            ),
            (
                with("settingSources", json!("project --flag")),
                "setting sources",
            ),
            (
                with("permissionMode", json!("plan --bare")),
                "permission mode",
            ),
            (with("permissionMode", json!("")), "permission mode"),
            (
                with("addDirs", json!(["--dangerously-skip-permissions"])),
                "directory",
            ),
            (with("addDirs", json!([""])), "directory"),
            (with("mcpConfigFile", json!("-x")), "MCP config"),
        ] {
            let error = policy_args(Some(&policy)).unwrap_err();
            assert!(error.contains(reason), "{policy}: {error}");
            assert!(error.contains(CLAUDE_POLICY_ENV), "{error}");
        }
    }
}
