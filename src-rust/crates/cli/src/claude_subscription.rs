use anyhow::{bail, Context, Result};
use claurst_core::accounts::{self, AccountProfile, AccountRegistry, PROVIDER_ANTHROPIC};
use tokio::process::Command;

pub async fn login(label: Option<&str>) -> Result<String> {
    let binary = claurst_api::providers::claude_cli::resolve_claude_binary().context(
        "Claude CLI is missing. Install Claude Code and retry /account login anthropic.",
    )?;
    let base = label.unwrap_or("claude");
    let registry = AccountRegistry::load();
    let base = accounts::slugify_profile_id(base);
    let parent = accounts::claurst_dir()
        .join("accounts")
        .join(PROVIDER_ANTHROPIC);
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.recursive(true).create(&parent)?;
    builder.recursive(false);
    let mut suffix = 1;
    let (id, directory) = loop {
        let id = if suffix == 1 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        suffix += 1;
        if registry.get(PROVIDER_ANTHROPIC, &id).is_some() {
            continue;
        }
        match builder.create(parent.join(&id)) {
            Ok(()) => {
                let directory = accounts::claude_cli::config_dir(&id);
                builder.create(&directory)?;
                break (id, directory);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    };
    let mut command = Command::new(binary);
    accounts::claude_cli::configure(&mut command, &directory);
    let status = command
        .args(["auth", "login", "--claudeai"])
        .status()
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => anyhow::anyhow!(
                "Claude CLI is missing. Install Claude Code and retry /account login anthropic."
            ),
            _ => anyhow::Error::new(error).context("Starting Claude login"),
        })?;
    if !status.success() {
        if status.code().is_none() || status.code() == Some(130) {
            bail!("Claude login canceled.");
        }
        bail!("Claude login did not complete ({status}). Retry /account login anthropic.");
    }
    if !accounts::claude_cli::signed_in(&directory).await {
        bail!("Claude CLI did not report a signed-in subscription.");
    }
    AccountRegistry::load().upsert(
        PROVIDER_ANTHROPIC,
        AccountProfile {
            id: id.clone(),
            label: label.map(str::to_owned),
            claude_cli: true,
            ..Default::default()
        },
        false,
    )?;
    Ok(id)
}
