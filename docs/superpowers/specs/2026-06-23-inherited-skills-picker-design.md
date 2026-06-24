# Inherited skills + interactive `/skills` picker

Date: 2026-06-23
Status: Approved (design), implementing

## Problem

coven-code only discovers skills under its own `.coven-code/` and `.agents/`
paths plus the in-binary bundled set. The standard Claude Code skill set
(brainstorming, using-superpowers, dispatching-parallel-agents, the higgsfield
family, …) lives under `~/.claude/` and in installed plugin repos, and OpenAI
Codex custom prompts live under `~/.codex/prompts/`. None of these surface in
coven-code today. We also lack the interactive, searchable, toggleable
`/skills` picker that other environments show.

## Goals

1. **Inherit skills from other environments.** Discover skills from Claude
   (`~/.claude/skills/`, `.claude/skills/`, `~/.claude/plugins/*/skills/`),
   Codex (`~/.codex/prompts/`), the coven plugin registry, and keep the existing
   `.coven-code/` / `.agents/` / config-path / git-url sources — merged and
   deduped by name.
2. **Interactive `/skills` picker** matching the reference UI: a search box, one
   row per skill showing on/off state, scope label, and a `~NN tok` estimate,
   with a selection cursor and scroll affordance.
3. **Persisted enable/disable.** Toggling a skill off persists and removes it
   from both the model-facing skill list and the always-on skill index, so
   disabling genuinely reclaims context tokens.

## Non-goals

- Running/invoking a skill from the picker (toggle-management only).
- A new fuzzy-match dependency (case-insensitive substring is enough, matching
  the existing model picker).
- A real BPE tokenizer dependency (use the repo's existing estimation approach).

## Design

### 1. Skill discovery (`crates/core/src/skill_discovery.rs`)

`DiscoveredSkill` gains:

- `scope: SkillScope` — new enum `{ Bundled, Project, User, Plugin }`.
- `origin: String` — human label of where it came from: `"claude"`, `"codex"`,
  `"coven"`, or the plugin directory name. Used only for diagnostics/tooltip;
  the picker renders the `scope` word.
- `est_tokens: usize` — estimated always-on context cost (see §2).

New roots scanned, in priority order (first-match-wins dedupe by name; a
higher-priority source keeps the entry):

| Priority | Scope   | Roots |
|----------|---------|-------|
| 1 | Project | `.coven-code/skills/`, `.agents/skills/`, `.claude/skills/` (walk up from cwd) |
| 2 | User    | `~/.coven-code/skills/`, `~/.claude/skills/`, `~/.codex/prompts/` |
| 3 | Plugin  | `~/.claude/plugins/*/skills/` and coven plugin-registry skill paths |
| 4 | Bundled | in-binary `BUNDLED_SKILLS` (merged at the listing layer, not in this fn) |
| —  | Config  | existing `SkillsConfig.paths` (User scope) and `urls` (User scope) |

Two on-disk layouts:

- **Directory layout** (Claude / superpowers / plugins): a skill is a directory
  containing `SKILL.md` with YAML frontmatter (`name`, `description`, optional
  `when-to-use` / `when_to_use`). The skill name defaults to the directory name.
  Scanning is one level deep per root: for each child dir, read `<child>/SKILL.md`.
- **Flat layout** (existing coven `.md`, Codex `~/.codex/prompts/*.md`): a single
  `.md` file. Codex prompts have no frontmatter → name = file stem, description =
  first non-empty body line (truncated), the rest is the template.

Implementation: `parse_skill_file` extended to also read `when_to_use`. A new
`scan_skill_root(dir, scope, origin)` handles both layouts and stamps
`scope`/`origin`. `discover_skills` adds the new roots. Existing tests keep
passing; new tests cover SKILL.md dirs, Codex flat prompts, and scope stamping.

### 2. Token estimate

The `~NN tok` figure is the **always-on cost** of the skill — the text injected
into the system prompt's skill index for it: `name + description + when_to_use`.
(This is why long-described skills like higgsfield read ~300+ while terse ones
read ~70.) Estimate with `est_skill_tokens(&DiscoveredSkill)` using the repo's
existing estimation convention from `token_budget.rs` (calibrated `chars/4`).
Rendered as `~{n} tok`.

### 3. Persistence (`crates/core/src/lib.rs` `Settings`)

Add:

```rust
#[serde(default, rename = "disabledSkills")]
pub disabled_skills: std::collections::HashSet<String>,
```

mirroring the existing `disabledPlugins`. A skill is enabled unless its name is
in this set. Toggling in the picker mutates the set and calls `save`.

Filtering points:

- `bundled_skills::user_invocable_skills()` and `skill_tool::list_skills()` skip
  disabled names (model-facing list).
- Wherever the always-on skill index is built into the system prompt, skip
  disabled names (token reclamation). Confirmed during implementation.

### 4. Picker overlay (`crates/tui/src/skills_picker.rs`, new)

Mirrors `effort_picker.rs` (modal/render) + `model_picker.rs` (filter box).

```rust
pub struct SkillRow {
    pub name: String,
    pub scope_label: &'static str, // "user" | "project" | "plugin" | "builtin"
    pub est_tokens: usize,
    pub enabled: bool,
}

pub struct SkillsPickerState {
    pub visible: bool,
    pub selected: usize,
    pub filter: String,
    pub scroll: usize,
    pub rows: Vec<SkillRow>,
}
```

- `open(rows)` populates and shows; `close()` hides.
- `filtered()` → indices whose name/scope contains the lowercased filter.
- Navigation clamps `selected` into the filtered set and adjusts `scroll`.
- Render: title ` Skills `; first body line is the search box
  `⌕ Search skills…` (shows typed filter); then a viewport of rows:
  `{› | space}{✓ on | ✗ off}  {name}  · {scope} · ~{tok} tok`, selected row in
  accent. Footer shows `↑ N above` / `↓ N more below` when clipped and the
  keybinding hint.
- Colors from `overlays.rs` palette (accent purple).

### 5. Wiring

- `App` gains `skills_picker: SkillsPickerState` (init in constructor).
- `render.rs`: render the picker after the effort_picker block.
- `handle_input`: guarded arm when `skills_picker.visible`:
  typing/backspace edits filter; `↑/↓` + `Ctrl-p/n` navigate; `Space`/`Enter`
  toggle the selected skill's enabled state (update `Settings.disabled_skills`,
  save, update row); `Esc` closes.
- `/skills` command (`crates/commands/src/lib.rs`): with no args opens the
  picker (builds rows from bundled + discovered, minus dedupe, marking enabled
  from settings); `/skills <query>` opens pre-filtered. The model-facing
  `SkillTool` `list` path is unchanged except for the disabled filter.

Building rows requires the discovered-skill set + settings on the TUI side. The
command handler sets a signal/opens the picker via `App` (matching how
`/effort` calls `self.effort_picker.open(...)`).

## Testing

- Unit: `parse_skill_file` with `when_to_use`; `scan_skill_root` for both
  layouts; `discover_skills` includes Claude/Codex/plugin roots and stamps
  scope; dedupe priority; `est_skill_tokens` monotonicity.
- Unit: `SkillsPickerState` filter/navigation/toggle logic (pure, no TTY).
- `cargo build` + `cargo test` across the workspace.
- Manual: launch TUI, `/skills`, confirm inherited skills appear with scope and
  token columns, toggle persists across restart.

## Risks / notes

- Plugin glob `~/.claude/plugins/*/skills/` — scan each immediate subdir of
  `~/.claude/plugins/` for a `skills/` child. Cheap, bounded.
- `SKILL.md` bodies can be large; we only token-estimate metadata, not bodies,
  so discovery stays cheap.
- 10 concurrent claude sessions on this checkout: additive commits on a
  dedicated branch only — no rebases/force-pushes/pulls.
