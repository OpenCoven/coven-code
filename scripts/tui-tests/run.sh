#!/usr/bin/env bash
#
# Coven Code interactive-terminal test suite.
#
# Drives the real binary through a tmux pseudo-terminal and asserts on the
# rendered output. Offline / headless-first: no live model call is made.
#
# Usage:
#   scripts/tui-tests/run.sh                 # auto-detect binary, run all cases
#   scripts/tui-tests/run.sh --build         # cargo build (debug) first
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
  info "Building debug binary (cargo build)…"
  ( cd "$ROOT/src-rust" && cargo build ) || { warn "cargo build failed"; exit 2; }
fi

if ! detect_bin; then
  warn "Could not find the coven-code binary."
  warn "Build it first:  (cd src-rust && cargo build)   or run:  $0 --build"
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

# Clean up any stray server from a previous aborted run, and guarantee the
# dedicated tmux server is torn down on exit.
if have_tmux; then
  tui_shutdown_server
  trap 'tui_shutdown_server' EXIT
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
done

if [ "${#REGISTERED_CASES[@]}" -eq 0 ]; then
  warn "No test cases matched."; exit 2
fi

for case_fn in "${REGISTERED_CASES[@]}"; do
  "$case_fn"
  have_tmux && command tmux -L "$TUI_SOCKET" kill-session -t "$TUI_SESSION" 2>/dev/null || true
done

print_summary
exit "$SUITE_RC"
