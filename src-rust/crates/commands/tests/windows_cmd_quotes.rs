//! #517 on Windows: a plugin's shell slash command is a command line for
//! `cmd.exe`, which must get the command's double quotes exactly as written.
#![cfg(windows)]

use claurst_commands::{CommandContext, CommandResult, PluginSlashCommandAdapter, SlashCommand};
use claurst_plugins::{CommandRunAction, PluginCommandDef};

#[tokio::test]
async fn plugin_shell_command_passes_quotes_as_written() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = CommandContext {
        config: Default::default(),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        messages: vec![],
        working_dir: root.path().to_path_buf(),
        session_id: "quotes".into(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        mcp_auth_runner: None,
    };
    let command = PluginSlashCommandAdapter {
        def: PluginCommandDef {
            name: "fixture:quotes".into(),
            description: String::new(),
            plugin_name: "fixture".into(),
            plugin_source_id: "fixture@user".into(),
            run_action: CommandRunAction::ShellCommand {
                // `echo` prints its argument with the quotes.
                command: r#"echo "a b"&echo"#.into(),
                plugin_root: root.path().to_string_lossy().into_owned(),
            },
            plugin_capabilities: Some(vec!["shell".into()]),
        },
    };
    // The arguments are appended to the command after a space.
    let result = command.execute(r#""c""#, &mut ctx).await;
    assert!(
        matches!(&result, CommandResult::Message(output) if output == "\"a b\"\r\n\"c\"\r\n"),
        "{result:?}"
    );
}
