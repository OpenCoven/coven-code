# TUI v2 dependency and feature-forwarding review

For [#408](https://github.com/OpenCoven/coven-code/issues/408), acceptance item
"New dependencies and feature forwarding are justified against current Cargo
manifests, including voice and intentional experimental flags".
Reviewed 2026-11-16 against main 04d6c0f8
(`04d6c0f8f8013d0ab6d8d0242221ff38c88d9e6a`).
Read with the [capability matrix](capability-matrix.md) and
[compatibility matrix](compatibility-matrix.md).

TUI v2 here means the typed presentation seam from `bf861665` (#426, 2026-11-01)
and later work in `src-rust/crates/tui/src/application/` and
`src-rust/crates/tui/src/next/`. This document changes no manifest, lockfile or
code. Paths are relative to the repository root. `Cargo.toml:N` without a crate
means the workspace manifest `src-rust/Cargo.toml`.

## Summary

- Neither the seam nor any later TUI v2 PR added, removed or bumped a dependency.
  Every external crate the TUI v2 code uses was declared in
  `src-rust/crates/tui/Cargo.toml` before the seam.
- Ten first-parent commits since the seam changed a manifest or `Cargo.lock`:
  release version stamps, Dependabot bumps, the v0.9.1 remediation (#461), a
  stream-input fix (#475) and an ACP test dependency (#404).
- No Cargo feature gates code in `application/` or `next/`. The renderer is
  chosen at run time by `COVEN_CODE_UI`, and both renderers are compiled in
  every feature combination.
- [Findings](#findings) lists three forwarding decisions and three notes.

## Method

Run from the repository root unless `cd src-rust` is shown. This review compiled
no feature combination. Every line number is at 04d6c0f8. The change that adds
this document also edits `src-rust/crates/tui/src/app.rs` and
`src-rust/crates/cli/src/main.rs`, so later lines in those two files move.

```sh
git show --stat bf861665
git log --format='%h %ad %p | %s' --date=short bf861665^..origin/main -- \
  src-rust/Cargo.toml 'src-rust/crates/*/Cargo.toml' src-rust/Cargo.lock
git log --first-parent --format='%h %ad %p | %s' --date=short bf861665^..origin/main -- \
  src-rust/Cargo.toml 'src-rust/crates/*/Cargo.toml' src-rust/Cargo.lock
git log --no-merges --full-history --format='%h %ad | %s' --date=short bf861665^..origin/main -- \
  src-rust/Cargo.toml 'src-rust/crates/*/Cargo.toml' src-rust/Cargo.lock
git diff --stat <merge>^1 <merge> -- <same manifest paths>      # each first-parent merge
git show --format= -U0 <sha> -- <same manifest paths>           # each non-merge commit
git show --format= --ignore-cr-at-eol 85ba0525 -- src-rust/crates/{mcp,tools,tui}/Cargo.toml
git diff --ignore-cr-at-eol -U0 bf861665 origin/main -- src-rust/Cargo.toml 'src-rust/crates/*/Cargo.toml'
git diff bf861665 origin/main -- src-rust/Cargo.lock | grep -E '^[-+]name = '
git log --first-parent --format=%h bf861665..origin/main -- src-rust/crates/tui   # then, per commit:
git diff --name-only <c>^1 <c> -- src-rust/Cargo.toml 'src-rust/crates/*/Cargo.toml' src-rust/Cargo.lock
git log --format='%h %ad %s' --date=short -S'<manifest line>' -- <manifest>  # origin of each line
grep -nE '^\s*(pub(\([a-z]+\))? )?use ' src-rust/crates/tui/src/{application,next}/*.rs
grep -nE '\b(crossterm|serde_json|reqwest|unicode_width|tracing|chrono|claurst_[a-z]+)::' \
  src-rust/crates/tui/src/{application,next}/*.rs src-rust/crates/tui/src/application/research/*.rs
grep -rn --include='*.rs' -E 'feature *= *"' src-rust/crates
grep -rn -E -- '--features|--all-features|--no-default-features' .github scripts
cd src-rust && cargo metadata --no-deps --format-version 1 | jq '.packages[] | {name, features}'
cd src-rust && cargo tree -p claurst -e normal -f '{p} [{f}]' --depth 1   # also with --no-default-features,
                                   # --features steer, --features claurst-tui/token_budget
cd src-rust && cargo tree --workspace -e normal -f '{p} [{f}]' --depth 0
cd src-rust && cargo tree -p claurst -i cpal -e normal --depth 1           # and with --no-default-features
cd src-rust && cargo tree -p claurst-tui -e normal -f '{p} [{f}]' --depth 1
```

Cargo ran as `rustup run 1.100.0 cargo <args> --locked --offline` with a scratch
`HOME`.

## Manifest changes since the seam

`git show --stat bf861665` lists 10 files: `cli/src/main.rs`, two CLI tests,
`goal_dispatch_tests.rs`, `tui/src/app.rs`, `tui/src/lib.rs`,
`application/mod.rs`, `application/view.rs`, `next/mod.rs` and
`tui/tests/presentation_parity.rs`. None is a manifest or the lockfile.

The table lists every first-parent commit in `bf861665^..origin/main` that changed
a manifest or `Cargo.lock`. The "Change" column comes from the non-merge commits
the merge brought in. `--no-merges --full-history` returns exactly these 11
non-merge commits. Any first-parent commit not listed here changed no manifest.

| First-parent | Date | Non-merge commit | Change | TUI v2 work |
|---|---|---|---|---|
| `d23b1eab` (direct) | 2026-11-08 | same | Workspace version 0.8.0 to 0.9.0 (`Cargo.toml:23`) and lock stamps | No, release |
| `1fbe8e20` #404 | 2026-11-10 | `2c9adb36` (2026-10-18) | acp dev-dependency `async-trait` (`src-rust/crates/acp/Cargo.toml:28`) | No, ACP |
| `b8f22148` #461 | 2026-11-13 | `85ba0525` (2026-11-10) | Version 0.9.1. `tokio-util` "0.7" to "0.7.19" (`Cargo.toml:102`; lock 0.7.18 to 0.7.19). Tools gains `sha2` (`src-rust/crates/tools/Cargo.toml:15`). Mcp gains a unix dev-dependency `nix` (`src-rust/crates/mcp/Cargo.toml:30-31`). Tui `ratatui` gains `unstable-rendered-line-info` (`src-rust/crates/tui/Cargo.toml:66`). The mcp, tools and tui manifests went from CRLF to LF line endings; nothing else in them changed | Mixed, see below |
| `31e15119` #475 | 2026-11-14 | `bf0d1ee9` | Cli gains `image` (`src-rust/crates/cli/Cargo.toml:46`) | No, stream input |
| `83e48d8d` #477 | 2026-11-15 | `9df40aeb` (2026-11-14) | Version 0.9.1 to 0.9.2 (`Cargo.toml:23`) | No, release |
| `272768ff` #467 | 2026-11-15 | `829742e3` (2026-11-13) | Lock only: `arboard` 3.6.1 to 3.7.0, drops `windows-sys` 0.60.2 | No, commands |
| `666d13bb` #464 | 2026-11-16 | `288881db` (2026-11-13) | `getrandom` "0.2" to "0.4" (`Cargo.toml:79`); core and mcp edges move to 0.4.2 | No |
| `6bd18ff0` #492 | 2026-11-16 | `7ec6c079` (2026-11-11, lock only), `43372c99` (2026-11-14) | `coven-runtime-spec` and `coven-runtime-registry` tag v0.1.3 to v0.2.0 (`Cargo.toml:111-112`) | No, commands/cli |
| `eb2c3944` #466 | 2026-11-16 | `fa9276b3` | `toml` "0.8" to "1.1" (`Cargo.toml:42`). Lock 0.8.23 to 1.2.1+spec-1.1.0; the commit title says 1.1.3 | No, core/plugins |
| `04d6c0f8` #463 | 2026-11-16 | `169c993e` (2026-11-11) | `which` "7" to "8" (`Cargo.toml:70`). Lock 7.0.3 to 8.0.6; drops `env_home` and `winsafe` | No, core |

`85ba0525` ("Prepare v0.9.1 release fixes", fixes #437 and #443) is a release
commit. It is the only commit in the range that changed both TUI v2 code
(`application/research.rs`, `next/mod.rs`) and a manifest:

- The `ratatui` feature serves `Paragraph::line_count` in Classic's MCP view
  (`src-rust/crates/tui/src/mcp_view.rs:706`). That is the only `.line_count(`
  or `.line_width(` call under `src-rust/crates`. TUI v2 code does not use it.
- The tools `sha2` edge computes `body_sha256` in
  `src-rust/crates/tools/src/web_fetch.rs:487`. The Next Research page shows that
  value (`src-rust/crates/tui/src/next/mod.rs:2029`). The edge belongs to
  claurst-tools, not claurst-tui. Before `85ba0525`, `sha2` was already a
  dependency of core, bridge and plugins, so the lockfile gained an edge, not a
  package.

After the seam, 25 first-parent commits touched `src-rust/crates/tui`. Apart
from `b8f22148` (#461), none changed a manifest. That covers the TUI v2 PRs #427,
#429, #430, #431, #432, #434, #436, #438, #440, #444, #445, #447, #448, #450,
#456, #458, #485, #494 and #495, and the other PRs #454, #469 (which also edited
`next/`), #473, #478 and #481. The #408 PRs #493 and #496 touched only CLI files
and scripts, and changed no manifest either.

Net change from `bf861665` to `04d6c0f8`, ignoring line endings:

- Workspace version, `toml`, `which`, `getrandom`, `tokio-util` and the two
  `coven-runtime-*` tags, as in the table.
- One new edge in each of four crates: acp `async-trait` (dev), cli `image`,
  mcp `nix` (unix dev), tools `sha2`.
- One feature on an existing tui dependency: `ratatui`
  `unstable-rendered-line-info`.
- `Cargo.lock` went from 619 to 613 packages. Removed: `env_home` 0.1.0,
  `winsafe` 0.0.19, `toml_write` 0.1.2, and the older duplicates
  `toml_datetime` 0.6.11, `toml_edit` 0.22.27, `winnow` 0.7.15 and
  `windows-sys` 0.60.2. Added: `toml_writer` 1.2.0+spec-1.1.0.

## What the TUI v2 code depends on

### External crates

Each row is declared in `src-rust/crates/tui/Cargo.toml` as `{ workspace = true }`.
Resolved versions come from `cargo tree -p claurst-tui`. "Since" is the commit that
added the tui manifest line, found with `git log -S`. Every one predates the seam.

| Crate (resolved) | Declared / workspace | Used in TUI v2 | Why | Since | Alternative |
|---|---|---|---|---|---|
| ratatui 0.29.0 | tui `:66` / `Cargo.toml:48` | `next/mod.rs:32-35` (`Frame`, `Rect`, `Line`, `Span`, `Clear`, `Paragraph`); `next/primitives.rs:11`; `next/style.rs:14`; `next/layout.rs:15`; `render_next` at `next/mod.rs:821` | Next draws into the same terminal as Classic. The CLI calls `render_app` or `render_next` inside one `terminal.draw` (`cli/src/main.rs:3593-3598`) | `40717b88` | None practical: a second terminal stack would duplicate setup and restore |
| unicode-width 0.2.0 | tui `:68` / `Cargo.toml:95` | `next/mod.rs:1107` (`composer_cursor`), `:1181` (`workspace_line`), `:2287` (`wrap_line`) | Cell width of each grapheme at a wrap or cursor point | `40717b88` | None needed: ratatui 0.29.0 depends on the same 0.2.0, so no extra package |
| serde_json 1.0.149 | tui `:63` / `Cargo.toml:41` | `application/view.rs:155-171` (`tool_target`); test-only at `application/research.rs:512-514` and `application/research/provenance_tests.rs:32` | A tool call's input arrives as JSON text. `tool_target` reads one field and returns `Option<String>`; the `Value` stays inside the function | `40717b88` | None practical |
| reqwest 0.13.3 (`Url` only) | tui `:76` / `Cargo.toml:37` | `application/research.rs:407`, `:431` (`display_url`); tests `:664`, `:683`, `:880`, `:912` | URLs are redacted using the same parser the HTTP client uses (doc comment `research.rs:394-404`; #456, `23f38b65`). `reqwest` re-exports `url::Url` (`reqwest-0.13.3/src/lib.rs:282`); web fetch also uses `reqwest::Url` (`src-rust/crates/tools/src/web_fetch/network.rs:7`) | `195eb4e1` | `url` directly (`Cargo.toml:82`, already locked): same type, but one more tui manifest line and no gain |
| tracing 0.1.44 | tui `:65` / `Cargo.toml:56` | `application/mod.rs:58` (debug line when a stale-generation event is discarded) | Diagnostics | `40717b88` | None needed |
| chrono 0.4.44 | tui `:69` / `Cargo.toml:61` | Tests only: `application/work.rs:275` | Fixed timestamps for goal and task fixtures | `40717b88` | Not applicable |
| crossterm 0.29.0 | tui `:67` / `Cargo.toml:49` | Not imported in `application/` or `next/`. `setup_terminal(renderer)` (`tui/src/lib.rs:296`) wraps `CrosstermBackend` in `QuietBackend`, which is quiet only for Next (`lib.rs:361-362`). Next tests send keys with `crossterm::event` (`tui/tests/next_memory.rs:17`, `tui/tests/next_hostile_output.rs:457`) | Shared terminal backend and input events | `40717b88` | None practical |

`QuietBackend` (`src-rust/crates/tui/src/quiet_backend.rs:15-19`, added by
`2dcedc88`, #444) is Next-only code outside the two directories. It uses only
ratatui and std. Outside tests, `next/tracked.rs` uses only std, and
`application/memory.rs` and `application/work.rs` use only claurst crates and std.

### Internal crates

All five edges are declared in `src-rust/crates/tui/Cargo.toml:56-60` and date
from `ce4650d9` (2026-04-04).

| Crate | Types that cross the seam |
|---|---|
| claurst-core (`:56`, `Cargo.toml:115`) | `types::{Message, ContentBlock, MessageContent, Role, ToolResultContent}` (`application/view.rs:15`, `application/research.rs:15`); `goal::{Goal, GoalStatus}` and `tasks::{BackgroundTask, TaskStatus}` (`application/work.rs:17-18`); `coven_daemon::{DaemonClient, DaemonError, MemoryOverview, MemorySummary}` (`application/memory.rs:12`); `appearance::Density` and `NextAppearanceSettings` (`next/mod.rs:28`, `next/style.rs:314`); `keybindings::{KeyContext, KeybindingResolver, UserKeybindings}` (`next/mod.rs:29`); `config::Settings` (`next/mod.rs:103`); `permissions::is_command_tool` (`next/mod.rs:1456`) |
| claurst-query (`:59`, `Cargo.toml:119`) | `QueryEvent`, carried in `ScopedQueryEvent` (`application/mod.rs:18`, `:67-71`); `TOOL_CANCELLED_RESULT` (`next/mod.rs:785`) |
| claurst-tools (`:58`, `Cargo.toml:118`) | `web_fetch::{Extraction, Retrieval}` (`application/research.rs:16`); `Task`, `TaskStatus` (`application/work.rs:19`) |
| claurst-api (`:57`, `Cargo.toml:117`) | Tests only: `AnthropicStreamEvent` and `streaming::ContentDelta`, used to build `QueryEvent::Stream` fixtures (`application/mod.rs:243-245`) |
| claurst-mcp (`:60`, `Cargo.toml:122`) | Not used by `application/` or `next/` |

## Feature forwarding

`cargo metadata` reports these feature counts: claurst (cli) 3, claurst-commands 2,
claurst-core 38, claurst-tools 1 and claurst-tui 37. The other seven members
declare none. The net manifest diff shows that no `[features]` table changed
since the seam.

`cargo tree` resolves the features as follows:

| Build | claurst | claurst-core | claurst-tui | claurst-commands | claurst-tools |
|---|---|---|---|---|---|
| `-p claurst` (default) | default, voice | default, ultraplan, voice | voice | none | none |
| `-p claurst --no-default-features` | none | default, ultraplan | none | none | none |
| `-p claurst --features steer` | default, steer, voice | default, ultraplan, voice | steer, voice | steer | none |
| `--workspace` (unified, as CI runs at `.github/workflows/rust-ci.yml:123-131`) | default, voice | default, ultraplan, voice | voice | none | none |
| `-p claurst-tui` alone | not built | default, ultraplan | none | not built | not built |

`cargo tree -i cpal` shows `cpal` v0.15.3 pulled in by both claurst-core and
claurst-tui in the default build, and absent with `--no-default-features`.

| Feature | Declared | Forwards to | Default | Gated code | Renderer effect |
|---|---|---|---|---|---|
| `default` | `src-rust/crates/cli/Cargo.toml:11` | `voice` | on | n/a | none |
| `voice` | cli `:12` | `claurst-core/voice`, `claurst-tui/voice` | on (cli default) | No gate in the cli crate | none |
| `steer` | cli `:15` | `claurst-commands/steer`, `claurst-tui/steer` | off | `cli/src/main.rs:3813` (`/thinking steer` in `run_interactive`, `main.rs:3025`) | Both: shared loop, no renderer condition |
| `ultraplan` | `src-rust/crates/commands/Cargo.toml:8` | `claurst-core/ultraplan` | core default makes it always on | Nothing, in any crate | none |
| `steer` | commands `:9` | `claurst-tui/steer` | off | `commands/src/catalog.rs:382-408`, `:1193`, `:1549-1551`; `commands/src/lib.rs:298`, `:5172-5196`, `:9029`, `:11189`, `:11637`, `:12431`, `:12495`, `:12897`; `commands/tests/slash_command_registry.rs:94` | none |
| `voice` | `src-rust/crates/core/Cargo.toml:10` | `dep:cpal` (`core :105-106`) | off unless cli default | `core/src/voice.rs:292-305`, `:309`, `:333`, `:391`, `:406`; `cfg_attr` at `:473`, `:536` | none |
| `default` | core `:13` | `ultraplan` | on | n/a | none |
| 35 flags | core `:16-26`, `:29-37`, `:40-54` | nothing | off except `ultraplan` | Only `cached_microcompact` (`core/src/context_collapse.rs:9`, `:55`, `:87`, `:98`, `:119`, `:154`, `:165`) | none |
| `dev_full` | core `:57-71` | `voice` and all 35 flags | off | Nothing gates on it; no crate forwards it | none |
| `voice` | `src-rust/crates/tui/Cargo.toml:8` | `claurst-core/voice`, `dep:cpal` (`tui :81-82`) | off unless cli default | `tui/src/voice_capture.rs:41`, `:78`, `:135` | none |
| `steer` | tui `:12` | nothing | off | Nothing in claurst-tui | none |
| 35 pass-throughs | tui `:15-25`, `:28-36`, `:39-53` | `claurst-core/<same name>` | off | Only `token_budget`: `tui/src/app.rs:1657`, `:1669`; `tui/src/render.rs:3048` | `token_budget` shows only in Classic's footer (`render_footer`, `render.rs:2760`) |
| `computer-use` | `src-rust/crates/tools/Cargo.toml:41` | `dep:enigo`, `dep:xcap`, `dep:image` (`tools :30-32`) | off | `tools/src/computer_use.rs:183`, `:191`, `:443`, `:508`, `:575`, `:770`, `:777`; `tools/src/lib.rs:756`, `:817`, `:881`, `:1088` | none |

No `cfg(feature = ...)` appears under `application/` or `next/`. `pub mod
application;` (`src-rust/crates/tui/src/lib.rs:78`) and `pub mod next;`
(`lib.rs:150`) carry no `cfg` attribute. The renderer comes from `select_renderer`
(`application/mod.rs:205-223`). It is called once, on the interactive route
(`cli/src/main.rs:3128-3136`). No feature changes which renderer runs.

### voice

- **Declarations.** The cli default turns voice on (`cli/Cargo.toml:11`,
  added in `74af1f68`, 2026-04-18). The feature forwards to core and tui
  (`:12`). Each of those makes `cpal` "0.15" optional, declared directly rather
  than through the workspace (`core/Cargo.toml:106`, `tui/Cargo.toml:82`).
  `27a9b40f` (#225) removed `hound` from the tui feature.
- **What it gates.** In core it gates microphone capture: the availability
  check, recording and `record_audio`. Without it, the availability check
  returns `NotEnabled` (`core/src/voice.rs:303-305`), and recording sends an
  error event and fails (`:391-399`).
- **What runs regardless.** Recording is opt-in at run time through
  `COVEN_CODE_VOICE_ENABLED` or `voice_enabled`, and `COVEN_CODE_VOICE=0` turns
  it off (`app.rs:67-75` at main). `App` holds a
  `claurst_core::voice::VoiceRecorder` (`app.rs:969`). `App::poll_voice_events`
  (`app.rs:4135`) is called before every draw by the CLI loop, for both
  renderers (`cli/src/main.rs:3550`), and by `App::run` (`app.rs:8289`).
  Classic and Next both read `app.voice_recording` (`render.rs:2622`,
  `next/mod.rs:2335-2336`).
- **The tui `cpal` path.** In claurst-tui, `cpal` is used only in
  `voice_capture.rs`: the `ActiveCapture.stream` field (`:42`) and
  `VoiceRecorder::start_recording` (`:78-133`). Nothing outside
  `voice_capture.rs` refers to `voice_capture::VoiceRecorder` (`:49`). The only
  other references to the module are its declaration (`tui/src/lib.rs:192`) and
  `transcribe`/`samples_to_wav_bytes` in
  `tui/tests/voice_transcription_privacy.rs:150-151`, which are not gated.
- **Build requirements.** Voice needs ALSA headers on Linux
  (`docs/installation.md:158-159`). CI installs `libasound2-dev` before its
  workspace build (`.github/workflows/rust-ci.yml:105`).

### steer

- **Forwarding.** cli `steer` forwards to both commands and tui
  (`cli/Cargo.toml:15`). commands `steer` forwards to tui again
  (`commands/Cargo.toml:9`).
- **What it gates.** The `/thinking steer` catalog entries and `SteerCommand`
  in commands, and the dispatch in `run_interactive` (`cli/src/main.rs:3813`).
  claurst-tui declares `steer = []` (`tui/Cargo.toml:12`, comment at `:10-11`)
  but gates nothing. The last tui gate was a slash-command list entry, removed by
  `998ddd11` (#191, 2026-10-13) when the entry moved to the commands catalog.
- **Testing.** No workflow under `.github` or `scripts` passes `--features steer`.
  `generated_command_reference_is_current` runs only without steer
  (`commands/tests/slash_command_registry.rs:93-94`).

### computer-use

- **What it gates.** `computer-use` adds `enigo` (`Cargo.toml:80`), `xcap`
  (`Cargo.toml:81`) and `image` (`Cargo.toml:89`) to claurst-tools. It also
  registers the `computer` tool (`tools/src/lib.rs:756`, `:817`, `:881`).
- **The `image` edge.** `image` is already a required dependency of tui
  (`tui/Cargo.toml:79`) and cli (`cli/Cargo.toml:46`). Only the tools edge is
  optional.
- **Not in the binary.** The cli crate does not forward the feature
  (`cli/Cargo.toml:10-15`), and claurst-tools resolves with no features in every
  `-p claurst` build above.
- **CI.** The `computer-use-macos` job (`.github/workflows/rust-ci.yml:151`)
  runs `cargo clippy --package claurst-tools --all-targets --features
  computer-use` (`:167`) and the `computer_use::tests` suite (`:171`).
- **Error text.** Without the feature, the tool returns an error that tells
  users to rebuild with `--features cc-tools/computer-use`
  (`tools/src/computer_use.rs:186-187`). The package is named `claurst-tools`
  (`tools/Cargo.toml:2`).

### Pass-through experimental flags

- **The forwards resolve.** claurst-tui forwards 35 flags, each to the
  claurst-core feature of the same name. A sorted comparison of `tui
  :15-53` with `core :16-54` gives identical sets of 35, so every forward
  target exists. `cargo tree -p claurst --features claurst-tui/token_budget`
  resolves claurst-core with `token_budget`.
- **Origins.** The tui forwards date from `f0c17ead` (2026-04-03). The core
  declarations date from the same day (for example `8cff9dc7` for `dev_full`
  and `87bdba13` for `kairos_brief`). The group comments in tui (`:14`, `:27`,
  `:38`) and commands (`:7`) still use the old crate name "cc-core".
- **Reachability.** The binary does not forward them: the cli crate forwards
  only `voice` and `steer`, and commands forwards only `ultraplan`. They can be
  turned on with `-p claurst-tui` or `-p claurst-core --features <name>`, or
  with dependency-feature syntax on the cli package.
  `docs/installation.md:154-156` says not to pass internal-crate features to
  the CLI package.

| Flags | Gated code | Purpose stated in a comment? |
|---|---|---|
| `token_budget` | tui only: `load_token_budget` (`app.rs:1657-1671`) and the Classic footer (`render.rs:3048`). Core declares it and gates nothing; `core/src/token_budget.rs` compiles in every build | Yes: `app.rs:1654-1656` |
| `cached_microcompact` | core only: `context_collapse.rs`. A grep found no caller of `claurst_core::context_collapse::collapse_context` outside that file | Yes: `context_collapse.rs:6` |
| `ultraplan` | Nothing. Always on through the core default (`core :13`). The `/ultraplan` command (`commands/src/named_commands.rs:841`) compiles in every build | Only the heading "Experimental/feature-flagged capabilities" (`core :12`) |
| The other 32: `ultrathink`, `history_picker`, `message_actions`, `quick_search`, `away_summary`, `hook_prompts`, `kairos_brief`, `kairos_channels`, `lodestone`, `agent_triggers`, `agent_triggers_remote`, `extract_memories`, `verification_agent`, `builtin_explore_plan_agents`, `compaction_reminders`, `agent_memory_snapshot`, `teammem`, `bash_classifier`, `bridge_mode`, `mcp_rich_output`, `connector_text`, `unattended_retry`, `new_init`, `powershell_auto_mode`, `shot_stats`, `tree_sitter_bash`, `tree_sitter_bash_shadow`, `native_clipboard_image`, `prompt_cache_break_detection`, `ccr_auto_connect`, `ccr_mirror`, `ccr_remote_setup` | Nothing in any crate. `away_summary` and `bash_classifier` are also module names, and those modules compile in every build | No; only group headings (`core :15`, `:28`, `:39`) |

Read as intentional: `voice`, `steer`, `computer-use`, `token_budget`,
`cached_microcompact` and `dev_full`. Each has a comment giving its purpose
(`core :7-9`, `:56`; `tui :7`, `:10-11`; `cli :14`;
`tools/src/computer_use.rs:3-4`), and `computer-use` also has a CI job. The 32
flags in the last row, and the default-on `ultraplan`, have no stated purpose and
gate nothing. Removing or renaming any flag is out of scope for #408.

## Findings

1. **No action:** TUI v2 added no dependency. The only TUI v2-visible manifest
   change since the seam is the tools `sha2` edge from `85ba0525`, which feeds
   the Next Research SHA-256 field. `sha2` was already locked.
2. **Decision:** `claurst-tui/steer` (`tui/Cargo.toml:12`) gates nothing, but
   `cli/Cargo.toml:15` and `commands/Cargo.toml:9` both forward to it. Keep it
   as a harmless forward, or remove it with both forwards.
3. **Decision:** `ultraplan` is on by default (`core/Cargo.toml:13`) and gates
   nothing. Of the 35 pass-through flags, 33 gate nothing in any crate.
   `token_budget` gates only Classic's footer and has no Next counterpart.
   `cached_microcompact` gates core code with no caller outside its file. Decide
   whether to keep them as reserved names or remove them in a separate change.
4. **Decision:** `dep:cpal` in claurst-tui (`tui/Cargo.toml:8`, `:82`) compiles
   `voice_capture::VoiceRecorder`, which no code outside `voice_capture.rs`
   refers to. Running voice uses the core recorder. This is not specific to
   either renderer.
5. **Note:** No CI step builds with `--features steer`, though the
   compatibility matrix (`docs/tui-v2/compatibility-matrix.md:137`) asks for
   steer and voice builds to be exercised before they are advertised. The
   default build covers voice.
6. **Note:** The `ratatui` feature `unstable-rendered-line-info`
   (`tui/Cargo.toml:66`) serves only Classic (`mcp_view.rs:706`). ratatui marks
   it unstable, so recheck it at the next ratatui bump.
7. **Wording only:** `tools/src/computer_use.rs:186-187` names `cc-tools`, and
   the manifest comments at `tui/Cargo.toml:14`, `:27`, `:38` and
   `commands/Cargo.toml:7` name `cc-core`.

No optional dependency is used by only one renderer.
