use crate::{
    AccountsCommand, AttachCommand, ChromeCommand, CommandContext, CommandResult, ConnectCommand,
    HooksCommand, LinkCommand, LoginCommand, LogoutCommand, McpCommand, ProvidersCommand,
    RefreshCommand, SlashCommand, SwitchCommand,
};
use async_trait::async_trait;

/// Split a parent command's arguments into its child name and remaining input.
pub(crate) fn split_subcommand(args: &str) -> (&str, &str) {
    let args = args.trim();
    match args.split_once(char::is_whitespace) {
        Some((subcommand, rest)) => (subcommand, rest.trim()),
        None => (args, ""),
    }
}

fn unknown_child(parent: &str, child: &str, usage: &str, valid_children: &[&str]) -> CommandResult {
    CommandResult::Error(format!(
        "Unknown /{parent} subcommand '{child}'.\nValid children: {}.\nUsage: {usage}",
        valid_children.join(", ")
    ))
}

pub struct AccountCommand;

#[async_trait]
impl SlashCommand for AccountCommand {
    fn name(&self) -> &str {
        "account"
    }

    fn description(&self) -> &str {
        "Manage accounts and provider connections"
    }

    fn help(&self) -> &str {
        "Usage: /account <login|logout|list|dedupe|providers|connect|switch|refresh>"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let (child, remaining_args) = split_subcommand(args);
        match child {
            "" | "list" => AccountsCommand.execute("", ctx).await,
            "dedupe" => AccountsCommand.execute("dedupe", ctx).await,
            "login" => LoginCommand.execute(remaining_args, ctx).await,
            "logout" => LogoutCommand.execute(remaining_args, ctx).await,
            "connect" => ConnectCommand.execute(remaining_args, ctx).await,
            "providers" => ProvidersCommand.execute(remaining_args, ctx).await,
            "switch" => SwitchCommand.execute(remaining_args, ctx).await,
            "refresh" => RefreshCommand.execute(remaining_args, ctx).await,
            unknown => unknown_child(
                "account",
                unknown,
                self.help(),
                &[
                    "login",
                    "logout",
                    "list",
                    "dedupe",
                    "providers",
                    "connect",
                    "switch",
                    "refresh",
                ],
            ),
        }
    }
}

pub struct IntegrationsCommand;

#[async_trait]
impl SlashCommand for IntegrationsCommand {
    fn name(&self) -> &str {
        "integrations"
    }

    fn description(&self) -> &str {
        "Configure external integrations"
    }

    fn help(&self) -> &str {
        "Usage: /integrations <hooks|mcp|chrome>"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let (child, remaining_args) = split_subcommand(args);
        match child {
            "" | "list" => CommandResult::Message(format!(
                "{}\n\nValid children: hooks, mcp, chrome",
                self.help()
            )),
            "hooks" => HooksCommand.execute(remaining_args, ctx).await,
            "mcp" => McpCommand.execute(remaining_args, ctx).await,
            "chrome" => ChromeCommand.execute(remaining_args, ctx).await,
            unknown => unknown_child(
                "integrations",
                unknown,
                self.help(),
                &["hooks", "mcp", "chrome"],
            ),
        }
    }
}

pub struct StashCommand;

#[async_trait]
impl SlashCommand for StashCommand {
    fn name(&self) -> &str {
        "stash"
    }

    fn description(&self) -> &str {
        "Store useful links and attachments"
    }

    fn help(&self) -> &str {
        "Usage: /stash <link|attach>\n\n\
         /stash link [arguments]       — store and manage links\n\
         /stash attach [arguments]     — store and manage attachments"
    }

    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let (child, remaining_args) = split_subcommand(args);
        match child {
            "" | "list" => CommandResult::Message(self.help().to_string()),
            "link" | "links" => LinkCommand.execute(remaining_args, ctx).await,
            "attach" | "attachments" => AttachCommand.execute(remaining_args, ctx).await,
            unknown => unknown_child(
                "stash",
                unknown,
                self.help(),
                &["link", "links", "attach", "attachments"],
            ),
        }
    }
}
