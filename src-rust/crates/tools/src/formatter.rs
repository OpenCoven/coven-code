// formatter.rs — run a configured file formatter after writes/edits.

use crate::ToolContext;
use std::time::Duration;

/// How long a formatter may run before it is killed with everything it
/// started.
const FORMAT_TIMEOUT: Duration = Duration::from_secs(30);

/// Try to format a file using any configured formatter.
/// Returns silently if no formatter is configured or the formatter fails.
pub async fn try_format_file(path: &str, ctx: &ToolContext) {
    format_file_within(path, ctx, FORMAT_TIMEOUT).await;
}

/// [`try_format_file`] with the formatter's time limit given.
async fn format_file_within(path: &str, ctx: &ToolContext, limit: Duration) {
    // Hosted repair promises a file-only surface under BypassPermissions.
    // Formatters are executable commands and may come from repository config.
    if ctx.config.hosted_review_enabled() && ctx.config.hosted_review.allow_file_write_tools {
        return;
    }

    let formatters = &ctx.config.formatter;
    if formatters.is_empty() {
        return;
    }

    // Determine the file's extension (with leading dot, e.g. ".ts").
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();

    for fmt in formatters.values() {
        if fmt.disabled || fmt.command.is_empty() {
            continue;
        }
        if !fmt.extensions.iter().any(|e| e == &ext) {
            continue;
        }

        let mut cmd = tokio::process::Command::new(&fmt.command[0]);
        let mut file_injected = false;
        for arg in &fmt.command[1..] {
            if arg == "$FILE" || arg == "{file}" {
                cmd.arg(path);
                file_injected = true;
            } else {
                cmd.arg(arg);
            }
        }
        // Append the file path if no explicit placeholder was present.
        if !file_injected {
            cmd.arg(path);
        }

        // Run with a time limit, past which the formatter and everything it
        // started are killed (#373); silently ignore all errors.
        let _ = crate::process_tree::output_within(&mut cmd, limit).await;

        // Only apply the first matching formatter.
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::format_file_within;
    use crate::ToolContext;
    use claurst_core::config::{Config, FormatterConfig, PermissionMode};
    use claurst_core::permissions::AutoPermissionHandler;
    use std::path::Path;
    use std::sync::{atomic::AtomicUsize, Arc};
    use std::time::Duration;
    #[cfg(unix)]
    use std::time::Instant;

    /// A context whose only formatter runs `command` for `.rs` files.
    fn formatting_with(directory: &Path, command: Vec<String>) -> ToolContext {
        let mut config = Config::default();
        config.formatter.insert(
            "fixture".to_string(),
            FormatterConfig {
                command,
                extensions: vec![".rs".to_string()],
                disabled: false,
            },
        );
        ToolContext {
            working_dir: directory.to_path_buf(),
            permission_mode: PermissionMode::BypassPermissions,
            permission_handler: Arc::new(AutoPermissionHandler {
                mode: PermissionMode::BypassPermissions,
            }),
            cost_tracker: claurst_core::cost::CostTracker::new(),
            session_id: uuid::Uuid::new_v4().to_string(),
            goal_turn: Default::default(),
            file_history: Default::default(),
            current_turn: Arc::new(AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: None,
            config,
            managed_agent_config: None,
            completion_notifier: None,
            pending_permissions: None,
            permission_manager: None,
            user_question_tx: None,
        }
    }

    /// #373: a formatter that runs past its limit is killed with what it
    /// started. The subshell would write the marker 2 s in; the shell waits
    /// for it, so the limit ends the call first.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_formatter_past_its_limit_is_stopped_with_its_descendants() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("survived");
        let ctx = formatting_with(
            directory.path(),
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "(sleep 2; echo survived > \"$1\") & wait".to_string(),
                "formatter".to_string(),
                marker.to_string_lossy().into_owned(),
                "$FILE".to_string(),
            ],
        );
        let source = directory.path().join("source.rs");
        let started = Instant::now();
        format_file_within(&source.to_string_lossy(), &ctx, Duration::from_millis(500)).await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the formatter call outlived its limit: {:?}",
            started.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(
            !marker.exists(),
            "the formatter's descendant kept running after the limit"
        );
    }

    /// #373 on Windows: the formatter is `cmd.exe`, which starts another
    /// `cmd.exe` that runs the loop.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_formatter_past_its_limit_is_stopped_with_its_child() {
        let ticker = crate::windows_ticker::Ticker::new();
        let ctx = formatting_with(
            ticker.directory(),
            ["cmd", "/D", "/C", "cmd", "/D", "/C"]
                .into_iter()
                .map(String::from)
                .chain([ticker.script().to_string_lossy().into_owned()])
                .collect(),
        );
        let source = ticker.directory().join("source.rs");
        format_file_within(&source.to_string_lossy(), &ctx, Duration::from_secs(4)).await;
        ticker.assert_stopped("formatter past its limit").await;
    }
}
