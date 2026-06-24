# @opencoven/coven-code

[![Version](https://img.shields.io/npm/v/@opencoven/coven-code?style=flat-square)](https://www.npmjs.com/package/@opencoven/coven-code)
[![License](https://img.shields.io/badge/License-GPL--3.0-blue?style=flat-square)](https://github.com/OpenCoven/coven-code/blob/main/LICENSE.md)

**Coven Code** — open-source, multi-provider agentic coding TUI built in Rust.  
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

Supports Anthropic (Claude), OpenAI, Google Gemini, Groq, Ollama, LM Studio, OpenRouter, Bedrock, Vertex, and any OpenAI-compatible endpoint.

```bash
coven-code --provider openai "refactor this"
coven-code --provider ollama --model llama3.2 "explain this"
```

## Configuration

Settings: `~/.coven-code/settings.json`  
Env prefix: `COVEN_CODE_*`

## Links

- [OpenCoven](https://opencoven.ai)
- [GitHub](https://github.com/OpenCoven/coven-code)
- [Issues](https://github.com/OpenCoven/coven-code/issues)
- [Upstream (Claurst)](https://github.com/Kuberwastaken/claurst)
