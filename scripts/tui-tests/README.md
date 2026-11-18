# Interactive terminal (TUI) test suite

Pre-release smoke tests that drive the real `coven-code` binary through a
`tmux` pseudo-terminal, send keystrokes, capture the rendered pane, and assert
on what the user actually sees. This automates the manual tmux workflow
described in [`AGENTS.md`](../../AGENTS.md) ("Testing the TUI in a controlled
terminal").

**Offline / headless-first:** no test makes a live model call or hits the
network. Every assertion is deterministic and runnable on any machine with the
binary, the panic probe (case 30) and `tmux`.

## Usage

```bash
# From the repo root. Auto-detects target/{debug,release}/coven-code. Case 30
# also needs the panic probe from that build; `--build` builds both.
scripts/tui-tests/run.sh

# Build the debug binary and the panic probe first, then test them.
scripts/tui-tests/run.sh --build

# Test a specific binary (e.g. an installed release).
COVEN_BIN=/usr/local/bin/coven-code scripts/tui-tests/run.sh

# Run only matching case files (substring match on the filename).
scripts/tui-tests/run.sh 03 05
```

Exit code is `0` only when at least one assertion passed and no assertion or
case execution failed. A nonzero case return, a case reporting no assertions
or skips, or a skipped-only run exits `1`. Case-file load errors and unmatched
filters exit `2`; a partially loaded case catalog is never executed.

Partial local runs can still pass executed headless checks while explicitly
reporting interactive skips. They are not full terminal acceptance. CI sets
`REQUIRE_TMUX=1` so a missing terminal dependency cannot silently skip that gate.

## Harness regression checks

```bash
python3 -m unittest discover -s scripts/tui-tests -p 'test_*.py' -v
```

These tests exercise the actual runner and assertion library with synthetic
case inputs and an inert tmux executable. They need no engine, tmux server,
credentials or network, and do not establish real-binary terminal acceptance.
`test_panic_restore_pty.py` also needs a PTY: it runs the panic driver, and
case 30 through the runner, against fake probes (about a minute).
CI runs them before building and exercising the real engine.

## CI

Runs automatically via [`.github/workflows/tui-tests.yml`](../../.github/workflows/tui-tests.yml)
on pushes to `main` and on PRs that touch `src-rust/**` or this suite (plus
manual `workflow_dispatch`). The job installs `tmux`, builds the debug binary
and the panic probe (`cargo build --locked --package claurst --package
claurst-tui --bins --example panic_restore`), seeds an offline settings file
(`hasCompletedOnboarding: true`) so a credential-less runner lands on the main
screen, and runs `run.sh`. No secrets or network access required.

## Requirements

- The `coven-code` binary (built debug/release, or on `PATH`).
- For `cases/30_panic_restore.sh`, the tui crate's `panic_restore` example
  (`run.sh --build` builds it). It is built from source, so the case looks
  for it in `COVEN_BIN`'s cargo output (`examples/panic_restore` next to
  the binary) or `COVEN_PANIC_PROBE`. A missing probe fails the case when
  `COVEN_BIN` is a cargo build (cargo's dep-info, `coven-code.d`, is next to
  it), and so does a probe older than one of the files cargo built it from,
  as its own dep-info (`panic_restore.d`) lists them (a plain `cargo build`
  does not rebuild it); for an installed binary the case is skipped, since
  the probe cannot test that binary. The freshness check sees the
  workspace's own files, not dependency updates in `Cargo.lock`. An explicit
  `COVEN_PANIC_PROBE` is used as given, without the freshness check, and
  fails the case if it is missing; CI sets it to the probe it builds.
- `tmux` (3.x). If absent, the headless cases still run and the interactive
  cases are reported as **skipped** rather than failed.

## What it covers

| Case file | Area | Sample assertions |
|---|---|---|
| `cases/01_headless.sh` | CLI surface (no TTY) | `--version` semver, `--help` usage + flags, unknown flag exits non-zero, `auth status`, `models` catalog, `--dump-system-prompt` |
| `cases/02_startup.sh` | TUI cold start | title banner, welcome/changelog sections, input glyph, footer keybindings, no panic on boot |
| `cases/03_command_palette.sh` | Slash palette | `/` shows contextual suggestions, typing filters the list, `Ctrl+K` opens grouped navigation, one Enter runs a leaf, retired paths are rejected |
| `cases/04_help_overlay.sh` | Help overlay | `?` opens keybinding + grouped command reference, search finds `/account login`, `Esc` closes it |
| `cases/05_input_editing.sh` | Prompt input | typed text echoes into the buffer, `Ctrl+U` clears it |
| `cases/06_quit.sh` | Shutdown | `Ctrl+C` twice exits cleanly back to the shell |
| `cases/07_paste_burst.sh` | Paste burst | a line delivered in one write still submits; interior newlines coalesce without submitting |
| `cases/19_diff_large.sh` | Reviewing changes with `/diff` | In Classic and Next: scrolling a 150,000-line change, and opening and leaving a minified file's one enormous changed line, each draw within 6 s. A binary change is marked `binary` in the file list and says it has no text diff, and none of its bytes reach the terminal: its text is not drawn and its title sequence does not retitle the pane, while one sent from the shell afterwards does, so the check can see a title change. A `git mv` and a `git mv` followed by an edit each name the old and new paths, and the list marks them `renamed` instead of `+0 -0`; the edited one shows its edit and counts its lines |
| `cases/22_next_resize_stream.sh` | Resize during a turn | `fake_anthropic.py` streams 150,000 lines into Next, then holds the stream open, adding a line every 0.2 s, while tmux moves through 80x24, 40x10, 200x50, 20x5, 120x40, 1x1, 2x2, 160x30, 10x3, 80x24 and 120x40; tmux must apply 200x50, 20x5, 1x1, 2x2, 10x3 and 120x40, each held past its 250 ms resize delay, and nothing saved before the release proves every size landed during the turn; the process survives, the draft stays as typed in the composer, the end of the answer is on screen, every streamed line is saved, including at least 20 sent while the stream was held, and the frame fills 40 rows at 120x40; needs `python3` |
| `cases/23_classic_zero_size.sh` | Zero and tiny sizes | Classic starts at 0x0, 0x40 and 120x0 and survives 120x0 with the exit toast, 0x40, 0x0, 1x1 and 20x6, then draws a full frame at 120x40, exits 0 and restores the terminal; tmux cannot make a zero-sized window, so `zero_size_pty.py` drives a raw PTY and needs `python3` |
| `cases/25_next_unicode_composer.sh` | Unicode in the Next composer | CJK, right-to-left text, a combining mark and an emoji ZWJ sequence typed through tmux stay unsent, and all but the emoji (drawn differently by tmux versions) are checked on screen; Left and Backspace act on whole CJK characters; no turn reaches the provider (`fake_anthropic.py`) before Enter, and it and the saved session get exactly the edited text. tmux sends committed characters only, so input-method preedit is not covered, and editing moves by Unicode scalar, so the ZWJ emoji and combining mark are not edited; needs `python3` |
| `cases/26_classic_cancel.sh` | Cancelling a turn | In Classic (`COVEN_CODE_UI` unset) and Next, `fake_anthropic.py` drives a real turn. Escape while an approved Bash command appends a line every second: once the status no longer says cancelling, at most 3 more lines were written than at the Escape, and the file stops growing. Escape while text is still arriving (a burst of 200,000 text deltas, then a stream held open, so the turn is still running in the runtime): the status acknowledges it, the backlog still arriving does not bring the running turn back within 10 s, and the status settles to `Cancelled`; needs `python3` |
| `cases/27_classic_quit_save.sh` | Quitting mid-turn | In Classic (`COVEN_CODE_UI` unset) and Next, `fake_anthropic.py` drives a real turn. Ctrl+D twice while a held stream is still sending text exits, leaves the alternate screen, restores the cursor and turns mouse reporting off, and the saved session holds the prompt and the text delivered so far. Only Next prints its report and the resume command after exiting. Ctrl+D twice while an approved `sleep 20` runs exits within 8 s and saves the prompt; needs `python3` |
| `cases/28_approval_queue.sh` | A second approval key | `fake_anthropic.py` asks for two Bash calls in one response, so the second request waits behind the first panel. The first panel is decided with `y` and a second `y` follows: in the same write, at once, and 150 ms later. In both renderers a `y` already waiting when the first panel closes lands in the composer as text and decides nothing. A `y` that reaches the open next panel is ignored by Next, which takes keys other than Escape only after 400 ms of quiet, and decides that panel in Classic, which takes keys at once. Each round checks whether the second call ran (its marker file); while it is undecided, its panel must still be open and the model must have been told nothing, and a deliberate `y` then runs it. Every round ends with the model told that both calls ran. In Next, a round whose two keys took 400 ms or more to send skips the decision check, since the next panel may have armed by then; needs `python3` |
| `cases/29_tool_results_kept.sh` | Tool results after a tool turn | In Classic (`COVEN_CODE_UI` unset) and Next, `fake_anthropic.py` asks for a Bash call, the user approves it, and the model answers its result. Once the turn ends, single key presses (`x`, Backspace, as typing sends them) precede a follow-up prompt, whose request, read from the full `messages` the fake logs to a file of their own, must carry the call and, in the user message right after it, the tool result answering it; needs `python3` |
| `cases/30_panic_restore.sh` | Panic restoration | The product has no known input that panics, so the tui crate's `panic_restore` example takes the terminal with the real `setup_terminal` and panics on request; `panic_restore_pty.py` runs it on a raw PTY, with `setup_terminal` for each renderer (Classic and Next; their event loops are not involved). Before the panic: raw termios (output processing off too), alternate screen, mouse capture, bracketed paste, hidden cursor and pushed keyboard flags. A main-thread panic restores every termios field and, before the panic message, turns output processing back on (the message's first line break arrives as CRLF), leaves the alternate screen, turns mouse capture and bracketed paste off, sends a keyboard pop and shows the cursor; the message is printed and the exit status is 101. A worker-thread panic restores nothing and the probe keeps drawing; its later main-thread panic still restores. After a suspend and resume through the real `suspend_terminal`, a panic still restores. With `COVEN_CODE_KEYBOARD_ENHANCEMENT=0` no flags are pushed. Lines containing "current behaviour" pin details that need a decision, not endorse them (Unix; Windows has its own restore branch, which pops before leaving the alternate screen, and is not exercised): the keyboard pop is sent after leaving the alternate screen the flags were pushed on (case 13 requires suspend to pop on the alternate screen), a pop is sent even when none was pushed, the startup title is not cleared, each `setup_terminal` call chains another hook (so after a resume the restore is written twice), and a worker-thread panic prints its message onto the live screen. Needs `python3` and the probe |
| `cases/31_classic_daemon_glyph.sh` | Coven daemon glyph | In Classic (`COVEN_CODE_UI` unset), a fake daemon on `coven.sock` in a disposable `COVEN_HOME` answers every request. The footer shows `✦ coven` while it runs, turns to `○ coven` within 25 s of the fake stopping and its socket being removed, with no input, and back to `✦ coven` within 25 s of a fresh fake starting; needs `python3` |
| `cases/32_one_runtime.sh` | One runtime per renderer (#408) | In Classic (`COVEN_CODE_UI` unset) and Next, `fake_anthropic.py` drives two real turns, an approved Bash call and then a text answer. Between them the window grows by 20 columns and 5 rows until the app draws a rule that wide, then goes back. There is one session file after the first turn, and after the second turn and the exit it is still that one file with that id; under the home and the project, the only session file (`sessions/*.json`) and session index (`sessions.db`) are that file and that index, which holds that one session. The fake saw three streamed requests, each carrying the one before it, its reply and the next user message, and the request of the second turn came on a TCP connection the first turn used (the fake numbers connections in a log of its own), so it came from the same HTTP connection pool. The saved session is the last request's conversation plus its answer, and every indexed message is that session's. A user message has no uuid, so the index keeps only the first one ever (current behaviour); the check uses the answers. The tool-result request inside the first turn is not checked on its own. Not shown: anything built but never used to send or save; clients that share one pool; and the other clients the process builds the same way in both renderers (the registry's Anthropic provider, the per-goal-turn wrapper, Agent sub-agents, memory extraction, which ignores `ANTHROPIC_BASE_URL` and starts at 20 messages, and the replacements made by `/refresh` and an in-session login); one store object, since the index is opened at every save; the scheduler and memory authority; relaunching the other renderer on the same session, `/resume` and `/clear`; any transport but the direct API-key one; needs `python3` |
| `cases/33_daemon_restart_unknown.sh` | Restarted Coven daemon (#412) | In Classic (`COVEN_CODE_UI` unset) and Next, with `daemonLedger` on, a fake daemon on `coven.sock` in a disposable `COVEN_HOME` answers health with its own pid and start time and logs every request. The session registers with it once. The fake is stopped and a second one, a new process, starts on the same path: within 30 s, with no input, Next's header reads `ledger unknown` instead of `ledger-registered`, and Classic's footer shows `◌ coven (unknown)` and its welcome line `unknown` instead of `online`. Across two more probes and one turn against `fake_anthropic.py`, the new daemon receives no registration. `/session resume <id>` then registers that session with it exactly once, and the state reads registered and online again (Classic's online glyph returns only once the new daemon has accepted the registration); needs `python3` |

## Configuration

Override via environment variables:

| Var | Default | Purpose |
|---|---|---|
| `COVEN_BIN` | auto-detected | Binary under test |
| `TUI_WIDTH` / `TUI_HEIGHT` | `120` / `40` | tmux pane size (the TUI is layout-sensitive) |
| `TUI_WAIT_TIMEOUT` | `20` | Seconds to wait for an expected string |
| `TUI_SESSION` | `coven-tui-test` | tmux session name |
| `TUI_SOCKET` | `coven-tui-test-<pid>` | tmux server socket (`-L`), one per run, so the harness never touches your own tmux sessions and runs in parallel never touch each other |
| `TUI_LOG_DIR` | _(unset)_ | If set, the full pane capture for each failing case is written here (CI uploads these as artifacts) |
| `REQUIRE_TMUX` | `0` | If `1`, a missing `tmux` is a hard error instead of skipping the interactive cases (set in CI) |
| `COVEN_PANIC_PROBE` | `examples/panic_restore` next to `COVEN_BIN` | The `panic_restore` probe for `cases/30_panic_restore.sh`, built from source. Used as given: not checked for freshness, and the case fails if it is missing |

## Writing a new case

Drop a `cases/NN_name.sh` file. Each file registers one function:

```bash
register_case tc_mything

tc_mything() {
  describe "My thing"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  tui_start || { tui_stop; return 0; }

  tui_keys C-k                 # send a binding (tmux key tokens)
  tui_type "some text"         # type literal characters
  tui_paste "line"$'\r'        # deliver bytes in ONE write (paste / host pane)
  tui_settle                   # let it redraw

  local s; s="$(tui_capture)"
  assert_contains "$s" "expected" "palette shows expected"

  tui_stop
}
```

`tui_type` sends characters the way a human types them; `tui_paste` delivers
the whole string in a single write, the way a host application drives an
embedded pane or a terminal without bracketed paste delivers a clipboard
paste. The distinction matters: only the second shape engages the TUI's
paste-burst detector.

Helpers from [`lib.sh`](lib.sh): `tui_start` / `tui_stop`, `tui_keys`,
`tui_type`, `tui_paste`, `tui_settle`, `tui_capture`, `wait_for`,
`tui_quit_and_check` (press quit keys, then check the exit and the terminal
restore), and the assertions
`assert_contains` / `assert_absent` / `assert_matches` / `assert_eq`. For
headless checks, call `run_bin <args...>` and read `$RUN_OUT` / `$RUN_RC`.

## Known tmux limitations

- `Ctrl+Shift+<key>` chords (e.g. the `Ctrl+Shift+A` model picker) cannot be
  sent reliably through `tmux send-keys`, so they are not asserted here. Use
  the slash palette path instead where one exists.
- After spawning a session, the inner shell needs a moment before it accepts
  input. `tui_start` proves readiness with a marker echo before launching the
  binary; replicate that pattern if you script sessions by hand.

## Performance baseline

```bash
(cd src-rust && cargo build --release --locked)
python3 scripts/tui-tests/perf_baseline.py --bin src-rust/target/release/coven-code \
  --runs 20 --keys 20 --size 40x12 --size 80x24 --size 120x40 --json perf.json
```

Spawns the binary directly on a pseudo-terminal (no tmux polling) with a
disposable home, a synthetic `USER`, no credentials and no model call. It
reports cold usable frame, input-to-paint and resize-to-paint percentiles,
idle RSS and idle PTY writes per second, and exits nonzero if any sample
timed out. Times are observed at the PTY boundary and include scheduling;
compare builds only with the same binary profile, host and load. It is a
baseline instrument, not #417's in-process render instrumentation.

```bash
python3 scripts/tui-tests/perf_baseline.py --bin src-rust/target/release/coven-code \
  --ui next --history-lines 100000 --stream --size 80x24 --size 120x40
```

`--ui next` selects the Next renderer. `--history-lines N` resumes a synthetic
session of N transcript lines. A resume must prove it loaded, so a failed one
cannot pass as a fast empty session: Next must show seeded text, and Classic
must print the CLI's load confirmation. `--stream` adds a turn against a fake
`claude` written to the disposable home, then reports input-to-paint during
the turn and the gap between paints. That gap is a boundary proxy for active
render cost; its floor is the event loop's 16 ms poll, and a slower fake
producer would widen it. The fake is set from launch, so every figure from a
`--stream` run, cold start and idle ones included, comes from a session with a
provider: compare it only with other `--stream` runs. The long-history fixture must prove live tail visibility; older Classic builds fail this check past 65,535 rows (#441).


### Actual render and completed draw timing

Add `--render-timing` to opt in to private diagnostic files for each synthetic benchmark session. The engine records renderer, terminal dimensions, active state, input character count, scroll offset and follow state, and numeric render/draw durations. It records no prompt text. The widget/layout measurement surrounds the renderer call; the complete draw measurement also includes Ratatui diffing and backend writes. These are actual in-process durations, separate from paint gaps.

In this mode, key latency waits for a completed draw with the new input length, and resize latency waits for a completed draw at the requested dimensions. Both match records written after the operation began. Repeated probe letters and the first output chunk cannot satisfy them. Observing the local diagnostic file adds overhead, so these latencies are upper bounds that include observation cost; they do not require clock alignment between Python and Rust. Cold timing requires both the usable PTY text and a completed draw. Existing runs without this flag retain their PTY boundary probes.

Timing logs use the same explicitly opted-in, bounded private diagnostics policy as `--diagnostics`. Missing or truncated logs fail the run. The JSON report includes raw numeric samples for audit and percentile recalculation; temporary homes and their diagnostic files are retired after each run. Compare Classic and Next with identical instrumentation, provider setup, binary profile, hardware, terminal dimensions and workload, and record quiet-host variance. A debug smoke measurement is not reference-machine release qualification.

```bash
python3 scripts/tui-tests/perf_baseline.py --bin src-rust/target/release/coven-code \
  --render-timing --stream --ui next --history-lines 100000 \
  --runs 20 --keys 20 --size 80x24 --size 120x40 --json measured-next.json
```
