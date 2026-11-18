//! #517 on Windows: a configured hook is a command line for `cmd.exe`, which
//! must get the command's double quotes exactly as written.
#![cfg(windows)]

use claurst_core::config::{HookEntry, HookEvent};
use claurst_core::hooks::{run_hooks, HookContext, HookOutcome};
use std::collections::HashMap;

#[tokio::test]
async fn hook_command_passes_quotes_as_written() {
    let directory = tempfile::tempdir().unwrap();
    let hooks = HashMap::from([(
        HookEvent::PreToolUse,
        vec![HookEntry {
            // `echo` prints its argument with the quotes.
            command: r#"echo "a b"&echo "c""#.to_string(),
            tool_filter: None,
            blocking: false,
        }],
    )]);
    let context = HookContext {
        event: "PreToolUse".to_string(),
        tool_name: None,
        tool_input: None,
        tool_output: None,
        is_error: None,
        session_id: None,
    };
    let outcome = run_hooks(&hooks, HookEvent::PreToolUse, &context, directory.path()).await;
    assert!(
        matches!(&outcome, HookOutcome::Modified(output) if output == "\"a b\"\r\n\"c\""),
        "{outcome:?}"
    );
}
