//! #517 on Windows: a `PostModelTurn` or `Stop` hook is a command line for
//! `cmd.exe`, which must get the command's double quotes exactly as written.
#![cfg(windows)]

use claurst_core::config::{Config, HookEntry, HookEvent};
use claurst_core::types::Message;
use claurst_query::{fire_post_sampling_hooks, stop_hooks_with_full_behavior};
use std::time::Duration;

fn config_with(event: HookEvent, command: &str) -> Config {
    let mut config = Config::default();
    config.hooks.insert(
        event,
        vec![HookEntry {
            command: command.to_string(),
            tool_filter: None,
            blocking: false,
        }],
    );
    config
}

/// A failed hook's output is handed back as a message. `echo` prints its
/// argument with the quotes.
#[test]
fn post_model_turn_hook_command_passes_quotes_as_written() {
    let command = r#"echo "a b"&exit 1"#;
    let result = fire_post_sampling_hooks(
        &Message::assistant("done"),
        &config_with(HookEvent::PostModelTurn, command),
    );
    let errors: Vec<String> = result
        .blocking_errors
        .iter()
        .map(Message::get_all_text)
        .collect();
    assert_eq!(errors, [format!("[Hook '{command}' error]:\n\"a b\"")]);
    assert!(!result.prevent_continuation);
}

/// A Stop hook's output is discarded, so it writes to a quoted file name.
#[tokio::test]
async fn stop_hook_command_passes_quotes_as_written() {
    let directory = tempfile::tempdir().unwrap();
    let written = directory.path().join("stop out.txt");
    stop_hooks_with_full_behavior(
        &Message::assistant("done"),
        &config_with(HookEvent::Stop, r#"echo "a b">"stop out.txt""#),
        directory.path().to_path_buf(),
        None,
    );
    // The hook runs on a thread of its own; `echo` ends its line with CRLF.
    let text = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match std::fs::read_to_string(&written) {
                Ok(text) if text.ends_with('\n') => break text,
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .expect("the Stop hook never wrote \"stop out.txt\"");
    assert_eq!(text, "\"a b\"\r\n");
}
