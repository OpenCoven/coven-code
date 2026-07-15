# Phase 5 — Brand / UX Naming Sweep

> **Status: COMPLETE (sealed 2026-07-14).** Landed via coven-code #154
> (user-facing sweep + direct-invocation notice), #158 (LLM-facing tool
> descriptions), #160 (ACP display name). Task 5.6 npm deprecation was
> executed with user approval — the registry now serves the deprecation
> notice — and #162 made engine npm-publish opt-in per release. Historical
> record; see [`docs/unification.md`](../../unification.md).

> Phase 5 sub-plan of the coven CLI unification. Curated, recon-grounded. Only USER-FACING surfaces change; the binary name (`coven-code`/`coven-cave`), `claurst-*` crates, `COVEN_CODE_*` env vars, `.coven-code` internal paths, and repo URLs STAY (per COVEN.md).

**Goal:** users see the unified "Coven" brand; nobody is told to install/use "coven-code" separately; `coven --version` surfaces the whole stack.

## Decisions (defaults — vetoable in review)
- npm package NAME stays `@opencoven/coven-code` (only description changes; it is DEPRECATED in 5.6, not renamed — renaming a published package is disruptive and breaks the engine install/release flow).
- Assistant identity → "You are Coven, an open-source agentic coding assistant by OpenCoven (based on Claurst, GPL-3.0)." (keep the credit).
- User-Agent `CovenCode/x.y` → `Coven/x.y`.
- Direct-invocation notice (dim, stderr): `coven-code is the Coven engine — the supported CLI is 'coven' (npm i -g @opencoven/cli)`.

## Already done (verified — no-op tasks)
- **Theme (5.4):** `crates/tui/src/theme_colors.rs default_theme()` is ALREADY the OpenCoven palette (accent `Rgb(139,92,246)` #8B5CF6, secondary `Rgb(236,72,153)` #EC4899). Nothing to change.
- **Completions (5.5):** `coven completions` generates from the clap `Command`, so the passthroughs (auth/models/acp/code) are already covered. Add a coven-side test asserting they appear (optional).

## Task 5.1 + 5.2 — coven-code brand sweep [coven-code]
### 5.2 — rename the ~17 user-facing strings (curated list; do NOT touch anything else)
- `crates/core/src/system_prompt.rs:174,177` — "You are Coven Code, ..." → "You are Coven, ..." (keep the "(based on Claurst.../GPL-3.0)" credit).
- `crates/cli/src/main.rs:123` — `about = "Coven Code - AI-powered coding assistant"` → `"Coven - AI-powered coding assistant"`. `:4689` — auth help "...Coven Code OAuth client" → "...Coven OAuth client".
- `crates/tools/src/web_search.rs:163` — `"CovenCode/1.0"` → `"Coven/1.0"`.
- `crates/core/src/update_check.rs:67` — `format!("CovenCode/{}", current)` → `format!("Coven/{}", current)`.
- `crates/tui/src/onboarding_dialog.rs:304,320,333,337` — "Welcome to Coven Code" / "Coven Code is..." / "Coven Code can..." → "Coven".
- `crates/tui/src/lib.rs:345,371` — terminal title `"✨ Coven Code"` → `"✨ Coven"`.
- `crates/tui/src/invalid_config_dialog.rs:159,164,168` — "Restart Coven Code." → "Restart Coven.".
- `crates/tui/src/export_dialog.rs:171,176` — "# Coven Code Conversation Export" → "# Coven Conversation Export"; the `"**Coven Code**"` assistant role label → "**Coven**".
- `crates/tui/src/feedback_survey.rs:146` — "How is Coven Code doing this session?" → "How is Coven doing this session?".
- `npm/package.json:4` — description: replace "Coven engine (agentic coding TUI)..." wording's "Coven Code" (if present) — keep pointing at @opencoven/cli. NAME unchanged.
DO NOT change: binary/crate/env/path/theme-const/test/comment/repo-URL references.

### 5.1 — direct-invocation notice [coven-code]
In `crates/cli/src/main.rs` after `Cli::parse()` (~line 503) and after the `is_headless` determination (~643): if the run is INTERACTIVE (not headless/print/prompt), stdout/stderr is a TTY (`crossterm::terminal::is_terminal`), AND `COVEN_PARENT` is unset → `eprintln!` the dim notice once (before `run_interactive`). Silent when `COVEN_PARENT` is set (driven by coven) or non-interactive. The `coven-cave` alias re-execs `coven-code`, so it inherits this. Test the pure decision (`should_show_engine_notice(is_interactive, is_tty, coven_parent_set) -> bool`).

## Task 5.3 — `coven --version` surfaces engine + pin [coven] — DONE
Custom `--version` intercept landed on coven main (`version_line()` in `crates/coven-cli/src/main.rs`): `coven <desc> (engine coven-code <installed|not installed>, pinned <pinned>)`.

## Task 5.6 — npm deprecation [DONE — executed with user approval]
`npm deprecate @opencoven/coven-code "Install @opencoven/cli — coven-code is now the Coven engine"` has been run; the registry serves the notice. Engine binaries keep shipping via GitHub Releases (npm publish is opt-in per release since #162).

## Exit check
A new user reaches install → first session without seeing the string "coven-code" in any prose surface; running `coven-code` directly prints the "use coven" hint; `coven --version` shows coven + engine + pin.
