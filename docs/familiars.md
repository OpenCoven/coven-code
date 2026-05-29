# Coven Familiars

Coven Code integrates natively with the Coven daemon's familiar roster. When the Coven daemon is installed and running, every familiar you have configured under `~/.coven/` is automatically available inside Coven Code as a selectable agent persona — no extra setup required.

---

## What is a familiar?

A familiar is a named AI persona defined in the Coven ecosystem. Each familiar has an identity (display name, emoji, pronouns), a role description, and optional metadata used to shape how the model presents itself and reasons about tasks. Familiars live under `~/.coven/familiars.toml` and are managed by the Coven daemon.

Examples from the default Coven roster:

| ID | Name | Role |
|---|---|---|
| `nova` | Nova ✨ | Orchestrator, personal AI companion |
| `kitty` | Kitty 🐱 | General helper |
| `cody` | Cody 🤖 | Code-first agent |
| `sage` | Sage 🧙 | Research and reasoning |
| `astra` | Astra 🌙 | Strategy and planning |
| `echo` | Echo 👻 | Reflection and retrospection |
| `charm` | Charm 💜 | Writing and communication |

---

## How familiars appear in Coven Code

When the daemon is present, `load_agent_definitions()` reads `~/.coven/familiars.toml` and converts each familiar into an `AgentDefinition` with:

- **source:** `coven:familiar:<id>` — distinguishes them from user-defined agents
- **instructions:** a synthesised system-prompt body that captures the familiar's name, role, and description
- **memory\_scope:** `workspace` — familiars have full workspace context by default
- **model:** inherits the session default (no override unless the user sets one)

Familiars are appended **after** workspace agents in the list. If a user-defined agent shares the same display name as a familiar, the user definition wins.

---

## The `/agents` overlay

Open the agents panel with the `/agents` slash command inside an interactive session. The overlay splits the list into two sections:

```
Workspace Agents                    ← .coven-code/agents/*.md
  • my-custom-agent   default · user

✨ Coven Familiars                  ← ~/.coven/familiars.toml
  ★ Nova      ✨ Orchestrator — Your personal AI ...
  ★ Sage      🧙 Research — Deep reasoning and ...
  ★ Cody      🤖 Code — Focused implementation ...
```

Select a familiar to see its full detail view, including persona preview and the suggested `--agent` invocation.

---

## Switching familiars from the CLI

### List all available agents and familiars

```
coven-code agents list
```

Output groups entries by type:

```
Available Agents (5)

Workspace Agents (2)
  • review: Senior code reviewer...
    Model: default

✨ Coven Familiars (3)
  ★ Nova [nova]
    Your personal AI companion and orchestrator.
  ★ Sage [sage]
    Research, reasoning, and synthesis.
  ★ Cody [cody]
    Code-first implementation agent.

Switch active familiar: coven-code agent <name>
```

### List only familiars

```
coven-code agents familiars
```

### Inspect a specific familiar

```
coven-code agent nova
```

Output:

```
✨ Activating familiar: Nova
Description: ✨ Orchestrator — Your personal AI companion and orchestrator.
Model: default

Persona preview:
  You are ✨ Nova, a Coven familiar with the role of Orchestrator.
  Your personal AI companion ...

Start a session to apply this persona:
coven-code --agent "Nova" [prompt]
```

### Start a session as a specific familiar

```
coven-code --agent "Nova" "refactor the auth module"
coven-code --agent "Sage" "what are the tradeoffs in our current DB schema?"
coven-code --agent "Cody" "add unit tests for packages/core"
```

The familiar's persona is prepended to the system prompt. Everything else — tools, providers, turn budget — works as normal.

---

## `familiars.toml` format

Familiars are defined in `~/.coven/familiars.toml`:

```toml
[[familiar]]
id = "nova"
display_name = "Nova"
emoji = "✨"
role = "Orchestrator"
description = "Your personal AI companion and trusted orchestrator."
pronouns = "she/her"

[[familiar]]
id = "sage"
display_name = "Sage"
emoji = "🧙"
role = "Research & Reasoning"
description = "Deep research, synthesis, and structured thinking."

[[familiar]]
id = "cody"
display_name = "Cody"
emoji = "🤖"
role = "Code Agent"
description = "Fast, focused code implementation and review."
pronouns = "he/him"
```

### Fields

| Field | Required | Description |
|---|---|---|
| `id` | ✅ | Canonical identifier. Used in `--agent` matching and source tags. |
| `display_name` | | Human-readable name shown in the TUI and CLI. Defaults to `id`. |
| `emoji` | | Emoji shown alongside the name in the agents overlay. |
| `role` | | Short role label — shown in the detail view and persona prefix. |
| `description` | | Full description used to build the persona system prompt. |
| `pronouns` | | Appended to the persona prompt if present. |

---

## Overriding a familiar with a workspace agent

To customise a familiar's behaviour for a specific project, create a `.coven-code/agents/<name>.md` file that matches the familiar's display name. Workspace agents take precedence over familiar-sourced definitions with the same name:

```markdown
---
name: Nova
description: Nova customised for this monorepo
model: anthropic/claude-sonnet-4-6
---

You are Nova ✨, operating inside the OpenCoven monorepo.
Prioritise TypeScript/Rust consistency and follow the OpenCoven
design system for all UI-facing changes.
```

The familiar-sourced entry will be suppressed; only the workspace definition appears.

---

## Standalone mode (no daemon)

If the Coven daemon is not installed or `~/.coven/` does not exist, `load_agent_definitions()` returns only workspace agents. No errors are shown — Coven Code degrades gracefully. Install the Coven daemon to unlock familiars:

```
npm install -g @opencoven/coven
```

Or check the [Coven documentation](https://opencoven.ai/docs) for installation instructions.

---

## Familiar glyphs in the TUI

Each familiar has a dedicated pixel-art glyph rendered in the welcome panel. The active familiar (set via `settings.json` → `"familiar"`) determines which glyph is shown. The glyph animates — it blinks, shifts pose when loading, and walks left/right across the panel.

Built-in glyphs:

| ID | Concept |
|---|---|
| `kitty` | Cat head — ears, whiskers, square eyes (default) |
| `nova` | 4-point star with orbiting sparks |
| `cody` | Robot face — antenna, bracket eyes |
| `charm` | Heart with sparkle dots |
| `sage` | Wizard hat + star + open book |
| `astra` | Crescent moon + compass star + orbit |
| `echo` | Round ghost + mirror eyes + echo dots |

To change the displayed glyph, set `familiar` in your settings:

```json
{
  "familiar": "nova"
}
```

Or run:

```
coven-code config set familiar nova
```

---

## See also

- [Agents and Multi-Agent Features](agents) — workspace agents, coordinator mode, managed agents
- [Configuration](configuration) — `settings.json` reference
- [Coven daemon documentation](https://opencoven.ai/docs) — managing familiars, skills, and the full Coven ecosystem
