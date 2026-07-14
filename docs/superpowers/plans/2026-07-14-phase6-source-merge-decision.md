# Phase 6 — Go/No-Go: Merge the engine source into the `coven` binary?

> Decision record (ADR) closing the coven CLI unification. Phases 0–5 delivered a single user-facing `coven` CLI driving `coven-code` as a managed engine **process**. Phase 6 is the one remaining question, and it is a **decision, not code**: should we go further and fold the engine's *source* into the `coven` binary itself?

**Status:** Decided — **NO-GO.** Keep the process boundary.
**Date:** 2026-07-14
**Owners:** OpenCoven maintainers
**Confidence:** High.

---

## The question

Two shapes for "one Coven":

- **A — Process boundary (what we built).** `coven` (MIT) is the only CLI users install and type. It resolves, installs, pins, version-checks, and drives `coven-code` (GPL-3.0) as a separate, checksum-verified engine **binary**, and registers its sessions in one daemon ledger under one `~/.coven` state root. Two works, two licenses, one experience.
- **B — Single linked binary.** Compile the engine's `claurst-*` crates directly into the `coven` binary. One artifact, one build, no subprocess.

Phase 6 asks whether to pursue **B**. The answer is no.

## Decision

**Do not merge the source trees. Keep A.** Revisit only if *all three* flip-criteria below hold — today none do.

## Why (in priority order)

### 1. License — B forces the whole product to GPL-3.0, and OpenCoven can't unilaterally relicense
`coven` is MIT. `coven-code` is GPL-3.0, **inherited from upstream Claurst**, whose copyright OpenCoven does not wholly own. Statically linking the `claurst-*` crates into the `coven` binary makes the combined work a GPL-3.0 derivative — the MIT binary would have to become GPL-3.0. There is no "link it but stay MIT" option; that's the core GPL term. Relicensing `coven-code` out of GPL would require the consent of the upstream Claurst copyright holders. So B is not merely an engineering task — it is a licensing commitment (ship `coven` as GPL-3.0) that isn't ours to make cheaply.

The process boundary (A) is the standard, well-understood way two differently-licensed works cooperate: `ripgrep` inside VS Code, `containerd` under Docker, any CLI that shells out to another CLI. Invoking a separate binary at arm's length keeps each work under its own license.

### 2. Upstream merges — B throws away the reason the crate names are `claurst-*`
`coven-code` deliberately preserves upstream crate names (`claurst-*`) and the `coven-code` binary name specifically so `git merge upstream/main` from Claurst stays low-friction (see `COVEN.md`). That is a live, ongoing value stream: bug fixes and features flow in from upstream. Absorbing the source into `coven/crates/` and rebranding would sever that channel or make every future merge a manual reconciliation. B trades a recurring benefit for a one-time cosmetic win users can't even see.

### 3. Engineering cost — B is a large, risky migration for zero user-visible gain
- **Dependency reconciliation:** `coven-cli` is deliberately **synchronous** (no tokio), on ratatui 0.30 / rusqlite 0.40; the engine is tokio 1.44 / ratatui 0.29 / rusqlite 0.31 across ~148k LoC. B means introducing tokio into the sync CLI, reconciling two TUI and two SQLite stacks, and absorbing ~148k LoC into a ~35k LoC workspace.
- **Pipeline merge:** two CI matrices, two release flows, two npm distribution stories collapse into one — the part that's *already done* (Phase 2's pinning + cross-repo contract CI) is what makes A robust; B would redo it.
- **Risk:** all of the above for an artifact users experience identically to what they have now.

### 4. The boundary costs nothing — unification is already real
This is the decisive point. Everything "one Coven" was supposed to deliver **already ships without crossing the license line**:
- One install, one command (`coven`), auto-installed managed engine.
- One state root (`~/.coven`), migrated in place, with a compat symlink.
- One session ledger — interactive engine sessions register themselves and show up in `coven sessions`; cross-session search spans them.
- One doctor, one Credentials view, one `--version` that surfaces the whole stack.
- One brand — no user-facing "coven-code" left.

A single linked binary would make the *implementation* marginally tidier (no subprocess spawn) at the cost of the license, the upstream channel, and a big migration. The user sees **nothing** different. That's a bad trade.

## Flip-criteria (revisit B only if ALL hold)

1. **Upstream is done mattering.** Claurst merges have stopped delivering value for ≥2 quarters (measure: upstream commits actually merged into `coven-code` in the trailing two quarters ≈ 0).
2. **GPL is acceptable for `coven`.** OpenCoven has explicitly decided it will ship the unified binary as GPL-3.0 — *or* has obtained the upstream Claurst copyright holders' consent to relicense the engine. (Absent one of these, B is legally impossible, full stop.)
3. **The migration is funded.** Someone owns the mechanical unification end to end: tokio-into-sync-CLI, ratatui 0.30↔0.29, rusqlite 0.40↔0.31, CI/release/npm consolidation, ~148k LoC absorbed — with a test bar equal to today's.

If any one is false, the answer stays NO-GO. Today all three are false.

## If B is ever taken (migration sketch — not a recommendation)

- `git subtree add` the engine into `coven/crates/` (history-preserving), not a flat copy.
- Replace the delegation seam (`engine::resolve()` → exec) with in-process dispatch; the daemon notifier, `/sessions/external` endpoints, contract tests, and `~/.coven` state model all **carry over unchanged** — which is exactly why A loses nothing while it waits.
- Set the workspace license to GPL-3.0; update `PROVENANCE.md`/`ATTRIBUTION.md`.
- npm/dist consolidation is already done (Phase 2), so distribution is the easy part.

The process-boundary design was built to keep this option open at zero ongoing cost. That is the point: **choosing A now does not foreclose B later.**

## What's on the record

The "singular, seamless `coven` CLI" goal is **met** by the process boundary. Merging the source would convert the whole product to GPL, sever upstream, and cost a large migration — for no user-visible benefit. Recommendation: **NO-GO**, indefinitely, with a concrete, measurable trigger to reopen if the landscape changes.
