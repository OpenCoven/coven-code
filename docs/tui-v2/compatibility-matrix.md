# TUI v2 compatibility and executable-baseline matrix

For [#407](https://github.com/OpenCoven/coven-code/issues/407), inspected
2026-10-29. Engine baseline: `87239e77acfd42dcbd2492c0ba36817626124549`.
All producer SHAs are in the [capability matrix](capability-matrix.md).
Read with the [design](../superpowers/specs/2026-10-28-tui-v2-design.md).

This is a preserve/adapt/defer contract for future implementation. The Classic
executable baseline is recorded in the evidence ledger below; producer integration
was not exercised. Test names below identify coverage; only the ledger's suites
are claimed as fresh passes.
No changes to runtime, Classic, storage, environment or wire behavior are made here.

## Public invocation, streams and exits

Source owner: [CLI main](../../src-rust/crates/cli/src/main.rs),
[headless](../../src-rust/crates/cli/src/headless.rs),
[stream loop](../../src-rust/crates/cli/src/stream_mode.rs),
[engine contract fixtures](../../src-rust/crates/cli/tests/engine_contract.rs),
[ACP smoke](../../src-rust/crates/cli/tests/acp_smoke.rs).
The producer's ENGINE-CONTRACT.md v1 and this repository's
[structured headless v3 contract](../headless-contract.md) cover **different**
invocation profiles. Do not collapse them into one exit/status table.

| Surface | Existing behavior / invariant to preserve | Existing evidence; v2 gate |
|---|---|---|
| Bare `coven-code` and `coven-cave` alias | Interactive engine process; quit exits 0; no mandatory daemon/account/provider connection merely for new presentation | Existing terminal quit/startup fixtures; #410/#417 actual unconfigured launch and guard restoration |
| `--version` / `-V` | One stdout line `coven-code <semver>`; no branding banner or escape sequences | `contract_version_is_single_line_semver`; preserve fast-path ordering, including upgrade --version |
| Positional `coven-code <prompt>` | A positional prompt alone also selects headless mode (`cli.prompt.is_some()`), even without --print; never open Next for it | CLI `is_headless` routing; preserve this behavior separately from no-argument interactive launch |
| `--print` / `-p` | Positional prompt; text/JSON/stream-json output as selected; bypass TUI | `contract_print_writes_result_to_stdout_and_exits_zero`, missing-prompt/provider-failure tests; #418 exact packaged engine |
| Ordinary print exits | 0 completed, 1 error/budget failure; parser failures remain parser errors; do not add a renderer-specific exit | ENGINE-CONTRACT v1, engine fixture negative cases; 0 does not prove verified external effects |
| Long-lived stream mode | `--print --input-format stream-json --output-format stream-json`; reads lines until EOF; positional prompt ignored; flush each output frame | `contract_stream_json_turn_matches_golden_frames`, `contract_stream_json_exits_zero_on_eof_without_a_turn` and stream parser unit tests |
| Stream input | Typed user/assistant nested-message frames; legacy role/content frames; assistant prefill does not run a turn; unknown type ignored; mismatched type/role and malformed JSON retain current errors | `stream_mode.rs::parse_input_frame`; preserve actual parser, including supported behavior beyond old prose |
| Stream output | `system.init`, assistant/tool-result and terminal result envelopes; preserve field names, session IDs, error and stop semantics; no TUI chrome on stdout | Engine stream golden and Coven consumer parser; basic golden has no tool use and is not exhaustive stream conformance |
| stdout/stderr separation | Machine output remains machine output; human diagnostics/logging follow existing stderr ownership; no ANSI/progress/palette text enters ACP or JSONL | Engine fixtures; #377 diagnostics lifecycle and #372/#375 redaction remain separate open gates |
| Structured `--headless --context BRIEF --output RESULT` | No TTY/stdin prompts; tokenless v3 input, structured result; output file required before exits 0/1/3, optional on 2 | `docs/headless-contract.md`, CLI `headless.rs` and schema fixtures; v2 adapter acceptance alone is not strict conformance because `review.memory` differs |
| Structured headless exits | 0 success, 1 nonretry failure, 2 infra error, 3 needs-input; result status may be success/partial/failure/needs_input; preserve adapter mapping | Headless v3 schemas/tests; do not apply ordinary print's narrower code summary to this profile |
| Historical structured timeout retry policy | Headless v3 prose calls adapter timeout/signal infra retry-safe | Existing adapter-specific policy is not a guarantee of no effect or permission for Next to replay protected writes. Reconciliation/idempotency must be qualified separately; do not silently change adapter behavior in #408 |
| `auth status --json` | Offline local auth configuration only; no refresh/network/harness turn. 0 plus loggedIn true; 1 plus false; other combinations unavailable | Logged-in-from-env and logged-out offline engine tests; configured auth is not provider reachability |
| `acp` | Fast path, JSON-RPC 2.0 newline-delimited stdio; no TUI; preserve initialize/auth/session/prompt/cancel semantics and request IDs | ACP `server.rs`, `connection.rs`, permission/runtime/session modules and smoke/dispatch/reader tests; #379/#383 remain open; unmerged #404/#405 are not baseline |
| All CLI options | Preserve names, aliases, defaults, value parsing, short flags, repetition, conflicts and precedence | `Cli` clap definition, engine fixtures and inventory below; new UI selection cannot consume a prompt or named command |
| Top-level named commands | Preserve pre-session dispatch before clap: agents, agent, add-dir, branch, tag, pr-comments, ultraplan, stats | `named_commands.rs::NAMED_COMMANDS`; names in old comments such as ide/desktop are not registration evidence |
| Other fast paths | upgrade, auth, codex, accounts, acp, models, memory | CLI dispatcher and `memory_admin.rs`/`upgrade.rs`; TUI redesign must not intercept them |

### Exact flag inventory

The current `Cli` struct declares the following options. Preserve all of them,
including the hidden diagnostic option; short aliases and special defaults are
owned by the source definition rather than redefined by the v2 renderer.

| Long option | Short | Visible alias |
|---|---|---|
| `--print` | `-p` | `-` |
| `--model` | `-m` | `-` |
| `--permission-mode` | `-` | `-` |
| `--resume` | `-` | `-` |
| `--max-turns` | `-` | `-` |
| `--system-prompt` | `-s` | `-` |
| `--append-system-prompt` | `-` | `-` |
| `--no-claude-md` | `-` | `-` |
| `--output-format` | `-` | `-` |
| `--verbose` | `-v` | `-` |
| `--api-key` | `-` | `-` |
| `--max-tokens` | `-` | `-` |
| `--cwd` | `-` | `-` |
| `--dangerously-skip-permissions` | `-` | `--yolo` |
| `--dump-system-prompt` | `-` | `-` |
| `--mcp-config` | `-` | `-` |
| `--no-auto-compact` | `-` | `-` |
| `--auto-commits` | `-` | `-` |
| `--add-dir` | `-` | `-` |
| `--input-format` | `-` | `-` |
| `--session-id` | `-` | `-` |
| `--prefill` | `-` | `-` |
| `--effort` | `-` | `-` |
| `--thinking` | `-` | `-` |
| `--continue` | `-c` | `-` |
| `--system-prompt-file` | `-` | `-` |
| `--allowed-tools` | `-` | `-` |
| `--disallowed-tools` | `-` | `-` |
| `--betas` | `-` | `-` |
| `--disable-slash-commands` | `-` | `-` |
| `--bare` | `-` | `-` |
| `--hosted-review` | `-` | `-` |
| `--hosted-repair` | `-` | `-` |
| `--workload` | `-` | `-` |
| `--max-budget-usd` | `-` | `-` |
| `--fallback-model` | `-` | `-` |
| `--provider` | `-` | `-` |
| `--api-base` | `-` | `-` |
| `--agent` | `-A` | `-` |
| `--headless` | `-` | `-` |
| `--context` | `-` | `-` |
| `--output` | `-` | `-` |

Inventory: 42 explicit options, plus clap help/version fast paths and positional prompt.

`--resume` accepts an optional ID with `__last__` as its parser sentinel;
`--continue` selects the latest conversation. `--add-dir` repeats. Preserve
`--permission-mode` enum values and the explicit dangerous-skip/--yolo meaning.
`--context` currently implies headless behavior and changes permission handling;
that is an existing trusted-worker contract, not a new interactive authorization
path. `--hosted-repair` requires headless/context/output and conflicts with hosted
review and dangerous skip. Keep its restricted tool surface. Do not enable it
through a presentation toggle or expand its available tools.

## Environment, configuration and persistence

| Surface | Existing invariant and precise limit | Source/evidence and v2 disposition |
|---|---|---|
| `COVEN_PARENT`, `COVEN_HOME`, `COVEN_CODE_HOME`, `COVEN_CODE_TEST_HOME` | Core unit-test-only override (`#[cfg(test)]`; ignored by production library/executable) -> explicit engine home -> managed Coven home/code when set or parent=coven -> standalone ~/.coven-code (or relative fallback without OS home). Empty values are filtered | Core `config::resolve_config_home`, config-home tests and engine-contract home-scoping fixtures. Preserve resolution; no new UI data root |
| Home migration | Best-effort legacy-to-managed relocation runs before settings/auth load; explicit overrides and retained rollback data matter | `home_migration.rs`; preserve existing migration behavior, do not add a v2 session/data migration |
| Settings precedence | Shared whitelist defaults under engine-global and project settings; existing settings application semantics, not a newly invented merge | Core config source, COVEN.md and settings diagnostics/atomic tests; current settings files remain authoritative |
| Session IDs | Display shortening never changes the opaque ID passed to resume/events/goals. A `--session-id` tag is distinct from `--resume`; startup tag and generated/restored conversation identity are not interchangeable | CLI main, stream_mode, engine resume fixture, session_id_display.rs; preserve actual IDs in frames and storage |
| Persistence domains | JSON session/history/transcript, SQLite indexes, GoalStore, stash, settings, workspace agent files and existing memory state remain with existing owners | Core session_storage/history/sqlite/goal paths; core atomic regression fixtures. No TUI-local replacement DB or canonical memory index |
| Save failures | Do not equate current in-memory state with a successful durable save; future projections must retain warning/unknown state | Current paths plus open #403 persistence-warning PR; its proposed correction is not main or an excuse to fold an unreviewed runtime fix into #407 |
| Daemon socket | ENGINE-CONTRACT preserves inherited `COVEN_DAEMON_SOCKET`; current older engine DaemonClient discovers coven_home()/coven.sock on Unix and is unavailable on non-Unix | `core/src/coven_daemon.rs`; socket-inheritance fixture proves no side effect, not that every method honors an override. Do not promise Windows IPC parity from the canonical producer's named-pipe client alone |
| Daemon ledger | Opt-in `daemonLedger`, best-effort external session registration/completion; errors do not abort the coding session | ENGINE-CONTRACT endpoints and RegisterExternalSession/complete_session tests; not a daemon-owned process, lease or kill guarantee |
| Health/capabilities | Current engine health DTO retains api/coven version and process metadata, not capability fields | Existing core client vs canonical health negotiation; #412 owns the richer typed projection and must land it before new dependent operations; #408 leaves daemon health/negotiation unchanged |
| `COVEN_CODE_PROVIDER`, `COVEN_CODE_API_BASE`, `--api-key` and provider-owned credentials | Existing Claude direct/keyless local CLI and Codex transport selection stays engine-owned; no OAuth-token import from another CLI | CLI/API registry and auth fast paths; preserve configuration precedence and private storage. No new provider introduced |
| `COVEN_CODE_HOSTED_REVIEW`, `COVEN_CODE_HOSTED_REPAIR` | Isolation/tool restrictions continue to apply regardless of renderer | CLI/main/headless/memory_admin; hosted-mode stash refusals in commands tests |
| `COVEN_GIT_TOKEN` | Trusted structured-worker environment channel; absent from brief/result, clone URLs and diagnostics | Headless contract v3; do not move credential data into TUI state, logs or fixtures |
| `COVEN_CODE_GOALS=0` | Existing durable-goal opt-out remains effective | core goal.rs, GoalCommand and CLI goal dispatch; no UI switch may bypass it |
| `COVEN_CODE_VOICE=0/false` and `COVEN_CODE_VOICE_DISABLED` | Preserve existing voice opt-outs: App-level env override and core recorder kill switch have separate implementations | App::voice_explicitly_disabled, core voice.rs, lib.rs issue_49 tests; do not merge their meanings casually |
| `COVEN_CODE_KEYBOARD_ENHANCEMENT`, `COVENCAVE` | Preserve current keyboard protocol/platform/Cave handling | tui/lib.rs terminal initialization tests; no new selector repurposes these |
| `COVEN_CODE_CLAUDE_BIN`, model/endpoint overrides and remaining `COVEN_CODE_*` namespace | Existing engine-owned meaning, diagnostics/privacy and discovery remain | CLI/provider source; fixtures use fake local transport only. Do not delete env handling because it is absent from a UI screen |
| Bridge | Preserve configured startup, state/peer information and remote-session isolation; headless resolve_bridge_config returns None | CLI resolve_bridge_config and tui bridge_state.rs tests; disconnected bridge is separate from daemon reachability/authorization |

## Presentation, features and rollback

| Surface | Preserve / adapt / defer | Qualification required |
|---|---|---|
| Current commands | Preserve all active/advanced paths, active aliases, grouped routes, argument-rich forms and dynamic precedence; preserve rejection of removed paths | Capability matrix inventory; command registry/reference tests, App preflight and dispatch parity |
| Keybindings | Preserve configured action resolution; Ctrl+K uses existing palette; F2 familiar behavior and input/history/copy/reader actions remain | core keybindings.rs and golden; App palette/reader/Unicode/paste-burst tests. Add v2 bindings only through configurable system |
| Classic/Next | **Proposed** engine-owned `COVEN_CODE_UI=classic|next`; absent from baseline, Classic remains default | #408 defines unset=classic, classic=classic, next=opt-in preview (always compiled; labeled preview until #417/#418 qualify it). The value is read only on the interactive route: there an invalid value prints a stderr warning naming the accepted values and Classic starts. Headless, stream-json, ACP and named-command routes never read it, so their bytes/exits are unchanged for any value. This is a design decision for review, not current parsing |
| Legacy selector | Do not repurpose `COVEN_LEGACY_TUI` (outer Coven dispatch compatibility) for engine Classic/Next | #406 constraint; absence in inspected engine files is not proof of absence in outer Coven |
| Runtime features | Preserve build-time feature forwarding and effective defaults; no renderer makes disabled tools or paths available | CLI/core/TUI Cargo manifests. Explicitly exercise default, supported steer and voice builds before advertising them |
| Voice | Feature-gated microphone/transcription remains explicit; plain v types into composer, not recording; existing toggle input remains | lib.rs issue_49 tests, voice_capture.rs/core voice.rs; #372 sensitive transcription error redaction remains open |
| Accessibility | Preserve deuteranopia/light/dark/named alternatives and terminal fonts; adapt semantic Brand roles; text/shape focus and status cues | #409/#417 must prove actual render path, NO_COLOR, ASCII/Unicode and reduced-motion behavior; palette presence alone is insufficient. Current gaps at baseline: the persisted `reduceMotion` setting (core `Settings::reduce_motion`, settings screen toggle) exists but no renderer reads it, and `NO_COLOR` is read nowhere. #409 wires the existing setting rather than adding a second one |
| Terminal safety | Preserve restoration guard and existing input semantics; sanitize untrusted text at actual PTY boundary | #417 handles controls/OSC52/link injection, resize, keyboard/mouse, failures, panic and cleanup-capable exit; SIGKILL/host loss cannot guarantee cleanup |
| Privacy | No memory preview, diagnostic, export or provider submission by browsing alone; no hidden mutation or new durable cache | #415 producer-backed privacy canaries and scope-generation tests; deliberate render can remain in terminal captures |
| Rollback | Classic preview selection rollback; separately retain prior binary, selected managed engine pin and settings/data backup | #418 must exercise same-data Classic/Next switch and packaged binary/settings rollback; standalone binary replacement does not necessarily change Coven's selected engine |
| Version/release | No version stamp, engine.lock, generated docs/lockfiles, release hold or default switch changes in #407 | Existing #370 and COVEN.md release gates; publication/merge are separate decisions |

The inspected TUI manifest forwards these optional features; preserve their owner
semantics rather than implementing a UI-specific replacement:

`voice`, `steer`, `token_budget`, `ultraplan`, `ultrathink`, `history_picker`, `message_actions`, `quick_search`, `away_summary`, `hook_prompts`, `kairos_brief`, `kairos_channels`, `lodestone`, `agent_triggers`, `agent_triggers_remote`, `extract_memories`, `verification_agent`, `builtin_explore_plan_agents`, `cached_microcompact`, `compaction_reminders`, `agent_memory_snapshot`, `teammem`, `bash_classifier`, `bridge_mode`, `mcp_rich_output`, `connector_text`, `unattended_retry`, `new_init`, `powershell_auto_mode`, `shot_stats`, `tree_sitter_bash`, `tree_sitter_bash_shadow`, `native_clipboard_image`, `prompt_cache_break_detection`, `ccr_auto_connect`, `ccr_mirror`, `ccr_remote_setup`.

These are feature names, not a claim that every combination is supported or tested.
`voice` adds microphone dependency; `steer` changes catalog/queue behavior. Runtime
settings/environment still govern whether compiled capabilities may be used.

## Retained synthetic baseline and evidence ledger

The existing [Classic baselines](../../src-rust/crates/tui/tests/classic_baselines.rs)
render idle, streaming, permission and error states at 40x12, 80x24 and 120x40:
12 frames with both text and style maps (24 golden files). They freeze synthetic
home/project/model/frame state. They are retained source artifacts, not captures
or performance measurements produced on this host. Do not regenerate them merely
to accept an unintended change.

| Evidence | Exact scope / result | Missing proof |
|---|---|---|
| Source revisions | Engine `87239e7` and eight canonical producer trees pinned in capability matrix | Packaged binary identities (#418) |
| Catalog cross-check | Every default registry active/advanced/removed row and active alias mapped; `slash_command_registry` passes (below) | Per-command effect journeys |
| Targeted suites (2026-10-31) | `slash_command_registry` 2/2, `keybindings_golden` 1/1, `classic_baselines` 4/4, `engine_contract` 17/17, `acp_smoke` 6/6 passed with `--locked`; `uv run scripts/test-headless-schemas.py` 4/4 OK | Windows (engine contract is Unix-gated) |
| Real-binary tmux journeys (2026-10-31) | Debug `coven-code` 0.8.0 built `--locked` at `87239e7` (sha256 prefix `bdfa608d3f53167a`), `scripts/tui-tests/run.sh` with `REQUIRE_TMUX=1`, disposable `HOME` seeded `{"hasCompletedOnboarding": true}`. 120x40: 48/48 checks passed, 0 skipped. 80x24: 40/42; the help overlay wraps command rows across its columns so `/session resume` is not rendered, and the paste-burst case that waits on that overlay fails with it. Recorded as a Classic 80x24 limitation for #410, not changed here | 40x12 real-terminal journeys; permission, cancellation and resume journeys need a fake-provider PTY fixture (#417) |
| Host | macOS 27.0.1 arm64 (Apple M5 Max), tmux 3.8, `TERM=xterm-256color`, cargo 1.99.1 | Linux/Windows terminals, other emulators, font/color settings |
| Performance (2026-10-31, Classic debug build) | Cold launch to first frame containing the footer (`F2 familiar`), 120x40, 20 samples, 5 ms capture polling via `tmux capture-pane`, quiet host: p50 946 ms, p95 1633 ms, min 577 ms. RSS 1 s after first frame: p50 88320 KiB, max 94736 KiB. A run with a concurrent cargo build measured p50 3076 ms and is discarded as contaminated | Release-build numbers; input-to-paint and render percentiles need in-process instrumentation (#417). #417 targets remain: input-to-paint p95 <16 ms, render p95 <33 ms, Next cold frame <=10% over Classic, RSS <=15% over Classic on the same binary profile |
| Producer behavior | Contracts, implementation modules, DTOs and test sources inspected | Real-daemon task/dependency/recovery, reveal/context-selection and protected-operation runtime evidence |

The capture-polling method bounds first-frame time from above by roughly one poll
interval plus `capture-pane` latency; compare Classic and Next only with the same
method, binary profile and host. No Next numbers exist until #408.

### Repeatable execution recipe for a complete checkout

Run the following from `src-rust/` in an isolated checkout on a host with the
repository's required Rust toolchain. The ledger above records their 2026-10-31 results:

```sh
cargo test -p claurst-commands --test slash_command_registry --locked
cargo test -p claurst-core --test keybindings_golden --locked
cargo test -p claurst-tui --test classic_baselines --locked
cargo test -p claurst --test engine_contract --locked
cargo test -p claurst --test acp_smoke --locked
```

Use the existing fixture binaries' disposable homes and fake Claude transport;
no paid model call, real credentials or private memory is needed. The engine
contract fixture is Unix-gated: Windows must be reported unsupported/unexecuted
rather than a zero-test pass. Run `uv run scripts/test-headless-schemas.py` from
the root after preparing the script's dependencies. The fixture definitions,
not these example commands, own their environment scrubbing.

For a controlled real binary, build the debug engine, then use the existing
[scripts/tui-tests](../../scripts/tui-tests/README.md) runner with `REQUIRE_TMUX=1`
and an absolute `COVEN_BIN`, disposable homes and its owned tmux socket. Capture
40x12/80x24/120x40 where applicable, recording terminal/OS/tool versions and engine
SHA. Capture fresh unconfigured, permission, cancellation, resume, palette,
paste/Unicode/resize and restoration journeys. Never execute an interactive
provider request just to obtain an idle baseline.

Record Classic/Next build SHAs, feature set, selected engine path/hash, Coven
consumer SHA, each enabled producer/SDK revision, OS/terminal/tmux versions,
terminal cells, color/Unicode/motion settings, synthetic fixture IDs, sample count,
latency percentiles/RSS and raw evidence location. Do not upload private prompt or
memory content. #417 must choose/record actual instrumentation and identical
workloads before measuring; no fabricated timing is a baseline.

For reverse-consumer CI follow [reverse-contract-ci.md](../reverse-contract-ci.md).
Its three tests cover version parsing, offline auth JSON and print-flag acceptance
only. Print acceptance merely rejects parser exit 2; it is not successful
inference, JSONL conformance or hosted-adapter acceptance. Run engine fixtures,
headless schemas, consumer contracts and native platform gates independently.

After Rust implementation, required formatting/check/strict Clippy/workspace tests
and the affected targeted suites remain mandatory. Documentation-only validation
is not a substitute for those later gates. Preserve original failures and skipped
cases; rerun success does not settle the original #379 intermittent timeout.
