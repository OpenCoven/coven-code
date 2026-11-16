#!/usr/bin/env bash
# shellcheck shell=bash
#
# Cancelling a turn, in Classic and in Next (#408). A local fake of the Messages
# API drives real turns through the real client and query loop:
#   J1  Escape while an approved Bash command appends a line to a file every
#       second: the cancel settles within a few lines of the Escape, so it
#       stopped the command rather than waiting for it, and the file stops
#       growing.
#   J2  Escape while text is still arriving: the fake sends a burst of text
#       deltas and then holds the stream open, so the turn is still running in
#       the runtime when Escape lands. The backlog does not restart the turn on
#       screen, and the status settles to cancelled, which it can only do once
#       the cancel has stopped the runtime.
# Classic runs with COVEN_CODE_UI unset; Next with COVEN_CODE_UI=next. Time
# limits are generous because this also runs on debug builds and loaded hosts.

register_case tc_classic_cancel

_cq_lines() { # file
  if [ -f "$1" ]; then wc -l <"$1" | tr -d ' '; else echo 0; fi
}

# Start the TUI for one renderer against the fake API.
_cq_launch() { # renderer
  local var ui=""
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$CQ_SCRATCH/project") && env"
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  if [ "$1" = next ]; then
    ui="COVEN_CODE_UI=next"; TUI_BOOT_STRING="preview"; CQ_BUSY="working"
  else
    # Classic's spinner row reads "<label>… · <elapsed>" only while a turn runs.
    TUI_BOOT_STRING="Coven v"; CQ_BUSY="… · "
  fi
  # shellcheck disable=SC2086 # an empty selector adds nothing
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$CQ_HOME" "COVEN_HOME=$CQ_HOME/.coven" $ui \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$CQ_PORT")"
  tui_start
}

_cq_j1() { # renderer
  local r="$1" log="$CQ_SCRATCH/project/j1-$1.log" waited=0 at_escape settled at3 at6
  tui_type "run: for i in \$(seq 30); do echo \$i >> j1-$r.log; sleep 1; done"
  sleep 0.3; tui_keys Enter
  if ! wait_for "allow once" 15; then
    _fail "J1 $r: the command asks for approval" "$(tui_capture)"; return 0
  fi
  sleep 0.7; tui_keys y
  while [ "$waited" -lt 200 ] && [ "$(_cq_lines "$log")" -lt 3 ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  if [ "$(_cq_lines "$log")" -lt 3 ]; then
    _fail "J1 $r: the approved command runs" "$(tui_capture)"; return 0
  fi
  at_escape="$(_cq_lines "$log")"
  tui_keys Escape
  if wait_for "ancel" 5; then _pass "J1 $r: Escape is acknowledged"
  else _fail "J1 $r: Escape is acknowledged" "$(tui_capture)"; fi
  if wait_absent "ancelling" 30; then _pass "J1 $r: the cancel settles"
  else _fail "J1 $r: the cancel settles" "$(tui_capture)"; fi
  settled="$(_cq_lines "$log")"; sleep 3
  at3="$(_cq_lines "$log")"; sleep 3
  at6="$(_cq_lines "$log")"
  log "       J1 $r lines: at Escape $at_escape, at settle $settled, +3 s $at3, +6 s $at6"
  # The command writes a line a second and would run for 30; 3 more lines
  # leave room for a loaded host.
  if [ "$settled" -le $((at_escape + 3)) ]; then
    _pass "J1 $r: the cancel stops the command instead of waiting for it"
  else
    _fail "J1 $r: the cancel stops the command instead of waiting for it" \
      "at Escape $at_escape lines, at settle $settled"
  fi
  assert_eq "$at3" "$settled" "J1 $r: the command has stopped 3 s after the cancel settles"
  assert_eq "$at6" "$settled" "J1 $r: the command has stopped 6 s after the cancel settles"
}

_cq_j2() { # renderer
  local r="$1" n=0 restarted="" s
  tui_type "flood: 200000 hold"; sleep 0.3; tui_keys Enter
  if ! wait_for "flood line" 30; then
    _fail "J2 $r: the burst starts" "$(tui_capture)"; return 0
  fi
  if wait_for "$CQ_BUSY" 5; then _pass "J2 $r: the turn shows as running"
  else _fail "J2 $r: the turn shows as running" "$(tui_capture)"; fi
  tui_keys Escape
  if wait_for "ancel" 5; then _pass "J2 $r: Escape is acknowledged"
  else _fail "J2 $r: Escape is acknowledged" "$(tui_capture)"; fi
  # The backlog keeps arriving after the cancel; watch it for 10 s.
  while [ "$n" -lt 20 ]; do
    sleep 0.5; n=$((n + 1)); s="$(tui_capture)"
    if grep -qF -- "$CQ_BUSY" <<<"$s"; then restarted="$s"; break; fi
  done
  if [ -z "$restarted" ]; then _pass "J2 $r: the backlog does not restart the turn"
  else _fail "J2 $r: the backlog does not restart the turn (after ${n} half-seconds)" "$restarted"; fi
  # Wait for the turn to end, then for anything still queued behind it.
  n=0
  while [ "$n" -lt 600 ]; do
    s="$(tui_capture)"
    if ! grep -qF -- "$CQ_BUSY" <<<"$s" && ! grep -qF -- "ancelling" <<<"$s"; then break; fi
    sleep 0.4; n=$((n + 1))
  done
  sleep 3; s="$(tui_capture)"
  log "       J2 $r ended after about $((n * 4 / 10)) s"
  assert_absent "$s" "$CQ_BUSY" "J2 $r: the turn stays stopped"
  assert_contains "$s" "Cancelled" "J2 $r: the status settles to cancelled"
}

tc_classic_cancel() {
  describe "Cancel a turn in each renderer"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  CQ_SCRATCH="$(mktemp -d)"
  mkdir -p "$CQ_SCRATCH/project" "$CQ_SCRATCH/markers"
  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$CQ_SCRATCH/port" "$CQ_SCRATCH/requests.log" "$CQ_SCRATCH/markers" 2>/dev/null &
  local fake_pid=$! waited=0 r
  while [ "$waited" -lt 50 ] && [ ! -s "$CQ_SCRATCH/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  if [ ! -s "$CQ_SCRATCH/port" ]; then
    _fail "the fake API starts"; kill "$fake_pid" 2>/dev/null; rm -rf "$CQ_SCRATCH"; return 0
  fi
  CQ_PORT="$(cat "$CQ_SCRATCH/port")"
  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"

  for r in classic next; do
    CQ_HOME="$CQ_SCRATCH/home-$r"
    mkdir -p "$CQ_HOME/.coven-code"
    printf '%s\n' '{"hasCompletedOnboarding": true}' >"$CQ_HOME/.coven-code/settings.json"
    if _cq_launch "$r"; then _cq_j1 "$r"; fi
    tui_stop
    if _cq_launch "$r"; then _cq_j2 "$r"; fi
    tui_stop
  done

  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  rm -rf "$CQ_SCRATCH"
}
