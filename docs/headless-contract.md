# coven-code Headless Execution Contract

**Contract version: `2`** - Status: **Locked** (V2 / structured review output)

This document is the single source of truth for the interface between
`coven-github` (the GitHub ingress adapter) and `coven-code` (the execution
runtime) when the runtime is invoked in headless mode.

It is normative. Where the prose in [`COVEN-GITHUB.md`](../COVEN-GITHUB.md) or any
other doc disagrees with this file, **this file wins**. Both repositories MUST
implement exactly what is specified here, and changes require a contract version
bump (see [Versioning](#versioning)).

The contract is enforced on the `coven-github` side by golden fixtures in
[`docs/contracts/`](contracts/) and a conformance test
(`crates/github/tests/contract.rs`) that round-trips those fixtures through the
Rust types. `coven-code` MUST validate its emitted `result.json` against
[`docs/contracts/result.schema.json`](contracts/result.schema.json) and its
accepted `session-brief.json` against
[`docs/contracts/session-brief.schema.json`](contracts/session-brief.schema.json).

The key words MUST, MUST NOT, SHOULD, and MAY are used as in RFC 2119.

---

## 1. Invocation

The adapter spawns the runtime as a child process:

```
coven-code --headless --context <session-brief.json> --output <result.json>
```

| Flag | Meaning |
|---|---|
| `--headless` | Disables the ratatui TUI entirely. All human-facing output is suppressed; the process is non-interactive and reads no stdin. |
| `--context <path>` | Path to a `session-brief.json` file the adapter has already written. Read-only input. |
| `--output <path>` | Path the runtime MUST write `result.json` to before exiting `0`, `1`, or `3`. |

The runtime MUST NOT require a TTY. It MUST NOT block on interactive prompts.

### 1.1 Environment

| Variable | Required | Meaning |
|---|---|---|
| `COVEN_GIT_TOKEN` | yes (for any push) | GitHub App **installation access token** used to authenticate `git push` over HTTPS. The runtime MUST use this token for git write operations and MUST NOT use ambient user credentials. |

The token is passed **only** through the environment. It MUST NOT appear in the
session brief, the result envelope, the clone URL, logs, or any durable
artifact. The 1-hour token TTL is the adapter's concern; the runtime treats the
token as opaque and valid for the session.

> **Drift note (supersedes `COVEN-GITHUB.md`):** earlier spec prose referenced
> `GIT_ASKPASS` / `GIT_TOKEN` and an `auth.token` field embedded in the brief.
> Those are **removed**. The brief is tokenless (issue #4) and the only git
> credential channel is `COVEN_GIT_TOKEN`.

---

## 2. Input — `session-brief.json`

The adapter is the **producer**; the runtime is the **consumer**. The brief is
**tokenless**: it carries read context only.

```json
{
  "contract_version": "2",
  "trigger": "issue_assigned",
  "repo": {
    "owner": "OpenCoven",
    "name": "coven-code",
    "clone_url": "https://github.com/OpenCoven/coven-code.git",
    "default_branch": "main"
  },
  "task": {
    "kind": "fix_issue",
    "issue_number": 42,
    "issue_title": "Fix OAuth token refresh",
    "issue_body": "The refresh path ignores clock skew…"
  },
  "familiar": {
    "id": "cody",
    "display_name": "Cody",
    "model": "anthropic/claude-sonnet-4-6",
    "skills": ["systematic-debugging"]
  },
  "workspace": {
    "root": "/tmp/task-abc123"
  }
}
```

### 2.1 Fields

| Field | Type | Notes |
|---|---|---|
| `contract_version` | string | MUST be `"2"`. Consumers MUST reject a brief whose major version they do not implement. |
| `trigger` | string enum | `issue_assigned` \| `pr_review_comment` \| `issue_mention`. |
| `repo.owner` | string | |
| `repo.name` | string | |
| `repo.clone_url` | string | HTTPS clone URL **without** embedded credentials. The runtime supplies auth via `COVEN_GIT_TOKEN`. |
| `repo.default_branch` | string | Resolved from live GitHub metadata, not assumed to be `main` (issue #9). |
| `task` | object | Tagged union discriminated by `kind`. See [2.2](#22-task-kinds). |
| `familiar.id` | string | Stable familiar identifier (e.g. `cody`). |
| `familiar.display_name` | string | Human label used in familiar-voice output. |
| `familiar.model` | string \| null | BYOM model id; `null`/absent means runtime default. |
| `familiar.skills` | string[] | Skill ids to load for the session. MAY be empty. |
| `workspace.root` | string | Absolute path to the pre-cloned, isolated workspace. The runtime operates **inside** this directory and MUST NOT write outside it. |

### 2.2 Task kinds

The `task` object is discriminated by a `kind` string (serde
`#[serde(tag = "kind", rename_all = "snake_case")]`).

| `kind` | Paired `trigger` | Fields |
|---|---|---|
| `fix_issue` | `issue_assigned` | `issue_number: u64`, `issue_title: string`, `issue_body: string` |
| `address_review_comment` | `pr_review_comment` | `pr_number: u64`, `comment_body: string`, `diff_hunk: string \| null` |
| `respond_to_mention` | `issue_mention` | `issue_number: u64`, `comment_body: string` |

---

## 3. Output — `result.json`

The runtime is the **producer**; the adapter is the **consumer**. The runtime
MUST write this file before exiting `0`, `1`, or `3`. On exit `2` (infra error)
the file MAY be absent.

```json
{
  "contract_version": "2",
  "status": "success",
  "branch": "cody/fix-issue-42",
  "commits": [
    { "sha": "a1b2c3d", "message": "Add clock-skew buffer to refresh path" }
  ],
  "files_changed": ["src/auth/refresh.rs"],
  "summary": "Fixed OAuth token refresh by adding a 60-second clock skew buffer.",
  "pr_body": "## Hey, I'm Cody\n\nI looked at issue #42...",
  "review": {
    "mode": "pull_request",
    "evidence_status": "complete",
    "reviewed_files": ["src/auth/refresh.rs"],
    "supporting_files": ["src/auth/mod.rs", "tests/auth_refresh.rs"],
    "findings": [],
    "tests_run": [],
    "no_findings_reason": "Reviewed the supplied PR file and found no blocking issues.",
    "limitations": []
  },
  "exit_reason": null
}
```

### 3.1 Fields

| Field | Type | Notes |
|---|---|---|
| `contract_version` | string | MUST be `"2"`. Producers MUST emit it. |
| `status` | string enum | `success` \| `failure` \| `partial` \| `needs_input`. See [3.3](#33-status). |
| `branch` | string \| null | Branch the runtime pushed. `null` when no branch was created. The adapter only opens a PR when `branch` is set **and** `commits` is non-empty. |
| `commits` | array | `{ "sha": string, "message": string }`. MAY be empty. |
| `files_changed` | string[] | Workspace-relative paths. MAY be empty. |
| `summary` | string | One-line familiar-voice summary. Used in the Check Run and PR title. |
| `pr_body` | string | Full PR body, **authored by the familiar** in its own voice — not a template. |
| `review` | object | Structured review evidence and findings. Required even when `mode` is `none`. See [3.2](#32-review). |
| `exit_reason` | string enum \| null | `null` on success; otherwise the terminal cause. See [3.4](#34-exit_reason). |

> **Drift note (supersedes `COVEN-GITHUB.md`):** the prose result envelope listed
> an `events` array. Progress/event streaming is **not** part of the v2 result
> envelope — it is deferred to M2 and will travel over a separate channel. The
> v2 envelope carries terminal task state only. Producers MUST NOT rely on
> `events` being read.

### 3.2 `review`

`review` is the machine-readable proof that a hosted review actually examined
the intended code. It is required on every result. Non-review tasks MUST set
`mode: "none"` and `evidence_status: "not_applicable"`.

| Field | Type | Notes |
|---|---|---|
| `mode` | string enum | `none`, `pull_request`, or `review_comment`. |
| `evidence_status` | string enum | `not_applicable`, `complete`, `partial`, or `missing`. PR review modes MUST NOT use `not_applicable`. |
| `reviewed_files` | string[] | Workspace-relative files supplied to or inspected by the runtime. PR review modes MUST include at least one file unless `evidence_status` is `missing`. |
| `supporting_files` | string[] | Workspace-relative files beyond the changed-file list that the runtime can prove were inspected for context. Empty means no broader-codebase inspection was proven, not that none was needed. |
| `findings` | array | Structured findings. Empty is allowed only when `no_findings_reason` is a non-empty string. |
| `tests_run` | array | Commands run while reviewing, with `passed`, `failed`, `not_run`, or `unknown` status. |
| `no_findings_reason` | string \| null | Required when `mode` is a review mode and `findings` is empty. |
| `limitations` | string[] | Evidence gaps, skipped checks, or other caveats. |

Each finding carries `severity`, `file`, optional `line`, `title`, `body`, and
optional `recommendation`. Valid severities are `info`, `low`, `medium`, `high`,
and `critical`.

### 3.3 `status`

| Value | Meaning |
|---|---|
| `success` | Work complete; commits made; ready for a PR. |
| `partial` | Some progress committed but the task is not fully done (e.g. tests still failing after the retry budget). The adapter still opens a PR if there are commits. |
| `failure` | The agent gave up; no usable result. |
| `needs_input` | The agent needs human clarification and has posted (or expects the adapter to surface) a question. Pairs with exit code `3`. |

The adapter treats `success` and `partial` as PR-opening outcomes; `failure`
and `needs_input` do not open a PR by themselves.

### 3.4 `exit_reason`

`null` on success. Otherwise one of:

| Value | Meaning |
|---|---|
| `test_failure` | Tests could not be made to pass within the retry budget. |
| `ambiguous_spec` | The request is underspecified; the agent chose to ask rather than guess. |
| `git_conflict` | A git conflict the agent could not safely resolve. |
| `infra_error` | Workspace, git, or tool failure. Retry-safe. |

---

## 4. Exit codes

The exit code is the authoritative signal; `status` is advisory detail. The
adapter's dispatch logic (`crates/worker`) keys on the exit code:

| Code | Name | `result.json` | Adapter behavior |
|---|---|---|---|
| `0` | success | MUST be present | Read result; open draft PR if `branch` + `commits` present; complete Check Run `success` (or `failure` if `status` is `failure`/`needs_input`). |
| `1` | failure | MUST be present | Agent gave up. Mark Check Run `failure` with `summary`. **Not** retried. |
| `2` | infra error | MAY be absent | Retry-safe. Adapter retries up to its configured `max_retries`, then marks `failure`. |
| `3` | needs input | MUST be present (`status: needs_input`) | Agent posted a clarifying question and exited cleanly. Adapter surfaces it; does not retry. |

A process **killed by signal**, or one that **times out** (the adapter enforces
`worker.timeout_secs`), is treated as a retry-safe failure equivalent to exit
`2`.

> **Drift note (supersedes `COVEN-GITHUB.md`):** earlier prose described exit `1`
> as "failure: result.json written with exit_reason" and exit `3` as
> "ambiguous". That intent is preserved, but the **locked** semantics are the
> table above: `2` = infra/retry-safe, `3` = needs-input/clean. The adapter's
> retry boundary is exit `2` (and timeout/signal), never exit `1`.

---

## 5. Security invariants

These are non-negotiable for v2:

1. The session brief is **tokenless**. Serializing a brief MUST NOT produce an
   `auth` field, a `"token"` field, or a credential-bearing `clone_url`
   (enforced by `brief_serialization_never_contains_token_or_auth_fields`).
2. The only git credential channel is the `COVEN_GIT_TOKEN` environment
   variable. It MUST NOT be persisted to the brief, the result, or logs.
3. GitHub **write authority** (comments, Check Runs, branches, PRs) stays with
   the adapter behind its publication gate. The runtime's only direct GitHub
   write is `git push` of its working branch, authenticated by the installation
   token.
4. The runtime confines all filesystem writes to `workspace.root`.

---

## 6. Versioning

The contract is versioned by the single `contract_version` string, which tracks
**major** compatibility only.

- A change that adds an **optional** field, a new enum variant a consumer can
  ignore, or clarifies prose is **backward-compatible** and does **not** bump
  the version.
- A change that adds a **required** field, removes a field, renames a field,
  changes a type, or changes exit-code/status semantics is **breaking** and MUST
  bump `contract_version` to `"2"`, update both schemas and fixtures, and ship a
  migration note here.
- Consumers MUST reject a payload whose major version they do not implement,
  rather than silently mis-parsing it.

---

## 7. Conformance artifacts

| Artifact | Purpose |
|---|---|
| [`docs/contracts/session-brief.schema.json`](contracts/session-brief.schema.json) | JSON Schema for the input brief. |
| [`docs/contracts/result.schema.json`](contracts/result.schema.json) | JSON Schema for the output envelope. |
| [`docs/contracts/session-brief.example.json`](contracts/session-brief.example.json) | Golden input fixture. |
| [`docs/contracts/result.example.json`](contracts/result.example.json) | Golden output fixture. |
| `crates/github/tests/contract.rs` | Round-trips the golden fixtures through the Rust types — fails the build if the adapter drifts from this contract. |

A `coven-code` change is contract-conformant when its emitted `result.json`
validates against `result.schema.json` and it accepts any brief that validates
against `session-brief.schema.json`.
