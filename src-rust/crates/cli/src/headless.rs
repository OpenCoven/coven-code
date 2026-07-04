//! coven-github headless execution contract (contract version `1`).
//!
//! This module is the **coven-code side** of the interface locked in the
//! `coven-github` repo (`docs/headless-contract.md` + `docs/contracts/`). The
//! adapter (coven-github worker) writes a *tokenless* `session-brief.json` and
//! spawns the runtime headless:
//!
//! ```text
//! coven-code --headless --context <session-brief.json> --output <result.json>
//! ```
//!
//! The runtime reads the brief, runs the familiar to completion inside the
//! pre-cloned workspace, writes a `result.json` envelope, and exits with a
//! contract exit code (`0`/`1`/`2`/`3`).
//!
//! Security invariants (contract §5), enforced here:
//! 1. The brief is **tokenless** — there is no `auth`/`token` field and the
//!    clone URL carries no embedded credential.
//! 2. The only git credential channel is the [`GIT_TOKEN_ENV`] environment
//!    variable; it is never persisted to the brief, the result, or `.git/config`
//!    (see [`configure_git_auth`], which installs an env-backed credential
//!    helper — the token stays in the process environment).
//! 3. All filesystem writes stay inside `workspace.root`.
//!
//! A drift here is a contract break: bump `contract_version` on **both** sides,
//! update the schemas + fixtures, and ship a migration note — do not silently
//! widen the types. Golden fixtures live in `crates/cli/tests/headless_contract/`.

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Major contract version this build implements (contract §6).
pub const CONTRACT_VERSION: &str = "1";

/// Environment variable carrying the GitHub App **installation access token**
/// used to authenticate `git push`. This is the ONLY git credential channel; it
/// is never written to the brief, the result, or `.git/config`.
pub const GIT_TOKEN_ENV: &str = "COVEN_GIT_TOKEN";

fn default_contract_version() -> String {
    CONTRACT_VERSION.to_string()
}

/// True when a non-empty installation token is present in the environment.
pub fn git_token_present() -> bool {
    std::env::var(GIT_TOKEN_ENV)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

// ───────────────────────────── Input: session-brief.json ────────────────────
//
// The runtime is the *consumer*. We intentionally do NOT use
// `deny_unknown_fields`: the contract's versioning rules say a consumer must
// tolerate additive, backward-compatible fields within the same major version
// (contract §6). We DO reject a brief whose major version we don't implement.

#[derive(Debug, Clone, Deserialize)]
pub struct SessionBrief {
    #[serde(default = "default_contract_version")]
    pub contract_version: String,
    pub trigger: String,
    pub repo: RepoBrief,
    pub task: TaskBrief,
    pub familiar: FamiliarBrief,
    pub workspace: WorkspaceBrief,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RepoBrief {
    pub owner: String,
    pub name: String,
    /// HTTPS clone URL **without** embedded credentials. Auth comes from
    /// [`GIT_TOKEN_ENV`], not from the URL.
    pub clone_url: String,
    pub default_branch: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskBrief {
    FixIssue {
        issue_number: u64,
        issue_title: String,
        issue_body: String,
    },
    AddressReviewComment {
        pr_number: u64,
        comment_body: String,
        diff_hunk: Option<String>,
    },
    RespondToMention {
        issue_number: u64,
        comment_body: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct FamiliarBrief {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceBrief {
    pub root: String,
}

impl SessionBrief {
    /// Read + parse a brief from a `--context` path. Returns `Ok(None)` when no
    /// path is given. Rejects a brief whose major contract version this build
    /// does not implement (contract §6).
    pub fn load(path: Option<&PathBuf>) -> anyhow::Result<Option<Self>> {
        let Some(path) = path else {
            return Ok(None);
        };
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read --context brief at {}", path.display()))?;
        let brief: SessionBrief = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse --context brief at {}", path.display()))?;
        brief.ensure_supported_version()?;
        Ok(Some(brief))
    }

    /// Reject a brief whose major version is not implemented. Contract §6:
    /// "Consumers MUST reject a payload whose major version they do not
    /// implement, rather than silently mis-parsing it."
    pub fn ensure_supported_version(&self) -> anyhow::Result<()> {
        if self.contract_version != CONTRACT_VERSION {
            bail!(
                "unsupported headless contract version {:?}; this build implements {:?}",
                self.contract_version,
                CONTRACT_VERSION
            );
        }
        Ok(())
    }

    /// Absolute path to the pre-cloned, isolated workspace. The runtime operates
    /// only inside this directory.
    pub fn workspace_root(&self) -> PathBuf {
        PathBuf::from(&self.workspace.root)
    }

    pub fn default_branch(&self) -> &str {
        &self.repo.default_branch
    }

    /// True for tasks that can legitimately complete without a code change (a
    /// mention that only wants a reply). Used when classifying a "no commits"
    /// completion — for those, no diff is still a success.
    pub fn is_comment_only(&self) -> bool {
        matches!(self.task, TaskBrief::RespondToMention { .. })
    }

    /// Build the first-turn user prompt injected into the headless session.
    pub fn to_prompt(&self) -> String {
        let mut lines = vec![
            format!(
                "You are {}, the Coven coding familiar assigned to {}/{} through the coven-github App.",
                self.familiar.display_name, self.repo.owner, self.repo.name
            ),
            format!("Trigger: {}", self.trigger),
            format!("Default branch: {}", self.repo.default_branch),
            format!("Repository: {}", self.repo.clone_url),
            format!(
                "Workspace (operate only inside this directory): {}",
                self.workspace.root
            ),
            format!(
                "Git push auth: {}. Credentials are pre-configured from the {} environment — \
                 push over the existing HTTPS remote and never embed a token in a URL, in git \
                 config, or in any file.",
                if git_token_present() {
                    "available"
                } else {
                    "unavailable"
                },
                GIT_TOKEN_ENV
            ),
            String::new(),
            self.task_instructions(),
        ];

        if !self.familiar.skills.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "Apply these skills where relevant: {}.",
                self.familiar.skills.join(", ")
            ));
        }

        lines.push(String::new());
        lines.push(
            "Complete the task end to end: make the change on a new branch named like \
             `<familiar>/<short-slug>`, run focused verification, commit, and push the branch. \
             End your final message with a short, familiar-voice PR description (a `## ` heading, \
             what changed, and why) — that message becomes the pull request body."
                .to_string(),
        );
        lines.join("\n")
    }

    fn task_instructions(&self) -> String {
        match &self.task {
            TaskBrief::FixIssue {
                issue_number,
                issue_title,
                issue_body,
            } => format!("Fix issue #{issue_number}: {issue_title}\n\nIssue body:\n{issue_body}"),
            TaskBrief::AddressReviewComment {
                pr_number,
                comment_body,
                diff_hunk,
            } => {
                let mut prompt = format!(
                    "Address the review comment on PR #{pr_number}.\n\nComment:\n{comment_body}"
                );
                if let Some(hunk) = diff_hunk {
                    prompt.push_str("\n\nDiff hunk:\n");
                    prompt.push_str(hunk);
                }
                prompt
            }
            TaskBrief::RespondToMention {
                issue_number,
                comment_body,
            } => format!(
                "Respond to the mention on issue #{issue_number}.\n\nComment:\n{comment_body}"
            ),
        }
    }
}

/// Apply a brief to the effective config: force bypass-permissions (there is no
/// TTY to approve prompts), pin the familiar's model when set, and append repo /
/// familiar context to the system prompt.
pub fn apply_to_config(config: &mut claurst_core::config::Config, brief: &SessionBrief) {
    if let Some(model) = &brief.familiar.model {
        config.model = Some(model.clone());
    }
    config.permission_mode = claurst_core::config::PermissionMode::BypassPermissions;

    let context = format!(
        "coven-github headless task for familiar {} ({}). Repository: {}/{}. Default branch: {}. Workspace: {}.",
        brief.familiar.display_name,
        brief.familiar.id,
        brief.repo.owner,
        brief.repo.name,
        brief.repo.default_branch,
        brief.workspace.root
    );
    config.append_system_prompt = Some(match config.append_system_prompt.take() {
        Some(existing) => format!("{existing}\n\n{context}"),
        None => context,
    });
}

// ─────────────────────────── Git auth (COVEN_GIT_TOKEN) ─────────────────────

/// Configure the workspace repo so `git push` authenticates with the
/// installation token from [`GIT_TOKEN_ENV`], without ever writing the token to
/// disk.
///
/// Installs a local credential helper that reads the token from the environment
/// at push time. Only the helper *script* (which references the env var by name)
/// is stored in `.git/config`; the token value stays in the process
/// environment. Returns `Ok(true)` when a helper was installed, `Ok(false)` when
/// there is no token or no repo.
pub fn configure_git_auth(workspace: &Path) -> anyhow::Result<bool> {
    if !git_token_present() {
        return Ok(false);
    }
    if !workspace.join(".git").exists() {
        return Ok(false);
    }

    // Credential helper (git's protocol): on `get`, print username + password.
    // `$COVEN_GIT_TOKEN` is expanded by the shell at push time, so the literal
    // token never touches `.git/config`.
    let helper = "!f() { test \"$1\" = get && \
                  printf 'username=x-access-token\\npassword=%s\\n' \"$COVEN_GIT_TOKEN\"; }; f";

    // Reset any inherited helper chain, then install ours as the sole helper.
    run_git(
        workspace,
        &[
            "config",
            "--local",
            "--replace-all",
            "credential.helper",
            "",
        ],
    )
    .context("failed to reset credential.helper")?;
    run_git(
        workspace,
        &["config", "--local", "--add", "credential.helper", helper],
    )
    .context("failed to install env-backed credential.helper")?;

    Ok(true)
}

fn run_git(workspace: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(workspace)
        .status()
        .with_context(|| format!("failed to spawn git {}", args.join(" ")))?;
    if !status.success() {
        bail!("git {} exited with {}", args.join(" "), status);
    }
    Ok(())
}

// ─────────────────────────── Git result collection ─────────────────────────

/// Terminal git state of the workspace after the session, used to fill the
/// result envelope.
#[derive(Debug, Clone, Default)]
pub struct GitSummary {
    pub branch: Option<String>,
    pub commits: Vec<CommitSummary>,
    pub files_changed: Vec<String>,
}

/// Inspect the workspace and summarize the branch, the commits ahead of the base
/// branch, and the changed files.
pub fn collect_git_summary(workspace: &Path, default_branch: &str) -> GitSummary {
    let branch =
        git_stdout(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).filter(|b| !b.is_empty());

    let base = default_branch;
    let log_range = format!("origin/{base}..HEAD");
    let commits = git_stdout(workspace, &["log", "--format=%H%x00%s", &log_range])
        .or_else(|| {
            git_stdout(
                workspace,
                &["log", "--format=%H%x00%s", &format!("{base}..HEAD")],
            )
        })
        .map(|out| {
            out.lines()
                .filter_map(|line| {
                    let (sha, message) = line.split_once('\0')?;
                    Some(CommitSummary {
                        sha: sha.to_string(),
                        message: message.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let files_changed = git_stdout(workspace, &["diff", "--name-only", "HEAD"])
        .into_iter()
        .chain(git_stdout(workspace, &["diff", "--name-only", "--cached"]))
        .chain(git_stdout(
            workspace,
            &["diff", "--name-only", &format!("origin/{base}...HEAD")],
        ))
        // Fall back to a local base branch when there is no `origin/<base>`
        // (e.g. a base tracked locally, or a differently-named remote).
        .chain(git_stdout(
            workspace,
            &["diff", "--name-only", &format!("{base}...HEAD")],
        ))
        .flat_map(|out| out.lines().map(str::to_string).collect::<Vec<_>>())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    GitSummary {
        branch,
        commits,
        files_changed,
    }
}

fn git_stdout(workspace: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

// ───────────────────────────── Output: result.json ─────────────────────────

/// Terminal task status (contract §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Success,
    Partial,
    Failure,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "contract v1 reserves needs_input for the M2 clarification path"
        )
    )]
    NeedsInput,
}

/// Terminal cause when the run did not succeed (contract §3.3). `None` on
/// success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "contract v1 reserves test_failure for future verifier integration"
        )
    )]
    TestFailure,
    AmbiguousSpec,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "contract v1 reserves git_conflict for future git conflict detection"
        )
    )]
    GitConflict,
    InfraError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitSummary {
    pub sha: String,
    pub message: String,
}

/// The `result.json` envelope (contract §3). Field order + presence match
/// `result.schema.json`; `branch`/`exit_reason` serialize as `null` when empty.
#[derive(Debug, Clone, Serialize)]
pub struct ResultEnvelope {
    pub contract_version: String,
    pub status: Status,
    pub branch: Option<String>,
    pub commits: Vec<CommitSummary>,
    pub files_changed: Vec<String>,
    pub summary: String,
    pub pr_body: String,
    pub exit_reason: Option<ExitReason>,
}

/// How the headless run terminated, decoupled from the query crate so this
/// module stays independently testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The model finished its turn cleanly (`end_turn`).
    Completed,
    /// The model hit the max-token ceiling mid-work (truncated).
    Truncated,
    /// The run was cancelled (no interactive user in headless → treated as
    /// retry-safe infra).
    Cancelled,
    /// A model/API/tool error — retry-safe infra failure.
    Errored,
    /// The USD budget cap tripped.
    BudgetExceeded,
}

/// Value returned by the headless runner: how it ended plus the familiar's final
/// prose (which becomes the PR body).
pub struct HeadlessRun {
    pub outcome: RunOutcome,
    pub final_text: String,
}

/// Map a run outcome to the contract's `(status, exit_reason, exit_code)`.
///
/// The exit **code** is authoritative for the adapter's dispatch (contract §4);
/// `status`/`exit_reason` are advisory detail. `needs_input` (exit `3`) is not
/// auto-emitted in M1 — it requires the agent to explicitly raise a clarifying
/// question, which is wired for M2. The variant + code path exist so that
/// wiring is additive.
fn classify(
    outcome: RunOutcome,
    has_commits: bool,
    comment_only: bool,
) -> (Status, Option<ExitReason>, i32) {
    match outcome {
        // Retry-safe infra failures → exit 2 (contract §4).
        RunOutcome::Errored | RunOutcome::Cancelled => {
            (Status::Failure, Some(ExitReason::InfraError), 2)
        }
        RunOutcome::Completed | RunOutcome::Truncated | RunOutcome::BudgetExceeded => {
            if has_commits {
                if outcome == RunOutcome::Completed {
                    (Status::Success, None, 0)
                } else {
                    // Progress committed but the run stopped early (truncated /
                    // budget). The adapter still opens a PR from the commits.
                    (Status::Partial, None, 0)
                }
            } else if comment_only && outcome == RunOutcome::Completed {
                // A reply-only task legitimately produces no diff.
                (Status::Success, None, 0)
            } else {
                // Finished with no diff on a change task: not retry-safe, so
                // exit 1 (the adapter must not retry). AmbiguousSpec is the
                // closest terminal cause.
                (Status::Failure, Some(ExitReason::AmbiguousSpec), 1)
            }
        }
    }
}

/// Build the `result.json` envelope and the process exit code from the run.
pub fn build_result(
    brief: Option<&SessionBrief>,
    git: &GitSummary,
    outcome: RunOutcome,
    final_text: &str,
) -> (ResultEnvelope, i32) {
    let comment_only = brief.map(SessionBrief::is_comment_only).unwrap_or(false);
    let (status, exit_reason, code) = classify(outcome, !git.commits.is_empty(), comment_only);

    let summary = compose_summary(brief, final_text, git, status);
    let pr_body = compose_pr_body(brief, final_text, git, status);

    let envelope = ResultEnvelope {
        contract_version: CONTRACT_VERSION.to_string(),
        status,
        branch: git.branch.clone(),
        commits: git.commits.clone(),
        files_changed: git.files_changed.clone(),
        summary,
        pr_body,
        exit_reason,
    };
    (envelope, code)
}

/// Build a result envelope for an infrastructure failure that happened before or
/// around the run (e.g. an unparseable brief). Exit `2` — retry-safe.
pub fn infra_error_result(
    brief: Option<&SessionBrief>,
    git: &GitSummary,
    message: &str,
) -> (ResultEnvelope, i32) {
    let name = familiar_name(brief);
    let envelope = ResultEnvelope {
        contract_version: CONTRACT_VERSION.to_string(),
        status: Status::Failure,
        branch: git.branch.clone(),
        commits: git.commits.clone(),
        files_changed: git.files_changed.clone(),
        summary: format!("{name} hit an infrastructure error before completing the task."),
        pr_body: format!(
            "## {name}\n\nThe headless session failed before completing the task:\n\n```\n{message}\n```"
        ),
        exit_reason: Some(ExitReason::InfraError),
    };
    (envelope, 2)
}

/// Serialize + write the result envelope to the `--output` path.
pub fn write_result(path: &Path, envelope: &ResultEnvelope) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(envelope).context("serialize result envelope")?;
    std::fs::write(path, json)
        .with_context(|| format!("failed to write result envelope to {}", path.display()))?;
    Ok(())
}

fn familiar_name(brief: Option<&SessionBrief>) -> String {
    brief
        .map(|b| b.familiar.display_name.clone())
        .unwrap_or_else(|| "Coven Code".to_string())
}

/// One-line, familiar-voice summary (Check Run + PR title). Prefers the first
/// meaningful line of the familiar's own final message.
fn compose_summary(
    brief: Option<&SessionBrief>,
    final_text: &str,
    git: &GitSummary,
    status: Status,
) -> String {
    if let Some(line) = first_meaningful_line(final_text) {
        return truncate_summary(&line);
    }
    let name = familiar_name(brief);
    let commits = git.commits.len();
    let files = git.files_changed.len();
    match status {
        Status::Success | Status::Partial => format!(
            "{name} made {commits} commit{} across {files} file{}.",
            plural(commits),
            plural(files)
        ),
        Status::Failure | Status::NeedsInput => {
            format!("{name} could not complete the task.")
        }
    }
}

/// Full PR body, authored by the familiar (contract §3.1: "not a template").
/// Prefers the familiar's final prose; falls back to a generated body only when
/// the model produced no closing text.
fn compose_pr_body(
    brief: Option<&SessionBrief>,
    final_text: &str,
    git: &GitSummary,
    status: Status,
) -> String {
    let trimmed = final_text.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }

    let name = familiar_name(brief);
    let files = if git.files_changed.is_empty() {
        "_No files changed._".to_string()
    } else {
        git.files_changed
            .iter()
            .map(|f| format!("- `{f}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let headline = match status {
        Status::Success => "Completed the requested change.",
        Status::Partial => "Made partial progress on the requested change.",
        Status::Failure => "Could not complete the requested change.",
        Status::NeedsInput => "Need clarification before continuing.",
    };
    format!("## {name}\n\n{headline}\n\n### Files changed\n\n{files}")
}

fn first_meaningful_line(text: &str) -> Option<String> {
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('#').trim();
        if !line.is_empty() {
            return Some(line.to_string());
        }
    }
    None
}

fn truncate_summary(line: &str) -> String {
    const MAX: usize = 140;
    if line.chars().count() <= MAX {
        return line.to_string();
    }
    let truncated: String = line.chars().take(MAX - 1).collect();
    format!("{}…", truncated.trim_end())
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

// ─────────────────────────────────── Tests ─────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::Mutex;

    /// Serializes the tests that mutate the process-global `COVEN_GIT_TOKEN`
    /// env var, so parallel test execution can't interleave their set/remove.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/tests/headless_contract/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("missing fixture {path}: {e}"))
    }

    fn sample_brief() -> SessionBrief {
        serde_json::from_str(&fixture("session-brief.example.json")).expect("golden brief parses")
    }

    // ── Input conformance ───────────────────────────────────────────────────

    #[test]
    fn golden_brief_parses_into_runtime_type() {
        let brief = sample_brief();
        assert_eq!(brief.contract_version, CONTRACT_VERSION);
        assert_eq!(brief.trigger, "issue_assigned");
        assert_eq!(brief.repo.owner, "OpenCoven");
        assert_eq!(brief.repo.default_branch, "main");
        assert_eq!(brief.familiar.id, "cody");
        assert!(matches!(
            brief.task,
            TaskBrief::FixIssue {
                issue_number: 42,
                ..
            }
        ));
        brief.ensure_supported_version().expect("v1 is supported");
    }

    #[test]
    fn brief_clone_url_carries_no_embedded_credential() {
        // Contract §2.1 + security invariant §5.1: the clone URL is tokenless.
        let brief = sample_brief();
        assert!(
            !brief.repo.clone_url.contains('@'),
            "clone_url leaked an embedded credential: {}",
            brief.repo.clone_url
        );
    }

    #[test]
    fn tokenless_brief_is_accepted_no_auth_field_required() {
        // The adapter emits a tokenless brief; the runtime must NOT require an
        // `auth`/`token` field to parse it.
        let raw = r#"{
            "contract_version": "1",
            "trigger": "issue_assigned",
            "repo": { "owner": "o", "name": "r", "clone_url": "https://github.com/o/r.git", "default_branch": "main" },
            "task": { "kind": "fix_issue", "issue_number": 1, "issue_title": "t", "issue_body": "b" },
            "familiar": { "id": "cody", "display_name": "Cody", "skills": [] },
            "workspace": { "root": "/tmp/ws" }
        }"#;
        let brief: SessionBrief = serde_json::from_str(raw).expect("tokenless brief must parse");
        assert_eq!(brief.familiar.display_name, "Cody");
    }

    #[test]
    fn brief_missing_contract_version_defaults_to_current() {
        let raw = r#"{
            "trigger": "issue_mention",
            "repo": { "owner": "o", "name": "r", "clone_url": "https://github.com/o/r.git", "default_branch": "main" },
            "task": { "kind": "respond_to_mention", "issue_number": 7, "comment_body": "hi" },
            "familiar": { "id": "cody", "display_name": "Cody", "skills": [] },
            "workspace": { "root": "/tmp/ws" }
        }"#;
        let brief: SessionBrief = serde_json::from_str(raw).expect("versionless brief parses");
        assert_eq!(brief.contract_version, CONTRACT_VERSION);
        brief
            .ensure_supported_version()
            .expect("defaulted version is supported");
    }

    #[test]
    fn brief_tolerates_unknown_forward_compatible_fields() {
        // Contract §6: additive fields within a major version are backward
        // compatible; the consumer must not choke on them.
        let raw = r#"{
            "contract_version": "1",
            "trigger": "issue_assigned",
            "repo": { "owner": "o", "name": "r", "clone_url": "https://github.com/o/r.git", "default_branch": "main", "topics": ["x"] },
            "task": { "kind": "fix_issue", "issue_number": 1, "issue_title": "t", "issue_body": "b" },
            "familiar": { "id": "cody", "display_name": "Cody", "skills": [] },
            "workspace": { "root": "/tmp/ws" },
            "future_field": { "anything": true }
        }"#;
        let brief: SessionBrief = serde_json::from_str(raw).expect("unknown fields tolerated");
        assert_eq!(brief.repo.owner, "o");
    }

    #[test]
    fn rejects_unsupported_major_version() {
        let mut brief = sample_brief();
        brief.contract_version = "2".to_string();
        assert!(
            brief.ensure_supported_version().is_err(),
            "a v2 brief must be rejected by a v1 runtime"
        );
    }

    #[test]
    fn prompt_is_derived_from_task_and_never_leaks_a_token() {
        let brief = sample_brief();
        let prompt = brief.to_prompt();
        assert!(prompt.contains("Fix issue #42: Fix OAuth token refresh"));
        assert!(prompt.contains("clock skew"));
        assert!(prompt.contains("systematic-debugging"));
        assert!(prompt.contains("COVEN_GIT_TOKEN"));
        // The prompt references the env var by name but never a token value.
        assert!(!prompt.contains("x-access-token:"));
    }

    // ── Output conformance ──────────────────────────────────────────────────

    #[test]
    fn every_status_serializes_to_its_wire_name() {
        for (variant, wire) in [
            (Status::Success, "success"),
            (Status::Partial, "partial"),
            (Status::Failure, "failure"),
            (Status::NeedsInput, "needs_input"),
        ] {
            assert_eq!(serde_json::to_value(variant).unwrap(), json!(wire));
        }
    }

    #[test]
    fn every_exit_reason_serializes_to_its_wire_name() {
        for (variant, wire) in [
            (ExitReason::TestFailure, "test_failure"),
            (ExitReason::AmbiguousSpec, "ambiguous_spec"),
            (ExitReason::GitConflict, "git_conflict"),
            (ExitReason::InfraError, "infra_error"),
        ] {
            assert_eq!(serde_json::to_value(variant).unwrap(), json!(wire));
        }
    }

    /// Minimal, dependency-free JSON Schema check against the vendored
    /// `result.schema.json`: required keys present, no keys outside `properties`
    /// (additionalProperties:false), and enum-typed fields carry allowed values.
    fn assert_matches_result_schema(value: &Value) {
        let schema: Value =
            serde_json::from_str(&fixture("result.schema.json")).expect("schema parses");
        let obj = value.as_object().expect("result is an object");
        let props = schema["properties"].as_object().unwrap();

        for req in schema["required"].as_array().unwrap() {
            let key = req.as_str().unwrap();
            assert!(obj.contains_key(key), "result missing required key `{key}`");
        }
        for key in obj.keys() {
            assert!(
                props.contains_key(key),
                "result has key `{key}` not permitted by schema (additionalProperties:false)"
            );
        }
        assert_eq!(obj["contract_version"], json!("1"));

        let status_enum = props["status"]["enum"].as_array().unwrap();
        assert!(status_enum.contains(&obj["status"]), "status out of enum");

        // exit_reason may be null or one of the enum values.
        let er = &obj["exit_reason"];
        if !er.is_null() {
            let er_enum = props["exit_reason"]["enum"].as_array().unwrap();
            assert!(er_enum.contains(er), "exit_reason `{er}` out of enum");
        }
    }

    #[test]
    fn success_envelope_validates_against_result_schema() {
        let git = GitSummary {
            branch: Some("cody/fix-issue-42".to_string()),
            commits: vec![CommitSummary {
                sha: "a1b2c3d".to_string(),
                message: "Add clock-skew buffer".to_string(),
            }],
            files_changed: vec!["src/auth/refresh.rs".to_string()],
        };
        let (env, code) = build_result(
            Some(&sample_brief()),
            &git,
            RunOutcome::Completed,
            "## Hey, I'm Cody\n\nAdded a 60s clock-skew buffer to the refresh path.",
        );
        assert_eq!(code, 0);
        assert_eq!(env.status, Status::Success);
        assert!(env.exit_reason.is_none());
        let value = serde_json::to_value(&env).unwrap();
        assert_matches_result_schema(&value);
    }

    #[test]
    fn infra_error_envelope_validates_and_is_retry_safe() {
        let (env, code) = infra_error_result(None, &GitSummary::default(), "workspace vanished");
        assert_eq!(code, 2, "infra error is retry-safe (exit 2)");
        assert_eq!(env.status, Status::Failure);
        assert_eq!(env.exit_reason, Some(ExitReason::InfraError));
        let value = serde_json::to_value(&env).unwrap();
        assert_matches_result_schema(&value);
    }

    // ── Exit-code contract (§4) ─────────────────────────────────────────────

    #[test]
    fn classify_maps_outcomes_to_contract_exit_codes() {
        // (outcome, has_commits, comment_only) → (status, exit_reason, code)
        let cases = [
            (RunOutcome::Completed, true, false, Status::Success, None, 0),
            (
                RunOutcome::Completed,
                false,
                false,
                Status::Failure,
                Some(ExitReason::AmbiguousSpec),
                1,
            ),
            (RunOutcome::Completed, false, true, Status::Success, None, 0),
            (RunOutcome::Truncated, true, false, Status::Partial, None, 0),
            (
                RunOutcome::BudgetExceeded,
                true,
                false,
                Status::Partial,
                None,
                0,
            ),
            (
                RunOutcome::Errored,
                false,
                false,
                Status::Failure,
                Some(ExitReason::InfraError),
                2,
            ),
            (
                RunOutcome::Cancelled,
                true,
                false,
                Status::Failure,
                Some(ExitReason::InfraError),
                2,
            ),
        ];
        for (outcome, has_commits, comment_only, status, reason, code) in cases {
            let got = classify(outcome, has_commits, comment_only);
            assert_eq!(got, (status, reason, code), "outcome {outcome:?}");
        }
    }

    #[test]
    fn pr_body_prefers_the_familiars_own_words() {
        let git = GitSummary {
            branch: Some("cody/x".to_string()),
            commits: vec![CommitSummary {
                sha: "deadbee".to_string(),
                message: "do the thing".to_string(),
            }],
            files_changed: vec!["a.rs".to_string()],
        };
        let (env, _) = build_result(
            Some(&sample_brief()),
            &git,
            RunOutcome::Completed,
            "## Fixed it\n\nI added the buffer and it works now.",
        );
        assert_eq!(
            env.pr_body,
            "## Fixed it\n\nI added the buffer and it works now."
        );
        assert_eq!(env.summary, "Fixed it");
    }

    #[test]
    fn pr_body_falls_back_to_generated_body_when_model_is_silent() {
        let git = GitSummary {
            branch: Some("cody/x".to_string()),
            commits: vec![CommitSummary {
                sha: "deadbee".to_string(),
                message: "m".to_string(),
            }],
            files_changed: vec!["a.rs".to_string(), "b.rs".to_string()],
        };
        let (env, _) = build_result(Some(&sample_brief()), &git, RunOutcome::Completed, "   ");
        assert!(env.pr_body.contains("Cody"));
        assert!(env.pr_body.contains("`a.rs`"));
        assert!(env.summary.contains("1 commit"), "summary: {}", env.summary);
        assert!(env.summary.contains("2 file"), "summary: {}", env.summary);
    }

    #[test]
    fn summary_is_truncated_to_a_single_short_line() {
        let long = "x".repeat(500);
        let out = truncate_summary(&long);
        assert!(
            out.chars().count() <= 140,
            "summary too long: {}",
            out.chars().count()
        );
        assert!(out.ends_with('…'));
    }

    // ── Git integration (temp repo) ─────────────────────────────────────────

    #[test]
    fn collect_git_summary_reports_branch_commits_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(ws)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "cody@opencoven.ai"]);
        git(&["config", "user.name", "Cody"]);
        std::fs::write(ws.join("base.txt"), "base").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "base"]);
        // A working branch with one new commit + one changed file.
        git(&["switch", "-q", "-c", "cody/fix"]);
        std::fs::write(ws.join("feature.rs"), "fn main() {}").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "add feature"]);

        let summary = collect_git_summary(ws, "main");
        assert_eq!(summary.branch.as_deref(), Some("cody/fix"));
        assert_eq!(summary.commits.len(), 1, "one commit ahead of main");
        assert_eq!(summary.commits[0].message, "add feature");
        assert!(summary.files_changed.iter().any(|f| f == "feature.rs"));
    }

    #[test]
    fn configure_git_auth_installs_env_backed_helper_without_leaking_token() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(ws)
            .status()
            .unwrap()
            .success());

        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Distinctive token value we assert never lands in .git/config.
        let token = "ghs_HEADLESS_TEST_TOKEN_do_not_persist";
        std::env::set_var(GIT_TOKEN_ENV, token);
        let installed = configure_git_auth(ws).expect("configure ok");
        std::env::remove_var(GIT_TOKEN_ENV);

        assert!(installed, "helper should be installed when token present");
        let config = std::fs::read_to_string(ws.join(".git/config")).unwrap();
        assert!(
            config.contains("credential"),
            "config should carry a credential helper: {config}"
        );
        assert!(
            config.contains("$COVEN_GIT_TOKEN"),
            "helper should reference the env var, not inline the token: {config}"
        );
        assert!(
            !config.contains(token),
            "token value must never be written to .git/config: {config}"
        );
    }

    #[test]
    fn configure_git_auth_is_a_noop_without_a_token() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::remove_var(GIT_TOKEN_ENV);
        let installed = configure_git_auth(dir.path()).expect("configure ok");
        assert!(!installed, "no token → no helper installed");
    }
}
