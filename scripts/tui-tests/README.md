# Interactive terminal (TUI) test suite

Pre-release smoke tests that drive the real `coven-code` binary through a
`tmux` pseudo-terminal, send keystrokes, capture the rendered pane, and assert
on what the user actually sees. This automates the manual tmux workflow
described in [`AGENTS.md`](../../AGENTS.md) ("Testing the TUI in a controlled
terminal").

**Offline / headless-first:** no test makes a live model call or hits the
network. Every assertion is deterministic and runnable on any machine with the
binary and `tmux`.

## Usage

```bash
# From the repo root. Auto-detects target/{debug,release}/coven-code.
scripts/tui-tests/run.sh

# Build the debug binary first, then test it.
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
CI runs them before building and exercising the real engine.

## CI

Runs automatically via [`.github/workflows/tui-tests.yml`](../../.github/workflows/tui-tests.yml)
on pushes to `main` and on PRs that touch `src-rust/**` or this suite (plus
manual `workflow_dispatch`). The job installs `tmux`, builds the debug binary
(`cargo build --locked --package claurst`), seeds an offline settings file
(`hasCompletedOnboarding: true`) so a credential-less runner lands on the main
screen, and runs `run.sh`. No secrets or network access required.

## Requirements

- The `coven-code` binary (built debug/release, or on `PATH`).
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

## Configuration

Override via environment variables:

| Var | Default | Purpose |
|---|---|---|
| `COVEN_BIN` | auto-detected | Binary under test |
| `TUI_WIDTH` / `TUI_HEIGHT` | `120` / `40` | tmux pane size (the TUI is layout-sensitive) |
| `TUI_WAIT_TIMEOUT` | `20` | Seconds to wait for an expected string |
| `TUI_SESSION` | `coven-tui-test` | tmux session name |
| `TUI_SOCKET` | `coven-tui-test` | Dedicated tmux server socket (`-L`) so the harness never touches your own tmux sessions |
| `TUI_LOG_DIR` | _(unset)_ | If set, the full pane capture for each failing case is written here (CI uploads these as artifacts) |
| `REQUIRE_TMUX` | `0` | If `1`, a missing `tmux` is a hard error instead of skipping the interactive cases (set in CI) |

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
`tui_type`, `tui_paste`, `tui_settle`, `tui_capture`, `wait_for`, and the assertions
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
