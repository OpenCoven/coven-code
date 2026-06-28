<div align="center">

# Coven Code

<img src="../public/Ship.png" alt="Rune on the ship" width="350" />

Coven Code is a high-performance Rust reimplementation of Claude Code — a terminal-native AI coding agent with streaming responses, 40+ built-in tools, 15+ LLM provider integrations, a full ratatui TUI, and an extensible plugin system.

**Version:** 0.2.1 (Beta) · **License:** GPL-3.0 · [GitHub](https://github.com/OpenCoven/coven-code)

</div>

---

## What Coven Code does

You give Coven Code a task in natural language. It plans, reads and writes files, runs shell commands, searches the web, and iterates — all inside your terminal, with every step visible in real time.

```
$ coven run codex "add input validation to the signup form"
```

Coven Code reads your codebase, implements the change across multiple files, runs your tests, and reports back — without you leaving the terminal.

---

## Key capabilities

### Agentic loop
Coven Code runs a multi-turn loop: it streams a response from the model, executes any tool calls (file read, bash, web search, …), feeds the results back, and continues until the task is done or the turn limit is reached.

### 40+ built-in tools
- **File operations** — read, write, edit, patch, batch-edit
- **Shell** — bash with persistent working directory and environment
- **Search** — glob file patterns, grep contents, web search, web fetch
- **Git** — commit, branch, worktree
- **Notebooks** — read and edit Jupyter notebooks
- **Desktop automation** — screenshot, click, type (optional feature)
- **Task management** — create, track, and complete tasks

### LLM providers
Anthropic Claude (default) and Codex (OpenAI Codex via ChatGPT/Codex login).

### AMOLED terminal UI
A ratatui-based TUI with real-time streaming, syntax-highlighted code blocks, diff viewer, permission dialogs, slash command autocomplete, session browser, and a full keybinding system.

### Multi-account credentials
Store multiple named Anthropic (Claude.ai / Console) and Codex (ChatGPT) accounts in one install and switch between them instantly with `/login switch` or `coven-code auth switch <id>`. Identity is detected from the OAuth JWT, so re-logging-in the same account is idempotent. See [Authentication](auth#multi-account-profiles).

### @file injection
Type `@path/to/file` anywhere in a prompt to inject the file's contents inline. Typeahead autocomplete suggests paths as you type, with size/binary safety checks before submit. See [@file Injection](keybindings#file-injection-with-typeahead).

### Plugin system
Extend Coven Code with TOML-manifest plugins that add custom slash commands, MCP servers, hooks, output styles, and tool overlays.

### Multi-agent orchestration
Run named agents (`build`, `plan`, `explore`) or spawn parallel sub-agents in coordinator mode. Agents communicate via a shared task registry and message channels.

### Goal system
Set a durable objective with `/coven goal` and Coven Code works autonomously across turns until the goal is verified complete — using the `GoalCompleteTool` for audited completion rather than just stopping.

### Managed agents (preview)
Configure a manager-executor architecture with `/familiar managed` where a manager model delegates subtasks to parallel executor agents with full budget split controls.

### Speech incantations
Cast `/incant caveman` or `/incant rocky` to compress model responses by 40–85%, saving tokens in long sessions. Lift the incantation with `/incant off`.

---

## Quick start

**1. Install**

```bash
# Linux / macOS
npm install -g @opencoven/coven
```

The package installs the `coven` CLI. Run `coven` with no arguments, or
`coven tui` explicitly, for the interactive UI. See [Installation](installation)
for npm, bun, standalone binary, and source install options.

**2. Set your API key**

```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

**3. Run interactively**

```bash
coven
```

Or launch a direct harness session:

```bash
coven run codex "explain the auth module"
```

---

## Configuration

Coven Code reads `~/.coven-code/settings.json` at startup. The most common settings:

```json
{
  "config": {
    "model": "claude-opus-4-6",
    "permission_mode": "default",
    "auto_compact": true,
    "compact_threshold": 0.8
  }
}
```

See [Configuration](configuration) for the full reference.

---

## Using a different provider

```bash
# Use Anthropic explicitly
coven-code --provider anthropic --model claude-opus-4-6

# Use Codex (requires a Codex OAuth login)
coven-code codex login
coven-code --provider codex
```

See [Providers](providers) for setup instructions for both supported providers.

---

## Interactive vs headless

| Mode | Command | Use case |
|------|---------|----------|
| Interactive TUI | `coven` or `coven tui` | Day-to-day coding |
| Direct harness run | `coven run codex "task"` | Quick one-shot tasks |
| Claude Code run | `coven run claude "task"` | Use Claude Code through Coven |
| Session browser | `coven sessions` | Rejoin, view, archive, or delete sessions |
| Stream JSON | `coven run codex "task" --stream-json` | Real-time piping |

---

## The welcome screen

When you launch the interactive UI with `coven` or `coven tui`, the home screen opens with a single rounded panel titled `Coven Code v<version>`. It's the at-a-glance status surface — every value comes from another subsystem, so use it as a jumping-off point rather than a source of truth.

**Left column** — your familiar's portrait (animated glyph for built-ins, static card for daemon-registered familiars) under a `Welcome back <user>!` greeting. The art is driven by the `"familiar"` field in your settings; see [Coven Familiars](familiars).

**Right column** — a rotating getting-started tip, then a **Status** block:

| Field | What it shows | Configured in |
|-------|---------------|---------------|
| `Model` | Active model id, or the effective default if unset | `model` in [settings.json](configuration), `/model` |
| `Provider` | Active provider id (`anthropic` when unset) | `provider` in [settings.json](configuration), see [Providers](providers) |
| `Daemon` | `online` / `offline` from a cheap socket check — no RPC | Install `@opencoven/coven` to bring it online |
| `Familiar` | Current familiar id, with an `(F2 to switch)` hint | `familiar` in settings, `/familiar`, or **F2** |
| `Goal` | Active autonomous goal (only shown when one is set) | `/coven goal <objective>` |

Press **F2** at any time to open the familiar switcher popup.

On terminals narrower than ~30 columns or shorter than 11 rows, the panel collapses to a single line — `Coven Code v… · <model> · <daemon> · <familiar>` — so the essentials stay visible even in a tiny pane.

---

## Slash commands

Inside the interactive TUI, type `/` to see all available commands. Common ones:

| Command | Description |
|---------|-------------|
| `/help` | Show all commands |
| `/model` | Switch model or provider |
| `/login` | OAuth login (Anthropic; `--codex` for ChatGPT, `--label <name>` to name) |
| `/login switch [<id>]` | Switch active account; with no id, lists stored accounts (`--codex` for Codex) |
| `/logout` | Clear credentials for the active account (`--all` to purge) |
| `/coven goal <objective>` | Set an autonomous multi-turn goal |
| `/familiar managed` | Configure manager-executor agents |
| `/compact` | Compress conversation history |
| `/cost` | Token usage and cost for this session |
| `/incant <voice>` | Cast a speech incantation (`caveman`, `rocky`); `/incant off` lifts it |
| `/whisper <q>` | Side question to your familiar, not kept in history |
| `/rewind` | Go back to a previous message |
| `/export copy` | Copy last response to clipboard |
| `/export` | Save session transcript |
| `/thinking back` | View thinking traces from previous responses |
| `/review ultra` | Exhaustive multi-dimensional code review |
| `/config advisor <model>` | Set a secondary advisor model |
| `/sandbox` | Toggle sandboxed shell execution |
| `/update` | Check for and download updates |
| `/exit` | Quit |

See [Slash Commands](commands) for the complete reference.

---

## Coven ecosystem integration

Coven Code connects natively to the [Coven daemon](https://opencoven.ai/docs) when it is running on your machine. With the daemon active:

- **Familiars appear as agents** — every familiar you have configured in `~/.coven/familiars.toml` is automatically surfaced in the `/familiar` overlay and the `coven-code agents` command.
- **Skills are visible** — daemon-registered skills are listed as awareness context so the model knows what capabilities are available.
- **Familiar glyphs animate** in the welcome panel using the glyph that matches your configured `"familiar"` setting.

Coven Code is fully standalone without the daemon — install it separately to unlock the Coven ecosystem features.

```
npm install -g @opencoven/coven
```

See [Coven Familiars](familiars) for the full integration reference.

---

## Next steps

- [Installation](installation) — download, build from source, system requirements
- [Authentication](auth) — API keys and OAuth
- [Configuration](configuration) — settings.json reference
- [Slash Commands](commands) — all 70+ commands
- [Tools Reference](tools) — all 40+ tools and permission levels
- [Providers](providers) — configuring each LLM provider
- [MCP Integration](mcp) — Model Context Protocol servers
- [Plugins](plugins) — building and using plugins
- [Agents](agents) — multi-agent orchestration
- [Familiars](familiars) — Coven daemon familiars as agent personas
- [Hooks](hooks) — event-driven automation
- [Advanced Features](advanced) — extended thinking, sessions, and more
