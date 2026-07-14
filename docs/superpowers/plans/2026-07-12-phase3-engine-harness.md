# Phase 3 — Engine as a First-Class Daemon-Ledgered Harness

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use `- [ ]`. This is the Phase 3 sub-plan of `2026-07-12-coven-cli-unification.md`.

**Goal:** `coven run coven-code "task"` produces a normal ledgered session (detach/attach/kill/archive/search); the bare-prompt cast defaults to the engine; and interactive engine TUI sessions register themselves in the daemon ledger so `coven sessions` sees them.

**Architecture:** Two independent tracks. **Track A (coven-side, self-contained):** add `coven-code` as a built-in harness whose availability/spawn consults `engine::resolve()`, and prefer it in bare-prompt selection. This needs no engine changes and delivers `coven run coven-code` immediately. **Track B (cross-repo, design-heavy):** a new daemon endpoint to *register an externally-owned running session* (distinct from the existing launch-a-PTY flow), plus an engine-side notifier that fires it on TUI session start/end using the engine's existing `DaemonClient`.

**Tech stack:** coven-cli (sync Rust, `harness.rs`/`api.rs`/`store.rs`), engine (tokio Rust, existing `claurst_core::coven_daemon::DaemonClient`).

**Repos:** `[coven]` = OpenCoven/coven (base branch `feat/engine-lock`, Phase 2), `[coven-code]` = OpenCoven/coven-code (base `main`).

---

## Grounding facts (verified 2026-07-12)

- `[coven] harness.rs`: `HarnessCommandSpec` (id/label/executable/prefix-args/system_prompt_flag/model_flag/sandbox/`Capabilities`/`StreamArgs`); `built_in_harness_specs()` has codex + claude (mirror claude for coven-code). **Availability is `executable_exists(&spec.executable)` — PATH-only (`harness.rs:1219`), called from `HarnessSummary::from_spec` (`harness.rs:349`).** Unix spawn is `spawn_executable_for_platform()` → returns the name as-is (`harness.rs:1239`). Stream parser `stream_json.rs` `Event` enum handles system/user/assistant/tool_result/output/result via `#[serde(tag="type")]` (rejects unknown types). `default_harness_id()` (`main.rs:1568`) = codex→claude. `attach_session()` (`main.rs:2685`) replays store events for non-running sessions; forwards stdin to the daemon PTY for running ones. Daemon UDS `~/.coven/coven.sock`; `POST /api/v1/sessions` (`api.rs:1459`) takes `SessionLaunch` and **spawns a harness (daemon owns the PTY)**. `SessionRecord` (`store.rs:17`) columns: id, project_root, harness, title, status, exit_code, archived_at, created_at, updated_at, conversation_id, familiar_id, labels, visibility.
- `[coven-code]`: the engine is ALREADY a daemon client — `crates/core/src/coven_daemon.rs` `DaemonClient` (blocking `UnixStream` + HTTP/1.0, `~/.coven/coven.sock`, `new() -> Option`, `is_online`, `request`, `create_session`, `send_input`, `kill_session`; re-exported via `coven_shared`). `tui/src/handoff.rs` already calls `create_session(CreateSessionRequest{familiar, project_root, harness, title, initial_message})` — that's the LAUNCH flow (daemon spawns openclaw). No "register my own session" flow exists. TUI session lifecycle: `run_interactive()` (`cli/src/main.rs:1937`, async, `#[tokio::main]`); session id/project_root established ~`main.rs:1985-2035`; clean exit before `restore_terminal()` ~`main.rs:4561-4568`. Transcript path: `claurst_core::session_storage::transcript_path(project_root, session_id)` → `~/.coven-code/projects/<b64>/<id>.jsonl`. Settings bool pattern: `#[serde(default, rename="camelCase")] pub field: bool` in `core/src/lib.rs` Settings (~1069). `coven_shared::coven_home()` reads `COVEN_HOME`→`~/.coven`.

---

## Track A — coven-side harness (do first; independent)

### Task 3.1: `coven-code` built-in harness spec + engine-aware availability `[coven]`

**Files:** `crates/coven-cli/src/harness.rs` (spec + availability hook), inline tests.

**Design:** Add a `coven-code` spec mirroring claude but with the engine's flag dialect. Make availability and spawn consult `engine::resolve()` so a *managed* engine (not on PATH) counts as available and spawns by absolute path.

- [ ] **Step 1 — failing test** (harness.rs tests): assert `built_in_harness_specs()` contains a `coven-code` spec with `executable == "coven-code"`, `non_interactive_prompt_prefix_args == ["--print"]`, `system_prompt_flag == Some("--append-system-prompt")`, `model_flag == Some("--model")`, `capabilities.stream == true`, `capabilities.preassigned_session_id == true`, and stream `session_id_flag == Some("--session-id")`, `resume_flag == Some("--resume")`. And a test that `coven_code_spawn_executable()` returns the `engine::resolve()` path when a managed engine exists.
- [ ] **Step 2** — run, confirm fail.
- [ ] **Step 3 — implement the spec** in `built_in_harness_specs()` (after the claude spec):
```rust
HarnessCommandSpec {
    id: "coven-code".to_string(),
    label: "Coven Code".to_string(),
    executable: "coven-code".to_string(),
    interactive_prompt_prefix_args: Vec::new(),
    non_interactive_prompt_prefix_args: vec!["--print".to_string()],
    install_hint: "Install the Coven engine with `coven engine install`.".to_string(),
    source: "bundled".to_string(),
    manifest_path: None,
    // The engine composes its own base system prompt; append, never replace.
    system_prompt_flag: Some("--append-system-prompt".to_string()),
    model_flag: Some("--model".to_string()),
    model_arg_template: None,
    // NOTE kebab-case values — the engine's --permission-mode differs from
    // Claude Code's camelCase bypassPermissions.
    sandbox: Some(SandboxMapping::Flag {
        flag: "--permission-mode".to_string(),
        full: "bypass-permissions".to_string(),
        read_only: "plan".to_string(),
    }),
    capabilities: Capabilities { stream: true, preassigned_session_id: true, think: true, speed: false },
    stream_args: Some(StreamArgs {
        prefix_args: vec![
            "--print".to_string(), "--input-format".to_string(), "stream-json".to_string(),
            "--output-format".to_string(), "stream-json".to_string(),
        ],
        session_id_flag: Some("--session-id".to_string()),
        resume_flag: Some("--resume".to_string()),
    }),
}
```
- [ ] **Step 4 — engine-aware availability.** In `executable_exists` (or a wrapper used by `HarnessSummary::from_spec`), special-case the coven-code executable: if `executable == "coven-code"`, return `crate::engine::resolve().is_some() || <normal PATH check>`. Keep other harnesses PATH-only. (Verify the `--effort` mapping: `think` capability maps to `--effort high` in `launch_option_args`; confirm the engine accepts `--effort high` — it does per Phase-0 contract — and that the claude-specific `launch_option_args` branch also covers coven-code or add a branch.)
- [ ] **Step 5 — engine-aware spawn.** Where the unix `spawn_executable_for_platform()` returns the name as-is, add: if the name is `coven-code`, return `engine::resolve().map(|e| e.path.display().to_string()).unwrap_or_else(|| name)`. So a managed engine spawns by absolute path; a PATH engine still works.
- [ ] **Step 6** — `cargo test -p coven-cli`, clippy, fmt. **Live smoke:** `coven run coven-code -- "print the word BANANA and nothing else" --permission read-only` produces a ledgered session that streams output; `coven doctor` still shows the engine; `coven sessions` lists the run.
- [ ] **Step 7** — signed commit `feat(harness): add coven-code as a first-class engine harness`.

### Task 3.4: bare-prompt prefers the engine `[coven]`

**Files:** `crates/coven-cli/src/main.rs` (`default_harness_id`, ~1568).

- [ ] Test: with all three available, `default_harness_id()` returns `coven-code`. Implement: prepend a `coven-code`-available check before codex/claude. Run/clippy/fmt. Signed commit `feat(cli): prefer the coven-code engine for bare-prompt casts`.

### Task 3.2: stream-json golden fixture + parser coverage `[coven]`

**Files:** `crates/coven-cli/tests/fixtures/engine/` (new golden transcript), a stream-parser test, `docs/ENGINE-CONTRACT.md` (+ "Stream-json output" note).

- [ ] Record a real transcript: `coven-code --print "say hi" --output-format stream-json > fixtures/engine/basic.stream.jsonl` (commit it; scrub any absolute paths/tokens). Add a test that feeds each line through `stream_json::read_event` and asserts every line parses to a known `Event` variant (no `Err`). If any engine event kind is unhandled, EXTEND the coven `Event` enum (do NOT change the engine's output — it must stay Claude-compatible). Add the observed event kinds to the contract doc. Signed commit `test(stream): golden engine stream-json fixture + parser coverage`.

---

## Track B — external-session registration (cross-repo, design-heavy)

### Design: registering an externally-owned session (resolves the launch-vs-register gap)

The daemon's `POST /api/v1/sessions` spawns a harness and owns its PTY. A TUI session is already running inside the engine, so it must be *registered*, not launched. Decision:

- **New daemon endpoint `POST /api/v1/sessions/external`** `[coven, api.rs]`: body `{id, projectRoot, harness, title, transcriptPath}`. It inserts a `SessionRecord` (status `running`) WITHOUT touching `LiveSessionRuntime` (no PTY), and marks it externally-owned.
- **Schema:** add an `external` boolean column to the sessions table (`store.rs`), default false, migration-safe (ALTER TABLE ADD COLUMN with default; existing rows read false). `attach` and `kill` consult it.
- **`POST /api/v1/sessions/:id/complete`** `[coven, api.rs]`: body `{exitCode?}`; sets status `completed`/`failed`, `updated_at`. Idempotent.
- **Attach semantics** `[coven, main.rs attach_session]`: for an `external` running session there is no daemon PTY — do NOT spawn the stdin forwarder; print "interactive engine session — attach shows the recorded ledger, not the live terminal" and replay events/transcript. Test with a synthetic external session row.
- **Transcript indexing (search):** the daemon has no PTY output for external sessions, so `coven sessions search` needs the transcript. Minimal Phase-3 scope: register the `transcriptPath` on the record (new nullable column or reuse labels); a follow-up (Phase 4 cross-session FTS) ingests it. For Phase 3, `coven sessions` LISTING the external session is the exit bar; full-text search over its transcript is Phase 4.

### Task 3.3a: daemon external-session endpoints + schema `[coven]`

**Files:** `store.rs` (external column + migration + optional transcript_path column), `api.rs` (two endpoints + payload structs), tests.

- [ ] TDD the store migration (add column, existing rows default false, insert/read roundtrip). TDD the two handlers (register inserts running+external, no runtime touch; complete updates status; unknown id → 404). Keep the daemon's existing endpoints untouched. Run/clippy/fmt. Signed commit `feat(daemon): register and complete externally-owned sessions`.

### Task 3.3b: engine session notifier `[coven-code]`

**Files:** `crates/core/src/coven_daemon.rs` (add `register_external_session` + `complete_session` methods mirroring `create_session`'s transport), a thin `crates/tui/src/coven_ledger.rs` (or in cli/main.rs) that fires them, settings flag `daemonLedger` in `core/src/lib.rs`, hooks in `cli/src/main.rs run_interactive` start (~2035) and exit (~4567).

- [ ] Add `DaemonClient::register_external_session(RegisterExternalRequest{id, project_root, harness:"coven-code", title, transcript_path})` and `complete_session(id, exit_code)` using the existing `request()` transport. Add `daemonLedger: bool` setting (default: decide — recommend default **true** once this is proven; ship opt-in `false` first). At session start, if the setting is on AND `DaemonClient::new().is_some()`, fire register in a `tokio::spawn` (best-effort; every error swallowed to a debug log — a dead daemon must never affect the TUI). At clean exit, fire complete likewise. Windows: `DaemonClient` is `#[cfg(unix)]`-guarded already; behind that guard, no-op on Windows (documented). Tests: the request builders serialize correctly; the fire path is a no-op when the setting is off or the daemon is absent. Respect AGENTS.md (no unwrap; scoped commits). Signed commit `feat(tui): register interactive sessions in the Coven ledger (opt-in)`.

### Task 3.5: contract + docs `[both]`

- [ ] Add the `/sessions/external` + `/sessions/:id/complete` endpoints and the `coven-code` harness/stream surfaces to `docs/ENGINE-CONTRACT.md` (bump contract note if the engine now depends on them — it depends on the daemon endpoints, which is the reverse direction, so document under a new "Daemon endpoints the engine calls" heading). Update the coven API-CONTRACT.md. Signed commits in each repo.

---

## Phase 3 exit check

`coven run coven-code -- "summarize this repo in 3 bullets" --detach` then `coven attach <id>` streams the recorded output; bare `coven "hello"` selects the engine; a session started via bare `coven` (interactive) appears in `coven sessions` (external, registered by the notifier); `coven attach` on that interactive session replays its ledger with the "shows the ledger, not the live terminal" notice.

## Sequencing

Track A (3.1 → 3.4 → 3.2) is pure `[coven]`, independent, and ships `coven run coven-code` + engine-preferred casts immediately — do it first. Track B (3.3a daemon endpoints → 3.3b engine notifier → attach semantics → 3.5 docs) is the cross-repo, schema-touching work; land 3.3a before 3.3b (the engine notifier needs the endpoints to exist). Each task keeps both repos' gates (fmt/clippy -D warnings/test) and signed commits.

## Risks

- **Daemon schema migration** (external column): use `ALTER TABLE ... ADD COLUMN ... DEFAULT 0`; test against a pre-existing DB fixture so old sessions still load.
- **Notifier must never affect the TUI:** fire-and-forget in `tokio::spawn`, all errors → debug log, gated behind a setting and a `DaemonClient::new().is_some()` check.
- **`--effort` / think mapping:** confirm `launch_option_args` emits the engine-compatible `--effort high` (not a claude-only flag) for the coven-code harness; add a coven-code branch if the current one is claude-gated.
- **Stream format drift:** extend coven's parser to the engine's events; never fork the engine's stream-json (it must stay Claude-compatible for its own consumers).
- **Attach on a live external session:** without the `external` flag, attach would try to follow a non-existent daemon PTY — the flag + the attach branch are load-bearing; test them.
