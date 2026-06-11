# Coven Code — Rust Codebase

## Overview

The active Coven Code implementation lives under `src-rust/`. It is a Rust
workspace for the `coven-code` binary, terminal UI, provider clients, tool
runtime, query loop, plugin system, MCP support, bridge support, and ACP server.

This document is the current architecture reference for the Rust codebase. The
other files in `spec/` mostly describe the upstream TypeScript Claude Code
system that Coven Code was originally modeled on; use this file, `docs/`, and
the Rust source for current Coven Code paths and names.

Important current identifiers:

| Item | Current value |
|---|---|
| Workspace path | `src-rust/` |
| Workspace version | see `src-rust/Cargo.toml` `[workspace.package]` (not stamped by `scripts/bump-version.py`) |
| Binary package | `claurst` |
| Installed binary | `coven-code` |
| Runtime config dir | `~/.coven-code/` |
| Project instruction file | `AGENTS.md` |
| Workspace crate prefix | `claurst-*` |

---

## Repository Layout

```text
src-rust/
├── Cargo.toml              # Cargo workspace root
└── crates/
    ├── acp/                # Agent Client Protocol server
    ├── api/                # Provider clients, model registry, streaming
    ├── bridge/             # Bridge integration surface
    ├── buddy/              # Buddy/familiar support crate
    ├── cli/                # `coven-code` binary entry point
    ├── commands/           # Slash/named command implementations
    ├── core/               # Shared config, types, auth, history, constants
    ├── mcp/                # Model Context Protocol client support
    ├── plugins/            # Plugin manifests, loader, registry, hooks
    ├── query/              # Agentic loop, compacting, tasks, sessions
    ├── tools/              # Built-in model tools and tool dispatcher
    └── tui/                # Ratatui/crossterm terminal UI
```

The workspace uses Cargo resolver `2`, Rust edition `2021`, and a single
workspace package version stamped across the crates.

---

## Workspace Members

| Member path | Package name | Primary role |
|---|---|---|
| `crates/acp` | `claurst-acp` | JSON-RPC 2.0 ACP server over stdio, session/runtime plumbing |
| `crates/api` | `claurst-api` | LLM provider abstraction, provider registry, SSE/streaming, model registry |
| `crates/bridge` | `claurst-bridge` | Bridge integration between Coven Code sessions and external surfaces |
| `crates/buddy` | `claurst-buddy` | Buddy/familiar support primitives |
| `crates/cli` | `claurst` | Binary package; emits the `coven-code` executable |
| `crates/commands` | `claurst-commands` | Slash commands, named commands, stats commands |
| `crates/core` | `claurst-core` | Shared domain types, config, constants, auth store, context, history |
| `crates/mcp` | `claurst-mcp` | MCP server connections, resources, prompts, auth |
| `crates/plugins` | `claurst-plugins` | Plugin manifests, marketplace metadata, hooks, plugin registry |
| `crates/query` | `claurst-query` | Query execution loop, agent tool, compaction, cron, sessions, goals |
| `crates/tools` | `claurst-tools` | Built-in tool implementations and permission-aware dispatch |
| `crates/tui` | `claurst-tui` | Interactive terminal UI, dialogs, overlays, message rendering, key handling |

Source file counts change often, but the current workspace has roughly 238 Rust
source files across these 12 crates.

---

## Dependency Shape

The dependency graph is intentionally layered around `claurst-core`.

```text
cli
├── tui
├── query
│   ├── api
│   ├── tools
│   └── plugins
├── commands
├── acp
├── bridge
└── mcp

tools ──> api, mcp, core
api   ──> core
tui   ──> api, tools, query, mcp, core
```

`claurst-core` owns shared types and constants. Higher-level crates should not
redefine provider IDs, tool names, permission modes, or persisted config shapes
when a core type already exists.

---

## Workspace Dependencies

The workspace root centralizes common dependencies:

| Area | Dependencies |
|---|---|
| Async/runtime | `tokio`, `tokio-stream`, `futures`, `async-trait`, `async-stream` |
| HTTP/streaming | `reqwest`, `tokio-tungstenite`, `tower-http`, `sse-stream` |
| Serialization/config | `serde`, `serde_json`, `toml`, `schemars` |
| CLI/TUI | `clap`, `ratatui`, `crossterm` |
| Persistence/utilities | `rusqlite`, `dirs`, `uuid`, `chrono`, `dashmap`, `parking_lot` |
| Text/files | `regex`, `glob`, `walkdir`, `similar`, `syntect`, `unicode-width` |
| Process/system | `nix`, `portable-pty`, `which`, `open` |
| Media/terminal images | `image`, `icy_sixel`, `qrcode` |

The `claurst` CLI package has the default feature `voice`, which enables voice
support in `claurst-core` and `claurst-tui`.

---

## CLI Surface

The binary is declared in `src-rust/crates/cli/Cargo.toml`:

```toml
[[bin]]
name = "coven-code"
path = "src/main.rs"
```

The `clap` entry point in `crates/cli/src/main.rs` defines the interactive and
headless surfaces. Key flags include:

| Flag | Purpose |
|---|---|
| `--print`, `-p` | Headless prompt mode: send prompt and exit |
| `--model`, `-m` | Select model |
| `--provider` | Select provider (`COVEN_CODE_PROVIDER`) |
| `--api-base` | Override selected provider base URL (`COVEN_CODE_API_BASE`) |
| `--permission-mode` | Select `default`, `accept-edits`, `bypass-permissions`, or `plan` |
| `--resume` | Resume a session by ID, or most recent when omitted |
| `--continue`, `-c` | Continue the most recent conversation |
| `--max-turns` | Cap agentic turn count |
| `--system-prompt`, `--system-prompt-file` | Override the system prompt |
| `--append-system-prompt` | Append extra system instructions |
| `--no-claude-md` | Disable instruction-file loading; retained as a compatibility flag |
| `--cwd` | Run from a specific working directory |
| `--dangerously-skip-permissions`, `--yolo` | Bypass permission checks |
| `--mcp-config` | Inline MCP server config JSON |
| `--no-auto-compact` | Disable automatic compaction |
| `--auto-commits` | Enable shadow-git snapshots |
| `--add-dir` | Grant access to additional directories |
| `--input-format stream-json` | Read newline-delimited JSON messages in print mode |
| `--output-format stream-json` | Emit stream JSON in print mode |
| `--agent`, `-A` | Select a named agent/familiar mode |
| `--context`, `--output` | Headless GitHub App session brief/result envelope |

The public command name is `coven-code`, even though several internal package
names still use the historical `claurst` prefix.

---

## Core Crate

`claurst-core` is the shared foundation. It includes:

- `Message`, `ContentBlock`, `Role`, usage/cost, and tool definition types.
- `Config`, `Settings`, `PermissionMode`, `OutputFormat`, MCP config, hooks, and
  persisted UI/config settings.
- Provider and model identifiers.
- Auth store helpers for provider keys and OAuth-style integrations.
- Context and instruction-file discovery.
- Session/conversation history and cost tracking.
- Shared constants such as default models, tool names, config directory names,
  and beta headers.

Current constants include:

| Constant | Current value |
|---|---|
| `DEFAULT_MODEL` | `claude-opus-4-8` |
| `SONNET_MODEL` | `claude-sonnet-4-6` |
| `HAIKU_MODEL` | `claude-haiku-4-5-20251001` |
| `OPUS_MODEL` | `claude-opus-4-8` |
| `FABLE_MODEL` | `claude-fable-5` |
| `CONFIG_DIR_NAME` | `.coven-code` |
| `CLAUDE_MD_FILENAME` | `AGENTS.md` |
| `HISTORY_FILENAME` | `conversations` |

The `--no-claude-md` CLI flag is still present for compatibility, but new
documentation should treat `AGENTS.md` as the current instruction filename.

---

## API and Provider Runtime

`claurst-api` defines the provider abstraction and the model registry.

Provider modules currently include:

- `anthropic`
- `azure`
- `bedrock`
- `codex`
- `cohere`
- `copilot`
- `free`
- `google`
- `minimax`
- `openai`
- `openai_compat`
- `openai_compat_providers`

The registry chooses runtime providers from explicit configuration, environment
variables, the auth store, and provider-specific constructors. It supports
Anthropic, OpenAI, Google, GitHub Copilot, Codex/OpenAI Codex, Cohere, Minimax,
Azure, Bedrock, the free provider chain, and OpenAI-compatible providers such as
OpenRouter.

`model_registry.rs` owns provider/model metadata and resolution. Bare model
names are resolved through heuristics for well-known families (`claude-*`,
`gpt-*`, `gemini*`, DeepSeek, Mistral, xAI, Cohere, Perplexity, Z.ai), while
`provider/model` strings resolve directly.

---

## Tools Runtime

`claurst-tools` owns built-in model tools, tool schemas, permissions, execution,
and lookup. `claurst-query` owns the agent-style `Agent` tool because it needs
query-loop orchestration.

Current built-in tool names include:

```text
Bash
Read
Edit
Write
BatchEdit
ApplyPatch
Glob
Grep
WebFetch
WebSearch
NotebookEdit
TaskCreate
TaskGet
TaskUpdate
TaskList
TaskStop
TaskOutput
TodoWrite
AskUserQuestion
EnterPlanMode
ExitPlanMode
PowerShell
Sleep
CronCreate
CronDelete
CronList
EnterWorktree
ExitWorktree
ListMcpResources
ReadMcpResource
ToolSearch
Brief
Config
SendMessage
Skill
LSP
REPL
TeamCreate
TeamDelete
StructuredOutput
mcp__auth
RemoteTrigger
monitor
GoalComplete
ComputerUse (feature-gated)
```

Tool definitions are generated from each tool's `Tool` implementation and sent
to providers through the API/query layers. Permission checks and tool context
flow through `ToolContext` and the configured permission mode/rules.

---

## Query Loop and Sessions

`claurst-query` coordinates the agentic loop:

- Prompt execution and streaming event handling.
- Tool calls and tool-result feedback.
- Compaction and context analysis.
- Session memory and conversation state.
- Cron scheduling and away summaries.
- Goal loop support.
- Managed agents and coordinator/orchestrator paths.
- Skill prefetch and command queues.

This crate is where the high-level "assistant turn" behavior lives. UI and CLI
surfaces drive it rather than duplicating the loop.

---

## Terminal UI

`claurst-tui` is the interactive terminal experience. It uses `ratatui` and
`crossterm`, and contains:

- Main `App` state and key handling.
- Message rendering, markdown rendering, thinking/tool blocks, and snapshots.
- Prompt input, slash suggestions, history search, and file references.
- Permission dialogs and tool approval flows.
- Model picker, settings, theme screen, stats, onboarding, help, and overlays.
- Familiar/mascot card rendering and familiar switcher.
- MCP view, plugin views, agents view, task overlays, and diff viewer.
- Voice recording/transcription UI when the `voice` feature is enabled.

The TUI should treat `claurst-core` constants and `claurst-tools` tool names as
canonical rather than hardcoding divergent names.

---

## Commands

`claurst-commands` implements visible slash commands and named command helpers.
The user-facing command reference lives in `docs/commands.md`; implementation
lives primarily in:

- `crates/commands/src/lib.rs`
- `crates/commands/src/named_commands.rs`
- `crates/commands/src/stats.rs`

Commands cover model/provider selection, configuration, permissions, hooks,
MCP, plugins, skills, agents, goals, session/history operations, stats, and
developer workflows. Some CLI-only named commands such as stats and ultraplan
are exposed through the binary rather than only the interactive slash-command
surface.

---

## MCP, Plugins, Bridge, and ACP

`claurst-mcp` handles Model Context Protocol client behavior: server
connections, resources, prompts, and auth flows.

`claurst-plugins` loads plugins from `~/.coven-code/plugins/`. Plugins can
provide command markdown, agents, skills, hooks, MCP server definitions, LSP
server definitions, and marketplace metadata.

`claurst-bridge` contains bridge integration used by external surfaces that
need to connect into Coven Code sessions.

`claurst-acp` implements the Agent Client Protocol server over stdio using
JSON-RPC 2.0. It wires ACP prompts, sessions, permissions, and runtime handling
onto the same core/query/tool pieces rather than creating a separate agent.

---

## Persistence and Local Files

Current Coven Code runtime state is rooted in `~/.coven-code/`.

Common persisted surfaces include:

| Surface | Location |
|---|---|
| Settings | `~/.coven-code/settings.json` |
| Conversations | `~/.coven-code/conversations/` |
| Plugins | `~/.coven-code/plugins/` |
| User agents | `~/.coven-code/agents/` |
| Keybindings | `~/.coven-code/keybindings.json` |
| UI settings | `~/.coven-code/ui-settings.json` |

Project instructions use `AGENTS.md`; new Coven Code documentation should use
that name for the current instruction-file surface.

---

## Release and Versioning

Coven Code uses a single workspace version. The release/bump flow stamps the
Cargo workspace, internal crates, npm package metadata, docs/badges, and related
templates together. Do not manually edit generated lockfile/version surfaces
when the release script owns them.

The root `AGENTS.md` release section is the current operational reference for
version stamping and release workflow constraints.

---

## Maintenance Notes

- Prefer `src-rust/Cargo.toml` and crate `Cargo.toml` files as the source of
  truth for workspace membership.
- Prefer `docs/commands.md`, `docs/tools.md`, `docs/providers.md`,
  `docs/plugins.md`, and `docs/mcp.md` for current user-facing behavior.
- Prefer `claurst-core` constants for tool names, model defaults, config paths,
  and instruction filenames.
- Keep this file focused on the Rust workspace. Upstream TypeScript behavior
  belongs in the earlier `spec/` files unless it has been verified against
  current Coven Code code.
