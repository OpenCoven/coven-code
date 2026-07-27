# OpenCoven Integration Guide

This document describes the extensibility seams in `coven-code` for OpenCoven-specific work.  
It is a living document — update it as new integration surfaces are added.

---

## Upstream sync strategy

Internal Rust crate names (`claurst-core`, `claurst-tui`, `claurst-acp`, etc.) are **intentionally preserved**
from upstream [Claurst](https://github.com/Kuberwastaken/claurst) to keep `git merge upstream/main` low-friction.

```bash
git fetch upstream
git merge upstream/main   # resolve conflicts in user-facing surfaces only
```

Only user-visible surfaces (binary name, env vars, data dirs, ACP registry, docs, README, npm package)
are rebranded. This boundary is explicit and documented below.

---

## Rebranded surfaces (safe to update without upstream conflict)

| Surface | File(s) | Current value |
|---|---|---|
| Binary name | `src-rust/crates/cli/src/main.rs`, `src-rust/crates/cli/src/bin/coven-cave.rs` | `coven-code`; `coven-cave` alias |
| npm package | `npm/package.json` | `@opencoven/coven-code` |
| Data/cache dirs | `src-rust/crates/core/src/snapshot/`, `skill_discovery.rs`, `update_check.rs`, `app.rs` | `coven-code/` |
| Engine home (standalone) | `src-rust/crates/core/src/lib.rs` (`config_home`) | `~/.coven-code/` |
| Engine home (under coven CLI) | `src-rust/crates/core/src/lib.rs` (`config_home`) | `~/.coven/code/` — migrated in-place from `~/.coven-code/` on first launch; legacy path is symlinked to new location |
| Shared config layer | `src-rust/crates/core/src/lib.rs` (`SharedSettings`, `load_hierarchical`) | `~/.coven/settings.json` — cross-tool defaults layered UNDER the engine and project settings |
| Env var prefix | throughout `src-rust/` | `COVEN_CODE_*` |
| User-Agent | `src-rust/crates/tools/src/web_search.rs`, `update_check.rs` | `CovenCode/x.y` |
| System prompt identity | `src-rust/crates/core/src/system_prompt.rs` | "You are Coven Code…" |
| ACP registry template | `src-rust/crates/acp/registry-template/agent.json` | `coven-code` |
| Install scripts | `install.sh`, `install.ps1`, `npm/install.js` | `OpenCoven/coven-code` |

## Intentionally preserved upstream names (internal crate identifiers)

These are **not** user-visible and are kept for merge-friendliness:

- Crate names: `claurst-core`, `claurst-tui`, `claurst-api`, `claurst-tools`, `claurst-query`,
  `claurst-mcp`, `claurst-bridge`, `claurst-buddy`, `claurst-plugins`, `claurst-acp`,
  `claurst-commands`
- Cargo workspace `[workspace]` resolver and member paths
- Internal Rust module paths and `use` statements referencing `claurst_*`

---

## Extensibility seams

### 1. Provider adapters — `src-rust/crates/api/src/providers/`

Every provider implements `LlmProvider`. To add an OpenCoven-specific or private provider:
1. Create `my_provider.rs` implementing `LlmProvider`.
2. Register it in `providers/mod.rs`.
3. Add routing in `src-rust/crates/api/src/registry.rs` (`provider_from_key` /
   `provider_from_config` / `runtime_provider_for`) and, if it needs a stable
   id, a constant in `src-rust/crates/core/src/provider_id.rs`. (There is no
   provider enum in `core/settings`; the only `Provider` enum is the
   two-variant client selector in `crates/api/src/lib.rs`.)

### 2. Plugin system — `src-rust/crates/plugins/`

Runtime plugin loading. Plugins can add tools, slash commands, and UI panels.  
Entry point: `PluginRuntime` in `crates/plugins/src/lib.rs`.

### 3. ACP server — `src-rust/crates/acp/`

JSON-RPC 2.0 over stdio (`coven-code acp`). This is the recommended Coven orchestration entry point.  
Extend `AcpServer` with OpenCoven-specific RPC methods here.  
See `registry-template/agent.json` for how Coven registers this agent.

### 4. Command/slash registry — `src-rust/crates/commands/`

Add new `/slash` commands by implementing the `Command` trait and registering in `commands/src/lib.rs`.

### 5. TUI theme — `src-rust/crates/tui/src/theme_colors.rs`

Target OpenCoven brand palette:
- Primary: `#8B5CF6` (violet-500)
- Accent: `#EC4899` (pink-500)
- Background/surface: existing dark palette

Replace `default_theme()` return values when brand assets are finalized.

### 6. Companion mascot — `src-rust/crates/tui/src/mascot.rs`

ASCII mascot renderer. Currently "Rune" (renamed from "Rustle" upstream).  
The internal module and pose naming now use companion/mascot terminology; update art in `mascot_lines_for()` and call-sites in `render.rs` / `app.rs`.

### 7. Memory / session hooks — `src-rust/crates/core/src/memdir.rs`, `session_storage.rs`

`memdir.rs`: controls where MEMORY.md / memory files live.  
`session_storage.rs`: session persistence format.  
Hook Coven's memory layer here to sync agent sessions with OpenCoven's session/memory store.

### 8. Tool registry — `src-rust/crates/tools/src/`

All built-in tools live here (file ops, bash, web fetch/search, git, etc.).  
Add Coven-specific tools (e.g. `coven_session_tool.rs`) and register in `tools/src/lib.rs`.

---

## Shared settings (`~/.coven/settings.json`)

When running under the unified `coven` CLI, a small whitelist of
cross-tool defaults can be placed at `~/.coven/settings.json`.
This is the **lowest-precedence** layer: values are used only when the
engine-global settings (`~/.coven/code/settings.json`) and project
settings (`.coven-code/settings.json`) do not override them.

### Load order (lowest → highest)

1. `~/.coven/settings.json` — shared, whitelisted keys only
2. `~/.coven/code/settings.json` — engine-global
3. `<project>/.coven-code/settings.json` — project

When there is no `~/.coven/` directory (standalone mode, no coven CLI),
the shared layer is absent and coven-code behaves exactly as before
(engine + project only).

### Whitelisted keys

| Key | Type | Description |
|---|---|---|
| `model` | string | Default model (e.g. `"claude-opus-4-8"`) |
| `theme` | string | UI theme: `"default"`, `"dark"`, `"light"`, `"deuteranopia"`, or a custom string |
| `permission_mode` | string | Permission posture: `"default"`, `"acceptEdits"`, `"bypassPermissions"`, `"plan"` |

Any other keys in `~/.coven/settings.json` are silently ignored by
coven-code, so future Coven tools can extend the schema without breaking
older engine versions.

### Example

```json
{
  "model": "claude-sonnet-4-6",
  "theme": "dark",
  "permission_mode": "acceptEdits"
}
```

### Implementation

`SharedSettings` struct in `src-rust/crates/core/src/lib.rs` (inside
`pub mod config`).  `SharedSettings::load()` reads the file via
`coven_shared::coven_home()`.  `SharedSettings::apply_to(&mut Settings)`
fills whitelisted fields only when the engine-global still has the
built-in default value (for `Option` fields: `None`; for enum fields:
the `Default` variant).

---

## Engine contract

coven-code is the engine behind the unified `coven` CLI. The exact CLI/env/
stream surfaces coven depends on are specified in
[coven/docs/ENGINE-CONTRACT.md](https://github.com/OpenCoven/coven/blob/main/docs/ENGINE-CONTRACT.md)
(`contract_version: 1`). Do not change flags, output formats, or exit codes
listed there without bumping the contract version and coordinating a coven
`engine.lock` update. The merge overview, user migration steps, and Cave
integration plan live in [docs/unification.md](docs/unification.md).

---

## Release checklist

When cutting a `coven-code` release:
1. Update version in `src-rust/Cargo.toml` `[workspace.package]` and run `scripts/bump-version.py <version>`.
2. Update `src-rust/crates/acp/registry-template/agent.json` archive URLs.
3. Update `npm/package.json` version.
4. Build release binaries for all 5 platforms; name them `coven-code-{platform}-{arch}[.exe]`.
5. Create GitHub release on `OpenCoven/coven-code` with those archives + `install.sh` + `install.ps1`.
6. `npm publish --access public` for `@opencoven/coven-code` from `npm/`.
