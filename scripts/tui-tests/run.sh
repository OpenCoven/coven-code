#!/usr/bin/env bash
#
# Coven Code interactive-terminal test suite.
#
# Drives the real binary through a tmux pseudo-terminal and asserts on the
# rendered output. Offline / headless-first: no live model call is made.
#
# Usage:
#   scripts/tui-tests/run.sh                 # auto-detect binary, run all cases
#   scripts/tui-tests/run.sh --build         # build the debug binary and panic probe first
#   COVEN_BIN=/path/to/coven-code run.sh     # test a specific binary
#   scripts/tui-tests/run.sh 03 05           # run only cases matching 03* / 05*
#
# Exit code: 0 if every assertion passed, 1 otherwise.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
# shellcheck source=lib.sh
source "$HERE/lib.sh"

DO_BUILD=0
declare -a FILTERS=()
for arg in "$@"; do
  case "$arg" in
    --build) DO_BUILD=1 ;;
    -h|--help)
      sed -n '2,16p' "$0"; exit 0 ;;
    *) FILTERS+=("$arg") ;;
  esac
done

if [ "$DO_BUILD" -eq 1 ]; then
  info "Building debug binary and panic probe (cargo build)…"
  ( cd "$ROOT/src-rust" && cargo build --package claurst --package claurst-tui --bins --example panic_restore ) \
    || { warn "cargo build failed"; exit 2; }
fi

if ! detect_bin; then
  warn "Could not find the coven-code binary."
  warn "Build it first:  $0 --build   (the binary and the panic probe)"
  warn "Or point COVEN_BIN at an installed binary."
  exit 2
fi
info "Binary:  $COVEN_BIN"
if have_tmux; then
  info "tmux:    $(tmux -V)"
elif [ "$REQUIRE_TMUX" = "1" ]; then
  warn "tmux not found but REQUIRE_TMUX=1 — refusing to skip interactive cases."
  exit 2
else
  warn  "tmux not found — interactive TUI cases will be skipped (headless cases still run)."
fi
info "Term:    ${TUI_WIDTH}x${TUI_HEIGHT}"
[ -n "$TUI_LOG_DIR" ] && info "Logs:    $TUI_LOG_DIR (pane captures on failure)"

# --- isolation from the invoking user's settings -------------------------
# The binary reads settings from its home and from a .coven-code directory in
# any ancestor of its working directory. Cases 01-07 start it without a home
# or working directory of their own, so give them a scratch home and a scratch
# working directory. The other cases make theirs with mktemp or tempfile, so
# point TMPDIR into the same checked scratch directory (on macOS a bare
# `mktemp -d` ignores it and uses the per-user directory under /var/folders).

# Paths the caller gave relative to the directory this run now leaves. A
# COVEN_BIN without a slash is a command name, found through PATH.
case "$COVEN_BIN" in /*) ;; */*) COVEN_BIN="$PWD/$COVEN_BIN" ;; esac
case "$TUI_LOG_DIR" in ''|/*) ;; *) TUI_LOG_DIR="$PWD/$TUI_LOG_DIR" ;; esac
case "${COVEN_PANIC_PROBE:-}" in ''|/*) ;; *) COVEN_PANIC_PROBE="$PWD/$COVEN_PANIC_PROBE" ;; esac
case "${COVEN_TUI_PTY_RECEIPTS:-}" in ''|/*) ;; *) COVEN_TUI_PTY_RECEIPTS="$PWD/$COVEN_TUI_PTY_RECEIPTS" ;; esac

TUI_RUN_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/coven-tui-run.XXXXXX")" \
  || { warn "Could not create a scratch directory under ${TMPDIR:-/tmp}."; exit 2; }
trap 'rm -rf "$TUI_RUN_ROOT"' EXIT
cd "$TUI_RUN_ROOT" || exit 2
# The physical path: a symlinked TMPDIR must not hide where the directory is.
TUI_RUN_ROOT="$(pwd -P)"

# The scratch directory must not be in a home tree, nor below a .coven-code
# directory. HOME may already be a throwaway one, so check the login home too
# (what ~ means with HOME unset).
unsafe=""
login_home="$(unset HOME; cd ~ 2>/dev/null && pwd -P)"
for home in "${HOME:-}" "$login_home"; do
  [ -n "$home" ] || continue
  home="$(cd "$home" 2>/dev/null && pwd -P)" || continue
  case "$TUI_RUN_ROOT/" in "$home"/*) unsafe="it is inside the home directory $home" ;; esac
done
dir="$TUI_RUN_ROOT"
while [ -z "$unsafe" ] && [ -n "$dir" ]; do
  dir="${dir%/*}"
  if [ -e "$dir/.coven-code" ]; then unsafe="$dir/.coven-code is above it"; fi
done
if [ -n "$unsafe" ]; then
  warn "Refusing to run: the scratch directory $TUI_RUN_ROOT is not isolated ($unsafe)."
  warn "The binary under test reads settings from every directory above its working directory,"
  warn "so cases started there could read your own. Set TMPDIR to a directory outside any home"
  warn "tree that has no .coven-code directory above it."
  exit 2
fi
unset unsafe login_home home dir

TUI_SCRATCH_HOME="$TUI_RUN_ROOT/home"
mkdir -p "$TUI_SCRATCH_HOME/.coven-code" "$TUI_RUN_ROOT/work" "$TUI_RUN_ROOT/tmp" || exit 2
# A home with no settings opens first-run setup instead of the main screen.
printf '%s\n' '{"hasCompletedOnboarding": true}' >"$TUI_SCRATCH_HOME/.coven-code/settings.json"
export TMPDIR="$TUI_RUN_ROOT/tmp"
# These change which home the binary uses.
unset COVEN_HOME COVEN_CODE_HOME COVEN_PARENT
cd "$TUI_RUN_ROOT/work" || exit 2
# The shell in a tmux pane has read the user's startup files, which can change
# directory or set those variables again, so the typed line undoes both. A
# prefix the caller exported goes between the two parts.
TUI_LAUNCH_PREFIX="cd $(printf '%q' "$TUI_RUN_ROOT/work") && ${TUI_LAUNCH_PREFIX}env -u COVEN_HOME -u COVEN_CODE_HOME -u COVEN_PARENT HOME=$(printf '%q' "$TUI_SCRATCH_HOME") "
info "Home:    $TUI_SCRATCH_HOME (scratch; cases 01-07 start in $TUI_RUN_ROOT/work)"

# With an explicit TUI_SOCKET, clean up any stray server from a previous aborted
# run. Either way, guarantee this run's tmux server is torn down on exit.
if have_tmux; then
  tui_shutdown_server
  trap 'tui_shutdown_server; rm -rf "$TUI_RUN_ROOT"' EXIT
fi

# Load all case files (each calls register_case).
shopt -s nullglob
for f in "$HERE"/cases/*.sh; do
  # Apply filename filters if any were supplied.
  if [ "${#FILTERS[@]}" -gt 0 ]; then
    keep=0
    for pat in "${FILTERS[@]}"; do
      [[ "$(basename "$f")" == *"$pat"* ]] && keep=1
    done
    [ "$keep" -eq 1 ] || continue
  fi
  # shellcheck source=/dev/null
  source "$f"
  load_rc=$?
  if [ "$load_rc" -ne 0 ]; then
    warn "Failed to load case file: $(basename "$f") (exit $load_rc)."
    exit 2
  fi
done

if [ "${#REGISTERED_CASES[@]}" -eq 0 ]; then
  warn "No test cases matched."; exit 2
fi

for case_fn in "${REGISTERED_CASES[@]}"; do
  # A case can fail before reaching an assertion. Do not let the final
  # summary erase its exit status or borrow checks from an earlier case.
  CURRENT_CASE="$case_fn"
  checks_before=$TESTS_RUN
  "$case_fn"
  case_rc=$?
  if [ "$case_rc" -ne 0 ]; then
    _fail "case returned nonzero status: $case_rc"
  elif [ "$TESTS_RUN" -eq "$checks_before" ]; then
    _fail "case reported no assertions or skips"
  fi
  have_tmux && command tmux -L "$TUI_SOCKET" kill-session -t "$TUI_SESSION" 2>/dev/null || true
done

# Partial local runs remain supported, but skipped-only execution is not
# evidence that the binary passed any check.
if [ "$TESTS_PASS" -eq 0 ] && [ "$TESTS_FAIL" -eq 0 ]; then
  CURRENT_CASE="suite"
  _fail "No assertions passed; a skipped-only run is not successful."
fi

print_summary
exit "$SUITE_RC"
