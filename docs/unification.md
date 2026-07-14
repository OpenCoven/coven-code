# The Coven merge — unified CLI guide

**Status:** in progress · **Last verified:** 2026-07-13 · **Contract:** v1

This guide explains the merge of `coven-code` into the unified `coven` CLI
("the Coven CLI unification"), what users need to do to move over seamlessly,
and how the merge is implemented comprehensively in the Cave
([OpenCoven/coven-cave](https://github.com/OpenCoven/coven-cave), the desktop
control room).

Authoritative sources this guide summarizes:

- Plan: [`docs/superpowers/plans/2026-07-12-coven-cli-unification.md`](superpowers/plans/2026-07-12-coven-cli-unification.md)
- Phase 3 sub-plan: [`docs/superpowers/plans/2026-07-12-phase3-engine-harness.md`](superpowers/plans/2026-07-12-phase3-engine-harness.md)
- Compatibility contract: [coven/docs/ENGINE-CONTRACT.md](https://github.com/OpenCoven/coven/blob/main/docs/ENGINE-CONTRACT.md) (`contract_version: 1`)
- Engine-side pointer: [`COVEN.md`](../COVEN.md) "Engine contract"

---

## 1. What the merge is

`coven` becomes the one CLI users install and type for everything —
interactive agent work, headless runs, session management, auth, models.
`coven-code` is absorbed as the **Coven engine**: a version-pinned,
checksum-verified binary that `coven` installs, upgrades, health-checks, and
drives. The name "coven-code" disappears from install instructions and
user-facing copy; the binary keeps its name internally and lives under
`~/.coven/engine/<version>/`.

### One surface, two processes — not one binary

This is a **product merge, not a source merge**. All integration crosses a
process boundary (exec/spawn, stream-json over stdio, HTTP over a Unix
socket). No Cargo dependency from `coven` on any `claurst-*` crate, ever.

Why:

1. **License.** `coven` is MIT; `coven-code` is GPL-3.0, inherited from
   upstream Claurst whose copyright OpenCoven does not wholly own. Statically
   linking would make the combined binary a GPL derivative and force `coven`
   to GPL-3.0. A process boundary keeps each work under its own license (the
   ripgrep-in-VS-Code / containerd-under-Docker pattern).
2. **Upstream merges.** This repo deliberately preserves `claurst-*` crate
   names so `git merge upstream/main` stays low-friction. Moving the source
   into the coven workspace would end that.
3. **Runtime split.** coven-cli is deliberately synchronous; the engine is
   tokio-based, ~148k LoC, with different ratatui/rusqlite majors. Unifying
   buys nothing users can see.

"Singular" means: one package to install, one command to type, one doc site,
one config root, one session ledger. The engine binary is an implementation
detail, like a bundled shared library.

### The compatibility contract

Coven drives the engine only through the surfaces listed in
[ENGINE-CONTRACT.md](https://github.com/OpenCoven/coven/blob/main/docs/ENGINE-CONTRACT.md):
`--version`, `--print`, the stream-json loop
(`--print --input-format stream-json --output-format stream-json`),
`--session-id`/`--resume`, `--model`/`--append-system-prompt`/`--cwd`,
`--permission-mode`, `auth status --json`, and `acp`. Breaking any of them
requires a contract version bump plus a coordinated `engine.lock` update in
coven (see [`AGENTS.md`](../AGENTS.md)). Coven's contract tests run in this
repo's CI against every engine build (the `coven-contract` job in
`rust-ci.yml`, still non-blocking while the job proves stable; the coven-side
suite landed with coven #353, so flipping it to a hard gate is the remaining
step), so a contract break is caught **before** release, not after.

### Target command tree (end state)

```
coven                        → engine interactive TUI (auto-installs engine on first run)
coven "fix the login bug"    → cast free-text task (defaults to the engine harness)
coven run <harness> [...]    → daemon-ledgered session: codex | claude | coven-code | ...
coven sessions|attach|kill|… → session ledger (includes engine sessions)
coven auth | models | acp    → engine passthroughs
coven engine status|install|which → managed-engine admin
coven code [anything…]       → raw passthrough escape hatch to the engine CLI
coven doctor                 → daemon + harnesses + engine + auth
```

Collision policy: coven-owned names always win (`run`, `sessions`, `daemon`,
…). New engine subcommands are reachable through `coven code <sub>` on day
one, without waiting for a coven release.

---

## 2. Where the merge stands (verified 2026-07-12)

| Phase | Deliverable | Status |
|---|---|---|
| 0 | ADRs + compatibility contract in both repos | **Landed** — coven-code [#150](https://github.com/OpenCoven/coven-code/pull/150) merged; contract doc landed with coven [#346](https://github.com/OpenCoven/coven/pull/346) |
| 1 | Engine resolver, `coven engine` admin, auto-install, passthroughs, doctor | **Landed** — coven [#346](https://github.com/OpenCoven/coven/pull/346) merged |
| 2 | `engine.lock` pin, cross-repo contract-test CI, one install story | **Landed** — engine side coven-code [#151](https://github.com/OpenCoven/coven-code/pull/151) merged (reverse contract-CI job + install-story notices); coven side [#353](https://github.com/OpenCoven/coven/pull/353) merged |
| 3A | Engine as first-class harness; bare prompts prefer the engine | **Landed** — coven [#354](https://github.com/OpenCoven/coven/pull/354) merged, zero engine-side changes |
| 3B | Externally-owned sessions in the daemon ledger | **Landed** — coven [#355](https://github.com/OpenCoven/coven/pull/355) (daemon endpoints) + coven-code [#152](https://github.com/OpenCoven/coven-code/pull/152) (TUI notifier, opt-in `daemonLedger` setting) merged |
| 4 | State/config/auth unification under `~/.coven` | **Landed** — coven-code [#153](https://github.com/OpenCoven/coven-code/pull/153) merged (engine home `~/.coven/code/` with in-place migration + shared settings layer) |
| 5 | Brand/UX sweep, npm deprecation of the engine package | **In progress** — coven-code [#154](https://github.com/OpenCoven/coven-code/pull/154) merged (user-facing rebrand + direct-invocation notice); npm deprecation pending |
| 6 | Decision gate: full source merge | Standing recommendation: **don't** — keep the process boundary |

The coven-side phases landed in order (#346, #353, #354, #355; the original
stacked PRs #347–#349 were superseded by these re-landed equivalents after
their base branches were cleaned up). coven-code #152 is independently
functional: the notifier is fire-and-forget and silently no-ops against an
older daemon.

What already works today, on `main` of this repo: every contract surface
(the engine has shipped them since 0.6.1), the reverse contract-CI job, and
the `coven-runtimes` adapter manifest
([`spec/runtime-manifest/coven-code.json`](../spec/runtime-manifest/coven-code.json))
that lets the coven daemon drive `coven-code` as a manifest-declared adapter
even on coven builds that predate the built-in harness (#354).

---

## 3. What users need to do

Short version: **nothing is required**. Every step of the merge is additive
and the legacy paths keep working. The steps below are what to do to be on
the unified path, per audience.

### New users

```bash
npm install -g @opencoven/cli
coven
```

First interactive run offers to install the engine (~15 MB, one keypress),
then opens the TUI. There is no separate coven-code install step. To
pre-fetch instead of lazily installing: `coven engine install`.

### Existing `coven` users (daemon/CLI already installed)

Upgrade `@opencoven/cli` when Phase 1–2 releases, then:

```bash
coven engine install   # installs the pinned, checksum-verified engine
coven doctor           # shows the Engine section: source, version, pin, auth
```

Nothing else changes: `coven run codex|claude` behave exactly as before.
After Phase 3A, bare-prompt casts (`coven "do X"`) prefer the engine when it
is available — use `coven run codex|claude -- "…"` to pick a specific
harness explicitly (bare prompts intentionally accept no flags).

### Existing coven-code users (PATH or npm installs)

Everything keeps working, in this order of resolution when coven looks for
the engine:

1. `COVEN_ENGINE_BIN` (explicit override — dev builds, custom paths)
2. Managed: `~/.coven/engine/<current>/coven-code`
3. `coven-code` on PATH
4. Legacy: `~/.coven-code/bin/coven-code`

Notes:

- A **managed engine wins over a PATH copy** once you run
  `coven engine install` — that command is authoritative. `coven engine which`
  prints which binary won and `coven doctor` prints why.
- Direct `coven-code …` invocation remains fully supported. Since the
  Phase 5 sweep (#154) it prints one dim notice line when run interactively
  outside coven (`COVEN_PARENT` unset); scripted/delegated runs stay silent.
- The `@opencoven/coven-code` npm package keeps working through Phase 5, and
  only then gets an `npm deprecate` notice. GitHub release archives remain
  the artifact source permanently (they are what `coven engine install`
  downloads). Note the bare `coven-code` npm name is an unrelated, already
  deprecated package — only the scoped name is real.
- Version drift: if your PATH engine is older than coven's pin you get a
  one-line warning; below `MIN_ENGINE_VERSION` (0.6.1) coven refuses with an
  actionable message. `coven engine install` fixes both.

### Script and CI authors (headless)

No changes required. `coven-code --print …`, the stream-json loop, exit codes
(0 success / 1 error), and `auth status --json` are contract-frozen surfaces.
Two useful additions:

- `COVEN_NO_AUTO_INSTALL=1` forces the no-engine error instead of an
  interactive prompt (the prompt is already TTY-gated, so CI never blocks).
- Prefer `coven code <args…>` in new scripts — it resolves the engine for
  you; `coven-code` on PATH keeps working for old ones.

### State, credentials, and settings

- Phase 4 landed (#153): the engine home migrates in place to
  `~/.coven/code/` on first launch, with a **compatibility symlink** at
  `~/.coven-code` for one deprecation cycle — automatic, no user action,
  and reversible (remove symlink, restore dir). Pre-migration installs keep
  reading `~/.coven-code/` until they upgrade.
- Credentials are never shared or imported across CLIs. `coven auth login`
  is a passthrough into the engine's own auth; the engine never replays
  OAuth tokens from Claude Code or any other CLI (hard rule, see
  [`AGENTS.md`](../AGENTS.md) Providers).

### Optional: sessions in the ledger (`daemonLedger`)

With coven #355 and coven-code #152 landed, add to the engine settings
(`~/.coven/code/settings.json`; the legacy `~/.coven-code/settings.json`
path still works through the compatibility symlink):

```json
{ "daemonLedger": true }
```

Interactive TUI sessions then register themselves in the Coven daemon ledger
(best-effort; a dead or absent daemon never affects the TUI; Unix only).
They appear in `coven sessions` as *external* sessions: `coven attach`
replays the recorded ledger (with an explicit "not the live terminal"
notice) and `coven kill` refuses them (`422 external_session_not_killable`)
because the daemon does not own the process. The setting ships opt-in
(default off) and is planned to default on in a later release now that the
Phase 4 config layering is in place.

### Windows

Everything above applies except the ledger notifier (the daemon client is
Unix-socket-only; the notifier is a documented no-op on Windows). The
managed-engine layout uses a plain-text `current` pointer file, not
symlinks, so `coven engine install` works identically. Passthroughs use
spawn-and-wait rather than exec.

### Environment variables (merge-relevant)

| Variable | Set by | Meaning |
|---|---|---|
| `COVEN_ENGINE_BIN` | user | Absolute path override for the engine binary; beats all resolution |
| `COVEN_NO_AUTO_INSTALL` | user/CI | `1` disables the first-run install prompt |
| `COVEN_PARENT=coven` | coven | Present on every delegated invocation; the engine uses it to know it is being driven |
| `COVEN_HOME` | user | Coven state root (`~/.coven` default); forwarded to the engine when set |
| `COVEN_DAEMON_SOCKET` | daemon env | Daemon UDS path; used by the session notifier |
| `COVEN_CODE_*` | user | Engine-owned namespace; coven never overrides it |

---

## 4. Implementing the merge in the Cave — comprehensive

The Cave (coven-cave) is the desktop control room. It already builds on the
`coven` CLI and daemon, which is exactly the seam the merge formalizes — so
the Cave's work is mostly *simplification*: collapse its two-tool story into
the one-CLI story and adopt the new engine/ledger surfaces as they land.

### 4.1 How the Cave integrates today (verified against coven-cave main)

| Surface | Cave code | Behavior today |
|---|---|---|
| Binary resolution | `src/lib/coven-bin.ts` | Probes well-known install dirs for `coven` (nvm/fnm, npm/pnpm/bun globals, homebrew, `~/.cargo/bin` last), falls back to login-shell PATH, caches; `covenSpawnEnv()` scrubs forbidden keys |
| Daemon lifecycle | `src/lib/daemon-start.ts` | Health-checks `/api/v1/health`, else spawns `coven daemon start` |
| Session launch | `src/app/api/sessions/route.ts` → daemon `POST /api/v1/sessions` | Sends `harness` id; allow-list (`src/lib/server/session-security.ts`) is derived from `COMPATIBILITY_ADAPTERS` |
| Runtime catalog | `src/lib/harness-adapters.ts` + `src/lib/runtime-registry.gen.ts` | Curated seed (codex, claude, copilot, hermes, openclaw) merged with the generated coven-runtimes registry, which includes `coven-code`; regenerate with `pnpm sync:runtimes` |
| Streaming | `src/lib/copilot-stream.ts` | Runtimes that declare `stream_args` (coven-code included) use the long-lived stdin-frame stream-json loop |
| Tool install/onboarding | `src/lib/opencoven-tools-status.ts`, `opencoven-tools-install.ts`, `onboarding-gate.ts`, install queue | Treats **two** npm packages as peers: `@opencoven/cli` and `@opencoven/coven-code`; detects the engine by finding `coven-code` on PATH; `npm i -g` both |
| Version probes | `src/lib/harness-version.ts`, `coven-version.ts` | Runs `<binary> --version` for each adapter binary found on PATH |

The structural insight: the Cave never talks to the engine directly for
sessions — it talks to the coven daemon, and the daemon spawns harnesses.
That means most merge phases reach the Cave "for free" through a coven
upgrade, and the Cave-side changes concentrate in **detection, onboarding
copy, and session-list semantics**.

### 4.2 Stage-by-stage implementation plan

Each stage is gated on a release landing; nothing here needs to move early.
Stages are independent, shippable, and ordered by dependency.

**Stage 1 — engine-aware detection and one-package onboarding**
*(gate: coven release containing #346 + #353)*

1. `opencoven-tools-status.ts`: stop equating "engine installed" with
   "`coven-code` on PATH". The managed engine lives at
   `~/.coven/engine/<current>/coven-code` and is deliberately **not** on
   PATH. Detect via `coven engine status --json` (it reports resolved path,
   source, version, and pin state) with the current PATH probe kept as a
   fallback for engine-only installs and older coven builds. `coven engine
   which` is the script-friendly variant.
2. `opencoven-tools-install.ts` + the onboarding install queue: collapse the
   two-package lane (`npm i -g @opencoven/cli @opencoven/coven-code`) into
   `npm i -g @opencoven/cli` followed by `coven engine install`. Keep the
   queue's serialize-behind-the-npm-lane behavior; the engine step is a
   `coven` subprocess, not an npm install. Keep a "direct engine install"
   escape hatch for users who intentionally run PATH engines.
3. `onboarding-gate.ts` (`skip-coven-code` key): the gate's question changes
   from "is the coven-code package installed" to "does `coven` resolve an
   engine". Preserve the skip semantics.
4. Compat/version checks (`harness-version.ts`): for the `coven-code`
   adapter, source the version from `coven engine status --json` (or
   `coven code --version`) so a managed-only install doesn't read as
   "unknown version".
5. Copy sweep: onboarding and settings surfaces should say "Coven engine"
   and stop instructing a separate coven-code npm install (matches the
   Phase 2 install-story change already landed here in #151).

**Stage 2 — sessions on the built-in engine harness**
*(gate: coven release containing #354)*

1. No payload change: the Cave keeps posting
   `POST /api/v1/sessions {harness: "coven-code", …}`. What changes is that
   the daemon now has `coven-code` as a **built-in** harness with
   engine-aware availability and spawn — a managed engine that is not on
   PATH becomes launchable, and the adapter-manifest scaffold under
   `$COVEN_HOME/adapters/coven-code.json` stops being a prerequisite.
2. Registry copy: the coven-runtimes entry's install hint
   (`npm install -g @opencoven/coven-code …`) should be updated upstream to
   the engine story (`coven engine install`), then regenerated into the Cave
   via `pnpm sync:runtimes`. Registry versions are immutable — this is a
   manifest version bump in
   [`spec/runtime-manifest/coven-code.json`](../spec/runtime-manifest/coven-code.json),
   submitted through coven-runtimes acceptance.
3. Expectation change worth surfacing in the UI: bare-prompt casts made
   through coven now default to the engine harness; the Cave's own launches
   are explicit about `harness`, so nothing changes for it — but "new
   session" affordances may want to present Coven Code first for parity.

**Stage 3 — external (TUI-owned) sessions in the session list**
*(gate: coven release containing #355; richer once coven-code #152 is in the
pinned engine and users opt into `daemonLedger`)*

External sessions are registered, not launched: the daemon has **no PTY**
for them. The Cave must treat them as a distinct kind:

1. Session list/status (`session-status.ts`, `session-list-merge.ts`,
   rail components): render external records (surface the `external` flag
   from the daemon API) with an "interactive TUI session" affordance instead
   of the usual live-attach affordance.
2. Kill affordances: the daemon answers `422 external_session_not_killable`
   — hide or disable kill for external sessions rather than surfacing a
   failed action.
3. Attach/inspect: attach-equivalent views should show the recorded ledger
   with the same "shows the ledger, not the live terminal" framing the CLI
   uses; do not attempt stdin forwarding.
4. Sweeps and health logic (e.g. `stuck-created-sweep.ts`): exclude external
   sessions — the daemon's own orphan recovery already skips them
   (`external = 0` filter) and the Cave must not re-reap what the daemon
   deliberately leaves alone. Completion arrives via the engine's
   `/complete` call (exit code 0 → completed, nonzero → failed).
5. Optional Cave setting: a toggle that writes `daemonLedger: true` into the
   engine settings, so users can opt into ledgered TUI sessions from the
   Cave UI. Unix only; hide on Windows.

**Stage 4 — state unification prep** *(gate: Phase 4 engine release)*

The engine home moves to `~/.coven/code/` with a one-cycle symlink at
`~/.coven-code`. Any Cave code that touches engine paths directly (transcript
readers, `coven-memory-path.ts`-style helpers) must resolve through the
engine's reported paths rather than hardcoding `~/.coven-code`. The Cave
already has a home-migration pattern (`server/cave-home-migration.ts`) to
model the detection on. Audit for hardcoded `.coven-code` strings before the
Phase 4 release, not after.

**Stage 5 — naming sweep** *(gate: Phase 5)*

The `coven-cave` bin alias shipped by the engine npm package gets a sunset
notice and is later removed — the Cave must not spawn or document it
(desktop app code paths use `coven`/`coven-code` names already; this is a
copy-and-docs check). "coven-code" disappears from user-facing Cave copy in
favor of "the Coven engine", matching both repos.

### 4.3 Validation recipe (Cave side)

Per stage, in coven-cave:

```bash
pnpm typecheck
pnpm test:app          # unit/component lanes
pnpm test:api          # server/API lanes
```

Manual smokes that map to the merge's exit criteria:

- Stage 1: fresh machine profile → onboarding installs one npm package,
  engine present via `coven engine status`; tools panel shows both ready.
- Stage 2: create a session with harness `coven-code` from the Cave with the
  engine **only** managed (removed from PATH) — it must launch and stream.
- Stage 3: start an interactive `coven-code` TUI with `daemonLedger` on →
  session appears in the Cave list as external; kill affordance absent;
  closing the TUI flips it to completed.
- Stage 4: populated `~/.coven-code` → upgrade → Cave still finds
  transcripts/settings through the moved home.

### 4.4 Design rules the Cave must not break

- **Process boundary:** the Cave talks to `coven` (CLI + daemon HTTP). It
  must never link engine code or bypass coven to manage engine internals —
  the escape hatch for engine-specific calls is `coven code …`.
- **No credential bridging:** the Cave never copies or injects tokens
  between CLIs; auth flows go through `coven auth …` passthrough or the
  engine's own login. Presentation may unify; tokens may not.
- **Additive adoption:** every stage must degrade gracefully against an
  older coven (missing `engine` subcommand → fall back to PATH detection;
  missing `/sessions/external` semantics → no external rows to render).
  The daemon and CLI are updated by users on their own schedule.

---

## 5. Compatibility and rollback guarantees

- Every phase is independently revertible; the legacy resolution order
  (PATH + `~/.coven-code/bin`) stays in the resolver permanently.
- The engine's stream-json output stays Claude-Code-compatible — coven and
  the Cave extend their parsers rather than forking the format.
- Supply chain: engine installs are SHA-256-pinned via coven's `engine.lock`
  and fail closed on mismatch; the pin is regenerated per engine release by
  an automated bump PR gated on the cross-repo contract tests.
- Contract drift is caught in both directions: coven CI tests the pinned
  engine; this repo's CI runs coven's contract tests against every engine
  change (the `coven-contract` job).

## Related reading

- [Coven runtime contract](coven-runtimes) — how the daemon drives
  `coven-code` as a registered runtime today
- [Headless contract](headless-contract) — `--print` / stream-json surfaces
- [Coven Familiars](familiars) — daemon familiars as agent personas
- [Installation](installation) — current install paths for this repo's CLI
