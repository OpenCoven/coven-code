# Phase 4 — State, Config & Auth Unification under `~/.coven`

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use `- [ ]`. Phase 4 sub-plan of `2026-07-12-coven-cli-unification.md`.

**Goal:** one state root (`~/.coven`), layered config, no user-visible `~/.coven-code` on fresh installs, one auth story surfaced by one doctor, and search that spans engine TUI sessions.

**Architecture:** Two tracks. **Track C (coven-code, HIGH RISK — moves user data):** consolidate the 8 scattered `.coven-code` path sites into one `config_home()` helper (safe refactor first), then add env-override precedence + a first-run migration to `~/.coven/code/` with a compatibility symlink, then config layering. **Track D (coven):** a unified "Credentials" doctor panel and cross-session FTS that ingests external-session transcripts.

**Repos:** `[coven-code]` base `main` (b3a35a0); `[coven]` base `feat/engine-ledger` (Phase 3 Track B #349).

---

## Grounding facts (verified 2026-07-12)

**`[coven-code]` — path construction is SCATTERED across 8 sites (NOT centralized):**
- Central-ish: `config::config_dir()` (`core/src/lib.rs:1484-1495`) → `~/.coven-code`; only checks `COVEN_CODE_TEST_HOME` (test). Covers settings.json, projects/, stash, output-styles.
- Independent constructions that DON'T use `config_dir()`: `auth_store::path()` (`auth_store.rs:32-37`, auth.json), `feature_flags::get_cache_path()` (`feature_flags.rs:72-74`), `prompt_history::claude_home()` (`prompt_history.rs:139-150`, history.jsonl + pastes/), `remote_settings::claude_config_dir()` (`remote_settings.rs:394-398`), `OAuthTokens::token_file_path()` (`lib.rs:4235-4240`, legacy oauth_tokens.json), `Settings::find_project_settings()` (`lib.rs:1626-1652`, walks cwd for `.coven-code/settings.json`), `skill_discovery` (`skill_discovery.rs:356,380`, skills/). Each re-implements `dirs::home_dir().join(".coven-code")`.
- `Settings::load_hierarchical` (`lib.rs:1603-1624`): global (`config_dir()/settings.json`) then project (walk cwd for `.coven-code/settings.{json,jsonc}`), project overrides global. `Settings { config: Config, version, projects, ... }`; the "shared" keys are `config.*` (model, theme, permission_mode, provider, mcp_servers).
- `coven_shared::coven_home()` (`coven_shared.rs:27-36`): `COVEN_HOME` env → else `~/.coven` (only if it's a dir). The pattern to mirror.
- Settings migrations exist (`migrations.rs`, value-level); NO dir-level version marker. No `canonicalize()`/symlink-rejection anywhere → symlinking `~/.coven-code` → `~/.coven/code` is safe.
- COVEN.md rebrand table names `.coven-code` data dir but gives no relocation guidance.

**`[coven]` — auth + FTS:**
- Doctor engine-auth: `engine_auth_summary(binary)` (`main.rs:1773-1807`) runs `<engine> auth status --json` (5s bounded), parses `loggedIn`; printed at `main.rs:1141-1145`. Harnesses section (`main.rs:1094-1112`) shows availability only — NO per-harness auth probe exists (codex/claude have no known `auth status --json`).
- FTS5: `events_fts` virtual table (`store.rs:454-471`) populated by an AFTER INSERT trigger on `events`; `search_events()` (`store.rs:1898-1928`); CLI `run_sessions_search` (`main.rs:751-775`). External sessions (Phase 3) have `transcript_path` + `external` columns but NO events → invisible to search.
- Privacy/retention: `PrivacyConfig` (`privacy.rs:12-27`, log_retention_days default 30); `prune_events_older_than` (`store.rs`); events must be redacted (`privacy::redact_payload_json_with_config`) + carry retention expiry. FTS auto-invalidates on event DELETE (trigger). No daemon background loop exists (only the accept loop) — ingestion hooks either lazily on search or as a new task.

---

## Track C — engine home relocation (coven-code)

### Task 4.1: Consolidate all `.coven-code` path construction into one `config_home()` helper (SAFE refactor, no behavior change) `[coven-code]`

**Rationale:** relocation is only safe if ALL engine data moves together. Today 8 sites build the path independently. This task introduces one helper that STILL returns `~/.coven-code` (identical behavior) and routes all 8 sites through it — turning relocation into a 1-helper change and eliminating the data-split risk. Ship this ALONE first; it's a pure refactor with zero user-visible change.

**Files:** `core/src/config.rs` (or `lib.rs` where `config_dir` lives) — add `config_home()`; refactor `config_dir`, `auth_store.rs`, `feature_flags.rs`, `prompt_history.rs`, `remote_settings.rs`, `lib.rs` (`OAuthTokens::token_file_path`, `find_project_settings`), `skill_discovery.rs`.

- [ ] **Step 1:** Add `pub fn config_home() -> PathBuf` returning EXACTLY today's value: honor `COVEN_CODE_TEST_HOME` (test) then `dirs::home_dir().join(".coven-code")`. (Env-override precedence comes in 4.2 — keep 4.1 behavior-identical.)
- [ ] **Step 2:** Route the CENTRAL `config_dir()` through `config_home()`. Route the 6 independent global helpers (auth_store, feature_flags, prompt_history, remote_settings, OAuthTokens legacy, skill_discovery-global) through `config_home()`. For the two cwd-walking sites (`find_project_settings`, `skill_discovery` project-level), extract the `.coven-code` PROJECT dir-name into a shared `const PROJECT_CONFIG_DIRNAME: &str = ".coven-code";` (they join it onto arbitrary cwd ancestors, NOT the home dir — do NOT route those through `config_home`; just de-magic the string so 4.x can rename consistently).
- [ ] **Step 3:** Tests: a test asserting `config_home()` == every subsystem's base (auth.json parent, history.jsonl parent, etc. all share `config_home()`); a test that `COVEN_CODE_TEST_HOME` still overrides. Grep to PROVE zero remaining independent `home_dir().join(".coven-code")` constructions outside `config_home()` + the project-dirname const.
- [ ] **Step 4:** `cargo fmt/check/clippy -D warnings/test` (workspace). Signed commit `refactor(core): route all engine-home paths through one config_home() helper`. NO AI trailer (coven-code AGENTS.md).

### Task 4.2: Env-override precedence + first-run migration to `~/.coven/code/` `[coven-code]`

**Files:** `config.rs`/`lib.rs` (`config_home` precedence), a new `core/src/home_migration.rs`, called once at startup (`cli/src/main.rs` early, before any path use).

- [ ] **Step 1:** `config_home()` precedence becomes: `COVEN_CODE_HOME` (explicit) → if `COVEN_HOME` set OR `COVEN_PARENT=coven` → `<coven_home>/code` (default `~/.coven/code`) → else legacy `~/.coven-code`. Keep `COVEN_CODE_TEST_HOME` first for tests. Unit-test each precedence branch with env isolation (there's an env lock — `coven_shared::COVEN_HOME_ENV_LOCK`).
- [ ] **Step 2:** `home_migration::migrate_if_needed()`: if the RESOLVED home is `~/.coven/code` AND it does not exist AND legacy `~/.coven-code` DOES exist → move the directory (`std::fs::rename`; fall back to recursive copy across filesystems) to `~/.coven/code`, then create a symlink `~/.coven-code` → `~/.coven/code` (unix `symlink`; Windows `symlink_dir`/junction, or skip-with-log on failure). Idempotent, best-effort, every error logged not fatal (a failed migration must not brick the engine — fall back to whichever dir has data). Write a `~/.coven/code/.migrated-from-coven-code` marker so it runs once.
- [ ] **Step 3:** Call `migrate_if_needed()` at the very start of `main` (before settings load / any `config_home()` consumer). Tests: fixture with a populated fake `~/.coven-code` + `COVEN_HOME` set → after migrate, files live under `<coven_home>/code`, symlink exists, marker present; second call is a no-op; absent-legacy-dir is a no-op.
- [ ] **Step 4:** gates + signed commit `feat(core): relocate engine home to ~/.coven/code under the unified CLI (migrating in place)`. Update COVEN.md rebrand table.

### Task 4.3: Config layering — shared `~/.coven/settings.json` `[coven-code]`

**Files:** `lib.rs` `load_hierarchical`.

- [ ] Load order becomes: `~/.coven/settings.json` (shared keys: model, theme, permission defaults — a documented subset) → `~/.coven/code/settings.json` → project `.coven-code/settings.json`; later overrides earlier. Only a whitelisted subset is read from the shared file (ignore unknown/engine-only keys there). Document the shared-key schema in COVEN.md + the coven contract. Tests for the 3-layer precedence. Signed commit.

---

## Track D — one auth story + cross-session search (coven)

### Task 4.4: Unified "Credentials" doctor panel `[coven]`

**Files:** `crates/coven-cli/src/main.rs` (`run_doctor`).

- [ ] Replace the scattered engine-auth line + harness-availability list with a single "Credentials" section that, per provider, shows: engine (`auth status --json` → logged in/out, the existing real check) and each available harness (codex/claude) with availability + a best-effort auth note. Since codex/claude expose no machine-readable auth probe, show availability + an install/login hint rather than inventing a status. Keep exit-code semantics (auth is a warning, missing engine/harness a blocker). Test the panel's pure formatting via extracted helpers. Signed commit (coven repo — trailer optional).

### Task 4.5: Cross-session FTS — ingest external-session transcripts `[coven]`

**Files:** `store.rs` (ingest fn + FTS), `daemon.rs` or the search path (hook), `main.rs` (`run_sessions_search`).

- [ ] On `coven sessions search` (lazy, simplest — no daemon loop exists), for each `external` session with a `transcript_path` not yet ingested (track via a `transcript_indexed_at` column or a per-session marker), read its JSONL transcript, extract the text content per line, REDACT via the existing privacy path, insert as `events` rows (kind `transcript_text`) with a retention-expiry so the FTS trigger indexes them and pruning still applies. Bound the work (cap lines/bytes; log truncation). Then the existing `search_events` spans them. Tests: an external session with a fixture transcript becomes searchable; redaction applied; re-search doesn't re-ingest. Signed commit.

---

## Phase 4 exit check

Fresh install writes nothing outside `~/.coven` (engine home is `~/.coven/code`); an existing `~/.coven-code` user upgrades in place with data intact + a compat symlink (populated-home migration test). `coven doctor` shows one Credentials panel answering "am I logged in, to what, via what." `coven sessions search <term>` finds content from an interactive engine session.

## Sequencing & risk

- **4.1 first, alone, and merge it before 4.2** — it's a zero-behavior-change refactor that de-risks everything after. Do NOT combine 4.1 (safe) with 4.2 (data migration) in one reviewable unit.
- 4.2 is the highest-risk task in the whole unification: it MOVES user data. Guard rails: idempotent, best-effort, never-fatal, marker-gated, populated-fixture migration test, symlink-back for one deprecation cycle, and a clear fallback (if migration fails, keep using whichever dir has data). Consider shipping 4.2 behind an off-by-default flag first, flipping to on after real-world validation.
- Track D (4.4, 4.5) is independent of Track C and can proceed in parallel on the coven side.
- Both repos keep their gates (fmt/clippy -D warnings/test) and signed commits; coven-code commits carry NO AI trailer.
