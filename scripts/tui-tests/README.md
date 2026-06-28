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

Exit code is `0` only if every assertion passed, `1` otherwise — suitable for
a release gate or CI step.

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
| `cases/03_command_palette.sh` | Slash palette | `/` opens it, lists `/clear` `/compact` `/config`, typing filters the list, `Ctrl+K` entry point |
| `cases/04_help_overlay.sh` | Help overlay | `?` opens keybinding + command reference, `Esc` closes it |
| `cases/05_input_editing.sh` | Prompt input | typed text echoes into the buffer, `Ctrl+U` clears it |
| `cases/06_quit.sh` | Shutdown | `Ctrl+C` twice exits cleanly back to the shell |

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
  tui_settle                   # let it redraw

  local s; s="$(tui_capture)"
  assert_contains "$s" "expected" "palette shows expected"

  tui_stop
}
```

Helpers from [`lib.sh`](lib.sh): `tui_start` / `tui_stop`, `tui_keys`,
`tui_type`, `tui_settle`, `tui_capture`, `wait_for`, and the assertions
`assert_contains` / `assert_absent` / `assert_matches` / `assert_eq`. For
headless checks, call `run_bin <args...>` and read `$RUN_OUT` / `$RUN_RC`.

## Known tmux limitations

- `Ctrl+Shift+<key>` chords (e.g. the `Ctrl+Shift+A` model picker) cannot be
  sent reliably through `tmux send-keys`, so they are not asserted here. Use
  the slash palette path instead where one exists.
- After spawning a session, the inner shell needs a moment before it accepts
  input. `tui_start` proves readiness with a marker echo before launching the
  binary; replicate that pattern if you script sessions by hand.
