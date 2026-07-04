# Headless execution contract (coven-github integration)

`coven-code` runs as the **execution runtime** behind the
[`coven-github`](https://github.com/OpenCoven/coven-github) GitHub App. When the
App's worker picks up a task it spawns:

```
coven-code --headless --context <session-brief.json> --output <result.json>
```

The wire interface between the two repos is **locked** and normative. Its single
source of truth lives in `coven-github`:

- Contract doc: `docs/headless-contract.md` (contract version **`1`**)
- JSON Schemas + golden fixtures: `docs/contracts/`

This document describes how `coven-code` **conforms** to that contract. Where
this file disagrees with the canonical contract, the canonical contract wins.

## Invocation

| Flag | Meaning |
|---|---|
| `--headless` | Disables the ratatui TUI entirely; non-interactive, structured output. Accepted as the canonical headless entry point (also implied by `--context`, `--print`, or a positional prompt). |
| `--context <session-brief.json>` | Reads a tokenless session brief (contract §2). Overrides model + working directory from the brief and forces bypass-permissions. A brief whose major `contract_version` this build does not implement is **rejected**. |
| `--output <result.json>` | Writes the terminal result envelope (contract §3) before exiting `0`/`1`/`3`. |

## Environment

| Variable | Meaning |
|---|---|
| `COVEN_GIT_TOKEN` | GitHub App installation access token. The **only** git credential channel. On a `--context` run the runtime installs a *local, env-backed* git credential helper in the workspace so `git push` authenticates over HTTPS. The token stays in the environment — it is never written to the brief, the result envelope, `.git/config`, or logs. |

## Exit codes (authoritative — contract §4)

| Code | Meaning | `result.json` |
|---|---|---|
| `0` | success / partial (commits made) | present |
| `1` | failure (agent finished with no usable diff on a change task) | present |
| `2` | infra error (model/tool/workspace failure) — **retry-safe** | best-effort |
| `3` | needs input (reserved; wired for M2) | present |

The runtime maps its terminal run outcome to these codes:

- clean finish **with** commits → `success` / exit `0`
- truncation or budget stop **with** commits → `partial` / exit `0`
- clean finish with **no** diff on a change task → `failure` / exit `1`
- reply-only task (`respond_to_mention`) with no diff → `success` / exit `0`
- model / tool / workspace error → `infra_error` / exit `2`

## Conformance tests

`crates/cli/src/headless.rs` carries the runtime's contract types and a
`#[cfg(test)]` conformance suite pinned to **vendored** golden fixtures in
`crates/cli/tests/headless_contract/` (verbatim copies of the coven-github
`docs/contracts/` artifacts). The suite asserts:

- every brief that validates against `session-brief.schema.json` is accepted
  (tokenless, version-defaulting, forward-compatible with unknown fields);
- an unsupported major version is rejected;
- every emitted `result.json` validates against `result.schema.json`;
- the exit-code mapping matches §4;
- `COVEN_GIT_TOKEN` never leaks into `.git/config`.

If a test drifts, the runtime broke the contract **or** the contract changed —
fix one deliberately and bump `contract_version` on both sides. Do not re-bless
fixtures casually.
