# Coven Familiars

Coven Code integrates natively with the Coven daemon's familiar roster. When the Coven daemon is installed and running, every familiar you have configured under `~/.coven/` is automatically available inside Coven Code as a selectable agent persona — no extra setup required.

---

## What is a familiar?

A familiar is a named AI persona defined in the Coven ecosystem. Each familiar has an identity (display name, emoji, pronouns), a role description, and optional metadata used to shape how the model presents itself and reasons about tasks. Familiars are user-defined and live in `~/.coven/familiars.toml`, managed by the Coven daemon.

For example, a minimal Coven setup might have:

| ID | Name | Role |
|---|---|---|
| `dev` | Dev 🤖 | Code-first implementation agent |
| `research` | Research 🧙 | Research and reasoning |
| `writer` | Writer ✍️ | Writing and communication |

You define your own familiars — the names, roles, and roster are entirely yours.

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
  ★ Dev       🤖 Code — Focused implementation ...
  ★ Research  🧙 Research — Deep reasoning and ...
  ★ Writer    ✍️ Writing — Docs and communication ...
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
Available Agents (4)

Workspace Agents (1)
  • review: Senior code reviewer...
    Model: default

✨ Coven Familiars (3)
  ★ Dev [dev]
    Fast, focused code implementation and review.
  ★ Research [research]
    Deep research, synthesis, and structured thinking.
  ★ Writer [writer]
    Clear writing, docs, and async communication.

Switch active familiar: coven-code agent <name>
```

### List only familiars

```
coven-code agents familiars
```

### Inspect a specific familiar

```
coven-code agent dev
```

Output:

```
✨ Activating familiar: Dev
Description: 🤖 Code Agent — Fast, focused code implementation and review.
Model: default

Persona preview:
  You are 🤖 Dev, a Coven familiar with the role of Code Agent.
  Fast, focused code implementation ...

Start a session to apply this persona:
coven-code --agent "Dev" [prompt]
```

### Start a session as a specific familiar

```
coven-code --agent "Dev" "refactor the auth module"
coven-code --agent "Research" "what are the tradeoffs in our current DB schema?"
coven-code --agent "Writer" "write release notes for v1.2"
```

The familiar's persona is prepended to the system prompt. Everything else — tools, providers, turn budget — works as normal.

---

## `familiars.toml` format

Familiars are defined in `~/.coven/familiars.toml`:

```toml
[[familiar]]
id = "dev"
display_name = "Dev"
emoji = "🤖"
role = "Code Agent"
description = "Fast, focused code implementation and review."
pronouns = "they/them"
access = "full"

[[familiar]]
id = "research"
display_name = "Research"
emoji = "🧙"
role = "Research & Reasoning"
description = "Deep research, synthesis, and structured thinking."
# access omitted → defaults to "read-only"

[[familiar]]
id = "writer"
display_name = "Writer"
emoji = "✍️"
role = "Writing & Communication"
description = "Clear writing, docs, and async communication."
pronouns = "she/her"
access = "read-only"
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
| `access` | | Tool-access tier: `"full"`, `"read-only"`, or `"search-only"`. Defaults to `"read-only"` when omitted. See [Tool access tiers](#tool-access-tiers) below. |
| `model` | | Optional model override for this familiar, e.g. `"claude-opus-4-8"`. When omitted the familiar inherits the session's default model. Lets you pin a persona to a model without shadowing it with a workspace agent. |

---

## Managing the roster

`~/.coven/familiars.toml` is **owned by the Coven daemon** when it is running. In that mode coven-code treats the roster as read-only and directs all edits to the daemon.

**Standalone mode** (no daemon socket at `~/.coven/coven.sock`) is different: coven-code owns the file and can write it directly.

- **First-run bootstrap.** Press **F2** with no familiars configured and coven-code writes a starter `~/.coven/familiars.toml` (a `read-only` guide and a `full`-access builder) and opens the switcher so you have something to pick and a template to edit.
- **Create / rename / remove** from the prompt:

  ```text
  /familiar new <id> [display name]   # create a read-only familiar
  /familiar rename <old-id> <new-id>  # rename, preserving all fields
  /familiar remove <id>               # delete (clears it if it was active)
  ```

  You can also remove a familiar from the visual `/familiar` menu. All of these refuse with a clear message when the daemon owns the file.
- **Switching:** `/familiar <id>`, the **F2** quick switcher (type to filter, `— none —` clears the active familiar), or the `/familiar` menu.
- **Clearing vs wiping:** `/familiar clear` (alias `reset`) steps back to no active familiar. `/familiar wipe-roster` (alias `reset-roster`) is **destructive** — it deletes the roster and workspace agents — and requires `/familiar wipe-roster confirm`.

If the roster file is malformed, coven-code surfaces the parse error at startup instead of silently dropping every familiar; unknown access tiers and duplicate/reserved ids are reported as warnings.

---

## Tool access tiers

The `access` field controls **which tools** a familiar may invoke once you select them as the active agent (via `--agent <id>` or the `/agents` picker). The same tool-filter pipeline used for the built-in `build` / `plan` / `explore` modes applies, so the rules are consistent across the product.

| Tier | What the familiar can do | Typical role |
|---|---|---|
| `full` | Read, write, and execute — full tool set (Edit/Write/Bash/etc.) | Build-tier familiars that edit and run code |
| `read-only` | Read & search the workspace plus `AskUserQuestion`, no writes or shell. **Default.** | Research / strategy familiars |
| `search-only` | Narrow read+search whitelist: `Grep`, `Glob`, `Read`, `WebSearch`, `WebFetch`. No writes or shell. | Pure-research personas with minimal codebase footprint |

> **Unknown values fail closed.** Case and surrounding whitespace are normalized silently — `"READ-ONLY"`, `"Read-Only"`, and `" full "` all map to their canonical tier. Anything else (a typo like `"readonly"`, an invented tier like `"super-admin"`, an empty string) is treated as `"read-only"` and a warning is printed to stderr. Typos cannot silently grant write/exec power.

### Why the default is restrictive

`access` defaults to `read-only`. Granting write/exec power is **opt-in**: you must set `access = "full"` explicitly on a familiar to let it edit files or run shell commands. This avoids surprise when a freshly-defined familiar (perhaps written for a research role) accidentally gains the ability to mutate the workspace.

### Recommended defaults per role

| Role | Suggested `access` |
|---|---|
| Code / Build / Ship | `"full"` |
| General Helper / Assistant | `"full"` (set if you want them to edit/run; otherwise leave to default) |
| Orchestrator / Queen | `"full"` (they coordinate work that requires writes) |
| Research / Synthesis | `"read-only"` (default — keep them honest) |
| Strategy / Navigation | `"read-only"` (default) |
| Memory / Reflection | `"read-only"` (default) |
| Comms / Social | `"read-only"` (default) |

### Example: minimal opt-in roster

```toml
# Build-tier — can edit and run.
[[familiar]]
id = "dev"
display_name = "Dev"
role = "Code"
access = "full"

# Research-tier — read-only by default (no `access` line needed).
[[familiar]]
id = "research"
display_name = "Research"
role = "Research"
```

### How `access` interacts with `settings.json` agents

User-defined agents in `.coven-code/agents/*.md` or `settings.json` continue to win on id collisions. Familiars are merged after the built-in `build` / `plan` / `explore` agents, before any user-defined agents — so a workspace override of the same name shadows the familiar entirely (including its `access` value).

---

## Overriding a familiar with a workspace agent

To customise a familiar's behaviour for a specific project, create a `.coven-code/agents/<id>.md` file that matches the familiar's display name. Workspace agents take precedence over familiar-sourced definitions with the same name:

```markdown
---
name: Dev
description: Dev customised for this monorepo
model: anthropic/claude-sonnet-4-6
---

You are 🤖 Dev, operating inside the my-monorepo project.
Prioritise TypeScript consistency and follow the project's
contributing guide for all code changes.
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

## Testing Familiar Contract adherence

Coven Code can run a familiar from `~/.coven/familiars.toml`, but a roster entry
alone is not a full Familiar Contract package. To claim adherence to the
[Familiar Contract](https://github.com/OpenCoven/familiar-contract), keep the
familiar's identity bundle in a directory with the contract artifacts:

| Artifact | Contract role |
|---|---|
| `SOUL.md` | Named Identity and Defined Purpose: name, pronouns, core work, what the familiar is not, and boundaries |
| `IDENTITY.md` | Stable machine-readable identity record |
| `MEMORY.md` | Persistent Memory: curated continuity across sessions |
| `ward.toml` | Bounded Authority and Human Belonging: protected surface, editable surface, approval tiers, familiar/person binding |

Run the upstream validator from `github.com/OpenCoven/familiar-contract` against
that directory:

```bash
git clone https://github.com/OpenCoven/familiar-contract .tmp/familiar-contract
cd .tmp/familiar-contract
node validators/validate.js ../../familiars/dev
```

The validator checks structural compliance for all five required properties:

1. Named Identity
2. Defined Purpose
3. Bounded Authority
4. Persistent Memory
5. Human Belonging

Add the validator to CI so contract drift fails before merge:

```yaml
# .github/workflows/familiar-contract.yml
name: familiar-contract

on:
  pull_request:
  push:
    branches: [main]

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
      - name: Fetch Familiar Contract validator
        run: git clone --depth 1 https://github.com/OpenCoven/familiar-contract .tmp/familiar-contract
      - name: Validate familiar package
        run: cd .tmp/familiar-contract && node validators/validate.js ../../familiars/dev
```

Structural compliance is necessary but not sufficient. Also test behavioral
compliance in Coven Code by activating the familiar and asking it to state its
name, purpose, boundaries, and person binding:

```bash
coven-code --agent "Dev" --print "State your name, purpose, boundaries, and who you belong to."
```

The response should match `SOUL.md`, `IDENTITY.md`, and `ward.toml`. If it
contradicts those files, that is a behavioral compliance failure even when the
validator passes.

Protected-surface changes require human approval. Treat proposed edits to
`SOUL.md`, `IDENTITY.md`, `MEMORY.md`, `ward.toml`, person binding, or the
five-property compliance claim as blocked unless explicitly authorized by the
person or team the familiar belongs to.

---

## Familiar cards in the TUI

Every saved familiar from `~/.coven/familiars.toml` renders as a **static themed card** in three places:

1. The **welcome panel** (top-left of the home screen): glyph, name, access tier dot, and on wider terminals the role and an accent rule.
2. The **F2 switcher popup**: one row per saved familiar, each painted in that familiar's accent palette with a coloured tier dot.
3. The **`/agents` detail view**: the card appears above the persona preview when you select a familiar-sourced agent.

The glyph is a procedural sigil framing the familiar's emoji. Its accent colour pulses gently while idle and pulses faster when the assistant has gone quiet for ~3 seconds, so you get a "thinking" signal without a walking mascot pulling attention from the work area.

Cards adapt to available room:

- **Compact** (narrow terminals): glyph only, no border.
- **Standard** (default): glyph + name + tier dot inside a rounded border.
- **Large** (wide terminals): adds the role line and an accent rule under the glyph.

### Procedural glyphs

There is **no built-in roster** — nothing ships with a named familiar, so a
fresh install never inherits one. Every familiar declared in
`~/.coven/familiars.toml` automatically gets a procedurally-generated card. The
accent palette and sigil frame (crystal, hexagon, rune, or seal) are picked
deterministically from the familiar's `id`, so the same familiar looks the same
across sessions and machines without storing extra config. The familiar's
`emoji` is rendered inside the frame.

If you want a hand-crafted image instead of the procedural sigil, drop a PNG/JPG/WebP at `~/.coven/assets/familiars/<id>.<ext>`. When the terminal supports Kitty or Sixel inline graphics, that image takes precedence over the card.

### Changing the displayed glyph

Set `familiar` in your settings:

```json
{
  "familiar": "dev"
}
```

Or run:

```
coven-code config set familiar dev
```

When `~/.coven/familiars.toml` contains saved familiars, you can also press
**F2** to open the switcher popup and pick a familiar interactively.
The welcome panel and footer only show a familiar when the selected id exists
in `~/.coven/familiars.toml`; stale or reset familiar settings render as
`Familiar: none`.

Every switching surface — the `/familiar` command, the F2 popup, and the
familiars/agents menu — performs the same full activation: it changes the
mascot, persists the choice to `~/.coven-code/settings.json`, and activates
the familiar's agent definition so the session's tool list is re-filtered to
the familiar's access tier (`full`, `read-only`, or `search-only`; omitted or
unknown tiers fail closed to `read-only`).

To erase custom familiars and reset the agent roster, open `/familiar` and
choose **Reset familiars and agents**, run `/familiar reset-roster`, or run
`coven-code agents reset`. This removes `~/.coven/familiars.toml`, custom
agent markdown files, and saved agent/familiar settings. After reset the
welcome panel renders `Familiar: none`, the footer shows no familiar label,
and the F2 familiar switcher does not open until a saved familiar roster
exists again. The `/familiar` command only selects familiars from
`~/.coven/familiars.toml`; stale settings are ignored when the roster file is
absent.

---

## See also

- [Agents and Multi-Agent Features](agents) — workspace agents, coordinator mode, managed agents
- [Configuration](configuration) — `settings.json` reference
- [Coven daemon documentation](https://opencoven.ai/docs) — managing familiars, skills, and the full Coven ecosystem
