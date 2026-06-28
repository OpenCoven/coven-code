#!/usr/bin/env bash
# shellcheck shell=bash
#
# Shared harness for Coven Code interactive-terminal (TUI) smoke tests.
#
# Drives the real `coven-code` binary inside a controlled tmux session,
# sends keystrokes, captures the rendered pane, and asserts on the output.
# Mirrors the tmux pattern documented in AGENTS.md ("Testing the TUI in a
# controlled terminal"). Offline / headless-first: no test here requires a
# live model call or network.
#
# Source this file; do not execute it directly.

# --- strictness (no -e: we want every assertion to run) -----------------
set -uo pipefail

# --- configuration (override via env) -----------------------------------
: "${COVEN_BIN:=}"                       # path to the binary; auto-detected if empty
: "${TUI_SESSION:=coven-tui-test}"       # tmux session name
: "${TUI_SOCKET:=coven-tui-test}"        # dedicated tmux server socket (-L)
: "${TUI_WIDTH:=120}"                    # terminal columns
: "${TUI_HEIGHT:=40}"                    # terminal rows
: "${TUI_BOOT_STRING:=Coven Code v}"     # string proving the TUI has drawn
: "${TUI_WAIT_TIMEOUT:=20}"              # seconds to wait for a string
: "${TUI_POLL_INTERVAL:=0.4}"            # seconds between capture polls
: "${TUI_SETTLE:=0.6}"                   # seconds to let a keypress redraw
: "${TUI_LOG_DIR:=}"                      # if set, failing-case pane captures are written here
: "${REQUIRE_TMUX:=0}"                    # if 1, a missing tmux is a hard error (CI), not a skip

# --- counters (persist across sourced case files) -----------------------
TESTS_RUN=0
TESTS_PASS=0
TESTS_FAIL=0
TESTS_SKIP=0
declare -a FAILED_NAMES=()
declare -a REGISTERED_CASES=()
CURRENT_CASE="(none)"

# --- colors -------------------------------------------------------------
if [ -t 1 ]; then
  C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'
  C_CYN=$'\033[36m'; C_DIM=$'\033[2m'; C_BLD=$'\033[1m'; C_RST=$'\033[0m'
else
  C_RED=''; C_GRN=''; C_YEL=''; C_CYN=''; C_DIM=''; C_BLD=''; C_RST=''
fi

# --- logging ------------------------------------------------------------
log()  { printf '%s\n' "$*"; }
info() { printf '%s%s%s\n' "$C_CYN" "$*" "$C_RST"; }
warn() { printf '%s%s%s\n' "$C_YEL" "$*" "$C_RST" >&2; }

# --- binary discovery ---------------------------------------------------
detect_bin() {
  if [ -n "$COVEN_BIN" ]; then return 0; fi
  local here root
  here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  root="$(cd "$here/../.." && pwd)"            # repo root
  local candidates=(
    "$root/src-rust/target/debug/coven-code"
    "$root/src-rust/target/release/coven-code"
  )
  local c
  for c in "${candidates[@]}"; do
    if [ -x "$c" ]; then COVEN_BIN="$c"; return 0; fi
  done
  if command -v coven-code >/dev/null 2>&1; then
    COVEN_BIN="$(command -v coven-code)"; return 0
  fi
  return 1
}

# --- dependency guards --------------------------------------------------
have_tmux() { command -v tmux >/dev/null 2>&1; }

# --- assertion primitives ----------------------------------------------
# _pass <message>
_pass() {
  TESTS_RUN=$((TESTS_RUN + 1)); TESTS_PASS=$((TESTS_PASS + 1))
  printf '  %sok%s   %s\n' "$C_GRN" "$C_RST" "$1"
}
# _fail <message> [evidence]
_fail() {
  TESTS_RUN=$((TESTS_RUN + 1)); TESTS_FAIL=$((TESTS_FAIL + 1))
  FAILED_NAMES+=("$CURRENT_CASE :: $1")
  printf '  %sFAIL%s %s\n' "$C_RED" "$C_RST" "$1"
  if [ "${2:-}" != "" ]; then
    printf '%s\n' "$2" | sed 's/^/       │ /' | head -45
    # Persist the full pane capture for post-mortem (uploaded as a CI artifact).
    if [ -n "$TUI_LOG_DIR" ]; then
      mkdir -p "$TUI_LOG_DIR"
      local safe; safe="$(printf '%s' "$CURRENT_CASE" | tr -c 'A-Za-z0-9._-' '_')"
      { printf '### %s :: %s\n\n' "$CURRENT_CASE" "$1"; printf '%s\n' "$2"; } \
        >> "$TUI_LOG_DIR/${safe}.log"
    fi
  fi
}
# _skip <message>
_skip() {
  TESTS_RUN=$((TESTS_RUN + 1)); TESTS_SKIP=$((TESTS_SKIP + 1))
  printf '  %sskip%s %s\n' "$C_YEL" "$C_RST" "$1"
}

# Note: all matching uses grep with a here-string (`<<<`), never `printf | grep`.
# `grep -q` closes its input early on a match, which sends SIGPIPE to a feeding
# printf; under `set -o pipefail` that turns a successful match into a failing
# pipeline. Here-strings avoid the pipe entirely.

# assert_contains <haystack> <needle> <message>
assert_contains() {
  local hay="$1" needle="$2" msg="$3"
  if grep -qF -- "$needle" <<<"$hay"; then
    _pass "$msg"
  else
    _fail "$msg" "$hay"
  fi
}

# assert_absent <haystack> <needle> <message>
assert_absent() {
  local hay="$1" needle="$2" msg="$3"
  if grep -qF -- "$needle" <<<"$hay"; then
    _fail "$msg (unexpectedly present: '$needle')" "$hay"
  else
    _pass "$msg"
  fi
}

# assert_matches <haystack> <ERE-pattern> <message>
assert_matches() {
  local hay="$1" pat="$2" msg="$3"
  if grep -qE -- "$pat" <<<"$hay"; then
    _pass "$msg"
  else
    _fail "$msg" "$hay"
  fi
}

# assert_eq <actual> <expected> <message>
assert_eq() {
  if [ "$1" = "$2" ]; then _pass "$3"; else _fail "$3 (got '$1', want '$2')"; fi
}

# --- headless helpers ---------------------------------------------------
# run_bin <args...>  -> sets globals RUN_OUT (stdout+stderr) and RUN_RC.
# Sets globals directly (not via command substitution) so the exit code
# survives — `$(run_bin ...)` would trap RUN_RC inside a subshell.
RUN_RC=0
RUN_OUT=""
run_bin() {
  RUN_OUT="$("$COVEN_BIN" "$@" 2>&1)"
  RUN_RC=$?
}

# --- tmux session helpers ----------------------------------------------
# All tmux calls go through a dedicated server socket (-L) so the harness
# never attaches to the user's existing tmux server (which would carry the
# wrong environment) and never disturbs their sessions.
_tmux() { command tmux -L "$TUI_SOCKET" "$@"; }

tui_capture() { _tmux capture-pane -t "$TUI_SESSION" -p 2>/dev/null; }

# tui_keys <tmux-key-tokens...>   (special keys: Enter, Escape, C-c, ...)
tui_keys() { _tmux send-keys -t "$TUI_SESSION" "$@"; }

# tui_type <literal-string>   (typed verbatim, no Enter)
tui_type() { _tmux send-keys -t "$TUI_SESSION" -l -- "$1"; }

# tui_settle [seconds]
tui_settle() { sleep "${1:-$TUI_SETTLE}"; }

# wait_for <substring> [timeout-seconds]  -> 0 found / 1 timeout
wait_for() {
  local needle="$1" timeout="${2:-$TUI_WAIT_TIMEOUT}"
  local waited=0
  # bash arithmetic is integer; scale by 10 to honor sub-second poll interval
  local step_ms=400 budget_ms=$((timeout * 1000)) elapsed_ms=0
  while [ "$elapsed_ms" -lt "$budget_ms" ]; do
    if tui_capture | grep -qF -- "$needle"; then return 0; fi
    sleep "$TUI_POLL_INTERVAL"
    elapsed_ms=$((elapsed_ms + step_ms))
  done
  return 1
}

# wait_absent <substring> [timeout-seconds]  -> 0 once gone / 1 still present
# Mirror of wait_for for disappearance (e.g. an overlay closing).
wait_absent() {
  local needle="$1" timeout="${2:-$TUI_WAIT_TIMEOUT}"
  local step_ms=400 budget_ms=$((timeout * 1000)) elapsed_ms=0
  while [ "$elapsed_ms" -lt "$budget_ms" ]; do
    if ! tui_capture | grep -qF -- "$needle"; then return 0; fi
    sleep "$TUI_POLL_INTERVAL"
    elapsed_ms=$((elapsed_ms + step_ms))
  done
  return 1
}

tui_session_alive() { _tmux has-session -t "$TUI_SESSION" 2>/dev/null; }

# tui_start [extra binary args...]
# Spins up a fresh tmux session, waits for the shell, launches the binary,
# and blocks until the TUI has drawn (or fails the current assertion).
tui_start() {
  _tmux kill-session -t "$TUI_SESSION" 2>/dev/null
  _tmux new-session -d -s "$TUI_SESSION" -x "$TUI_WIDTH" -y "$TUI_HEIGHT"

  # The shell inside the new pane is not immediately ready to accept input;
  # sending keys too early drops them (observed: command echoed, never run).
  # Prove readiness with a marker before launching the binary.
  tui_type "echo __SHELL_READY__"; tui_keys Enter
  if ! wait_for "__SHELL_READY__" 6; then
    warn "shell did not become ready in tmux session"
  fi
  tui_type "clear"; tui_keys Enter; sleep 0.3

  local args=""
  if [ "$#" -gt 0 ]; then printf -v args ' %q' "$@"; fi
  tui_type "$COVEN_BIN$args"; tui_keys Enter

  if wait_for "$TUI_BOOT_STRING" "$TUI_WAIT_TIMEOUT"; then
    return 0
  fi
  _fail "TUI failed to boot (no '$TUI_BOOT_STRING' within ${TUI_WAIT_TIMEOUT}s)" "$(tui_capture)"
  return 1
}

# tui_stop  — best-effort graceful quit, then hard kill.
tui_stop() {
  if tui_session_alive; then
    tui_keys C-c 2>/dev/null; sleep 0.3
    tui_keys C-c 2>/dev/null; sleep 0.4
    _tmux kill-session -t "$TUI_SESSION" 2>/dev/null
  fi
}

# tui_shutdown_server — tear down the dedicated tmux server entirely.
tui_shutdown_server() { _tmux kill-server 2>/dev/null || true; }

# --- case framework -----------------------------------------------------
# register_case <function-name>   (called at top of each cases/*.sh file)
register_case() { REGISTERED_CASES+=("$1"); }

# describe <human-readable case title>
describe() {
  CURRENT_CASE="$1"
  printf '\n%s▸ %s%s\n' "$C_BLD" "$1" "$C_RST"
}

# print_summary  -> sets global SUITE_RC (0 pass / 1 fail)
SUITE_RC=0
print_summary() {
  printf '\n%s────────────────────────────────────────%s\n' "$C_DIM" "$C_RST"
  printf '%sSummary%s  %s%d passed%s  %s%d failed%s  %s%d skipped%s  (%d checks)\n' \
    "$C_BLD" "$C_RST" \
    "$C_GRN" "$TESTS_PASS" "$C_RST" \
    "$C_RED" "$TESTS_FAIL" "$C_RST" \
    "$C_YEL" "$TESTS_SKIP" "$C_RST" \
    "$TESTS_RUN"
  if [ "$TESTS_FAIL" -gt 0 ]; then
    printf '\n%sFailures:%s\n' "$C_RED" "$C_RST"
    local f
    for f in "${FAILED_NAMES[@]}"; do printf '  - %s\n' "$f"; done
    SUITE_RC=1
  fi
}
