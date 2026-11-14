// providers/claude_cli_policy.rs — Coven's permission policy for the Claude CLI.
//
// The CLI runs its own tools under its own permission system. Started bare, it
// reads whatever the operator's `~/.claude` and the workspace's `.claude` files
// allow, so one chat behaves differently on each machine. Coven hands its
// policy to a headless engine in `COVEN_CLAUDE_POLICY` (OpenCoven/coven#1738),
// and every CLI launch then carries it. Without the variable (an interactive
// or standalone engine, or an older Coven) the CLI starts as it always has.
//
// The policy says which rules and settings files apply. It does not name a
// permission mode: each turn's own mode reaches the CLI through `build_args`,
// so exactly one `--permission-mode` is ever passed.
//
// It names two settings files. The CLI applies allow rules in plan mode too,
// so a read-only turn gets the file without Coven's allow list and every
// other turn gets the full one.

use serde::Deserialize;
use std::path::Path;

/// Set by Coven to a JSON object: `version`, `settingsFile`,
/// `readOnlySettingsFile`, `settingSources`, `addDirs` and `mcpConfigFile`.
pub const CLAUDE_POLICY_ENV: &str = "COVEN_CLAUDE_POLICY";

const SUPPORTED_VERSION: u32 = 1;

// A field this engine does not know is a part of the policy it cannot apply,
// so it is refused rather than dropped.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClaudePolicy {
    /// Checked against `SUPPORTED_VERSION` before the rest is read.
    #[serde(rename = "version")]
    _version: u32,
    /// Claude Code settings that sit above every settings file.
    settings_file: String,
    /// The same, without the allow rules, for a read-only turn.
    read_only_settings_file: String,
    /// The settings files the CLI still reads: a comma-separated subset of
    /// `user`, `project` and `local`, or nothing.
    setting_sources: String,
    #[serde(default)]
    add_dirs: Vec<String>,
    /// The operator's own MCP servers, which the user source would have read.
    #[serde(default)]
    mcp_config_file: Option<String>,
}

/// The policy in this process's environment, as CLI arguments for one turn.
/// `read_only` says whether that turn reaches the CLI in plan mode.
pub(crate) fn args_from_environment(read_only: bool) -> Result<Vec<String>, String> {
    match std::env::var(CLAUDE_POLICY_ENV) {
        Ok(value) => policy_args(Some(&value), read_only),
        Err(std::env::VarError::NotPresent) => policy_args(None, read_only),
        Err(std::env::VarError::NotUnicode(_)) => Err(unusable("it is not UTF-8")),
    }
}

/// The CLI arguments for the policy in `value`. A policy this engine cannot
/// apply in full is an error: starting the CLI without it would quietly put
/// the operator's own settings back in charge.
fn policy_args(value: Option<&str>, read_only: bool) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(unusable("it is empty"));
    }
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
    // Both are checked whichever this turn uses, so a policy with a missing
    // file fails on the first turn and not on the first read-only one.
    for file in [&policy.settings_file, &policy.read_only_settings_file] {
        if !Path::new(file).is_absolute() || !Path::new(file).is_file() {
            return Err(unusable("one of its settings files does not exist"));
        }
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
    // `--add-dir` and `--mcp-config` take several values. These two take one
    // each and come last, so nothing after them can be read as a directory.
    args.extend([
        "--setting-sources".to_string(),
        policy.setting_sources,
        "--settings".to_string(),
        if read_only {
            policy.read_only_settings_file
        } else {
            policy.settings_file
        },
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

    /// The full settings file and its read-only form.
    fn settings_files() -> (Scratch, String, String) {
        let dir =
            std::env::temp_dir().join(format!("coven-claude-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = |name: &str| {
            let file = dir.join(name);
            std::fs::write(&file, "{}").unwrap();
            file.to_str().unwrap().to_owned()
        };
        let (settings, read_only) = (
            path("claude-settings.json"),
            path("claude-settings-read-only.json"),
        );
        (Scratch(dir), settings, read_only)
    }

    #[test]
    fn no_policy_leaves_the_cli_launch_as_it_was() {
        for read_only in [false, true] {
            assert_eq!(policy_args(None, read_only).unwrap(), Vec::<String>::new());
        }
    }

    #[test]
    fn a_full_policy_becomes_cli_arguments_ending_in_single_value_flags() {
        let (_dir, settings, read_only_settings) = settings_files();
        let policy = json!({
            "version": 1,
            "settingsFile": settings,
            "readOnlySettingsFile": read_only_settings,
            "settingSources": "project",
            "addDirs": ["/granted/one", "/granted/two"],
            "mcpConfigFile": "/operator/.claude.json",
        });
        // A read-only turn differs in the settings file and nothing else.
        for (read_only, file) in [(false, &settings), (true, &read_only_settings)] {
            let args = policy_args(Some(&policy.to_string()), read_only).unwrap();
            assert_eq!(
                args,
                [
                    "--add-dir",
                    "/granted/one",
                    "--add-dir",
                    "/granted/two",
                    "--mcp-config",
                    "/operator/.claude.json",
                    "--setting-sources",
                    "project",
                    "--settings",
                    file.as_str(),
                ]
            );
            // The turn's own mode is the only one the CLI is given.
            assert!(!args.iter().any(|arg| arg == "--permission-mode"));
        }
    }

    #[test]
    fn optional_parts_may_be_null_or_absent() {
        let (_dir, settings, read_only_settings) = settings_files();
        let expected = ["--setting-sources", "", "--settings", settings.as_str()];
        let explicit = json!({
            "version": 1, "settingsFile": settings,
            "readOnlySettingsFile": read_only_settings, "settingSources": "",
            "addDirs": [], "mcpConfigFile": null,
        });
        assert_eq!(
            policy_args(Some(&explicit.to_string()), false).unwrap(),
            expected
        );
        let minimal = json!({
            "version": 1, "settingsFile": settings,
            "readOnlySettingsFile": read_only_settings, "settingSources": "",
        });
        assert_eq!(
            policy_args(Some(&minimal.to_string()), false).unwrap(),
            expected
        );
    }

    #[test]
    fn a_policy_that_cannot_be_applied_in_full_is_refused() {
        let (_dir, settings, read_only_settings) = settings_files();
        let valid = json!({
            "version": 1, "settingsFile": settings,
            "readOnlySettingsFile": read_only_settings, "settingSources": "project",
        });
        let without_read_only = {
            let mut policy = valid.clone();
            policy
                .as_object_mut()
                .unwrap()
                .remove("readOnlySettingsFile");
            policy.to_string()
        };
        let with = |key: &str, value: serde_json::Value| {
            let mut policy = valid.clone();
            policy[key] = value;
            policy.to_string()
        };
        for (policy, reason) in [
            ("".to_owned(), "it is empty"),
            (" \t\n".to_owned(), "it is empty"),
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
            // A policy without the read-only file would leave a read-only
            // turn with the allow list, so it is refused on every turn.
            (without_read_only, "missing field `readOnlySettingsFile`"),
            (
                with("readOnlySettingsFile", json!("/absent/read-only.json")),
                "does not exist",
            ),
            (
                with("settingSources", json!("project --flag")),
                "setting sources",
            ),
            // The turn's own mode is the only one: a policy may not name another.
            (
                with("permissionMode", json!("bypassPermissions")),
                "unknown field `permissionMode`",
            ),
            (with("permissionMode", json!(null)), "unknown field"),
            (with("futureRule", json!(true)), "unknown field"),
            (
                with("addDirs", json!(["--dangerously-skip-permissions"])),
                "directory",
            ),
            (with("addDirs", json!([""])), "directory"),
            (with("mcpConfigFile", json!("-x")), "MCP config"),
        ] {
            for read_only in [false, true] {
                let error = policy_args(Some(&policy), read_only).unwrap_err();
                assert!(error.contains(reason), "{policy}: {error}");
                assert!(error.contains(CLAUDE_POLICY_ENV), "{error}");
            }
        }
    }
}
