# @opencoven/coven-code

[![Version](https://img.shields.io/npm/v/@opencoven/coven-code?style=flat-square)](https://www.npmjs.com/package/@opencoven/coven-code)
[![License](https://img.shields.io/badge/License-GPL--3.0-blue?style=flat-square)](https://github.com/OpenCoven/coven-code/blob/main/LICENSE.md)

> **Recommended install:** `npm install -g @opencoven/cli` — the unified `coven` CLI installs and manages this engine for you.
> This package (`@opencoven/coven-code`) still installs the engine binary directly and continues to work.

**Coven Code** — open-source agentic coding TUI built in Rust.  
OpenCoven fork of [Claurst](https://github.com/Kuberwastaken/claurst) by Kuber Mehta (GPL-3.0).

## Install

```bash
npm install -g @opencoven/coven-code
# or
bun install -g @opencoven/coven-code
```

On install, the correct pre-built native binary for your platform is automatically downloaded from [GitHub Releases](https://github.com/OpenCoven/coven-code/releases). No compilation required.

## Usage

```bash
coven-code                    # interactive TUI
coven-cave                    # alias for coven-code
coven-code -p "fix this bug"  # headless one-shot
```

## Providers

Supports Anthropic (Claude) and Codex (OpenAI Codex via ChatGPT/Codex login).

```bash
coven-code --provider anthropic "refactor this"
coven-code --provider codex "explain this"
```

## Configuration

Settings: `~/.coven-code/settings.json`  
Env prefix: `COVEN_CODE_*`

## Links

- [OpenCoven](https://opencoven.ai)
- [GitHub](https://github.com/OpenCoven/coven-code)
- [Issues](https://github.com/OpenCoven/coven-code/issues)
- [Upstream (Claurst)](https://github.com/Kuberwastaken/claurst)
