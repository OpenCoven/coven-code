use anyhow::{bail, Context};
use claurst_core::claudemd::{
    effective_memory_expires_at, memory_id, parse_frontmatter, MemoryFileInfo, MemoryScope,
    RetentionClass,
};
use claurst_core::hosted_review::{HostedReviewScope, MemoryDomain, MemorySourceTrust};
use claurst_core::memdir::{
    auto_memory_path, delete_hosted_memory_for_scope, delete_memory_file_with_force,
    expire_memory_file, memory_file_has_legal_hold, redact_memory_file, MEMORY_ENTRYPOINT,
};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryStatus {
    Active,
    Expired,
    Redacted,
    Deleted,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MemoryEntry {
    pub id: String,
    pub path: PathBuf,
    pub retention_class: Option<RetentionClass>,
    pub trust: Option<String>,
    pub created_at: Option<String>,
    pub expires_at: Option<String>,
    pub status: MemoryStatus,
    pub redacted_at: Option<String>,
    pub deleted_at: Option<String>,
    pub source: Option<String>,
    #[serde(skip_serializing)]
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MemoryLedgerEntry {
    pub id: String,
    pub path: PathBuf,
    pub redacted_at: Option<String>,
    pub deleted_at: Option<String>,
    pub retention_class: Option<RetentionClass>,
    pub reason: Option<String>,
    pub source: Option<String>,
}

pub(crate) async fn handle_memory_command(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(|arg| arg.as_str()) {
        Some("list") => handle_list(&args[1..]),
        Some("expire") => handle_expire(&args[1..]),
        Some("redact") => handle_redact(&args[1..]),
        Some("delete") => handle_delete(&args[1..]),
        Some("conflicts") => handle_conflicts(&args[1..]),
        Some("resolve-conflict") => handle_resolve_conflict(&args[1..]),
        Some("ledger") => handle_ledger(&args[1..]),
        Some("-h") | Some("--help") | None => {
            print_usage();
            Ok(())
        }
        Some(command) => {
            bail!("unknown memory subcommand '{command}'");
        }
    }
}

fn handle_list(args: &[String]) -> anyhow::Result<()> {
    let options = parse_common_options(args)?;
    let mut entries = collect_entries_from_dirs(&options.dirs)?;
    apply_filters(&mut entries, &options);
    if options.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        print_entries_table(&entries);
    }
    Ok(())
}

fn handle_expire(args: &[String]) -> anyhow::Result<()> {
    let mut target = None;
    let mut at = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let mut force = false;
    let mut dirs = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--at" => {
                index += 1;
                at = required_arg(args, index, "--at")?.to_string();
            }
            value if value.starts_with("--at=") => {
                at = value.trim_start_matches("--at=").to_string();
            }
            "--force" => force = true,
            "--dir" => {
                index += 1;
                dirs.push(PathBuf::from(required_arg(args, index, "--dir")?));
            }
            flag if flag.starts_with("--") => bail!("unknown memory expire flag: {flag}"),
            value => set_single_target(&mut target, value)?,
        }
        index += 1;
    }
    let target = target
        .context("usage: coven-code memory expire <id-or-path> [--at YYYY-MM-DD] [--force]")?;
    let entry = resolve_for_operation(&target, dirs)?;
    expire_memory_file(&entry.path, &at, force)
        .with_context(|| format!("failed to expire {}", entry.path.display()))?;
    println!("expired {} at {}", entry.id, at);
    Ok(())
}

fn handle_redact(args: &[String]) -> anyhow::Result<()> {
    let (target, reason, _force, dirs) = parse_target_reason_args(
        args,
        "usage: coven-code memory redact <id-or-path> --reason <text>",
    )?;
    let entry = resolve_for_operation(&target, dirs)?;
    redact_memory_file(&entry.path, &reason)
        .with_context(|| format!("failed to redact {}", entry.path.display()))?;
    println!("redacted {}", entry.id);
    Ok(())
}

fn handle_delete(args: &[String]) -> anyhow::Result<()> {
    let mut scope = None;
    let mut passthrough = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--scope" => {
                index += 1;
                scope = Some(required_arg(args, index, "--scope")?.to_string());
            }
            value if value.starts_with("--scope=") => {
                scope = Some(value.trim_start_matches("--scope=").to_string());
            }
            value => passthrough.push(value.to_string()),
        }
        index += 1;
    }

    if let Some(scope) = scope {
        let mut reason = None;
        let mut force = false;
        for (flag, value) in parse_flags_with_values(&passthrough)? {
            match (flag.as_str(), value) {
                ("--reason", Some(text)) => reason = Some(text),
                ("--force", None) => force = true,
                (unknown, _) => bail!("unknown memory delete flag: {unknown}"),
            }
        }
        let _reason = reason
            .context("usage: coven-code memory delete --scope <scope> --reason <text> [--force]")?;
        let scope = parse_scope(&scope)?;
        if !force && hosted_scope_has_legal_hold(&scope)? {
            bail!("refusing to delete legal_hold memory scope without --force");
        }
        let path = claurst_core::memdir::hosted_memory_path_for_scope(&scope);
        delete_hosted_memory_for_scope(&scope)
            .with_context(|| format!("failed to delete hosted scope {}", path.display()))?;
        println!("deleted hosted memory scope {}", path.display());
        return Ok(());
    }

    let (target, reason, force, dirs) = parse_target_reason_args(
        &passthrough,
        "usage: coven-code memory delete <id-or-path> --reason <text> [--force]",
    )?;
    let entry = resolve_for_operation(&target, dirs)?;
    delete_memory_file_with_force(&entry.path, &reason, force)
        .with_context(|| format!("failed to delete {}", entry.path.display()))?;
    println!("deleted {}", entry.id);
    Ok(())
}

fn handle_conflicts(args: &[String]) -> anyhow::Result<()> {
    let mut json = false;
    let mut team_dir = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--dir" => {
                index += 1;
                team_dir = Some(PathBuf::from(required_arg(args, index, "--dir")?));
            }
            value if value.starts_with("--dir=") => {
                team_dir = Some(PathBuf::from(value.trim_start_matches("--dir=")));
            }
            flag if flag.starts_with("--") => bail!("unknown memory conflicts flag: {flag}"),
            value => bail!("unexpected memory conflicts argument: {value}"),
        }
        index += 1;
    }

    let team_dir = match team_dir {
        Some(dir) => dir,
        None => default_team_memory_dir()?,
    };
    let conflicts = claurst_core::team_memory_sync::pending_conflicts(&team_dir);
    if json {
        println!("{}", serde_json::to_string_pretty(&conflicts)?);
        return Ok(());
    }
    if conflicts.is_empty() {
        println!("no pending team-memory conflicts in {}", team_dir.display());
        return Ok(());
    }
    println!("pending team-memory conflicts ({})", conflicts.len());
    for conflict in &conflicts {
        println!(
            "  {} [{:?}]: {}",
            conflict.key, conflict.kind, conflict.reason
        );
    }
    Ok(())
}

fn handle_resolve_conflict(args: &[String]) -> anyhow::Result<()> {
    let mut key = None;
    let mut team_dir = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dir" => {
                index += 1;
                team_dir = Some(PathBuf::from(required_arg(args, index, "--dir")?));
            }
            value if value.starts_with("--dir=") => {
                team_dir = Some(PathBuf::from(value.trim_start_matches("--dir=")));
            }
            flag if flag.starts_with("--") => {
                bail!("unknown memory resolve-conflict flag: {flag}")
            }
            value => set_single_target(&mut key, value)?,
        }
        index += 1;
    }
    let key = key.context("usage: coven-code memory resolve-conflict <key> [--dir <path>]")?;
    claurst_core::team_memory_sync::validate_memory_path(&key)
        .with_context(|| format!("invalid memory key '{key}'"))?;

    let team_dir = match team_dir {
        Some(dir) => dir,
        None => default_team_memory_dir()?,
    };
    let resolved = claurst_core::team_memory_sync::resolve_conflict(&team_dir, &key)
        .with_context(|| format!("failed to resolve conflict for '{key}'"))?;
    if !resolved {
        bail!(
            "no pending conflict for key '{key}' in {}",
            team_dir.display()
        );
    }
    println!("resolved team-memory conflict {key}");
    Ok(())
}

fn default_team_memory_dir() -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    Ok(claurst_core::memdir::team_memory_path(&auto_memory_path(
        &cwd,
    )))
}

fn handle_ledger(args: &[String]) -> anyhow::Result<()> {
    let options = parse_common_options(args)?;
    let mut entries = collect_entries_from_dirs(&options.dirs)?;
    apply_filters(&mut entries, &options);
    let ledger = export_ledger(&entries);
    if options.json {
        println!("{}", serde_json::to_string_pretty(&ledger)?);
    } else {
        print_ledger_table(&ledger);
    }
    Ok(())
}

#[derive(Default)]
struct CommonOptions {
    dirs: Vec<PathBuf>,
    json: bool,
    tenant: Option<String>,
    repo: Option<String>,
    domain: Option<String>,
}

fn parse_common_options(args: &[String]) -> anyhow::Result<CommonOptions> {
    let mut options = CommonOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => options.json = true,
            "--dir" => {
                index += 1;
                options
                    .dirs
                    .push(PathBuf::from(required_arg(args, index, "--dir")?));
            }
            "--tenant" => {
                index += 1;
                options.tenant = Some(required_arg(args, index, "--tenant")?.to_string());
            }
            "--repo" => {
                index += 1;
                options.repo = Some(required_arg(args, index, "--repo")?.to_string());
            }
            "--domain" => {
                index += 1;
                options.domain = Some(required_arg(args, index, "--domain")?.to_string());
            }
            value if value.starts_with("--dir=") => {
                options
                    .dirs
                    .push(PathBuf::from(value.trim_start_matches("--dir=")));
            }
            value if value.starts_with("--tenant=") => {
                options.tenant = Some(value.trim_start_matches("--tenant=").to_string());
            }
            value if value.starts_with("--repo=") => {
                options.repo = Some(value.trim_start_matches("--repo=").to_string());
            }
            value if value.starts_with("--domain=") => {
                options.domain = Some(value.trim_start_matches("--domain=").to_string());
            }
            flag if flag.starts_with("--") => bail!("unknown memory flag: {flag}"),
            value => bail!("unexpected memory argument: {value}"),
        }
        index += 1;
    }

    if options.dirs.is_empty() {
        options.dirs = default_memory_dirs()?;
    }
    Ok(options)
}

pub(crate) fn collect_entries_from_dirs(dirs: &[PathBuf]) -> anyhow::Result<Vec<MemoryEntry>> {
    let mut entries = Vec::new();
    for dir in dirs {
        let mut paths = Vec::new();
        collect_admin_memory_paths(dir, &mut paths);
        for path in paths {
            entries.push(load_entry_from_path(&path)?);
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn collect_admin_memory_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_admin_memory_paths(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some(MEMORY_ENTRYPOINT) {
            continue;
        }
        out.push(path);
    }
}

fn load_entry_from_path(path: &Path) -> anyhow::Result<MemoryEntry> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read memory file {}", path.display()))?;
    let (frontmatter, body) = parse_frontmatter(&raw);
    let file = MemoryFileInfo {
        path: path.to_path_buf(),
        scope: MemoryScope::Managed,
        content: body.to_string(),
        frontmatter,
        mtime: None,
    };
    let effective_expires_at = effective_memory_expires_at(&file.frontmatter)
        .map(|date| date.format("%Y-%m-%d").to_string());
    let status = if file.frontmatter.deleted_at.is_some() {
        MemoryStatus::Deleted
    } else if file.frontmatter.redacted_at.is_some() {
        MemoryStatus::Redacted
    } else if effective_memory_expires_at(&file.frontmatter)
        .map(|date| date < chrono::Local::now().date_naive())
        .unwrap_or(false)
    {
        MemoryStatus::Expired
    } else {
        MemoryStatus::Active
    };
    let reason = if matches!(status, MemoryStatus::Redacted | MemoryStatus::Deleted) {
        tombstone_reason(body)
    } else {
        None
    };

    Ok(MemoryEntry {
        id: memory_id(&file),
        path: path.to_path_buf(),
        retention_class: file.frontmatter.retention_class,
        trust: file.frontmatter.trust.map(trust_label).map(String::from),
        created_at: file.frontmatter.created_at,
        expires_at: effective_expires_at,
        status,
        redacted_at: file.frontmatter.redacted_at,
        deleted_at: file.frontmatter.deleted_at,
        source: file.frontmatter.source,
        reason,
    })
}

pub(crate) fn resolve_entry_target<'a>(
    entries: &'a [MemoryEntry],
    target: &str,
) -> anyhow::Result<&'a MemoryEntry> {
    let target_path = Path::new(target);
    let mut matches = Vec::new();
    for entry in entries {
        if entry.id == target || entry.path == target_path || paths_match(&entry.path, target_path)
        {
            matches.push(entry);
        }
    }

    match matches.len() {
        0 => bail!("no memory entry matched '{target}'"),
        1 => Ok(matches[0]),
        _ => bail!("memory target '{target}' matched multiple entries"),
    }
}

pub(crate) fn export_ledger(entries: &[MemoryEntry]) -> Vec<MemoryLedgerEntry> {
    entries
        .iter()
        .filter(|entry| matches!(entry.status, MemoryStatus::Redacted | MemoryStatus::Deleted))
        .map(|entry| MemoryLedgerEntry {
            id: entry.id.clone(),
            path: entry.path.clone(),
            redacted_at: entry.redacted_at.clone(),
            deleted_at: entry.deleted_at.clone(),
            retention_class: entry.retention_class,
            reason: entry.reason.clone(),
            source: entry.source.clone(),
        })
        .collect()
}

fn resolve_for_operation(target: &str, dirs: Vec<PathBuf>) -> anyhow::Result<MemoryEntry> {
    let target_path = PathBuf::from(target);
    if target_path.exists() {
        return load_entry_from_path(&target_path);
    }
    let dirs = if dirs.is_empty() {
        default_memory_dirs()?
    } else {
        dirs
    };
    let entries = collect_entries_from_dirs(&dirs)?;
    resolve_entry_target(&entries, target).cloned()
}

fn default_memory_dirs() -> anyhow::Result<Vec<PathBuf>> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let mut dirs = vec![auto_memory_path(&cwd)];
    let hosted_root = claurst_core::config::Settings::config_dir().join("hosted-review");
    collect_hosted_memory_dirs(&hosted_root, &mut dirs);
    Ok(dirs)
}

fn collect_hosted_memory_dirs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) == Some("memory") {
                out.push(path);
            } else {
                collect_hosted_memory_dirs(&path, out);
            }
        }
    }
}

fn apply_filters(entries: &mut Vec<MemoryEntry>, options: &CommonOptions) {
    entries.retain(|entry| {
        path_contains_filter(&entry.path, options.tenant.as_deref())
            && path_contains_filter(&entry.path, options.repo.as_deref())
            && path_contains_filter(&entry.path, options.domain.as_deref())
    });
}

fn path_contains_filter(path: &Path, filter: Option<&str>) -> bool {
    filter
        .map(|value| path.to_string_lossy().contains(value))
        .unwrap_or(true)
}

fn hosted_scope_has_legal_hold(scope: &HostedReviewScope) -> anyhow::Result<bool> {
    let dir = claurst_core::memdir::hosted_memory_path_for_scope(scope);
    let mut files = Vec::new();
    collect_markdown_files(&dir, &mut files);
    for file in files {
        if memory_file_has_legal_hold(&file)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn collect_markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

fn parse_target_reason_args(
    args: &[String],
    usage: &str,
) -> anyhow::Result<(String, String, bool, Vec<PathBuf>)> {
    let mut target = None;
    let mut reason = None;
    let mut force = false;
    let mut dirs = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--reason" => {
                index += 1;
                reason = Some(required_arg(args, index, "--reason")?.to_string());
            }
            value if value.starts_with("--reason=") => {
                reason = Some(value.trim_start_matches("--reason=").to_string());
            }
            "--force" => force = true,
            "--dir" => {
                index += 1;
                dirs.push(PathBuf::from(required_arg(args, index, "--dir")?));
            }
            flag if flag.starts_with("--") => bail!("unknown memory flag: {flag}"),
            value => set_single_target(&mut target, value)?,
        }
        index += 1;
    }
    let target = target.with_context(|| usage.to_string())?;
    let reason = reason.with_context(|| usage.to_string())?;
    Ok((target, reason, force, dirs))
}

fn parse_flags_with_values(args: &[String]) -> anyhow::Result<Vec<(String, Option<String>)>> {
    let mut flags = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--force" => flags.push(("--force".to_string(), None)),
            "--reason" => {
                index += 1;
                flags.push((
                    "--reason".to_string(),
                    Some(required_arg(args, index, "--reason")?.to_string()),
                ));
            }
            value if value.starts_with("--reason=") => flags.push((
                "--reason".to_string(),
                Some(value.trim_start_matches("--reason=").to_string()),
            )),
            flag if flag.starts_with("--") => flags.push((flag.to_string(), None)),
            value => bail!("unexpected memory delete argument: {value}"),
        }
        index += 1;
    }
    Ok(flags)
}

fn parse_scope(raw: &str) -> anyhow::Result<HostedReviewScope> {
    let mut tenant = None;
    let mut installation = None;
    let mut repo = None;
    let mut repo_full_name = None;
    let mut domain = None;
    for part in raw.split(',') {
        let (key, value) = part
            .split_once('=')
            .with_context(|| format!("invalid scope component '{part}'"))?;
        match key.trim() {
            "tenant" | "tenant_id" => tenant = Some(value.trim().to_string()),
            "install" | "installation" | "installation_id" => {
                installation = Some(value.trim().to_string());
            }
            "repo" | "repo_id" => repo = Some(value.trim().to_string()),
            "repo_full_name" | "full_name" => repo_full_name = Some(value.trim().to_string()),
            "domain" => domain = Some(value.trim().to_string()),
            unknown => bail!("unknown scope component '{unknown}'"),
        }
    }
    let repo = repo.context("scope requires repo=<id>")?;
    let mut scope = HostedReviewScope::new(
        tenant.context("scope requires tenant=<id>")?,
        installation.context("scope requires install=<id>")?,
        repo.clone(),
        repo_full_name.unwrap_or(repo),
    );
    if let Some(domain) = domain {
        scope = scope.with_domain(parse_domain(&domain)?);
    }
    scope.validate().map_err(anyhow::Error::msg)?;
    Ok(scope)
}

fn parse_domain(value: &str) -> anyhow::Result<MemoryDomain> {
    match value {
        "default" | "default-branch" => Ok(MemoryDomain::DefaultBranch),
        "security" | "security-private" => Ok(MemoryDomain::SecurityPrivate),
        _ => {
            if let Some(branch) = value.strip_prefix("branch:") {
                return Ok(MemoryDomain::Branch(branch.to_string()));
            }
            if let Some(release) = value.strip_prefix("release:") {
                return Ok(MemoryDomain::Release(release.to_string()));
            }
            if let Some(pr) = value.strip_prefix("pr:") {
                let number = pr.parse().context("invalid pull request domain number")?;
                return Ok(MemoryDomain::PullRequest(number));
            }
            bail!("unknown memory domain '{value}'")
        }
    }
}

fn set_single_target(target: &mut Option<String>, value: &str) -> anyhow::Result<()> {
    if target.is_some() {
        bail!("only one memory id or path may be supplied");
    }
    *target = Some(value.to_string());
    Ok(())
}

fn required_arg<'a>(args: &'a [String], index: usize, flag: &str) -> anyhow::Result<&'a str> {
    args.get(index)
        .map(|value| value.as_str())
        .with_context(|| format!("{flag} requires a value"))
}

fn paths_match(entry_path: &Path, target_path: &Path) -> bool {
    let Ok(entry) = entry_path.canonicalize() else {
        return false;
    };
    let Ok(target) = target_path.canonicalize() else {
        return false;
    };
    entry == target
}

fn tombstone_reason(body: &str) -> Option<String> {
    let line = body.lines().map(str::trim).find(|line| !line.is_empty())?;
    let canonical_redaction = line.starts_with("[REDACTED:") && line.ends_with(']');
    let canonical_deletion = line.starts_with("[DELETED:") && line.ends_with(']');
    if canonical_redaction || canonical_deletion {
        Some(line.to_string())
    } else {
        None
    }
}

fn trust_label(trust: MemorySourceTrust) -> &'static str {
    match trust {
        MemorySourceTrust::SystemPolicy => "system_policy",
        MemorySourceTrust::MaintainerApproved => "maintainer_approved",
        MemorySourceTrust::DefaultBranchCode => "default_branch_code",
        MemorySourceTrust::ContributorInput => "contributor_input",
        MemorySourceTrust::ForkInput => "fork_input",
        MemorySourceTrust::ModelInferred => "model_inferred",
        MemorySourceTrust::Unknown => "unknown",
    }
}

fn print_entries_table(entries: &[MemoryEntry]) {
    println!("ID\tSTATUS\tRETENTION\tTRUST\tCREATED_AT\tEXPIRES_AT\tPATH");
    for entry in entries {
        println!(
            "{}\t{:?}\t{}\t{}\t{}\t{}\t{}",
            entry.id,
            entry.status,
            entry
                .retention_class
                .map(RetentionClass::as_str)
                .unwrap_or("standard"),
            entry.trust.as_deref().unwrap_or("unknown"),
            entry.created_at.as_deref().unwrap_or("-"),
            entry.expires_at.as_deref().unwrap_or("-"),
            entry.path.display()
        );
    }
}

fn print_ledger_table(entries: &[MemoryLedgerEntry]) {
    println!("ID\tREDACTED_AT\tDELETED_AT\tRETENTION\tSOURCE\tREASON\tPATH");
    for entry in entries {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            entry.id,
            entry.redacted_at.as_deref().unwrap_or("-"),
            entry.deleted_at.as_deref().unwrap_or("-"),
            entry
                .retention_class
                .map(RetentionClass::as_str)
                .unwrap_or("standard"),
            entry.source.as_deref().unwrap_or("-"),
            entry.reason.as_deref().unwrap_or("-"),
            entry.path.display()
        );
    }
}

fn print_usage() {
    eprintln!("Usage: coven-code memory <subcommand>");
    eprintln!("  list [--dir <path>] [--tenant <id>] [--repo <id>] [--domain <name>] [--json]");
    eprintln!("  expire <id-or-path> [--at YYYY-MM-DD] [--dir <path>] [--force]");
    eprintln!("  redact <id-or-path> --reason <text> [--dir <path>]");
    eprintln!("  delete <id-or-path> --reason <text> [--dir <path>] [--force]");
    eprintln!(
        "  delete --scope tenant=<t>,install=<i>,repo=<r>[,domain=<d>] --reason <text> [--force]"
    );
    eprintln!("  conflicts [--dir <team-memory-path>] [--json]");
    eprintln!("  resolve-conflict <key> [--dir <team-memory-path>]");
    eprintln!("  ledger [--dir <path>] [--json]");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).expect("write memory file");
        path
    }

    fn write_conflict_record(team_dir: &std::path::Path, key: &str) {
        let conflicts_dir = team_dir.join(".conflicts");
        std::fs::create_dir_all(&conflicts_dir).expect("create conflicts dir");
        let record = serde_json::json!({
            "conflict": {
                "key": key,
                "kind": "both_changed",
                "local_checksum": "aaa",
                "base_checksum": "bbb",
                "remote_checksum": "ccc",
                "reason": "local and remote both changed",
            }
        });
        std::fs::write(
            conflicts_dir.join(format!("{}.json", key.replace('/', "__"))),
            record.to_string(),
        )
        .expect("write conflict record");
    }

    #[test]
    fn conflicts_lists_pending_records_from_dir_override() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_conflict_record(dir.path(), "MEMORY.md");

        handle_conflicts(&[
            "--dir".to_string(),
            dir.path().to_string_lossy().to_string(),
        ])
        .expect("list conflicts");
        let pending = claurst_core::team_memory_sync::pending_conflicts(dir.path());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].key, "MEMORY.md");
    }

    #[test]
    fn resolve_conflict_removes_record_and_rejects_unknown_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_conflict_record(dir.path(), "MEMORY.md");

        handle_resolve_conflict(&[
            "MEMORY.md".to_string(),
            "--dir".to_string(),
            dir.path().to_string_lossy().to_string(),
        ])
        .expect("resolve conflict");
        assert!(claurst_core::team_memory_sync::pending_conflicts(dir.path()).is_empty());

        let missing = handle_resolve_conflict(&[
            "MEMORY.md".to_string(),
            "--dir".to_string(),
            dir.path().to_string_lossy().to_string(),
        ]);
        assert!(missing.is_err());
    }

    #[test]
    fn resolve_conflict_rejects_traversal_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = handle_resolve_conflict(&[
            "../escape.md".to_string(),
            "--dir".to_string(),
            dir.path().to_string_lossy().to_string(),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_entry_target_matches_memory_id_or_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(
            dir.path(),
            "auth.md",
            "---\nid: mem_auth\nsource: operator\n---\nauth memory",
        );
        let entries = collect_entries_from_dirs(&[dir.path().to_path_buf()]).expect("entries");

        let by_id = resolve_entry_target(&entries, "mem_auth").expect("resolve id");
        let by_path =
            resolve_entry_target(&entries, &path.to_string_lossy()).expect("resolve path");

        assert_eq!(by_id.path, path);
        assert_eq!(by_path.path, path);
    }

    #[test]
    fn ledger_export_only_includes_tombstone_stubs() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_file(
            dir.path(),
            "deleted.md",
            "---\nid: mem_deleted\ndeleted_at: 2026-07-07T00:00:00Z\nsource: deletion\n---\n\n[DELETED: user request]\n",
        );
        write_file(
            dir.path(),
            "active.md",
            "---\nid: mem_active\nsource: operator\n---\nactive sensitive body",
        );
        let entries = collect_entries_from_dirs(&[dir.path().to_path_buf()]).expect("entries");

        let ledger = export_ledger(&entries);

        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger[0].id, "mem_deleted");
        assert_eq!(ledger[0].reason.as_deref(), Some("[DELETED: user request]"));
        assert!(!format!("{ledger:?}").contains("active sensitive body"));
    }

    #[test]
    fn ledger_export_omits_noncanonical_tombstone_body() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_file(
            dir.path(),
            "deleted.md",
            "---\nid: mem_deleted\ndeleted_at: 2026-07-07T00:00:00Z\nsource: deletion\n---\n\noriginal sensitive body\n",
        );
        let entries = collect_entries_from_dirs(&[dir.path().to_path_buf()]).expect("entries");

        let ledger = export_ledger(&entries);

        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger[0].reason, None);
        assert!(!format!("{ledger:?}").contains("original sensitive body"));
    }

    #[test]
    fn collect_entries_from_dirs_is_not_limited_to_prompt_scan_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        for index in 0..=200 {
            write_file(
                dir.path(),
                &format!("memory-{index:03}.md"),
                &format!("---\nid: mem_{index:03}\nsource: operator\n---\nbody {index}"),
            );
        }

        let entries = collect_entries_from_dirs(&[dir.path().to_path_buf()]).expect("entries");

        assert_eq!(entries.len(), 201);
        assert!(entries.iter().any(|entry| entry.id == "mem_000"));
        assert!(entries.iter().any(|entry| entry.id == "mem_200"));
    }

    #[test]
    fn memory_status_marks_redacted_and_deleted_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_file(
            dir.path(),
            "redacted.md",
            "---\nid: mem_redacted\nredacted_at: 2026-07-07T00:00:00Z\nsource: redaction\n---\n\n[REDACTED: operator request]\n",
        );
        write_file(
            dir.path(),
            "deleted.md",
            "---\nid: mem_deleted\ndeleted_at: 2026-07-07T00:00:00Z\nsource: deletion\n---\n\n[DELETED: operator request]\n",
        );
        let entries = collect_entries_from_dirs(&[dir.path().to_path_buf()]).expect("entries");

        assert!(entries
            .iter()
            .any(|entry| entry.id == "mem_redacted" && entry.status == MemoryStatus::Redacted));
        assert!(entries
            .iter()
            .any(|entry| entry.id == "mem_deleted" && entry.status == MemoryStatus::Deleted));
    }
}
