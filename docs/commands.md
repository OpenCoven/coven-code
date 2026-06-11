# Coven Code Slash Commands Reference

This document is the reference for the visible slash commands available in Coven Code. Commands are invoked by typing `/command-name` at the REPL prompt.

---

## Table of Contents

1. [Command System Overview](#command-system-overview)
2. [Session & Navigation](#session--navigation)
3. [Model & Provider](#model--provider) — `/model`, `/providers`, `/connect`, `/thinking`, `/effort`, `/advisor`, `/fast`
4. [Configuration & Settings](#configuration--settings) — `/config`, `/keybindings`, `/permissions`, `/hooks`, `/mcp`, `/output-style`, `/theme`, `/statusline`, `/vim`, `/voice`, `/terminal-setup`
5. [Code & Git](#code--git) — `/commit`, `/diff`, `/undo`, `/revert`, `/review`, `/init`, `/search`
6. [Search & Files](#search--files) — `/context`
7. [Memory & Context](#memory--context) — `/memory`, `/usage`, `/cost`, `/status`
8. [Agents & Tasks](#agents--tasks) — `/agents`, `/tasks`, `/goal`, `/managed-agents`, `/agent`
9. [Planning & Review](#planning--review) — `/plan`, `ultraplan` (CLI)
10. [MCP & Integrations](#mcp--integrations) — `/mcp`, `/skills`, `/plugin`, `/chrome`
11. [Authentication](#authentication) — `/login`, `/logout`, `/switch`, `/refresh`
12. [Display & Terminal](#display--terminal) — `/theme`, `/output-style`, `/statusline`, `/vim`, `/terminal-setup`, `/incant`, `/color`
13. [Diagnostics & Info](#diagnostics--info) — `/doctor`, `/version`, `/update`
14. [Export & Sharing](#export--sharing) — `/export`, `/copy`, `/share`
15. [Advanced & Internal](#advanced--internal) — `/thinking`, `/connect`, `/fork`, `/effort`, `/whisper`, `/sandbox`, `/think-back`
16. [Coven Substrate](#coven-substrate) — `/coven`, `/handoff`, `/familiar`
17. [Additional Commands](#additional-commands) — feedback, config import, plugin reload, named CLI commands
18. [Command Availability](#command-availability)

---

## Command System Overview

Commands are resolved in a priority-ordered registry. When you type a command name, Coven Code checks:

```
built-in commands -> user command templates -> discovered skills -> plugin commands
```

Commands support aliases — for example `/h`, `/?`, and `/help` all invoke the same handler.

### Usage Syntax

```
/command-name [arguments]
```

Arguments are passed as a single string after the command name.

---

## Session & Navigation

### /help
**Aliases:** `h`, `?`

Display all available commands with their descriptions. Hidden and setup-only commands are suppressed from the default listing.

```
/help
/h
/?
```

---

### /clear
**Aliases:** `c`, `reset`, `new`

Clear the current conversation history and start a fresh session. The session file is retained on disk; only the in-memory message list is cleared.

```
/clear
```

---

### /exit
**Aliases:** `quit`, `q`

Exit the Coven Code REPL. Equivalent to pressing `Ctrl+D`. Unsaved session state is flushed before exit.

```
/exit
/quit
```

---

### /resume
**Aliases:** `r`, `continue`

Resume a previous session from the session store. Displays a list of recent sessions with timestamps and summaries. Select one to restore its message history and file state.

```
/resume
/resume <session-id>
```

---

### /session
**Aliases:** `remote`

Show or manage conversation sessions. Without arguments, shows the current session status (including the remote session URL when a bridge is active).

```
/session                       — show current session status
/session list                  — list recent sessions
/session rename <new-name>     — rename the current session
```

`/session rename` absorbs the former standalone `/rename` command. The new name is used in session listings and exports.

---

### /fork

Fork the current session into a new independent session that begins from the current conversation state. Useful for exploring two different approaches without losing either.

```
/fork
/fork <new-session-name>
```

---

### /rewind

Rewind the conversation to a previous message. Displays a numbered list of messages; enter a number to truncate history to that point and resume from there.

```
/rewind
/rewind <message-index>
```

---

### /compact

Summarize and compress the conversation history to reduce context window usage. The model is asked to produce a dense summary of the prior exchange; that summary replaces the raw messages.

```
/compact
```

---

## Model & Provider

### /model

Open the interactive model picker. Displays a searchable list of available models from all configured providers. The selected model is used for all subsequent inference in the current session.

```
/model
/model claude-opus-4-5
/model claude-sonnet-4-6
```

---

### /providers

List all configured AI providers and their connection status. Shows provider name, base URL, and whether credentials are present.

```
/providers
```

---

### /connect

Connect to a remote AI provider or configure a custom provider endpoint. Supports OpenAI-compatible APIs, Anthropic direct, and others.

```
/connect
/connect <provider-name>
/connect openai https://api.openai.com/v1
```

---

### /thinking
**Aliases:** `think`

Configure extended thinking for the current session. Extended thinking allows the model to reason through problems before responding, at the cost of additional tokens.

```
/thinking
/thinking on
/thinking off
```

See also `/effort` for a higher-level interface to thinking depth.

---

### /effort

Set the thinking effort level. This is a convenience wrapper over `/thinking` that maps human-readable levels to token budgets.

| Level | Description |
|-------|-------------|
| `low` | Minimal thinking; fastest responses |
| `medium` | Balanced thinking and speed |
| `high` | Deep reasoning; slower responses |
| `max` | Maximum token budget for thinking |

```
/effort low
/effort medium
/effort high
/effort max
```

---

### /advisor

Set or unset a secondary advisor model that provides supplementary suggestions alongside the main model. When set, the advisor model's context is available to improve main-model responses.

```
/advisor                          — show current advisor setting
/advisor claude-opus-4-6          — set advisor model by name
/advisor provider/model           — set advisor using provider/model format
/advisor off                      — disable the advisor
/advisor unset                    — disable the advisor
```

The advisor model persists to `~/.coven-code/settings.json` under `advisorModel`. Model IDs must start with `claude-` or contain a `/` (provider/model format).

---

### /fast
**Aliases:** `speed`

Toggle fast mode. In fast mode, Coven Code switches to the active provider's smaller, faster model for quick responses. Useful when you want rapid answers and deep reasoning is not required.

```
/fast          — toggle fast mode on/off
/fast on       — enable fast mode
/fast off      — disable fast mode
```

Setting persists to `~/.coven-code/ui-settings.json`.

---

## Configuration & Settings

### /config
**Aliases:** `settings`

View or modify Coven Code configuration values. Without arguments, renders an interactive settings panel. With arguments, acts as a key-value accessor.

```
/config
/config get <key>
/config set <key> <value>
/config reset <key>
```

Common keys:

| Key | Description |
|-----|-------------|
| `model` | Default model name |
| `theme` | Color theme name |
| `vim` | Vim mode enabled (`true`/`false`) |
| `outputStyle` | Output rendering style |
| `autoApprove` | Auto-approve tool calls |

---

### /keybindings

Open the interactive keybinding configurator. Displays all bound actions with their current shortcuts. Select an action to rebind it. Changes are written to `~/.coven-code/keybindings.json`.

```
/keybindings
```

See [keybindings.md](./keybindings.md) for the full keybindings reference.

---

### /permissions

View and manage tool permission rules. Permissions control which tools can run without prompting, which are blocked, and which always require confirmation.

```
/permissions
/permissions list
/permissions allow <tool-name>
/permissions deny <tool-name>
/permissions reset
```

---

### /hooks

Manage event hooks. Hooks are shell commands or scripts that execute when lifecycle events fire (e.g., before/after tool calls, on session start/end).

```
/hooks
/hooks list
/hooks add <event> <command>
/hooks remove <hook-id>
```

Available events: `pre-tool`, `post-tool`, `session-start`, `session-end`, `message-send`, `message-receive`.

---

### /mcp

Inspect Model Context Protocol (MCP) servers and reconnect configured servers. MCP servers expose additional tools and resources to the agent.

```
/mcp
/mcp list
/mcp status
/mcp auth <name>
/mcp connect <name>
/mcp logs <name>
/mcp resources [name]
/mcp prompts [name]
/mcp get-prompt <name> <prompt> [key=value ...]
```

Add or remove MCP servers by editing `~/.coven-code/settings.json`.

---

### /output-style

Select how the model's output is rendered in the terminal. Choices include `auto`, `plain`, `markdown`, `streaming`, and others depending on terminal capabilities.

```
/output-style
/output-style plain
/output-style markdown
```

---

### /theme

Open the interactive theme picker. Preview and select a color theme for the Coven Code TUI.

```
/theme
/theme dark
/theme light
/theme solarized
```

---

### /statusline

Configure the status line displayed at the bottom of the TUI. Toggle individual elements such as model name, token count, session name, and git branch.

```
/statusline
/statusline toggle model
/statusline toggle tokens
```

---

### /vim
**Aliases:** `vi`

Toggle vim keybinding mode on or off. In vim mode the input field behaves like a vim editor (normal/insert/visual modes). Persisted to config.

```
/vim
/vim on
/vim off
```

---

### /voice

Configure voice input/output. Requires a supported audio backend. Subcommands control microphone selection, TTS voice, and push-to-talk behavior.

```
/voice
/voice on
/voice off
/voice mic <device>
/voice tts <voice-name>
```

---

### /terminal-setup

Run the terminal capability detection and setup wizard. Checks for true-color support, font ligatures, Unicode rendering, and configures Coven Code accordingly.

```
/terminal-setup
```

---

## Code & Git

### /commit

Stage and commit changes to the current git repository. The model drafts a commit message based on the diff. You can review and edit the message before confirming.

```
/commit
/commit "optional message override"
```

---

### /diff

Show file diffs for changes made during the current session. Displays a unified diff of all files Coven Code has written or edited since the session started.

```
/diff
/diff <file-path>
```

---

### /undo

Undo file changes made during the current session. Restores files to their state before Coven Code's last write operation. Can be called multiple times to step further back.

```
/undo
/undo <file-path>
```

---

### /revert

Revert file changes from an assistant turn back to their pre-turn state, using the shadow-git snapshot. `/undo` reverts the most recent edit; `/revert` reverts a chosen turn and removes that turn (and any later turns) from the session transcript.

```
/revert            — revert the most recent assistant turn
/revert 2          — revert the second-to-last turn
/revert abc123     — revert the turn whose message id starts with 'abc123'
/revert list       — list turns that have recorded file changes
/revert diff [n]   — preview the shadow-git diff for a turn without reverting
```

---

### /review

Review code changes via the model and optionally post the review to the associated GitHub PR. Runs `git diff <base>...HEAD` (or `git diff --cached` when no base is given) and sends the diff for a structured review.

```
/review                    — review staged changes
/review main               — review the diff from main..HEAD
/review origin/main        — review against a remote base ref
```

Variants (the former `/security-review` and `/ultrareview` commands fold in here as subcommands):

```
/review security [path]    — security-focused review: vulnerabilities, credential
                             exposure, injection risks, and other security concerns
/review ultra [path]       — exhaustive multi-dimensional review covering security,
                             performance, maintainability, error handling, test
                             coverage, API design, and architecture; each finding
                             is tagged by category and severity
```

GitHub posting requires `GITHUB_TOKEN` (a personal access token with repo scope); the PR number is auto-detected from `git remote` or supplied via `CLAUDE_PR_NUMBER`.

---

### /init

Initialize Coven Code project configuration in the current directory. Creates a `CLAUDE.md` file that acts as persistent project-level context injected at the start of every session.

```
/init
```

---

### /search

Search the codebase using natural language or regex patterns. Wraps the GrepTool and GlobTool with a higher-level interface.

```
/search <query>
/search "TODO" --type ts
/search "function.*export" --regex
```

---

## Search & Files

### /context

Analyze context window usage. Shows a breakdown of tokens consumed by system prompt, conversation history, file contents, and tool results. Helps identify what to compact or drop.

```
/context
```

---

## Memory & Context

### /memory

Manage session memory. Memory entries are short notes persisted across sessions. The model can read these at session start to maintain continuity.

```
/memory
/memory list
/memory add <note>
/memory delete <id>
/memory clear
```

---

### /usage

Display a detailed token usage breakdown for the current session. Shows input tokens, output tokens, cache reads, cache writes, and estimated cost per API call.

```
/usage
```

---

### /cost

Show the total token usage and estimated cost for the current session. Provides a quick summary without the per-call breakdown of `/usage`. In the TUI, `/cost` opens the interactive stats dialog.

```
/cost
```

For aggregate token / cost / tool statistics across saved sessions, use the `stats` CLI command: `coven-code stats [summary|sessions|tools|daily|session <id>]`.

---

### /status

Show the current session status. Includes active model, permission mode, thinking config, connected MCP servers, and loaded plugins.

```
/status
```

---

## Agents & Tasks

### /agents

Browse and manage saved workspace agents and Coven familiars.

```
/agents                         — open the agents/familiars menu
/agents reset                   — open reset confirmation
coven-code agents reset         — erase saved user agents and familiar roster
```

The reset action removes `~/.coven/familiars.toml`, custom `*.md` agent files
from `~/.coven-code/agents/` and the current workspace's `.coven-code/agents/`,
and clears `agents`, `familiar`, and `managed_agents` settings. It does not
remove built-in agents, plugin packages, sessions, credentials, or history.

---

### /tasks
**Aliases:** `bashes`

Manage tracked background tasks. Tasks are shell commands or model invocations running asynchronously. Monitor progress, fetch output, or stop tasks from this interface.

```
/tasks
/tasks list
/tasks output <task-id>
/tasks stop <task-id>
```

---

### /goal

Set a durable multi-turn autonomous goal. When a goal is active, Coven Code continues working across turns until the goal is marked complete, paused, or a 200-turn runaway guard fires. Designed for complex, sustained tasks that would otherwise require repeated manual re-prompting.

```
/goal <objective>                    — set a new goal and begin working autonomously
/goal --tokens 250K <objective>      — set a goal with a soft token budget cap
/goal                                — show current goal status
/goal status                         — show current goal status
/goal pause                          — pause the active goal
/goal resume                         — resume a paused goal
/goal clear                          — delete the current goal
/goal complete                       — request a completion audit
```

When the model believes the goal has been achieved, it calls the `GoalComplete` tool with an audit summary and evidence. Goals can be disabled globally by setting `COVEN_CODE_GOALS=0` in your environment.

See [Goal System](./advanced.md#goal-system) in the advanced guide.

---

### /managed-agents

Configure the manager-executor agent architecture, where a manager model delegates subtasks to one or more executor agents working in parallel. Includes budget controls and isolation options.

```
/managed-agents                                       — show current configuration
/managed-agents status                                — show current configuration
/managed-agents presets                               — list built-in presets
/managed-agents preset <name>                         — apply a named preset
/managed-agents setup                                 — show setup instructions
/managed-agents enable                                — enable managed agents
/managed-agents disable                               — disable managed agents
/managed-agents reset                                 — remove all managed-agent configuration
/managed-agents configure manager-model <model>       — set the manager model
/managed-agents configure executor-model <model>      — set the executor model
/managed-agents configure executor-turns <n>          — set executor max turns
/managed-agents configure concurrent <n>              — set max concurrent executors
/managed-agents configure isolation on|off            — toggle executor isolation
/managed-agents configure budget-split shared         — shared token pool
/managed-agents configure budget-split percentage:<n> — percentage split (manager gets n%)
/managed-agents configure budget-split fixed:<m>:<e>  — fixed USD caps (manager / executor)
/managed-agents budget <amount>                       — set total budget in USD (0 to clear)
```

Model format: `provider/model` (e.g., `anthropic/claude-opus-4-6`, `openai/gpt-4o`). Configuration persists to `~/.coven-code/settings.json` under `managed_agents`.

> **Preview feature.** Behaviour may change across releases.

See [Managed Agents](./advanced.md#managed-agents) in the advanced guide.

---

### /agent

List all available named agents, or show details for a specific agent. Named agents are predefined configurations with their own system prompts, model bindings, and access levels. Useful for discovering what agents are available before starting a session.

```
/agent             — list all visible named agents with access levels
/agent <name>      — show full details for a specific named agent
```

To activate an agent, start Coven Code with `--agent <name>`. See [agents.md](./agents.md) for defining custom agents.

---

## Planning & Review

### /plan

Enter plan mode (read-only). In plan mode the model can read files and reason about changes but cannot write, edit, or execute anything. Use this to draft an approach before allowing writes.

```
/plan
```

To exit plan mode, use `/plan off` or the `/exit-plan` internal action.

---

### ultraplan (CLI)

Launch the Ultraplan agentic code planner with extended thinking. Like `/plan` but with an elevated thinking budget to allow more thorough analysis before acting. Ultraplan is a named CLI command — it has no slash form.

```
coven-code ultraplan [--effort=medium|high|maximum]
```

For an exhaustive review pass, see [`/review ultra`](#review).

---

## MCP & Integrations

### /mcp

Documented above under [Configuration & Settings](#configuration--settings).

---

### /skills
**Aliases:** `skill`

List and manage skills. Skills are bundled prompt-commands that extend Coven Code's capabilities without writing code. They appear alongside built-in commands in the registry.

```
/skills
/skills list
/skills enable <skill-name>
/skills disable <skill-name>
/skills reload
```

---

### /plugin
**Aliases:** `plugins`

Manage plugins. Plugins are loadable modules that can register new commands, tools, hooks, agents, skills, and MCP server definitions.

```
/plugin
/plugin list
/plugin info <name>
/plugin enable <name>
/plugin disable <name>
/plugin install <path>
/plugin reload
```

`/plugin reload` refreshes the active session plugin registry, hook registry, plugin commands, agents, skills, and in-memory MCP server definitions. New plugin MCP servers are included in the initial MCP connection at startup; if a reload adds a new MCP server after startup, start a new session before expecting its tools in the model tool list.

---

### /chrome

Browser automation via Chrome DevTools Protocol (CDP). Connects to a running Chrome or Chromium instance and lets Coven Code control it — navigate pages, click elements, fill forms, evaluate JavaScript, and take screenshots.

First, launch Chrome with remote debugging enabled:

```bash
chrome --remote-debugging-port=9222 --no-first-run
```

Then:

```
/chrome connect [--port 9222]      — connect to Chrome on the given port (default: 9222)
/chrome navigate <url>             — navigate to a URL
/chrome screenshot                 — take a screenshot, saved to a temp file
/chrome click <selector>           — click a CSS selector
/chrome fill <selector> <text>     — fill an input field
/chrome eval <js>                  — evaluate JavaScript and return the result
/chrome disconnect                 — disconnect from Chrome
```

Useful for testing web applications, scraping, or automating browser-based workflows without a separate browser-automation tool.

---

## Authentication

Coven Code supports **multiple named accounts per provider** — Anthropic (Claude.ai or Console) and Codex (OpenAI ChatGPT subscription). Each login creates a profile under `~/.coven-code/accounts/<provider>/<id>/` and the registry at `~/.coven-code/accounts.json` tracks which one is active.

See [Authentication Guide](./auth.md#multi-account-profiles) for the full story and on-disk layout.

### /login

Authenticate with Anthropic or Codex via OAuth PKCE. Opens a browser for the flow and saves tokens under the active profile (or creates a new profile if none exists).

```
/login                            — Claude.ai OAuth (Bearer token, default)
/login --console                  — Console OAuth (creates an API key)
/login --codex                    — Codex / ChatGPT OAuth
/login --label work               — name the new profile "work"
/login --codex --label personal   — Codex login, name the profile "personal"
```

If a profile matching the JWT's email or account_id already exists, that profile is refreshed in place — re-logging-in is idempotent. Use `--label` to either name a fresh profile or to disambiguate.

---

### /logout

Remove credentials. By default removes only the **active** profile for the provider; other stored profiles remain switchable.

```
/logout                — clear active Anthropic profile (drops it from registry)
/logout --codex        — clear active Codex profile
/logout --all          — purge every Anthropic profile + clear any API key in settings
/logout --codex --all  — purge every Codex profile
```

---

### /switch

Switch the active account for a provider. Anthropic by default; pass `--codex` for Codex. Run `/switch` with no arguments to list every stored account and see available profile ids — the active profile in each provider is marked with `*`.

```
/switch                          — list stored accounts across providers
/switch work                     — set active Anthropic profile to "work"
/switch --codex personal         — set active Codex profile to "personal"
```

Sample listing output:

```
Anthropic:
  * personal [pro]    kuber@personal.example
    work     [max]    kuber@company.example
Codex:
    work              kuber@company.example
```

---

### /refresh

Refresh the provider authentication state. Forces a token refresh without full re-authentication. Useful when a session token has expired mid-session.

```
/refresh
```

---

## Display & Terminal

### /theme

Documented above under [Configuration & Settings](#configuration--settings).

---

### /output-style

Documented above under [Configuration & Settings](#configuration--settings).

---

### /statusline

Documented above under [Configuration & Settings](#configuration--settings).

---

### /vim

Documented above under [Configuration & Settings](#configuration--settings).

---

### /terminal-setup

Documented above under [Configuration & Settings](#configuration--settings).

---

### /incant

Cast a speech incantation — change the model's voice, trading flourish for tokens. The former `/caveman`, `/rocky`, and `/normal` commands fold in here: voices become arguments and `/incant off` replaces `/normal`.

```
/incant <voice> [lite|full|ultra]    — cast an incantation at the given intensity
/incant off                          — lift the active incantation, return to normal speech
```

Voices:

| Voice | Description |
|-------|-------------|
| `caveman` | Why use many token when few token do trick. Strips pleasantries, hedging, articles, and transitional phrases (~75% token reduction) |
| `rocky` | Eridian engineer from *Project Hail Mary*. Save big token. Good good good. |

Intensity:

| Level | Description |
|-------|-------------|
| `lite` | Light touch (~40% reduction) |
| `full` | The default (~75% reduction) |
| `ultra` | Maximum compression |

```
/incant caveman            — full caveman mode
/incant rocky lite         — Rocky grammar, light touch
/incant off                — back to normal speech
```

---

### /color

Set the prompt bar color for the current session. Accepts standard color names or hex values. The color resets when the session ends unless saved via `/config`.

```
/color               — open the interactive color picker
/color <name>        — set to a named color (e.g., blue, red, green)
/color #ff6b6b       — set to a hex color value
/color default       — reset to the theme default
```

---

## Diagnostics & Info

### /doctor

Run the Coven Code diagnostics suite. Checks configuration integrity, provider connectivity, tool availability, MCP server health, and reports any issues.

```
/doctor
```

---

### /version
**Aliases:** `v`

Display the current Coven Code version string and build metadata.

```
/version
/v
```

---

### /update
**Aliases:** `upgrade`

Check for available updates. Queries the GitHub releases API and displays the latest version. If a newer version exists, prints the download URL or upgrade instructions. Does not auto-update.

```
/update
/upgrade
```

---

## Export & Sharing

### /export

Export the current session transcript. Supported formats include Markdown, JSON, and plain text. The output is written to a file or printed to stdout.

```
/export
/export --format markdown
/export --format json --output session.json
/export --stdout
```

---

### /copy

Copy the most recent assistant response to the system clipboard. Pass a number to copy the Nth most-recent response. On Linux a `wl-clipboard` or `xclip` backend is used; on macOS and Windows the native clipboard API is used.

```
/copy         — copy the most recent response
/copy 2       — copy the second most recent response
/copy N       — copy the Nth most recent response
```

---

### /share

Upload the current session as a secret GitHub gist and return a shareable URL. The session is rendered as a single self-contained HTML file and uploaded via the `gh` CLI; a viewer URL of the form `https://opencoven.github.io/coven-code/session/#<gist-id>` is printed.

```
/share
```

Requires the GitHub CLI (`gh`) installed and logged in (`gh auth login`). The viewer base URL can be overridden with `COVEN_CODE_SHARE_VIEWER_URL`. Secret gists are unlisted but readable by anyone who has the link.

---

## Advanced & Internal

### /thinking

Documented above under [Model & Provider](#model--provider).

---

### /connect

Documented above under [Model & Provider](#model--provider).

---

### /fork

Documented above under [Session & Navigation](#session--navigation).

---

### /effort

Documented above under [Model & Provider](#model--provider).

---

### /context

Documented above under [Search & Files](#search--files).

---

### /whisper
**Aliases:** `btw`

Whisper a side question to your familiar without adding it to history. The question goes to the model out-of-band — the response is shown inline but does not become part of the main conversation context. Replaces the former `/btw` command.

```
/whisper <question>
/whisper what is the capital of France?
```

---

### /sandbox
**Aliases:** `sandbox-toggle`

Enable or disable sandboxed execution of shell commands. When sandbox mode is on, bash/shell commands run in an isolated environment to limit unintended side effects. Supported on macOS, Linux, and WSL2.

```
/sandbox                          — toggle sandbox mode on/off
/sandbox on                       — enable sandbox mode
/sandbox off                      — disable sandbox mode
/sandbox status                   — show current state and excluded patterns
/sandbox exclude <pattern>        — add a command pattern to the exclusion list
```

> A restart is recommended after toggling for full effect. On Windows (non-WSL), sandbox mode is not supported.

---

### /think-back
**Aliases:** `thinkback`

Display the extended-thinking traces from previous model responses in the current session. Only available when extended thinking was used for those responses. Pass a number to view the Nth most-recent trace.

```
/think-back         — show the most recent thinking trace
/think-back 2       — show the second most recent thinking trace
/think-back play    — replay the most recent trace as an animated walkthrough
/think-back play 2  — replay the second most recent trace
/thinkback          — alias
```

`/think-back play` absorbs the former `/thinkback-play` command. Thinking traces appear when the model uses extended thinking mode (see `/thinking`). If no traces are found, Coven Code suggests enabling extended thinking.

---

## Coven Substrate

These commands integrate Coven Code with the local Coven daemon
(`~/.coven/coven.sock`, contract `coven.daemon.v1`). They degrade
gracefully when the daemon is absent — the daemon makes coven-code
richer; it is not required.

### /coven

Drive the local Coven daemon (sessions, harness runs, rituals,
familiars, capability discovery) without leaving the TUI. The
top-level `/coven` (or `/coven status`) prints daemon health and the
active session count.

```
/coven                                   show daemon health
/coven status                            same as /coven
/coven capabilities                      daemon capability catalog
/coven familiars                         list familiar statuses
/coven doctor                            detect installed harness CLIs
/coven daemon start|status|stop|restart  daemon lifecycle
```

#### Sessions — read-only

```
/coven sessions [--all]                  list active (or all) sessions
/coven info <session-id>                 full session record
/coven log <session-id>                  redacted log preview
/coven events <id> [--after N] [--limit M]
                                          paginate session events
```

`/coven events` defaults to `--limit 50` and prints
`End of events stream.` when there are no more events. Pass
`--limit 0` to fall back to the daemon's default cap.

#### Sessions — live control

```
/coven run <harness> <prompt>            launch a new harness session
/coven send <session-id> <text>          forward input to a live session
/coven kill <session-id>                 terminate a live session
/coven attach <session-id>               replay/follow a running session
```

`send` and `kill` surface the daemon's structured error codes — when
a session is missing or no longer running you'll see
`Session is not running (409 session_not_live)` or
`Session not found (404 session_not_found)` rather than a generic
"daemon offline" message.

#### Session rituals

```
/coven summon <session-id>               restore an archived session
/coven archive <session-id>              archive a non-running session
/coven sacrifice <session-id>            permanently delete a session
```

Sacrifice is irreversible. `/coven sacrifice` automatically passes
`--yes` to the underlying `coven sacrifice` command (mirroring how
the slash invocation acts as the confirmation).

#### Control plane and integrations

```
/coven actions <action> [json-args]      POST /api/v1/actions
/coven calls [--limit N]                 read the Coven Calls
                                          delegation ledger
                                          (~/.coven/cave-coven-calls.json)
/coven claim acquire|release|status|heartbeat|canary [args]
                                          parallel-work claim protocol
/coven hooks-install                     install pre-commit/pre-push
                                          hooks for the claim protocol
/coven adapter list|doctor [id]          inspect harness adapters
/coven logs prune [--days N]             prune session logs
/coven wt <branch> | --list | --doctor | --prune-merged | --prune-stale [DAYS]
                                          worktree management
/coven patch [name] [issue]              open the OpenClaw repair flow
/coven pc [status|top|disk|...]          macOS system diagnostics
```

#### Offline behaviour

When `~/.coven/coven.sock` is missing, every `/coven` subcommand
returns the same hint:
`Coven daemon offline. Try /coven daemon start.`
The rest of coven-code keeps working — you just lose substrate
integration.

#### Error surfacing

Non-2xx responses from the daemon are parsed for the
`{ "error": { "code", "message" } }` envelope and surfaced as
`<message> (<status> <code>)`. See
[`docs/API-CONTRACT.md` in OpenCoven/coven](https://github.com/OpenCoven/coven/blob/main/docs/API-CONTRACT.md)
for the canonical error code list.

---

### /handoff

Hand off the current session context to a Coven familiar. Sends the
recent transcript to `~/.coven/coven.sock` as a new session under
the named familiar, returning the session id.

```
/handoff <familiar-id>
```

Requires the Coven daemon to be running and the familiar id to be
defined in `~/.coven/familiars.toml`. See
[familiars.md](./familiars.md) for the full familiar concept.

---

### /familiar
**Aliases:** `familiars`

Set your active familiar — changes the TUI mascot live and updates
the persona that gets injected into the system prompt when launched
with `--agent <familiar-id>`.

```
/familiar               show current familiar
/familiar <id>          switch to a familiar by id
```

Press **F2** at any time to open the familiar switcher popup
interactively (the welcome screen's status block hints at this).

---

## Additional Commands

The following commands ship with Coven Code but do not have a full
section above. They are grouped by purpose.

### Feedback & configuration

| Command | Description |
|---------|-------------|
| `/feedback` (alias `bug`) | Submit feedback about Coven Code. `/feedback report` for a bug report. |
| `/import-config` | Import user-level Claude Code configuration (`CLAUDE.md`, `settings.json`) from `~/.claude` via an interactive dialog with preview and confirmation. |
| `/reload-plugins` | Reload the active session plugin registry, hooks, agents, skills, and MCP definitions. |

### Workspace & GitHub

| Command | Description |
|---------|-------------|
| `/add-dir` | Add a directory to Coven Code's allowed workspace paths. |
| `/branch` | Create a branch of the current conversation at this point. |
| `/tag` | Toggle a searchable tag on the current session. |
| `/ide` | Manage IDE integrations and show status. |
| `/pr-comments` | Get comments from a GitHub pull request. |

### Named CLI commands

These run as `coven-code <name>` from the shell. Most have slash
adapters (`/agents`, `/add-dir`, `/branch`, `/tag`, `/ide`,
`/pr-comments`); `ultraplan` and `stats` are CLI-only.

| Command | Description |
|---------|-------------|
| `agents` | Manage and configure sub-agents. |
| `agent` | List or inspect named agents. |
| `add-dir` | Add a directory to the allowed workspace paths. |
| `branch` | Branch the current conversation. |
| `tag` | Toggle a searchable session tag. |
| `ide` | Manage IDE integrations. |
| `pr-comments` | Get comments from a GitHub PR. |
| `ultraplan` | Launch the Ultraplan agentic code planner with extended thinking. |
| `stats` | Aggregate token / cost / tool stats across saved sessions (in the TUI, `/cost` opens the stats dialog). |

---

## Command Availability

Not all commands are available in all contexts.

### Remote Mode

When running with `--remote`, only a restricted set of commands is available:

`session`, `exit`, `clear`, `help`, `theme`, `vim`, `cost`, `usage`, `plan`, `keybindings`, `statusline`

### Bridge Mode

Over the Remote Control bridge (used by IDE integrations), only `local`-type commands are forwarded:

`compact`, `clear`, `cost`

### Availability-Restricted Commands

Some commands are available only under certain account or platform conditions:

| Command | Restriction |
|---------|-------------|
| `/fast` | Available when a fast-mode model is configured for the active provider |
| `/sandbox` | Functional on macOS, Linux, WSL2 only; no-op on native Windows |
| `/voice` | Requires an audio backend plus `OPENAI_API_KEY` or `WHISPER_ENDPOINT_URL` for transcription |
| `/chrome` | Requires a running Chrome/Chromium instance launched with remote debugging enabled |
