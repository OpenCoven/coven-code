# Coven Code

**Coven Code** is an open-source agentic coding TUI built in Rust. It is maintained by [OpenCoven](https://opencoven.ai) as a GPL-3.0 fork of [Claurst](https://github.com/Kuberwastaken/claurst) by Kuber Mehta.

> **Attribution:** Coven Code is derived from Claurst v0.0.36 under the GNU General Public License v3.0. The full license is in [`LICENSE.md`](LICENSE.md) and upstream attribution is in [`ATTRIBUTION.md`](ATTRIBUTION.md).

---

## What it is

Terminal coding agent with a rich ratatui TUI: chat forking, memory consolidation, diff viewer, plugin system, MCP support, session branching, and a mascot companion. No telemetry, no tracking.

**Supported providers:** Anthropic (Claude) and Codex (OpenAI Codex via ChatGPT/Codex login).

---

## Status

> **Beta (v0.0.36).** Core agent, provider routing, and TUI are stable for daily use. Experimental features are flagged below.

Recent highlights:
- **/share** — share sessions via unlisted GitHub Gists `[EXPERIMENTAL]`
- **/goal** — `/goal <objective>` keeps the agent working across multiple turns `[EXPERIMENTAL]`
- **/coven** — drive the local [Coven daemon](https://github.com/OpenCoven/coven) (sessions, harness runs, rituals) without leaving the TUI. `/coven` is the unified replacement for `coven-cli`'s interactive menu and the legacy `coven-tui` slash shell; when `coven-code` is on `PATH`, `coven` and `coven tui` exec into it automatically (opt out with `COVEN_LEGACY_TUI=1`). Run `/coven help` for the subcommand list.

---

## Requirements

| Requirement        | Notes                                           |
| ------------------ | ----------------------------------------------- |
| **Node.js 18+**    | Required for the npm package                    |
| **Credentials**    | An Anthropic API key (or OAuth login), or a Codex login |

---

## Getting Started

### Install

```bash
npm install -g @opencoven/coven
```

Then open a new terminal and run `coven` or `coven tui`. The lower-level `coven-code` binary and `coven-cave` alias are also installed for compatibility.

### Upgrade

```bash
npm install -g @opencoven/coven@latest
```

---

## Manual install

Pre-built archives are on [**GitHub Releases**](https://github.com/OpenCoven/coven-code/releases):

| Platform | Archive |
|---|---|
| **Windows** x86_64 | `coven-code-windows-x86_64.zip` |
| **Linux** x86_64 | `coven-code-linux-x86_64.tar.gz` |
| **Linux** aarch64 | `coven-code-linux-aarch64.tar.gz` |
| **macOS** Intel | `coven-code-macos-x86_64.tar.gz` |
| **macOS** Apple Silicon | `coven-code-macos-aarch64.tar.gz` |

Each archive contains a single `coven-code` (or `coven-code.exe`) binary.

---

## Build from source

```bash
git clone https://github.com/OpenCoven/coven-code.git
cd coven-code/src-rust
cargo build --release --package claurst   # binary outputs as coven-code; coven-cave is an alias target
```

> Internal Rust crate names (`claurst-core`, `claurst-tui`, etc.) are preserved from upstream for merge-friendliness. The compiled binary is named `coven-code`.

---

## CLI Flags

| Flag | Short | Description |
|---|---|---|
| `--model <MODEL>` | `-m` | Model to use (e.g. `claude-sonnet-4-6`, `claude-opus-4-6`) |
| `--provider <PROVIDER>` | | LLM provider: `anthropic` or `codex` (env: `COVEN_CODE_PROVIDER`) |
| `--resume [<ID>]` | | Resume a previous session by ID; omit ID to resume the most recent |
| `--print` | `-p` | Print mode: send prompt and exit (non-interactive / headless) |
| `--permission-mode <MODE>` | | `default`, `accept-edits`, `bypass-permissions`, or `plan` |
| `--max-turns <N>` | | Maximum agentic turns before stopping (default: 10) |
| `--system-prompt <PROMPT>` | `-s` | Override the system prompt |
| `--append-system-prompt <PROMPT>` | | Append text to the system prompt |
| `--no-claude-md` | | Disable AGENTS.md memory file injection |
| `--output-format <FMT>` | | Output format: `text` (default), `json`, or `stream-json` |
| `--api-key <KEY>` | | API key for the active provider (overrides env vars) |
| `--api-base <URL>` | | Override the provider API base URL (env: `COVEN_CODE_API_BASE`) |
| `--fallback-model <MODEL>` | | Fallback model when the primary is overloaded or unavailable |
| `--max-budget-usd <USD>` | | Abort the query loop when spend exceeds this amount |
| `--agent <AGENT>` | `-A` | Named agent profile to use (e.g. `build`, `plan`, `explore`) |
| `--verbose` | `-v` | Enable verbose logging |
| `--version` | `-V` | Print version |

---

## Configuration

Coven Code is a **local CLI tool** — it runs entirely on your machine. You bring your own Anthropic API key or Codex login. Nothing is sent to OpenCoven servers; all requests go directly from your terminal to the provider.

Settings live in `~/.coven-code/settings.json`. Set your Anthropic key in the environment or via `/config`:

```bash
export ANTHROPIC_API_KEY=<your-key>
coven-code
```

Or log in via OAuth after configuring a Coven Code OAuth client:

```bash
export COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID=<registered-client-id>
coven-code auth login
```

Or sign in to Codex with your ChatGPT/Codex subscription:

```bash
coven-code codex login
coven-code --provider codex
```

Environment variable prefix: `COVEN_CODE_*` (e.g. `COVEN_CODE_SKIP_PROMPT_HISTORY=1`).

---

## Providers

See [docs/providers.md](docs/providers.md) for the full provider reference.

Quick example:

```bash
coven-code --provider anthropic "refactor this module"
coven-code --provider codex "explain this function"
coven-code --model claude-opus-4-6 "write tests"
```

---

## Extensibility seams

Coven Code is designed to grow into the OpenCoven ecosystem. Key seams for future integration:

| Surface | Location | Notes |
|---|---|---|
| Provider adapters | `src-rust/crates/api/src/providers/` | Add new `LlmProvider` impls here |
| Plugin system | `src-rust/crates/plugins/` | Runtime plugin loading |
| ACP server | `src-rust/crates/acp/` | JSON-RPC 2.0 over stdio — OpenCoven adapter entry point |
| Command registry | `src-rust/crates/commands/` | Add `/slash` commands |
| TUI theme | `src-rust/crates/tui/src/theme_colors.rs` | OpenCoven violet/pink palette is the default; deuteranopia variant ships too. Diff viewer routes through `DiffPalette` so colour-blind users see orange/blue diffs. |
| Memory / session | `src-rust/crates/core/src/memdir.rs`, `session_storage.rs` | Hook for Coven session/memory integration |
| Companion mascot | `src-rust/crates/tui/src/mascot.rs` | ASCII mascot renderer; seven archetypes ship (`kitty`, `nova`, `cody`, `charm`, `sage`, `astra`, `echo`). F2 opens the live switcher. |
| Coven daemon client | `src-rust/crates/core/src/coven_daemon.rs` | Typed `DaemonClient` over `~/.coven/coven.sock` speaking `coven.daemon.v1`. Powers `/coven` + the welcome status block. |

---

## OpenCoven fork notes

- Internal crate names (`claurst-core`, `claurst-tui`, etc.) are **intentionally preserved** from upstream to keep `git merge upstream/main` low-friction.
- User-visible surfaces (binary, env vars, data dirs, ACP registry, docs) are fully rebranded to `coven-code` / `COVEN_CODE_`.
- To sync upstream improvements: `git fetch upstream && git merge upstream/main`.
- License: GPL-3.0. See [`LICENSE.md`](LICENSE.md) and [`ATTRIBUTION.md`](ATTRIBUTION.md).

---

## Links

- [OpenCoven](https://opencoven.ai)
- [GitHub](https://github.com/OpenCoven/coven-code)
- [Issues](https://github.com/OpenCoven/coven-code/issues)
- [Upstream (Claurst)](https://github.com/Kuberwastaken/claurst) — original project by Kuber Mehta
