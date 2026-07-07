# Coven runtime contract

`coven-code` is a registered *runtime* in the
[OpenCoven/coven-runtimes](https://github.com/OpenCoven/coven-runtimes)
canonical registry: an agent CLI the Coven daemon can drive without any
daemon-core edits. The contract is declared in
[`spec/runtime-manifest/coven-code.json`](../spec/runtime-manifest/coven-code.json)
and conformance-tested on every CI run.

## Flag → manifest mapping

| Manifest field | CLI surface | Notes |
| --- | --- | --- |
| `non_interactive_prompt_prefix_args: ["--print"]` | `coven-code --print <prompt>` | One-shot run, response on stdout. |
| `interactive_prompt_prefix_args: []` | `coven-code` | A positional prompt switches to print mode, so interactive launches must not append one. |
| `model_flag: "--model"` | `--model <id>` | |
| `system_prompt_flag: "--append-system-prompt"` | `--append-system-prompt <text>` | Identity preamble; does not clobber the built-in system prompt. |
| `sandbox.flag: "--permission-mode"` | `--permission-mode bypass-permissions` (full) / `--permission-mode plan` (read-only) | |
| `capabilities.stream` + `stream_args.prefix_args` | `--print --input-format stream-json --output-format stream-json` | Long-lived stream mode (below). |
| `stream_args.session_id_flag: "--session-id"` | `--session-id <uuid>` | Pre-assigns the session id at launch. Creating only — reusing an existing id starts a fresh transcript file under that id. |
| `stream_args.resume_flag: "--resume"` | `--resume <id>` | Continues a persisted session in place. |
| `capabilities.think` | `--thinking <TOKENS>` | Extended-thinking budget. |
| `capabilities.speed` | `--effort <LEVEL>` | `low`, `medium`, `high`, `max`. |

## Stream mode

Implemented in `src-rust/crates/cli/src/stream_mode.rs`. With
`--print --input-format stream-json --output-format stream-json` the process
stays alive across chat turns:

- **stdin** — one JSON frame per line. A
  `{"type":"user","message":{"role":"user","content":...}}` frame (content as
  a string or an array of `{"type":"text","text":...}` blocks) triggers a
  turn; `assistant` frames append as prefill; the legacy
  `{"role":...,"content":"..."}` shape is also accepted. The positional
  prompt argument is ignored in this mode, matching Claude Code.
- **stdout** — strictly JSONL: one `system`/`init` frame at startup, then per
  turn `assistant` frames (tool-use and final text), `tool_result` frames,
  and a closing `result` frame (`subtype`, `duration_ms`, `is_error`,
  `num_turns`, `session_id`). Logs and warnings go to stderr, never stdout.
- **exit** — stdin EOF ends the chat; per-turn model errors are reported in
  `result` frames and do not kill the process.

The transcript is persisted after every turn, so a later relaunch with
`--resume <session-id>` continues the conversation.

## Stability guarantee

Registry versions are immutable. The flags named in the manifest —
`--print`, `--input-format`, `--output-format`, `--session-id`, `--resume`,
`--model`, `--append-system-prompt`, `--permission-mode` (and its
`bypass-permissions`/`plan` values), `--thinking`, `--effort` — are contract
surface. The `runtime_manifest_*` tests in `crates/cli/src/main.rs` re-parse
the manifest against the real clap definition on every test run; if one
fails, either fix the flag regression or publish a new manifest version:

1. Add the change to `spec/runtime-manifest/coven-code.json` and bump its
   `version`.
2. Re-validate: `conjure validate spec/runtime-manifest/coven-code.json` and
   `conjure test` against a built binary (`conjure` lives in coven-runtimes).
3. Re-accept in coven-runtimes: `conjure registry add` + PR (the old version
   stays resolvable for pinned consumers).

Never edit the shipped definition of an accepted version in place.

## Consuming the registry

`coven-code` also consumes the canonical registry (pinned git dependency on
`coven-runtime-spec` / `coven-runtime-registry`):

- `/coven runtimes [--json]` — accepted runtimes, capabilities, and local
  install status (with install hints for missing binaries).
- `/coven run <harness> <prompt>` — validates the harness id against the
  registry and local adapters before shelling out.
- `/coven adapter doctor` — additionally validates
  `$COVEN_HOME/adapters/*.json` against the shared manifest spec.

Bumping the pinned rev in `src-rust/Cargo.toml` is how newly accepted
runtimes are adopted.
