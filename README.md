# Coven Code

**Coven Code** is an open-source, multi-provider agentic coding TUI built in Rust. It is maintained by [OpenCoven](https://opencoven.ai) as a GPL-3.0 fork of [Claurst](https://github.com/Kuberwastaken/claurst) by Kuber Mehta.

> **Attribution:** Coven Code is derived from Claurst v0.0.12 under the GNU General Public License v3.0. The full license is in [`LICENSE.md`](LICENSE.md) and upstream attribution is in [`ATTRIBUTION.md`](ATTRIBUTION.md).

---

## What it is

Multi-provider terminal coding agent with a rich ratatui TUI: chat forking, memory consolidation, diff viewer, plugin system, MCP support, session branching, and a mascot companion. No telemetry, no tracking.

**Supported providers:** Anthropic (Claude), OpenAI, Google (Gemini), Groq, Ollama, LM Studio, llama.cpp, OpenRouter, AWS Bedrock, Google Vertex, and any OpenAI-compatible endpoint.

---

## Status

> **Beta (v0.0.12).** Core agent, multi-provider routing, and TUI are stable for daily use. Experimental features are flagged below.

Recent highlights:
- **/share** — share sessions via unlisted GitHub Gists `[EXPERIMENTAL]`
- **Free Mode** — try `/connect` for a free-tier agentic coding experience `[EXPERIMENTAL]`
- **/goal** — `/goal <objective>` keeps the agent working across multiple turns `[EXPERIMENTAL]`

---

## Getting Started

### Quick install (Linux / macOS)

```bash
curl -fsSL https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh | bash
```

### Quick install (Windows PowerShell)

```powershell
irm https://github.com/OpenCoven/coven-code/releases/latest/download/install.ps1 | iex
```

This drops `coven-code` into `~/.coven-code/bin` (or `%USERPROFILE%\.coven-code\bin` on Windows) and adds it to your `PATH`. Open a new terminal and run `coven-code`.

### npm / Bun

```bash
npm install -g @opencoven/coven-code
# or
bun install -g @opencoven/coven-code
```

```bash
npx @opencoven/coven-code
bunx @opencoven/coven-code
```

### Upgrade

```bash
coven-code upgrade
```

Pin a version: `coven-code upgrade --version 0.1.0`.

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
cargo build --release --package claurst   # binary outputs as coven-code
```

> Internal Rust crate names (`claurst-core`, `claurst-tui`, etc.) are preserved from upstream for merge-friendliness. The compiled binary is named `coven-code`.

---

## Configuration

Coven Code is a **local CLI tool** — it runs entirely on your machine. You bring your own API key for whichever provider you use. Nothing is sent to OpenCoven servers; all requests go directly from your terminal to the provider.

Settings live in `~/.coven-code/settings.json`. Set your provider key in the environment or via `/config`:

```bash
export ANTHROPIC_API_KEY=<your-key>
coven-code
```

Or log in via OAuth after configuring a Coven Code OAuth client:

```bash
export COVEN_CODE_ANTHROPIC_OAUTH_CLIENT_ID=<registered-client-id>
coven-code auth login
```

Or use a local model with no key at all:

```bash
coven-code --provider ollama
```

Environment variable prefix: `COVEN_CODE_*` (e.g. `COVEN_CODE_SKIP_PROMPT_HISTORY=1`).

---

## Providers

See [docs/providers.md](docs/providers.md) for the full provider reference.

Quick example:

```bash
coven-code --provider openai "refactor this module"
coven-code --provider ollama "explain this function"
coven-code --provider groq --model llama-3.3-70b-versatile "write tests"
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
| TUI theme | `src-rust/crates/tui/src/theme_colors.rs` | Color palette; OpenCoven violet theme pending |
| Memory / session | `src-rust/crates/core/src/memdir.rs`, `session_storage.rs` | Hook for Coven session/memory integration |
| Companion mascot | `src-rust/crates/tui/src/rustle.rs` | ASCII mascot renderer; rename/skin pending |

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
