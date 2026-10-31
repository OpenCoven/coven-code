# TUI v2 research evidence: October 27 reconciliation and dated product review

Issue: [#420](https://github.com/OpenCoven/coven-code/issues/420), under
[epic #406](https://github.com/OpenCoven/coven-code/issues/406). Base: `main` at
`4bbef53e5db6790ed49802fb7f2adcc25f64bb19`. Access date for every external source:
2026-10-30. Status: proposed for review. These are advisory inputs to #410 and
#414 (and to the other #406 children named below). They do not gate #408 or #409.

Read with the #407 baseline from [PR #421](https://github.com/OpenCoven/coven-code/pull/421)
(head `022238ceddfe7fa4a1d304d7e8ee3d8dbe38cbab`, not yet on `main`):
[design specification](../superpowers/specs/2026-10-28-tui-v2-design.md),
[capability matrix](capability-matrix.md) and
[compatibility matrix](compatibility-matrix.md). Those relative links resolve once
#421 lands. Where this document and that baseline disagree, the baseline and the
#406 constraints win. Competitor behavior is evidence of a pattern that exists, not a
justification on its own.

## Rules applied

- Primary sources only: vendor documentation, vendor repositories (README, CHANGELOG,
  releases) and vendor release notes or announcements. Third-party blogs and news are
  not evidence. Vendor blog posts are labeled `announcement`.
- Evidence types: `shipped` (documented or in a released version without an
  experimental label), `experiment` (flag-gated, beta, research preview or
  experimental label), `announcement` (vendor post, not a reference doc), `proposal`
  (open issue or RFC). No proposal-type sources were reviewed.
- Anything not confirmed from a fetched primary page is marked `unverified`.
- No rankings. A product appearing more often below reflects documentation depth, not
  quality.
- Concept images and competitor screenshots are inspiration only. They do not
  override Brand, the #407 capability baseline or #409's terminal translation of
  Brand roles, and none were used as evidence here.

## 1. October 27 research: locations checked

Outcome: **not located**. No artifact matching "October 27 expanded TUI market/product
research" was found. It is recorded as unavailable, and section 2 replaces it.

| Location | Method | Result |
|---|---|---|
| `~/.coven/cave/research-missions/` (63 entries) | Listed by mtime; read `mission.json` `createdAt`/`title` for all missions created on or after 2026-10-20; grepped `mission.json` and `findings.md` for `tui`, `grok build`, `opencode` | Only one mission created after 2026-10-20 (2026-10-30, unrelated topic). The other recently modified mission was created 2026-09-10 (unrelated). Term matches were August missions on unrelated topics |
| `~/.coven/research/` | Listed by mtime | Newest file modified 2026-10-07; nothing from 2026-10-25..29 |
| `~/.coven/cave/research-context-packs`, `research-generations`, `research-mission-backups`, `research-resources` | `find -newermt 2026-10-25 ! -newermt 2026-10-30` | 0 files in each |
| `~/.coven/cave/research-links.json` | mtime | Last modified 2026-10-11 |
| `~/.coven` `.md`/`.json`/`.jsonl` modified 2026-10-25..29 (450 files) | `grep -liE 'grok build\|opencode\|\btui\b'` | 20 hits, all Cave conversations (`~/.coven/cave/conversations/`). Inspected: signal sweeps and blocker threads quoting Coven adapter source (the Grok Build harness recipe), and the 2026-10-28 conversation that created #406/#420, which itself records the report as missing |
| Cave conversations created 2026-10-26..28 | Listed titles; grepped for `tui v2`, `terminal UI`, `concept image`, `coven terminal` | Two hits: the 2026-10-28 #406/#420 planning conversation and one unrelated thread. No research report |
| `~/.coven/coven.sqlite3` (read-only `mode=ro`) | Listed tables matching research/mission/report | Only `automation_missions` (0 rows) and `sensitive_artifacts` (not opened) |
| `~/.claude/projects/` and `~/.codex/sessions/` files modified 2026-10-26..28 | grep for files mentioning all of Grok Build, OpenCode and Aider | One hit: the 2026-10-28 #406/#420 planning session transcript. None in Codex sessions |
| `~/Documents/GitHub/OpenCoven/` working copies (same filter) | grep | Coven harness-adapter docs only (`docs/HARNESS-ADAPTERS.md`, `docs/harnesses/`), not TUI research |
| This repo `docs/` and `docs/superpowers/` on `main` | grep `grok build\|opencode\|october 27` | No matches |
| #406, #407, #420, #421 comments; `gh search issues` (`October 27`, `market research TUI`); `gh search code --owner OpenCoven "Grok Build"` | Read-only `gh` | Issue search returns only #406/#420. Code search returns `OpenCoven/coven` harness docs. One #407 comment (2026-10-31, design decisions) and a Vercel bot comment on #421; no research link |

Not checked: the encrypted Cave local vault (`~/.coven/cave/local-vault.enc.json`) and
`sensitive_artifacts`, because reading them would need key material or protected
access, which #420 rules out. Remote stores (claude.ai, ChatGPT, Gemini histories)
were not checked because that needs account access. If the report was produced in one
of those, its owner must export it; until then nothing in it counts as verified.

## 2. Products reviewed

Method: the pages were fetched on 2026-10-30 by delegated review agents. The author
then cross-checked the release versions and dates below directly against the GitHub
releases API. Timestamps are UTC as published.

| Product | Version / commit (date) | Status | Primary sources |
|---|---|---|---|
| xAI Grok Build (`grok`) | No tagged release. Repo `xai-org/grok-build` at commit `2bdd1d6a6369` (2026-09-29, monorepo sync); crate `xai-grok-shell` declares `1.0.45` | Early beta (announcement 2026-05-25); open source 2026-07-15 | [docs.x.ai/build/overview](https://docs.x.ai/build/overview), [modes-and-commands](https://docs.x.ai/build/modes-and-commands), [cli/reference](https://docs.x.ai/build/cli/reference), [cli/headless-scripting](https://docs.x.ai/build/cli/headless-scripting), [settings/reference](https://docs.x.ai/build/settings/reference), [release notes](https://docs.x.ai/developers/release-notes), [x.ai/news/grok-build-cli](https://x.ai/news/grok-build-cli), [x.ai/news/grok-build-open-source](https://x.ai/news/grok-build-open-source), [repo](https://github.com/xai-org/grok-build) |
| Anthropic Claude Code | v2.1.316 (2026-10-31T01:01:35Z) | Shipped; fullscreen renderer is a research preview | [CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md), [fullscreen](https://code.claude.com/docs/en/fullscreen), [permission-modes](https://code.claude.com/docs/en/permission-modes), [sessions](https://code.claude.com/docs/en/sessions), [memory](https://code.claude.com/docs/en/memory), [accessibility](https://code.claude.com/docs/en/accessibility) |
| OpenAI Codex CLI | 0.166.0, tag `rust-v0.166.0` (2026-10-29T19:47:53Z) | Shipped; alpha pre-releases precede stable | [releases](https://github.com/openai/codex/releases), [README](https://github.com/openai/codex/blob/main/README.md), [CLI](https://learn.chatgpt.com/docs/codex/cli), [approvals and security](https://learn.chatgpt.com/docs/agent-approvals-security), [customization](https://learn.chatgpt.com/docs/cli-customization), [config reference](https://learn.chatgpt.com/docs/config-file/config-reference) |
| Google Gemini CLI | v0.66.0 (2026-10-27T20:26:28Z); preview v0.67.0-preview.0 | Shipped. Docs banner: replaced by Antigravity CLI for unpaid and Google One users on 2026-06-18 | [releases/v0.66.0](https://github.com/google-gemini/gemini-cli/releases/tag/v0.66.0), [settings](https://geminicli.com/docs/cli/settings/), [keyboard shortcuts](https://geminicli.com/docs/reference/keyboard-shortcuts/), [sandbox](https://geminicli.com/docs/cli/sandbox/), [rewind](https://geminicli.com/docs/cli/rewind/), [sessions](https://geminicli.com/docs/cli/tutorials/session-management/), [plan mode](https://geminicli.com/docs/cli/plan-mode/), [subagents](https://geminicli.com/docs/core/subagents/), [GEMINI.md](https://geminicli.com/docs/cli/gemini-md/), [themes](https://geminicli.com/docs/cli/themes/), [changelogs](https://geminicli.com/docs/changelogs/) |
| OpenCode (`anomalyco/opencode`; `sst/opencode` serves the same repo) | Two lines: v1.19.5 is GitHub "Latest" (2026-10-28T06:17:49Z); v2.2.5 is newest, not prerelease (2026-10-30T23:17:23Z) | Shipped on both lines; per-feature experimental flags | [releases](https://github.com/anomalyco/opencode/releases), [v2.2.1](https://github.com/anomalyco/opencode/releases/tag/v2.2.1), [TUI](https://opencode.ai/docs/tui/), [permissions](https://opencode.ai/docs/permissions/), [server](https://opencode.ai/docs/server/), [CLI](https://opencode.ai/docs/cli/), [agents](https://opencode.ai/docs/agents/), [tools](https://opencode.ai/docs/tools/), [rules](https://opencode.ai/docs/rules/), [themes](https://opencode.ai/docs/themes/), [v2 config](https://opencode.ai/v2/docs/cli/config) |
| Aider | v0.86.0 GitHub "Latest" (2025-08-09T17:42:19Z); HISTORY lists v0.86.1 with no date | Shipped; no release in about 14 months | [releases/v0.86.0](https://github.com/Aider-AI/aider/releases/tag/v0.86.0), [HISTORY](https://aider.chat/HISTORY.html), [commands](https://aider.chat/docs/usage/commands.html), [git](https://aider.chat/docs/git.html), [options](https://aider.chat/docs/config/options.html), [conventions](https://aider.chat/docs/usage/conventions.html) |
| Experience: Gemini Deep Research (research and citations) | No version. Help page shows only copyright 2026; vendor blog dated 2025-03-19 | Shipped (help center); blog items are announcements | [support.google.com/gemini/answer/15719111](https://support.google.com/gemini/answer/15719111?hl=en), [blog.google tips](https://blog.google/products-and-platforms/products/gemini/tips-how-to-use-deep-research/) |
| Experience: Claude memory (inspectable memory) | No version. Release notes entries dated 2026-03-02, 2026-07-10, 2026-08-25 | Shipped; import/export is experimental | [help article 11817273](https://support.claude.com/en/articles/11817273-use-claude-s-chat-search-and-memory-to-build-on-previous-context), [release notes](https://support.claude.com/en/articles/12138966-release-notes) |

ChatGPT deep research and ChatGPT Memory were planned as the experience sources, but
help.openai.com returned HTTP 403, so they were replaced by Gemini Deep Research and
Claude memory. No OpenAI help-center claim is used.

## 3. Findings register

Finding IDs are cited by the decisions in section 4. "Ver." is the version or date the
source attributes the behavior to; a dash means the source states none.

### Grok Build (beta product: `shipped` means documented in beta docs)

| ID | Finding | Type | Ver. | Source |
|---|---|---|---|---|
| GB1 | Modes cycle with Shift+Tab. Plan mode lets the agent edit only the plan file until the plan is approved. Auto mode approves "safe" tools via a classifier (when enabled). Always-approve still applies deny rules and hooks | shipped (auto: gated) | - | modes-and-commands |
| GB2 | Approve the plan, comment on steps or rewrite it before execution; parallel subagents, optionally in worktrees | announcement | 2026-05-25 | news/grok-build-cli |
| GB3 | `/resume`, `/sessions`, `/fork`, `/rewind`, `/export`, `/share`; `-r/--resume`, `-c/--continue`, `--fork-session` | shipped | - | cli/reference |
| GB4 | `/tasks`, `/workflows`, `/goal`, `/deep-research`; workflows can be disabled (`GROK_WORKFLOWS=0`) | shipped (partly gated) | - | modes-and-commands |
| GB5 | Memory (`/remember`, `/memory`, `/dream`, `grok memory clear`) is off by default; flag is named `--experimental-memory` | experiment | - | settings/reference, cli/reference |
| GB6 | `[ui] screen_mode = fullscreen\|minimal`; `/density` levels; mouse capture toggle (default off); theme `auto` follows OS. No screen-reader or reduced-motion setting documented | shipped | - | settings/reference |
| GB7 | `--no-alt-screen` runs inline; `-p/--single`; `--output-format plain\|json\|streaming-json`; `--no-auto-update` | shipped | - | cli/headless-scripting |
| GB8 | Update channels `stable\|alpha\|enterprise`; `grok update --version <V>` is the only documented rollback | shipped | - | settings/reference, cli/reference |

### Claude Code

| ID | Finding | Type | Ver. | Source |
|---|---|---|---|---|
| CC1 | Fullscreen renderer on the alt screen: fixed input, visible-only rendering, mouse. `/tui fullscreen` and `/tui default` switch mid-session and keep the conversation | experiment (research preview) | 2.1.89, `/tui` 2.1.110 | fullscreen |
| CC2 | Classic renderer kept as fallback. After a failed fullscreen start the next session uses classic; after two failures it stays classic until update or explicit `/tui fullscreen`. `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1` keeps native scrollback | shipped (within preview) | 2.1.132 | fullscreen#research-preview |
| CC3 | Some installs default to fullscreen via feature flags fetched on first use on or after 2026-05-06 | experiment | - | fullscreen |
| CC4 | Transcript mode (Ctrl+O): `/` search, `n/N`, `g/G`, jump between prompts, dump to native scrollback, open in `$EDITOR`. Scrolling up pauses follow and shows "Jump to bottom" with an "N new messages" count | shipped | - | fullscreen#search-and-review-the-conversation |
| CC5 | `/focus` quiet view: prompt, one-line tool summary with diffstat, final reply | shipped | 2.1.97 | fullscreen |
| CC6 | Context-scoped keybindings with chords (`keybindings.json`) | shipped | 2.1.18 | CHANGELOG |
| CC7 | `/diff` panel beside the conversation shows uncommitted changes live; collapsed tool results expand | shipped | 2.1.260 | CHANGELOG |
| CC8 | Six permission modes including classifier-based `auto`; status bar shows the active mode as a text label; `auto` is the starting mode from 2.1.283 | shipped | 2.1.283 | permission-modes |
| CC9 | Resume picker with search, preview, rename, worktree/project scope; fork; resume restores model, permission mode and goal. Background work that ended with the old process shows "didn't finish" | shipped | 2.1.144 | sessions |
| CC10 | `/goal` with live elapsed/turns/tokens overlay; `/tasks` lists background tasks and subagents; agent teams are a research preview behind an env flag | shipped; teams: experiment | 2.1.139, 2.1.32 | CHANGELOG |
| CC11 | `/memory` lists instruction files and auto memory, toggles auto memory and opens files for editing; UI shows "Saved N memories" / "Recalled N memories" | shipped | 2.1.280 | memory |
| CC12 | Screen-reader mode prints linear labeled text (`Permission Required:`), numbered menus, plain +/- diffs, OSC 133 markers; it forces the classic renderer. `prefersReducedMotion`; daltonized themes | shipped | 2.1.208, 2.1.30 | accessibility |
| CC13 | Terminal compatibility controls: ConPTY full-repaint, tmux mouse and `tmux -CC` notes, synchronized-output probing. `NO_COLOR` in settings `env` reaches subprocesses only | shipped | 2.1.111, 2.1.143 | fullscreen#troubleshooting, CHANGELOG |
| CC14 | Release channel `stable\|latest` selectable in `/config` | shipped | 2.1.3 | CHANGELOG |

### Codex CLI

| ID | Finding | Type | Ver. | Source |
|---|---|---|---|---|
| CX1 | Fullscreen transcript opt-in via `/tui` in 0.156.0, default in 0.157.0 | shipped | 0.156.0, 0.157.0 | releases rust-v0.156.0, rust-v0.157.0 |
| CX2 | `tui.alternate_screen = auto\|always\|never`; `auto` skips the alt screen in Zellij; respects tmux mouse settings; native scrollback for Terminal.app over SSH; `tui.raw_output_mode` | shipped | 0.157.0 | config reference |
| CX3 | Keyboard transcript selection, integrated Find; keymap contexts include `composer`, `pager`, `approval`; `/presets` numbered prompt slots | shipped | 0.164.0, 0.166.0 | releases, config reference |
| CX4 | Kitty/Sixel image previews in the transcript | shipped | 0.164.0 | releases |
| CX5 | Sandbox modes `read-only`, `workspace-write`, `danger-full-access`; `auto_review` reviewer agent fails closed; `/permissions` shows active sandbox and writable roots | shipped | - | agent-approvals-security |
| CX6 | Resume/fork keep approval and permission settings; repository filter in resume picker; plan mode restored on resume | shipped | 0.156.0, 0.165.0, 0.166.0 | releases |
| CX7 | TUI reconnection recovery including offline `/quit`; explicit launch permissions survive terminal reconnects | shipped | 0.161.0, 0.166.0 | releases |
| CX8 | `/goal` persists with pause/resume/clear (experimental in 0.129, default from 0.133) | shipped | 0.128.0, 0.133.0 | releases |
| CX9 | Web search activity stays visible in the transcript and is restored on resume | shipped | 0.136.0 | releases, CLI |
| CX10 | Memory mode and reset controls in TUI; memory v2 default for fresh profiles, with import from other agents. Config reference lists `features.memories` default off: unreconciled | shipped (conflicting docs) | 0.121.0, 0.165.0 | releases, config reference |
| CX11 | `/theme`, custom `.tmTheme` files; `tui.animations` disables shimmer/spinner; accents preserved on 16-color terminals | shipped | 0.156.0, 0.163.0 | cli-customization, releases |
| CX12 | `features.*` labeled `stable`, `experimental`, `under development`; `/experimental` menu | shipped / experiment | 0.155.0 | config reference |
| CX13 | 0.144.2 restored a prior auto-review policy after a regression; alpha pre-releases before each stable | shipped | 0.144.2 | releases |

### Gemini CLI

| ID | Finding | Type | Ver. | Source |
|---|---|---|---|---|
| GE1 | Inline rendering is the default; alternate buffer is opt-in (`ui.useAlternateBuffer=false`) | shipped | - | settings |
| GE2 | Scrollable alt-screen UI added in 0.15.0, rolled back to opt-in in 0.18.0; "Terminal Buffer" mode in 0.38.0 | shipped (with rollback) | 0.15.0, 0.18.0, 0.38.0 | changelogs |
| GE3 | Synchronized output, scrollback preserved during viewport refresh, border-bleed fix; nightlies bound re-render on width change and handle ambiguous-width glyphs | shipped (nightly: pre-release) | 0.66.0 | releases/v0.66.0 |
| GE4 | `/rewind` lists turns with files-changed count; restore conversation and code, conversation only, or code only; does not undo shell or manual edits | shipped | - | rewind |
| GE5 | `/resume` browser with timestamp, first message, turn count; retention defaults to 30 days | shipped | - | session-management |
| GE6 | Plan mode is read-only except Markdown plans; exit offers auto-accept or manual-accept edits; `write_todos` list above input with compact/full toggle (Ctrl+T) | shipped | 0.34.0 | plan-mode, task-planning |
| GE7 | `/memory show` and `/memory reload`; footer shows context-file count; `experimental.autoMemory` routes extracted memories to `/memory inbox` for review | shipped; autoMemory: experiment | 0.39.0 | gemini-md, settings |
| GE8 | `ui.accessibility.screenReader` plain-text output; colorblind themes; ANSI themes; per-item footer hiding | shipped | 0.40.0 | settings, themes |
| GE9 | Nightly, preview and stable channels; `experimental.*` settings namespace | shipped / experiment | - | changelogs, settings |

### OpenCode

| ID | Finding | Type | Ver. | Source |
|---|---|---|---|---|
| OC1 | v1 TUI: `@` file refs, `!` shell, leader-key binds, Ctrl+P command palette, `/details` toggles tool detail, `/undo` and `/redo` (git required) | shipped | v1 | tui |
| OC2 | `diff_style = auto` adapts to width, `stacked` single column; v2 settings add split/unified diffs, file tree, `opencode mini` with monochrome option | shipped | v1, v2 | tui, v2 config |
| OC3 | Permission prompt choices `once`, `always` (session-scoped, tool-suggested pattern), `reject`; deny rules still apply under `--auto` | shipped | - | permissions |
| OC4 | `opencode serve` HTTP server with OpenAPI and SSE; `opencode attach [url]` connects a TUI to a local or remote backend; basic auth via env | shipped | - | server, cli |
| OC5 | `/goal` keeps working until a reviewer confirms completion; status below the prompt with pause/resume/clear | shipped | 2.2.1 | releases/v2.2.1 |
| OC6 | Child (subagent) sessions navigable with leader + arrows | shipped | - | agents |
| OC7 | Truecolor themes with 256-color fallback; `system` theme uses ANSI 0-15; `none` inherits terminal. No NO_COLOR or accessibility section | shipped | - | themes |
| OC8 | `OPENCODE_EXPERIMENTAL_*` per-feature flags; v2 "Experiments" dialog; v1 and v2 ship in parallel | experiment | 2.2.1 | cli, v2 config |

### Aider

| ID | Finding | Type | Ver. | Source |
|---|---|---|---|---|
| AI1 | Inline prompt_toolkit chat; fancy input and watch mode disabled on dumb terminals | shipped | 0.71.0 | commands, HISTORY |
| AI2 | Git is the undo layer: auto-commit per edit with "(aider)" attribution; `/undo`, `/diff` | shipped | - | git |
| AI3 | Approvals are y/n prompts; `--yes-always`; no sandbox documented | shipped | - | options |
| AI4 | `/web <url>` adds a scraped page to chat; no citation feature | shipped | 0.24.0 | commands |
| AI5 | Conventions files loaded read-only (`/read`, `read:` config) | shipped | - | conventions |
| AI6 | `--check-update` defaults on; release notes shown on first run of a new version | shipped | - | options, HISTORY |

### Experiences

| ID | Finding | Type | Ver. | Source |
|---|---|---|---|---|
| DR1 | Deep Research drafts a plan the user can edit before "Start research" | shipped | - | answer/15719111 |
| DR2 | Runs about 5-10 minutes; user may leave; notification on completion; reports listed under "Recent" only with activity history on | shipped | - | answer/15719111 |
| DR3 | "Sites browsed" visible while running | announcement | 2025-03-19 | blog.google |
| DR4 | Export includes a works-cited section; follow-ups can update the report in place | announcement | 2025-03-19 | blog.google |
| DR5 | Gmail, Drive and NotebookLM can be added as sources once connected | shipped | - | answer/15719111 |
| CM1 | Memory listed as editable/deletable Topics in settings; edits apply to future chats | shipped | 2026-08-25 | 11817273, release notes |
| CM2 | Sensitive categories excluded unless opted in; IDs and account numbers never saved | shipped | 2026-08-25 | 11817273 |
| CM3 | Pause vs irreversible Reset; incognito chats not saved; per-chat opt-out before the first message | shipped | - | 11817273 |
| CM4 | Per-project memory; disabling "Use account memory" isolates a project | shipped | - | 11817273 |
| CM5 | Memory import/export labeled experimental | experiment | - | 11817273 |

## 4. Decisions

Each decision names its primary #406 child issue. "Adopt" means the pattern fits the
#407 baseline as is; "adapt" means it is taken only in the modified form stated;
"reject" records a conflict with a #406 constraint or the baseline. Every decision is
advisory until the owning issue accepts it. Nothing here makes UI state authoritative,
changes the Classic default or alters an engine contract.

### #410 responsive transcript/composer and navigation

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 410-1 | **Adapt.** Next supports both native-scrollback (inline) and alt-screen presentation, with an `auto` that respects known multiplexer/terminal limits | CC1, CC2, CX2, GE1, GB6, GB7 | Four products ship both modes, and two (GE2, CX1) changed the default and one rolled it back. The choice is a presentation setting, not a new engine selector; Classic stays default |
| 410-2 | **Adopt.** In-transcript search and pager motions (search, next/previous, top/bottom, jump between prompts) bound through `src-rust/crates/core/src/keybindings.rs` | CC4, CX3 | Fits the existing configurable-binding rule |
| 410-3 | **Adopt.** Scrolling away pauses auto-follow and shows a "new messages" count with a return-to-bottom action | CC4 | Preserves reading position without losing new events |
| 410-4 | **Adapt.** A quiet one-line-per-tool view becomes the Focus layout of the single model, not a separate mode or density system | CC5, GB6 | #407 defines Focus/Work/Inspect as layouts of one model |
| 410-5 | **Adapt.** Context-scoped bindings (composer, pager, approval) only as contexts of the existing keybinding system | CC6, CX3 | Avoid a parallel keymap |
| 410-6 | **Reject.** A second command palette or numbered prompt-preset slots | OC1, CX3 | #407: reuse Ctrl+K and the grouped catalog; no second palette without a migration decision |
| 410-7 | **Adapt.** Mouse support is optional and off by default; native terminal selection must keep working | CC1, GB6, CX2 | Mouse capture breaks selection in some multiplexers; #417 verifies |
| 410-8 | **Reject.** Inline image previews (Kitty/Sixel) | CX4 | #407: no terminal graphics protocol is qualified; plain `Coven` text fallback for the mark |

### #411 tools, diffs and approvals

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 411-1 | **Adapt.** Show the active permission mode as a text label (not color alone), sourced from engine state | CC8, GB1 | The label displays runtime state; the TUI never infers or grants it |
| 411-2 | **Reject.** A TUI-side classifier or reviewer that approves actions | CC8, CX5, GB1 | Authority belongs to the runtime/Threads; "TUI state is not permission" (#406) |
| 411-3 | **Adapt.** Approval choices state their exact scope (this request vs. session pattern), limited to options the engine permission contract supports | OC3 | Scope must be explicit; no client-invented grant types |
| 411-4 | **Adopt.** A view of the effective permission scope (sandbox mode, writable roots) where the engine reports it | CX5 | Read-only presentation of producer data |
| 411-5 | **Adapt.** A live diff surface is the single contextual inspector; the layout (side-by-side vs stacked) follows the #407 cell thresholds | CC7, OC2 | One inspector at a time; breakpoints in cells |
| 411-6 | **Adapt.** Rewind/undo shows files-changed counts and separates conversation from code, only where an engine checkpoint producer exists; otherwise Unavailable | GE4, AI2, OC1 | No client-local checkpoint store; GE4 shows shell and manual edits are not covered, and the UI must say so |

### #412 session/familiar navigation and reconnect

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 412-1 | **Adopt.** Searchable resume picker with preview and an explicit scope filter (project/worktree) | CC9, CX6, GE5 | Matches the #407 Resume journey; exact session ID is still required |
| 412-2 | **Adopt.** Work that ended with the previous process is labeled unfinished, never completed | CC9 | Matches Outcome unknown / Partial in the #407 taxonomy |
| 412-3 | **Adapt.** Reconnect recovery renders Disconnected then Reconciling, with no automatic mutation replay; quitting stays available while offline | CX7 | Codex shows the need; the #407 taxonomy governs behavior |
| 412-4 | **Adapt.** Resume restores goal and mode linkage only from what the session format persists; permission state comes from the engine on resume | CC9, CX6 | Preserves session formats and runtime authority |
| 412-5 | **Reject.** Attaching the TUI to a network server (HTTP + basic auth) | OC4 | Conflicts with the separate-process boundary and authenticated IPC peer binding; no new transport |
| 412-6 | **Reject.** Session share/publish links | GB3 | Publishing is outside #406 scope and risks leaking private content |

### #413 tasks, lanes and goals

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 413-1 | **Adapt.** Goal status near the composer (elapsed, turns, pause/resume/clear) on top of the existing `/coven goal`, not a new command | CC10, CX8, OC5 | Reuse the existing command; one goal surface |
| 413-2 | **Reject.** Treating a model or reviewer-agent judgment as verified goal completion | OC5, CC10 | #407: a model assertion cannot stand in for verified completion |
| 413-3 | **Adapt.** A todo/plan list with a compact/full toggle, labeled by origin (local vs Psyche) | GE6 | #407 requires local and canonical task origins to be shown separately |
| 413-4 | **Adapt.** Child-agent and lane navigation through the origin-labeled Work list | OC6, CC10 | Psyche owns lanes; no local scheduler |
| 413-5 | **Adapt.** Plan review before execution, where the plan is shown as a proposal, not committed state | GB1, GB2, GE6 | "A proposal is not committed state" (#406) |

### #414 research workbench and citations

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 414-1 | **Adapt.** An editable research plan before start, run in the background with a completion notice, through the supported task lifecycle | DR1, DR2 | #414 consumes #413's lifecycle; no separate research runner |
| 414-2 | **Adopt.** Show sources being consulted during a run, each with URL and retrieval time | DR3, CX9 | Matches the #407 Cited research journey |
| 414-3 | **Adopt.** Search and fetch activity persists in the transcript and survives resume | CX9 | Provenance must outlive the session |
| 414-4 | **Adapt.** A follow-up creates a new artifact revision rather than rewriting the report in place; an interrupted artifact stays Partial | DR4 | Keeps claim-to-evidence mapping auditable |
| 414-5 | **Reject.** Adding account sources (mail, drive) to research | DR5 | #406: no account connection without explicit configuration; no producer is qualified |

No CLI reviewed had a verified citation display (see section 5). #414's citation
design therefore rests on the #407 journey, not on competitor precedent.

### #415 read-only memory inspection

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 415-1 | **Adapt.** List memory sources by kind: engine-local instruction files separately from canonical familiar memory, read-only | CC11, GE7, AI5 | #406 requires these kinds to stay distinct |
| 415-2 | **Adapt.** An indicator that memory entered context (count and source only, no content in the default view) | CC11, GE7 | Private content stays out of toasts and titles (#407) |
| 415-3 | **Adopt.** Memory use is off or explicit by default, and sensitive categories are excluded | GB5, CM2, CM3 | Matches explicit reveal and context-selection consent |
| 415-4 | **Adopt.** Scope isolation: changing project or familiar redacts the previous scope's memory | CM4 | Matches the #407 Private memory inspection journey |

### #416 governed memory proposals

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 416-1 | **Adapt.** A review inbox for proposed memory changes, only behind operation-qualified producer contracts | GE7 (experiment) | The pattern fits proposals; it stays unavailable until #416 qualifies a producer |
| 416-2 | **Reject.** In-TUI edit, delete, clear or reset of familiar memory | CM1, CM3, GB5, CX10 | Coven Memory stays read-only; protected mutation needs producer authorization |
| 416-3 | **Reject.** Importing memory from other agents | CX10, CM5 | No client-local substitute; governed identity is owned by Familiar Contract |

### #409 Brand primitives and customization

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 409-1 | **Adapt.** Colorblind variants, a 256/16-color fallback and an inherit-terminal option, all derived from Brand semantic roles | CC12, GE8, OC7, CX11 | Keeps the existing deuteranopia palette; roles stay Brand-owned |
| 409-2 | **Reject.** Arbitrary user theme files that redefine semantic roles | CX11, OC7 | Brand owns roles; #409 permits only safe presentation customization |

### #417 accessibility, terminal safety and performance

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 417-1 | **Adopt.** A linear screen-reader mode with labeled lines, numbered choices and plain +/- diffs; if Next cannot provide it, route to Classic | CC12, GE8 | Claude Code's mode forces its classic renderer; Coven keeps Classic, so the same fallback works |
| 417-2 | **Adopt.** Wire the existing persisted `reduceMotion` setting to stop spinners and shimmer | CC12, CX11 | #407 already requires this; no second setting |
| 417-3 | **Adopt.** Synchronized output, scrollback preservation on refresh, bounded re-render on width change and ambiguous-width glyph tests | GE3, CC13 | Concrete #417 cell-test cases |
| 417-4 | **Adopt.** A terminal matrix covering tmux (mouse on/off, `-CC`), Zellij, Windows ConPTY, Terminal.app over SSH and dumb terminals | CX2, CC13, AI1 | Each is a documented failure class elsewhere |

`NO_COLOR` remains a #409/#417 requirement on its own terms. The only verified
competitor note (CC13) concerns subprocess environments, which is not evidence for
renderer behavior.

### #418 rollout and rollback

| # | Decision | Evidence | Rationale |
|---|---|---|---|
| 418-1 | **Adopt.** Automatic fallback to Classic after a failed Next start, sticky after repeated failures until the user explicitly retries | CC2, GE2 | Directly supports tested rollback with Classic as default |
| 418-2 | **Adapt.** Renderer switching at launch through the proposed `COVEN_CODE_UI`; switching mid-session only after #408 proves state parity | CC1, CX1 | Mid-session switching depends on the shared seam |
| 418-3 | **Adapt.** Labeled preview stages (preview vs stable) using existing release mechanics and #370 holds, without new channel infrastructure | CC14, CX13, GE9, GB8 | Staging evidence, not a new service |
| 418-4 | **Adapt.** A labeled experiment list for Next features, owned by the engine-side selector | CX12, OC8, GE9 | One selector; no second flag system |
| 418-5 | **Reject.** Switching the default via remotely fetched feature flags | CC3 | Classic stays default; no account or network dependency at launch |
| 418-6 | **Reject.** Update checks or auto-update on launch | AI6, GB8 | #406: no account or network action at launch without explicit configuration |

### Decision counts

| Issue | Adopt | Adapt | Reject |
|---|---|---|---|
| #409 | 0 | 1 | 1 |
| #410 | 2 | 4 | 2 |
| #411 | 1 | 4 | 1 |
| #412 | 2 | 2 | 2 |
| #413 | 0 | 4 | 1 |
| #414 | 2 | 2 | 1 |
| #415 | 2 | 2 | 0 |
| #416 | 0 | 1 | 2 |
| #417 | 4 | 0 | 0 |
| #418 | 1 | 3 | 2 |
| Total (49) | 14 | 23 | 12 |

#408 has no decisions. Its seam work does not depend on competitor evidence.

## 5. Unverified or unresolved

- Grok Build "1.0.0" (the version in Coven's adapter recipe) is not stated on any
  official xAI page; only crate `1.0.45` was found. Whether that crate builds the
  `grok` binary is unverified. There is no GA date. The beta release-notes entry has
  no day or year. `docs.x.ai/build` and `docs.x.ai/build/permissions` returned 404, so
  Grok's allow/deny rules and sandbox are unverified.
- Citation display in Claude Code, Codex CLI, Gemini CLI and OpenCode is unverified.
  Aider documents no citation feature.
- No screen-reader or reduced-motion mode is documented for Codex (only
  `tui.animations`), Grok Build or OpenCode. NO_COLOR handling is unverified for
  Codex, Gemini CLI, OpenCode and Aider.
- Codex memory default: release notes for 0.165.0 and the config reference disagree.
- Codex: a return path to inline rendering beyond `tui.alternate_screen = never` is
  unverified. The `learn.chatgpt.com` slash-command page appears to describe the app,
  not the CLI, so CLI commands are cited from release notes.
- Claude Code: how sandbox state appears in the UI is unverified. The tails of the
  permission-modes page and of the Codex config reference were not read.
- OpenCode: v2 vs v1 architecture differences and the v2.0.0 year are unverified;
  `/docs/share/` returned 404.
- Aider: the v0.86.1 date is not published.
- Gemini Deep Research: citation and in-place update details come only from a 2025
  vendor blog post (announcement). The help page has no date.
- ChatGPT deep research and ChatGPT Memory were not reviewed (HTTP 403).
- The source pages were fetched by delegated review agents. The author re-checked
  versions and dates against the GitHub API but did not re-fetch every page. A
  reviewer should spot-check the rows behind any decision before an owning issue
  relies on it.

## 6. Validation

| Check | Result |
|---|---|
| `git diff --check` (file staged intent-to-add, then unstaged) | Pass, no output |
| Repo paths referenced | `src-rust/crates/core/src/keybindings.rs` and `AGENTS.md` exist on `main`. `docs/superpowers/specs/2026-10-28-tui-v2-design.md`, `docs/tui-v2/capability-matrix.md` and `docs/tui-v2/compatibility-matrix.md` exist on the #421 branch only |
| Issue numbers | All 16 referenced (#370, #406-#418, #420, #421) resolved with `gh api repos/OpenCoven/coven-code/issues/N --jq .title` |
| External links | Fetched on 2026-10-30 by the review agents; not re-validated by a link checker |
| Not run | Cargo and the TUI were not run: this is a documentation-only change. No paid model calls, credentials or account access were used |

Follow-up: if the October 27 report surfaces, reconcile it against section 3 and
mark any claim it adds as unverified until a primary source confirms it.
