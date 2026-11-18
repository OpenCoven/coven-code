//! #517 on Windows: a plugin hook is a command line for `cmd.exe`, which must
//! get the command's double quotes exactly as written.
#![cfg(windows)]

use claurst_plugins::hooks::{
    dispatch_post_tool_hooks, run_hook_sync, HookOutcome, HookScope, RegisteredHook,
};
use claurst_plugins::HookRegistry;

fn hook(command: &str) -> RegisteredHook {
    RegisteredHook {
        command: command.to_string(),
        matcher: None,
        blocking: true,
        plugin_root: ".".into(),
        plugin_name: "quotes-fixture".into(),
        plugin_source: "test".into(),
        plugin_capabilities: Some(vec!["shell".into()]),
        scope: HookScope::Global,
    }
}

/// A blocking hook's denial carries what the hook wrote to stderr. `echo`
/// prints its argument with the quotes.
#[test]
fn blocking_hook_command_passes_quotes_as_written() {
    let directory = tempfile::tempdir().unwrap();
    let outcome = run_hook_sync(&hook(r#"echo "a b">&2&exit 7"#), "{}", directory.path());
    assert!(
        matches!(
            &outcome,
            HookOutcome::Deny(message)
                if message == r#"Plugin 'quotes-fixture' blocked operation (exit 7): "a b""#
        ),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn post_tool_hook_command_passes_quotes_as_written() {
    let directory = tempfile::tempdir().unwrap();
    let registry = HookRegistry::from([(
        "PostToolUse".to_string(),
        vec![hook(r#"echo "a b"&echo "c""#)],
    )]);
    let outputs = dispatch_post_tool_hooks(&registry, "Bash", "{}", "{}", directory.path()).await;
    assert_eq!(outputs, ["\"a b\"\r\n\"c\""]);
}
