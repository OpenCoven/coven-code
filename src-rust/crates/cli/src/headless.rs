//! coven-github headless execution contract (contract version `2`).
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
use serde_json::Value;
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// Major contract version this build implements (contract §6).
pub const CONTRACT_VERSION: &str = "2";

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
    #[serde(default)]
    pub review_context: Option<ReviewContext>,
    #[serde(default)]
    pub audit_instruction: Option<String>,
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

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewContext {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub files: Vec<ReviewContextFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewContextFile {
    pub filename: String,
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
            || self.review_mode() != ReviewMode::None
    }

    pub fn review_mode(&self) -> ReviewMode {
        if matches!(self.task, TaskBrief::AddressReviewComment { .. }) {
            return ReviewMode::ReviewComment;
        }
        if self
            .review_context
            .as_ref()
            .and_then(|context| context.kind.as_deref())
            == Some("pull_request")
        {
            return ReviewMode::PullRequest;
        }
        ReviewMode::None
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

        if self.review_mode() != ReviewMode::None {
            lines.push(String::new());
            lines.push("Review mode: inspect the changed files and read relevant supporting code before reaching conclusions. Use Read, Grep, or Glob for the supporting context you rely on.".to_string());
            lines.push(
                "Keep the review bounded: start from the changed files and patch text already supplied in this brief, read only targeted supporting files you expect to cite, and avoid broad repository scans unless a concrete finding requires them."
                    .to_string(),
            );
            lines.push(
                "After inspecting the changed files and the relevant supporting context, stop using tools and write the final review sections."
                    .to_string(),
            );
            lines.push("Your final review must use these exact markdown sections:".to_string());
            lines.push("### Files inspected".to_string());
            lines.push("List the changed files you inspected.".to_string());
            lines.push("### Supporting context used".to_string());
            lines.push("List supporting files you inspected and why each mattered.".to_string());
            lines.push("### Findings".to_string());
            lines.push("List each finding as `- [severity] `path:line` Title - body. Recommendation: ...`, or write `None`.".to_string());
            lines.push("### No-findings justification".to_string());
            lines.push("If there are no findings, explain why with specific file references from the changed or supporting files.".to_string());
            lines.push("### Tests/commands considered".to_string());
            lines.push(
                "List commands as `- `command` - passed|failed|not run: summary`.".to_string(),
            );
            lines.push("### Confidence/limitations".to_string());
            lines.push("State confidence and any limitations. Do not end with a generic completion message.".to_string());
        }

        if let Some(instruction) = self
            .audit_instruction
            .as_ref()
            .filter(|s| !s.trim().is_empty())
        {
            lines.push(String::new());
            lines.push("Additional review instruction:".to_string());
            lines.push(instruction.trim().to_string());
        }

        if self.review_mode() != ReviewMode::None {
            lines.push(String::new());
            lines.push(
                "Complete the review end to end. Do not modify files, create commits, or push a branch unless the review comment explicitly asks for code changes. Your final message is the hosted review body and must include the exact review sections above."
                    .to_string(),
            );
            return lines.join("\n");
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
    if brief.review_mode() != ReviewMode::None {
        config.hosted_review.enabled = true;
        config.hosted_review.allow_user_memory = false;
        config.hosted_review.allow_write_tools = false;
        config.hosted_review.allow_mcp_servers = false;
        config.hosted_review.allow_plugins = false;
        config.hosted_review.allow_auto_memory_persistence = false;
    }

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
            reason = "contract v2 reserves needs_input for the M2 clarification path"
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
            reason = "contract v2 reserves test_failure for future verifier integration"
        )
    )]
    TestFailure,
    AmbiguousSpec,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "contract v2 reserves git_conflict for future git conflict detection"
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
    pub review: ReviewResult,
    pub exit_reason: Option<ExitReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewResult {
    pub mode: ReviewMode,
    pub evidence_status: ReviewEvidenceStatus,
    pub reviewed_files: Vec<String>,
    pub supporting_files: Vec<String>,
    pub findings: Vec<ReviewFinding>,
    pub tests_run: Vec<ReviewTestRun>,
    pub no_findings_reason: Option<String>,
    pub limitations: Vec<String>,
    /// Memory entries and domains that were loaded for this review, so the
    /// artifact records the trust level and provenance scope of every memory
    /// input that could have influenced findings.
    pub memory: ReviewMemoryUse,
}

/// Memory usage report attached to a review artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ReviewMemoryUse {
    /// Hosted memory domains that were eligible for this review (e.g.
    /// `default-branch`). Empty for local, non-hosted runs.
    pub domains_loaded: Vec<String>,
    /// Every memory entry loaded into the review context.
    pub entries: Vec<ReviewMemoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewMemoryEntry {
    /// Stable memory id (frontmatter `id` or content hash).
    pub id: String,
    /// Effective trust after hosted caps/floors (kebab-case label).
    pub trust: String,
    /// Declared visibility, when present.
    pub visibility: Option<String>,
    /// Memory scope the entry was loaded from (managed/user/project/local).
    pub scope: String,
}

/// Enumerate the memory entries and domains the current configuration loads
/// for a review of `workspace_root`. Uses the same load options as the live
/// context build, so the report matches what the model actually saw.
pub fn collect_review_memory(
    workspace_root: &Path,
    config: &claurst_core::config::Config,
) -> ReviewMemoryUse {
    let options = config.memory_load_options();
    let files =
        claurst_core::claudemd::load_all_memory_files_with_options(workspace_root, &options);
    let entries = files
        .iter()
        .map(|file| ReviewMemoryEntry {
            id: claurst_core::claudemd::memory_id(file),
            trust: serde_enum_label(&claurst_core::claudemd::effective_memory_trust(
                file, &options,
            )),
            visibility: file.frontmatter.visibility.map(|v| serde_enum_label(&v)),
            scope: serde_enum_label(&file.scope),
        })
        .collect();
    let domains_loaded = if config.hosted_review_enabled() {
        // Hosted review currently loads only the default-branch domain;
        // security-private and branch domains are excluded by policy.
        vec![claurst_core::hosted_review::MemoryDomain::DefaultBranch.path_component()]
    } else {
        Vec::new()
    };
    ReviewMemoryUse {
        domains_loaded,
        entries,
    }
}

/// Render a unit enum's serde label (kebab/snake-case string form).
fn serde_enum_label<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(label)) => label,
        _ => "unknown".to_string(),
    }
}

impl ReviewResult {
    pub fn none() -> Self {
        Self {
            mode: ReviewMode::None,
            evidence_status: ReviewEvidenceStatus::NotApplicable,
            reviewed_files: Vec::new(),
            supporting_files: Vec::new(),
            findings: Vec::new(),
            tests_run: Vec::new(),
            no_findings_reason: None,
            limitations: Vec::new(),
            memory: ReviewMemoryUse::default(),
        }
    }

    pub fn from_brief(
        brief: Option<&SessionBrief>,
        trace: Option<&ReviewTrace>,
        final_text: &str,
    ) -> Self {
        Self::from_brief_with_memory(brief, trace, final_text, ReviewMemoryUse::default())
    }

    pub fn from_brief_with_memory(
        brief: Option<&SessionBrief>,
        trace: Option<&ReviewTrace>,
        final_text: &str,
        memory: ReviewMemoryUse,
    ) -> Self {
        let Some(brief) = brief else {
            return Self::none();
        };
        let mode = brief.review_mode();
        if mode == ReviewMode::None {
            return Self::none();
        }

        let reviewed_files = brief
            .review_context
            .as_ref()
            .map(|context| {
                context
                    .files
                    .iter()
                    .map(|file| file.filename.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let supporting_files = trace
            .map(|trace| trace.supporting_files(&reviewed_files))
            .unwrap_or_default();

        let mut limitations = Vec::new();
        let evidence_status = if reviewed_files.is_empty() {
            limitations.push("No PR file evidence was supplied in the session brief.".to_string());
            ReviewEvidenceStatus::Missing
        } else {
            ReviewEvidenceStatus::Complete
        };
        let has_supporting_evidence = !supporting_files.is_empty();
        if !has_supporting_evidence {
            limitations.push(
                "No supporting-code file reads or search results were captured during this review."
                    .to_string(),
            );
        }
        let parsed = ParsedReviewOutput::from_text(final_text, &reviewed_files, &supporting_files);
        limitations.extend(parsed.limitations.clone());

        if !parsed.has_substantive_review {
            limitations.push(
                "Review output did not include structured findings, a file-backed no-findings justification, or an explicit limitation explaining why substantive review was not possible."
                    .to_string(),
            );
        }

        Self {
            mode,
            evidence_status: if evidence_status == ReviewEvidenceStatus::Missing {
                evidence_status
            } else if parsed.has_substantive_review
                && evidence_status == ReviewEvidenceStatus::Complete
                && has_supporting_evidence
            {
                ReviewEvidenceStatus::Complete
            } else {
                ReviewEvidenceStatus::Partial
            },
            reviewed_files,
            supporting_files,
            findings: parsed.findings,
            tests_run: parsed.tests_run,
            no_findings_reason: parsed.no_findings_reason,
            limitations,
            memory,
        }
    }
}

#[derive(Debug, Default)]
struct ParsedReviewOutput {
    findings: Vec<ReviewFinding>,
    tests_run: Vec<ReviewTestRun>,
    no_findings_reason: Option<String>,
    limitations: Vec<String>,
    has_substantive_review: bool,
}

impl ParsedReviewOutput {
    fn from_text(text: &str, reviewed_files: &[String], supporting_files: &[String]) -> Self {
        let sections = ReviewSections::parse(text);
        let findings = sections
            .named("findings")
            .map(parse_findings)
            .unwrap_or_default();
        let tests_run = sections
            .named("tests/commands considered")
            .map(parse_tests_run)
            .unwrap_or_default();
        let mut limitations = sections
            .named("confidence/limitations")
            .map(parse_limitations)
            .unwrap_or_default();

        let no_findings_reason = sections
            .named("no-findings justification")
            .and_then(|lines| parse_no_findings_reason(lines, reviewed_files, supporting_files));
        let has_file_backed_no_findings = no_findings_reason.is_some();
        let supporting_context = sections
            .named("supporting context used")
            .and_then(|lines| parse_supporting_context(lines, supporting_files));
        let has_required_supporting_context =
            supporting_files.is_empty() || supporting_context.is_some();

        if findings.is_empty()
            && !has_file_backed_no_findings
            && is_generic_review_text(text)
            && limitations.is_empty()
        {
            limitations.push(
                "Review narrative was generic and did not explain the review outcome.".to_string(),
            );
        }
        if !has_required_supporting_context {
            limitations.push(
                "Review output did not explain supporting context with traced file references."
                    .to_string(),
            );
        }

        let has_explicit_limitation = limitations.iter().any(|item| {
            contains_any_ci(item, &["limitation", "unable", "could not", "not possible"])
        });

        Self {
            has_substantive_review: (!findings.is_empty()
                || has_file_backed_no_findings
                || has_explicit_limitation)
                && has_required_supporting_context,
            findings,
            tests_run,
            no_findings_reason,
            limitations,
        }
    }
}

#[derive(Debug)]
struct ReviewSections {
    sections: Vec<(String, Vec<String>)>,
}

impl ReviewSections {
    fn parse(text: &str) -> Self {
        let mut sections: Vec<(String, Vec<String>)> = Vec::new();
        let mut current: Option<(String, Vec<String>)> = None;

        for raw in text.lines() {
            let line = raw.trim();
            if let Some(title) = markdown_heading_title(line) {
                if let Some(section) = current.take() {
                    sections.push(section);
                }
                current = Some((normalize_section_title(title), Vec::new()));
            } else if let Some((_, lines)) = current.as_mut() {
                lines.push(line.to_string());
            }
        }

        if let Some(section) = current {
            sections.push(section);
        }
        Self { sections }
    }

    fn named(&self, name: &str) -> Option<&[String]> {
        let normalized = normalize_section_title(name);
        self.sections
            .iter()
            .find(|(title, _)| title == &normalized)
            .map(|(_, lines)| lines.as_slice())
    }
}

fn markdown_heading_title(line: &str) -> Option<&str> {
    let trimmed = line.trim_start_matches('#').trim();
    (line.starts_with('#') && !trimmed.is_empty()).then_some(trimmed)
}

fn normalize_section_title(title: &str) -> String {
    title
        .trim()
        .trim_matches(':')
        .to_ascii_lowercase()
        .replace("commands/tests", "tests/commands")
}

fn parse_findings(lines: &[String]) -> Vec<ReviewFinding> {
    lines
        .iter()
        .filter_map(|line| {
            let item = clean_list_item(line);
            if item.is_empty() || is_none_marker(item) {
                return None;
            }

            let severity = parse_severity(item);
            let (file, line_number) = parse_backticked_file_ref(item)?;
            let title = item
                .split_once('`')
                .and_then(|(_, rest)| rest.split_once('`').map(|(_, tail)| tail))
                .map(|tail| {
                    tail.trim()
                        .trim_start_matches('-')
                        .trim_start_matches(':')
                        .trim()
                })
                .filter(|tail| !tail.is_empty())
                .unwrap_or("Review finding");

            let (body, recommendation) = split_recommendation(title);
            Some(ReviewFinding {
                severity,
                file,
                line: line_number,
                title: first_sentence(body).unwrap_or_else(|| "Review finding".to_string()),
                body: body.to_string(),
                recommendation: recommendation.map(str::to_string),
            })
        })
        .collect()
}

fn parse_tests_run(lines: &[String]) -> Vec<ReviewTestRun> {
    lines
        .iter()
        .filter_map(|line| {
            let item = clean_list_item(line);
            if item.is_empty() || is_none_marker(item) {
                return None;
            }
            let command = backticked_segments(item)
                .into_iter()
                .next()
                .unwrap_or_else(|| item.split(" - ").next().unwrap_or(item).trim().to_string());
            if command.is_empty() {
                return None;
            }
            let lower = item.to_ascii_lowercase();
            let status = if lower.contains("failed") {
                ReviewTestStatus::Failed
            } else if lower.contains("passed") || lower.contains("pass") {
                ReviewTestStatus::Passed
            } else if lower.contains("not run") || lower.contains("not-run") {
                ReviewTestStatus::NotRun
            } else {
                ReviewTestStatus::Unknown
            };
            Some(ReviewTestRun {
                command,
                status,
                output_summary: item
                    .split_once(':')
                    .map(|(_, summary)| summary.trim().to_string()),
            })
        })
        .collect()
}

fn parse_limitations(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| clean_list_item(line).trim().to_string())
        .filter(|line| {
            !line.is_empty()
                && !is_none_marker(line)
                && !matches!(
                    line.trim().to_ascii_lowercase().as_str(),
                    "no limitations" | "no limitations."
                )
                && !line.to_ascii_lowercase().contains("no limitations")
                && contains_any_ci(line, &["limitation", "unable", "could not", "not possible"])
        })
        .collect()
}

fn parse_no_findings_reason(
    lines: &[String],
    reviewed_files: &[String],
    supporting_files: &[String],
) -> Option<String> {
    let reason = lines
        .iter()
        .map(|line| clean_list_item(line))
        .filter(|line| !line.is_empty() && !is_none_marker(line))
        .collect::<Vec<_>>()
        .join(" ");
    if reason.len() < 40 {
        return None;
    }

    let mentions_known_file = reviewed_files
        .iter()
        .chain(supporting_files.iter())
        .any(|file| reason.contains(file));
    mentions_known_file.then_some(reason)
}

fn parse_supporting_context(lines: &[String], supporting_files: &[String]) -> Option<String> {
    let context = lines
        .iter()
        .map(|line| clean_list_item(line))
        .filter(|line| !line.is_empty() && !is_none_marker(line))
        .collect::<Vec<_>>()
        .join(" ");
    if context.len() < 20 {
        return None;
    }

    supporting_files
        .iter()
        .any(|file| context.contains(file))
        .then_some(context)
}

fn parse_severity(item: &str) -> ReviewSeverity {
    let lower = item.to_ascii_lowercase();
    if lower.contains("[critical]") || lower.starts_with("critical") {
        ReviewSeverity::Critical
    } else if lower.contains("[high]") || lower.starts_with("high") {
        ReviewSeverity::High
    } else if lower.contains("[medium]") || lower.starts_with("medium") {
        ReviewSeverity::Medium
    } else if lower.contains("[low]") || lower.starts_with("low") {
        ReviewSeverity::Low
    } else {
        ReviewSeverity::Info
    }
}

fn parse_backticked_file_ref(item: &str) -> Option<(String, Option<u64>)> {
    backticked_segments(item).into_iter().find_map(|segment| {
        let (path, line) = split_file_line(&segment);
        Path::new(&path)
            .extension()
            .is_some()
            .then_some((path, line))
    })
}

fn split_file_line(segment: &str) -> (String, Option<u64>) {
    let colon_start = if segment.len() > 2 && segment.as_bytes()[1] == b':' {
        2
    } else {
        0
    };
    for (idx, _) in segment
        .char_indices()
        .rev()
        .filter(|(idx, ch)| *ch == ':' && *idx >= colon_start)
    {
        if let Ok(line) = segment[idx + 1..].parse::<u64>() {
            return (segment[..idx].to_string(), Some(line));
        }
    }
    (segment.to_string(), None)
}

fn backticked_segments(item: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut rest = item;
    while let Some((_, after_open)) = rest.split_once('`') {
        if let Some((segment, after_close)) = after_open.split_once('`') {
            segments.push(segment.trim().to_string());
            rest = after_close;
        } else {
            break;
        }
    }
    segments
}

fn clean_list_item(line: &str) -> &str {
    line.trim()
        .trim_start_matches('-')
        .trim_start_matches('*')
        .trim()
}

fn is_none_marker(text: &str) -> bool {
    matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "none" | "none." | "no findings" | "no findings." | "n/a" | "not applicable"
    )
}

fn is_generic_review_text(text: &str) -> bool {
    let normalized = text
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase();
    normalized.is_empty()
        || contains_any_ci(
            &normalized,
            &[
                "completed the requested change",
                "reviewed the pr",
                "looks good to me",
                "no issues found",
                "no findings",
            ],
        ) && normalized.len() < 120
}

fn split_recommendation(text: &str) -> (&str, Option<&str>) {
    if let Some((body, recommendation)) = text.split_once("Recommendation:") {
        (body.trim(), Some(recommendation.trim()))
    } else {
        (text.trim(), None)
    }
}

fn first_sentence(text: &str) -> Option<String> {
    text.split('.')
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
}

fn contains_any_ci(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

/// Files observed through read/search tool telemetry during headless review.
#[derive(Debug, Default, Clone)]
pub struct ReviewTrace {
    workspace_root: PathBuf,
    files: BTreeSet<String>,
    pending_reads: VecDeque<String>,
}

impl ReviewTrace {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            files: BTreeSet::new(),
            pending_reads: VecDeque::new(),
        }
    }

    pub fn record_tool_start(&mut self, tool_name: &str, input_json: &str) {
        let Ok(input) = serde_json::from_str::<Value>(input_json) else {
            return;
        };
        match tool_name {
            "Read" => {
                if let Some(path) = input.get("file_path").and_then(Value::as_str) {
                    self.pending_reads.push_back(path.to_string());
                }
            }
            "Grep" | "Glob" => {}
            _ => {}
        }
    }

    pub fn record_tool_end(&mut self, tool_name: &str, result: &str, is_error: bool) {
        if tool_name == "Read" {
            let path = self.pending_reads.pop_front();
            if !is_error {
                if let Some(path) = path {
                    self.record_path(&path);
                }
            }
            return;
        }
        if is_error || !matches!(tool_name, "Grep" | "Glob") {
            return;
        }

        for line in result
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if line.starts_with("No files matched")
                || line.starts_with("No matches found")
                || line.starts_with("...")
                || line == "--"
            {
                continue;
            }
            if let Some(candidate) = path_prefix(line) {
                self.record_path(candidate);
            }
        }
    }

    pub fn supporting_files(&self, reviewed_files: &[String]) -> Vec<String> {
        let reviewed: BTreeSet<String> = reviewed_files
            .iter()
            .filter_map(|path| normalize_relative_path(path))
            .collect();
        self.files
            .iter()
            .filter(|path| !reviewed.contains(*path))
            .cloned()
            .collect()
    }

    fn record_path(&mut self, path: &str) {
        if let Some(path) = self.normalize_path(path) {
            self.files.insert(path);
        }
    }

    fn normalize_path(&self, raw: &str) -> Option<String> {
        let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
        if trimmed.is_empty() {
            return None;
        }

        let path = PathBuf::from(trimmed);
        let absolute = if path.is_absolute() {
            path
        } else {
            self.workspace_root.join(path)
        };

        let normalized = normalize_existing_or_lexical(&absolute);
        let root = normalize_existing_or_lexical(&self.workspace_root);
        let relative = normalized.strip_prefix(&root).ok()?;
        if !normalized.is_file() {
            return None;
        }
        let path = normalize_relative_path(&relative.to_string_lossy())?;
        if path.split('/').any(|part| part == ".git") {
            return None;
        }
        Some(path)
    }
}

fn normalize_existing_or_lexical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        let mut out = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        out
    })
}

fn normalize_relative_path(path: &str) -> Option<String> {
    let trimmed = path.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized.contains('\0')
        || normalized.split('/').any(|part| part == "..")
    {
        return None;
    }

    let normalized = normalized
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn path_prefix(line: &str) -> Option<&str> {
    let colon_search_start = if line.len() > 2 && line.as_bytes()[1] == b':' {
        2
    } else {
        0
    };
    for (idx, _) in line
        .char_indices()
        .filter(|(idx, ch)| *ch == ':' && *idx >= colon_search_start && line[..*idx].contains('.'))
    {
        let candidate = &line[..idx];
        if looks_like_tool_path(candidate) {
            return Some(candidate);
        }
    }

    if looks_like_tool_path(line) {
        return Some(line);
    }
    None
}

fn looks_like_tool_path(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    !trimmed.is_empty()
        && trimmed == candidate
        && !trimmed.contains("://")
        && !trimmed
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '`' | '<' | '>' | '|' | '{' | '}'))
        && Path::new(trimmed).extension().is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    None,
    PullRequest,
    ReviewComment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewEvidenceStatus {
    NotApplicable,
    Complete,
    #[allow(
        dead_code,
        reason = "contract v2 reserves partial review evidence for future adapters"
    )]
    Partial,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewFinding {
    pub severity: ReviewSeverity,
    pub file: String,
    pub line: Option<u64>,
    pub title: String,
    pub body: String,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(
    dead_code,
    reason = "contract v2 reserves structured findings for the review parser"
)]
pub enum ReviewSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewTestRun {
    pub command: String,
    pub status: ReviewTestStatus,
    pub output_summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(
    dead_code,
    reason = "contract v2 reserves structured test evidence for verifier integration"
)]
pub enum ReviewTestStatus {
    Passed,
    Failed,
    NotRun,
    Unknown,
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
    pub review_trace: ReviewTrace,
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

/// Test convenience: build the result envelope with an empty memory report.
#[cfg(test)]
fn build_result(
    brief: Option<&SessionBrief>,
    git: &GitSummary,
    outcome: RunOutcome,
    final_text: &str,
    review_trace: Option<&ReviewTrace>,
) -> (ResultEnvelope, i32) {
    build_result_with_memory(
        brief,
        git,
        outcome,
        final_text,
        review_trace,
        ReviewMemoryUse::default(),
    )
}

/// Build the result envelope with an explicit memory-usage report attached to
/// the review artifact.
pub fn build_result_with_memory(
    brief: Option<&SessionBrief>,
    git: &GitSummary,
    outcome: RunOutcome,
    final_text: &str,
    review_trace: Option<&ReviewTrace>,
    memory: ReviewMemoryUse,
) -> (ResultEnvelope, i32) {
    let comment_only = brief.map(SessionBrief::is_comment_only).unwrap_or(false);
    let (mut status, mut exit_reason, code) =
        classify(outcome, !git.commits.is_empty(), comment_only);

    let review = ReviewResult::from_brief_with_memory(brief, review_trace, final_text, memory);
    if review.mode != ReviewMode::None
        && status == Status::Success
        && review.evidence_status != ReviewEvidenceStatus::Complete
    {
        status = Status::Partial;
        exit_reason = None;
    }
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
        review,
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
        review: ReviewResult::from_brief(brief, None, ""),
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

    fn sample_review_brief() -> SessionBrief {
        let raw = r#"{
            "contract_version": "2",
            "trigger": "issue_mention",
            "repo": { "owner": "o", "name": "r", "clone_url": "https://github.com/o/r.git", "default_branch": "main" },
            "task": { "kind": "respond_to_mention", "issue_number": 7, "comment_body": "review this" },
            "familiar": { "id": "cody", "display_name": "Cody", "skills": ["code-review"] },
            "workspace": { "root": "/tmp/ws" },
            "audit_instruction": "Inspect supporting code.",
            "review_context": {
                "kind": "pull_request",
                "files": [
                    { "filename": "src/lib.rs" },
                    { "filename": "README.md" }
                ]
            }
        }"#;
        serde_json::from_str(raw).expect("review brief parses")
    }

    fn review_workspace() -> (tempfile::TempDir, ReviewTrace) {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::create_dir_all(ws.join("src")).unwrap();
        for path in ["src/lib.rs", "src/support.rs", "src/config.rs", "README.md"] {
            std::fs::write(ws.join(path), "").unwrap();
        }
        (dir, ReviewTrace::new(ws))
    }

    fn record_successful_read(trace: &mut ReviewTrace, path: &str) {
        trace.record_tool_start("Read", &json!({ "file_path": path }).to_string());
        trace.record_tool_end("Read", "", false);
    }

    // ── Review memory report ────────────────────────────────────────────────

    #[test]
    fn collect_review_memory_local_lists_project_entries() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(
            ws.path().join("AGENTS.md"),
            "---\nid: mem_local_fact\ntrust: maintainer_approved\nsource: unit-test\n---\nLocal fact.",
        )
        .unwrap();
        let config = claurst_core::config::Config::default();

        let memory = collect_review_memory(ws.path(), &config);

        assert!(memory.domains_loaded.is_empty());
        let entry = memory
            .entries
            .iter()
            .find(|entry| entry.id == "mem_local_fact")
            .expect("project memory entry is reported");
        assert_eq!(entry.scope, "project");
        assert_eq!(entry.trust, "maintainer-approved");
    }

    #[test]
    fn collect_review_memory_hosted_reports_domain_and_excludes_untrusted() {
        let ws = tempfile::tempdir().unwrap();
        // A repo file self-attesting high trust must not survive hosted caps.
        std::fs::write(
            ws.path().join("AGENTS.md"),
            "---\nid: mem_attacker\ntrust: maintainer_approved\nsource: repo\n---\nAttacker fact.",
        )
        .unwrap();
        let mut config = claurst_core::config::Config::default();
        config.hosted_review.enabled = true;

        let memory = collect_review_memory(ws.path(), &config);

        assert_eq!(memory.domains_loaded, vec!["default-branch".to_string()]);
        assert!(
            memory.entries.is_empty(),
            "hosted review must not report untrusted repo memory as loaded: {:?}",
            memory.entries
        );
    }

    #[test]
    fn result_envelope_serializes_review_memory_report() {
        let (dir, mut trace) = review_workspace();
        record_successful_read(
            &mut trace,
            dir.path().join("src/support.rs").to_str().unwrap(),
        );
        let memory = ReviewMemoryUse {
            domains_loaded: vec!["default-branch".to_string()],
            entries: vec![ReviewMemoryEntry {
                id: "mem_policy".to_string(),
                trust: "maintainer-approved".to_string(),
                visibility: Some("public_review".to_string()),
                scope: "managed".to_string(),
            }],
        };

        let (envelope, _) = build_result_with_memory(
            Some(&sample_review_brief()),
            &GitSummary::default(),
            RunOutcome::Completed,
            "## Findings\n- [low] src/lib.rs:1 — fine\n\n## Supporting Context Used\n- src/support.rs: checked",
            Some(&trace),
            memory,
        );

        let value = serde_json::to_value(&envelope).unwrap();
        let memory_value = &value["review"]["memory"];
        assert_eq!(memory_value["domains_loaded"][0], "default-branch");
        assert_eq!(memory_value["entries"][0]["id"], "mem_policy");
        assert_eq!(memory_value["entries"][0]["trust"], "maintainer-approved");
        assert_eq!(memory_value["entries"][0]["scope"], "managed");
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
        brief.ensure_supported_version().expect("v2 is supported");
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
            "contract_version": "2",
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
            "contract_version": "2",
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
        brief.contract_version = "3".to_string();
        assert!(
            brief.ensure_supported_version().is_err(),
            "a v3 brief must be rejected by a v2 runtime"
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

    #[test]
    fn review_prompt_requires_structured_review_sections() {
        let prompt = sample_review_brief().to_prompt();
        for section in [
            "### Files inspected",
            "### Supporting context used",
            "### Findings",
            "### No-findings justification",
            "### Tests/commands considered",
            "### Confidence/limitations",
        ] {
            assert!(prompt.contains(section), "prompt missing {section}");
        }
        assert!(prompt.contains("specific file references"));
        assert!(prompt.contains("Do not end with a generic completion message."));
        assert!(prompt.contains("Do not modify files, create commits, or push a branch"));
        assert!(!prompt.contains("make the change on a new branch"));
    }

    #[test]
    fn review_brief_applies_hosted_read_only_lockdown_to_config() {
        let brief = sample_review_brief();
        let mut config = claurst_core::config::Config {
            hosted_review: claurst_core::hosted_review::HostedReviewConfig {
                allow_write_tools: true,
                allow_mcp_servers: true,
                allow_plugins: true,
                allow_user_memory: true,
                allow_auto_memory_persistence: true,
                ..Default::default()
            },
            ..Default::default()
        };

        apply_to_config(&mut config, &brief);

        assert!(config.hosted_review.enabled);
        assert!(!config.hosted_review.allow_write_tools);
        assert!(!config.hosted_review.allow_mcp_servers);
        assert!(!config.hosted_review.allow_plugins);
        assert!(!config.hosted_review.allow_user_memory);
        assert!(!config.hosted_review.allow_auto_memory_persistence);
        assert_eq!(
            config.permission_mode,
            claurst_core::config::PermissionMode::BypassPermissions
        );
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
        assert_eq!(obj["contract_version"], json!("2"));

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
    fn session_brief_schema_defines_review_mode_fields() {
        let schema: Value =
            serde_json::from_str(&fixture("session-brief.schema.json")).expect("schema parses");
        let props = schema["properties"].as_object().unwrap();

        assert!(props.contains_key("review_context"));
        assert!(props.contains_key("audit_instruction"));
    }

    #[test]
    fn result_schema_allows_partial_review_without_oneof_ambiguity() {
        let schema: Value =
            serde_json::from_str(&fixture("result.schema.json")).expect("schema parses");
        let review_then = &schema["properties"]["review"]["allOf"][0]["then"];

        assert!(
            review_then.get("oneOf").is_none(),
            "review schema must not use oneOf for findings/no-findings because degraded and mixed outputs are valid"
        );
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
            None,
        );
        assert_eq!(code, 0);
        assert_eq!(env.status, Status::Success);
        assert_eq!(env.review.mode, ReviewMode::None);
        assert!(env.exit_reason.is_none());
        let value = serde_json::to_value(&env).unwrap();
        assert_matches_result_schema(&value);
    }

    #[test]
    fn review_context_produces_structured_pr_review_evidence() {
        let raw = r#"{
            "contract_version": "2",
            "trigger": "issue_mention",
            "repo": { "owner": "o", "name": "r", "clone_url": "https://github.com/o/r.git", "default_branch": "main" },
            "task": { "kind": "respond_to_mention", "issue_number": 7, "comment_body": "review this" },
            "familiar": { "id": "cody", "display_name": "Cody", "skills": [] },
            "workspace": { "root": "/tmp/ws" },
            "review_context": {
                "kind": "pull_request",
                "files": [
                    { "filename": "src/lib.rs" },
                    { "filename": "README.md" }
                ]
            }
        }"#;
        let brief: SessionBrief = serde_json::from_str(raw).expect("brief parses");
        let (env, code) = build_result(
            Some(&brief),
            &GitSummary::default(),
            RunOutcome::Completed,
            "Reviewed the PR.",
            None,
        );

        assert_eq!(code, 0);
        assert_eq!(env.status, Status::Partial);
        assert_eq!(env.review.mode, ReviewMode::PullRequest);
        assert_eq!(env.review.evidence_status, ReviewEvidenceStatus::Partial);
        assert_eq!(
            env.review.reviewed_files,
            vec!["src/lib.rs".to_string(), "README.md".to_string()]
        );
        assert!(env.review.supporting_files.is_empty());
        assert!(env.review.findings.is_empty());
        assert!(env.review.no_findings_reason.is_none());
        assert!(env
            .review
            .limitations
            .iter()
            .any(|item| item.contains("No supporting-code")));
    }

    #[test]
    fn review_trace_records_supporting_files_and_filters_reviewed_files() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/lib.rs"), "").unwrap();
        std::fs::write(ws.join("src/support.rs"), "").unwrap();
        std::fs::write(ws.join("src/config.rs"), "").unwrap();
        std::fs::write(ws.join("README.md"), "").unwrap();
        std::fs::create_dir_all(ws.join(".git/hooks")).unwrap();
        std::fs::write(ws.join(".git/config"), "").unwrap();

        let mut trace = ReviewTrace::new(ws);
        record_successful_read(&mut trace, "src/lib.rs");
        record_successful_read(&mut trace, &ws.join("src/support.rs").to_string_lossy());
        trace.record_tool_end(
            "Grep",
            &format!(
                "{}:12:fn helper() {{}}\n{}",
                ws.join("src/config.rs").display(),
                ws.join("README.md").display()
            ),
            false,
        );
        trace.record_tool_end("Glob", "src/lib.rs\nsrc/support.rs\n", false);
        trace.record_tool_end("Glob", ".git\n.git/config\nsrc\n", false);
        trace.record_tool_start("Read", r#"{"file_path":"../outside.rs"}"#);
        trace.record_tool_end("Read", "outside", true);

        assert_eq!(
            trace.supporting_files(&["src/lib.rs".to_string()]),
            vec![
                "README.md".to_string(),
                "src/config.rs".to_string(),
                "src/support.rs".to_string()
            ]
        );
    }

    #[test]
    fn review_trace_ignores_failed_reads_and_non_file_paths() {
        let (_dir, mut trace) = review_workspace();

        trace.record_tool_start("Read", r#"{"file_path":"src/support.rs"}"#);
        trace.record_tool_end("Read", "not found", true);
        trace.record_tool_start("Read", r#"{"file_path":"src/missing.rs"}"#);
        trace.record_tool_end("Read", "", false);
        trace.record_tool_end("Glob", "src\n.git/config\n", false);

        assert!(trace.supporting_files(&[]).is_empty());
    }

    #[test]
    fn review_trace_ignores_grep_match_text_that_is_not_a_path_prefix() {
        let (_dir, mut trace) = review_workspace();

        trace.record_tool_end(
            "Grep",
            "let version = config.rs:42;\nsrc/config.rs:12:fn config() {}\n",
            false,
        );

        assert_eq!(trace.supporting_files(&[]), vec!["src/config.rs"]);
    }

    #[test]
    fn review_result_uses_trace_backed_supporting_files() {
        let brief = sample_review_brief();
        let (_dir, mut trace) = review_workspace();
        record_successful_read(&mut trace, "src/lib.rs");
        record_successful_read(&mut trace, "src/support.rs");

        let (env, _) = build_result(
            Some(&brief),
            &GitSummary::default(),
            RunOutcome::Completed,
            "### Supporting context used\n- `src/support.rs` provides read-only context for the review trace used by `src/lib.rs`.\n\n### Findings\nNone\n\n### No-findings justification\nNo issues were found because `src/lib.rs` keeps the review trace isolated and `src/support.rs` only provides read-only context for validation.\n\n### Tests/commands considered\n- `cargo test -p claurst headless` - not run: unit coverage is represented in this PR.\n\n### Confidence/limitations\nConfidence is medium. No limitations.",
            Some(&trace),
        );

        assert_eq!(env.review.supporting_files, vec!["src/support.rs"]);
        assert!(env.review.limitations.is_empty());
        assert_eq!(env.review.evidence_status, ReviewEvidenceStatus::Complete);
        assert_eq!(env.status, Status::Success);
        assert!(brief
            .to_prompt()
            .contains("Additional review instruction:\nInspect supporting code."));
    }

    #[test]
    fn concise_file_backed_no_findings_reason_is_substantive() {
        let brief = sample_review_brief();
        let (_dir, mut trace) = review_workspace();
        record_successful_read(&mut trace, "src/lib.rs");
        record_successful_read(&mut trace, "src/support.rs");

        let (env, _) = build_result(
            Some(&brief),
            &GitSummary::default(),
            RunOutcome::Completed,
            "### Files inspected\n- `src/lib.rs`\n\n### Supporting context used\n- `src/support.rs` validates the API behavior used by `src/lib.rs`.\n\n### Findings\nNone\n\n### No-findings justification\nNo findings: `src/lib.rs` and `src/support.rs` agree on the review contract.\n\n### Tests/commands considered\n- `cargo test -p claurst-cli headless` - not run: parser unit coverage applies.\n\n### Confidence/limitations\nConfidence is medium. No limitations.",
            Some(&trace),
        );

        assert_eq!(
            env.review.no_findings_reason.as_deref(),
            Some("No findings: `src/lib.rs` and `src/support.rs` agree on the review contract.")
        );
        assert_eq!(env.review.evidence_status, ReviewEvidenceStatus::Complete);
        assert_eq!(env.status, Status::Success);
    }

    #[test]
    fn structured_review_without_supporting_trace_is_partial() {
        let brief = sample_review_brief();
        let (_dir, mut trace) = review_workspace();
        record_successful_read(&mut trace, "src/lib.rs");

        let (env, _) = build_result(
            Some(&brief),
            &GitSummary::default(),
            RunOutcome::Completed,
            "### Files inspected\n- `src/lib.rs`\n\n### Supporting context used\nNone.\n\n### Findings\nNone\n\n### No-findings justification\nNo issues were found because `src/lib.rs` preserves the expected review result contract.\n\n### Tests/commands considered\n- `cargo test -p claurst headless` - not run: regression test covers this path.\n\n### Confidence/limitations\nConfidence is medium. No limitations.",
            Some(&trace),
        );

        assert!(env.review.supporting_files.is_empty());
        assert_eq!(env.review.evidence_status, ReviewEvidenceStatus::Partial);
        assert_eq!(env.status, Status::Partial);
        assert!(env
            .review
            .limitations
            .iter()
            .any(|item| item.contains("No supporting-code")));
    }

    #[test]
    fn generic_review_output_is_marked_partial() {
        let brief = sample_review_brief();
        let (_dir, mut trace) = review_workspace();
        record_successful_read(&mut trace, "src/lib.rs");
        record_successful_read(&mut trace, "src/support.rs");

        let (env, code) = build_result(
            Some(&brief),
            &GitSummary::default(),
            RunOutcome::Completed,
            "## Cody\n\nCompleted the requested change.",
            Some(&trace),
        );

        assert_eq!(code, 0);
        assert_eq!(env.status, Status::Partial);
        assert_eq!(env.review.evidence_status, ReviewEvidenceStatus::Partial);
        assert!(env.review.no_findings_reason.is_none());
        assert!(env.review.findings.is_empty());
        assert!(env
            .review
            .limitations
            .iter()
            .any(|item| item.contains("generic")));
    }

    #[test]
    fn missing_supporting_context_rationale_is_marked_partial() {
        let brief = sample_review_brief();
        let (_dir, mut trace) = review_workspace();
        record_successful_read(&mut trace, "src/lib.rs");
        record_successful_read(&mut trace, "src/support.rs");

        let (env, _) = build_result(
            Some(&brief),
            &GitSummary::default(),
            RunOutcome::Completed,
            "### Findings\nNone\n\n### No-findings justification\nNo issues were found because `src/lib.rs` and `src/support.rs` preserve the expected trace-backed review behavior.\n\n### Confidence/limitations\nConfidence is medium. No limitations.",
            Some(&trace),
        );

        assert_eq!(env.status, Status::Partial);
        assert_eq!(env.review.evidence_status, ReviewEvidenceStatus::Partial);
        assert!(env
            .review
            .limitations
            .iter()
            .any(|item| item.contains("supporting context")));
    }

    #[test]
    fn structured_review_parser_extracts_findings_tests_and_limitations() {
        let brief = sample_review_brief();
        let (_dir, mut trace) = review_workspace();
        record_successful_read(&mut trace, "src/lib.rs");
        record_successful_read(&mut trace, "src/support.rs");
        let final_text = r#"### Files inspected
- `src/lib.rs`

### Supporting context used
- `src/support.rs` explains the helper behavior used by `src/lib.rs`.

### Findings
- [high] `src/lib.rs:42` Missing error handling - this path can silently drop a failed review parse. Recommendation: return a limitation instead.

### No-findings justification
N/A

### Tests/commands considered
- `cargo test -p claurst headless` - passed: parser tests covered the review contract.
- `cargo clippy --workspace --all-targets -- -D warnings` - not run: local linting was deferred to CI.

### Confidence/limitations
- Limitation: this parser is conservative and ignores findings without file references.
"#;

        let (env, _) = build_result(
            Some(&brief),
            &GitSummary::default(),
            RunOutcome::Completed,
            final_text,
            Some(&trace),
        );

        assert_eq!(env.status, Status::Success);
        assert_eq!(env.review.evidence_status, ReviewEvidenceStatus::Complete);
        assert_eq!(env.review.findings.len(), 1);
        assert_eq!(env.review.findings[0].severity, ReviewSeverity::High);
        assert_eq!(env.review.findings[0].file, "src/lib.rs");
        assert_eq!(env.review.findings[0].line, Some(42));
        assert_eq!(env.review.tests_run.len(), 2);
        assert_eq!(env.review.tests_run[0].status, ReviewTestStatus::Passed);
        assert_eq!(env.review.tests_run[1].status, ReviewTestStatus::NotRun);
        assert!(env.review.no_findings_reason.is_none());
        assert_eq!(env.review.limitations.len(), 1);
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
    fn address_review_comment_without_commits_is_successful_review_output() {
        let raw = r#"{
            "contract_version": "2",
            "trigger": "pr_review_comment",
            "repo": { "owner": "o", "name": "r", "clone_url": "https://github.com/o/r.git", "default_branch": "main" },
            "task": { "kind": "address_review_comment", "pr_number": 7, "comment_body": "Please review this behavior.", "diff_hunk": null },
            "familiar": { "id": "cody", "display_name": "Cody", "skills": [] },
            "workspace": { "root": "/tmp/ws" },
            "review_context": { "kind": "pull_request", "files": [{ "filename": "src/lib.rs" }] }
        }"#;
        let brief: SessionBrief = serde_json::from_str(raw).expect("brief parses");
        let (_dir, mut trace) = review_workspace();
        record_successful_read(&mut trace, "src/lib.rs");
        record_successful_read(&mut trace, "src/support.rs");

        let (env, code) = build_result(
            Some(&brief),
            &GitSummary::default(),
            RunOutcome::Completed,
            "### Files inspected\n- `src/lib.rs`\n\n### Supporting context used\n- `src/support.rs` confirms the behavior used by `src/lib.rs`.\n\n### Findings\nNone\n\n### No-findings justification\nNo findings: `src/lib.rs` and `src/support.rs` are consistent.\n\n### Tests/commands considered\n- `cargo test -p claurst-cli headless` - not run: parser unit coverage applies.\n\n### Confidence/limitations\nConfidence is medium. No limitations.",
            Some(&trace),
        );

        assert_eq!(code, 0);
        assert_eq!(env.status, Status::Success);
        assert!(env.exit_reason.is_none());
        assert_eq!(env.review.mode, ReviewMode::ReviewComment);
        assert!(env.commits.is_empty());
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
            None,
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
        let (env, _) = build_result(
            Some(&sample_brief()),
            &git,
            RunOutcome::Completed,
            "   ",
            None,
        );
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
