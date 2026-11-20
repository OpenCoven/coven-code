//! #517 on Windows: a configured hook is a command line for `cmd.exe`, which
//! must get the command's double quotes exactly as written. So must any line
//! given to `claurst_core::shell::command`, which every site that runs a
//! command line calls.
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

/// The shared shell itself, for the caller that no test can drive on Windows:
/// the status line command only runs inside the interactive terminal.
#[test]
fn shell_command_passes_quotes_as_written() {
    let directory = tempfile::tempdir().unwrap();
    let note = directory.path().join("with space").join("note.txt");
    std::fs::create_dir(directory.path().join("with space")).unwrap();
    std::fs::write(&note, "from a quoted path\r\n").unwrap();
    for (line, printed) in [
        (r#"echo "a b"&echo "c""#.to_string(), "\"a b\"\r\n\"c\"\r\n"),
        (
            format!(r#"type "{}""#, note.display()),
            "from a quoted path\r\n",
        ),
        // An odd number of quotes: the two that are stripped are still the
        // added pair.
        (r#"echo "a"#.to_string(), "\"a\r\n"),
        // A backslash at the very end is not doubled.
        (r"echo a b\".to_string(), "a b\\\r\n"),
    ] {
        let output = claurst_core::shell::command(&line).output().unwrap();
        assert!(output.status.success(), "{line}: {output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), printed, "{line}");
    }
}

/// `start` gets its target through a variable that `cmd.exe` expands after
/// it has read the line, and a target that would split `start`'s argument
/// is refused, as is an empty one. What `start` then opens is covered by the callers' tests, in
/// claurst-tui and claurst-mcp.
#[test]
fn shell_start_hands_over_its_target_in_a_variable() {
    use claurst_core::shell::{start, START_TARGET_VARIABLE};
    let target = r"C:\temp\safe & %PATH% ^ literal.txt";
    let shell = start(std::process::Command::new("cmd"), target).unwrap();
    assert_eq!(shell.get_program(), "cmd");
    let arguments: Vec<String> = shell
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        arguments,
        [format!(
            r#"/D /V:ON /C "start "" "!{START_TARGET_VARIABLE}!"""#
        )]
    );
    assert_eq!(
        shell.get_envs().collect::<Vec<_>>(),
        [(
            std::ffi::OsStr::new(START_TARGET_VARIABLE),
            Some(std::ffi::OsStr::new(target))
        )]
    );
    for refused in [r#"x" & echo injected & ""#, ""] {
        let error = start(std::process::Command::new("cmd"), refused).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{refused}");
    }
}
