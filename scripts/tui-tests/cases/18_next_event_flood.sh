#!/usr/bin/env bash
# shellcheck shell=bash
#
# Event pressure (#408): a local fake of the Messages API sends a burst of text
# deltas through the real client and query loop, far faster than a real provider.
# Nothing may be lost, keys and Escape must stay responsive, and quitting mid-turn
# must exit, restore the terminal and keep what the turn delivered, and must not
# wait long for a tool that ignores the stop. Time limits are generous because this also
# runs on debug builds and loaded hosts; they catch a hang, not a slow frame.

register_case tc_next_event_flood

# Occurrences of the fake's flood text in this home's saved sessions.
_flood_saved_count() { # home
  find "$1" -type f -path '*/sessions/*.json' -exec grep -o 'flood line' {} + 2>/dev/null \
    | wc -l | tr -d ' '
}

# Wall-clock time in tenths of a second.
_flood_now_tenths() { python3 -c 'import time; print(int(time.time() * 10))'; }

# Press the quit keys, wait for the TUI to exit, then check it gave the terminal back.
# Sets FLOOD_QUIT_TENTHS to how long the exit took after the last key, in tenths
# of a second by the wall clock. Each pass of the wait also runs tmux, so the
# pass count gives up later than 30 s; it only stops a hung exit.
_flood_quit() { # how, keys...
  local how="$1" key passes=0 running start
  shift
  # The running process's name, whatever the binary under test is called.
  running="$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')"
  for key in "$@"; do tui_keys "$key"; sleep 0.25; done
  start="$(_flood_now_tenths)"
  while [ "$passes" -lt 300 ] && \
    [ "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" = "$running" ]; do
    sleep 0.1; passes=$((passes + 1))
  done
  FLOOD_QUIT_TENTHS=$(($(_flood_now_tenths) - start))
  if [ "$passes" -lt 300 ]; then _pass "$how exits (after about $((FLOOD_QUIT_TENTHS / 10))s)"
  else _fail "$how exits" "$(tui_capture)"; fi
  assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{alternate_on}')" "0" \
    "$how leaves the alternate screen"
  assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{cursor_flag}')" "1" \
    "$how restores the cursor"
  assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{mouse_any_flag}')" "0" \
    "$how turns mouse reporting off"
}

_flood_wait_saved() { # home, count
  local waited=0
  while [ "$waited" -lt 100 ] && [ "$(_flood_saved_count "$1")" -lt "$2" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
}

tc_next_event_flood() {
  describe "Next under a burst of events (COVEN_CODE_UI=next)"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/acme-next" "$scratch/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"

  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$scratch/port" "$scratch/requests.log" "$scratch/markers" 2>/dev/null &
  local fake_pid=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -s "$scratch/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/acme-next") && env"
  local var
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$(cat "$scratch/port" 2>/dev/null)")"
  TUI_BOOT_STRING="preview"

  if [ -s "$scratch/port" ] && tui_start; then
    _next_event_flood_journey "$scratch/home"
  fi
  tui_stop
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

_next_event_flood_journey() {
  local home="$1" count=20000 s

  # ---- every delta of a finished burst reaches the transcript that is saved
  tui_type "flood: $count"; sleep 0.3; tui_keys Enter
  wait_for "flood line" 20 >/dev/null
  if wait_absent "working" 120; then _pass "a burst of $count deltas finishes"
  else _fail "a burst of $count deltas finishes" "$(tui_capture)"; fi
  _flood_wait_saved "$home" "$count"
  assert_eq "$(_flood_saved_count "$home")" "$count" "no delta of the burst is lost"

  # ---- keys and Escape stay responsive while a burst is being delivered
  tui_type "flood: 300000"; sleep 0.3; tui_keys Enter
  if wait_for "working" 20; then _pass "a larger burst starts"
  else _fail "a larger burst starts" "$(tui_capture)"; fi
  wait_for "flood line" 20 >/dev/null
  tui_type "Q7"
  if wait_for "Q7" 5; then _pass "a typed key paints during the burst"
  else _fail "a typed key paints during the burst" "$(tui_capture)"; fi
  s="$(tui_capture)"
  assert_contains "$s" "working" "the burst was still running when the key painted"
  tui_keys Escape
  if wait_for "cancelling" 5; then _pass "Escape is acknowledged during the burst"
  else _fail "Escape is acknowledged during the burst" "$(tui_capture)"; fi
  if wait_absent "cancelling" 180; then _pass "the cancelled turn settles"
  else _fail "the cancelled turn settles" "$(tui_capture)"; fi
  assert_contains "$(tui_capture)" "Q7" "the draft survives the cancelled burst"
  tui_keys C-u; sleep 0.2

  # ---- quitting during a burst exits, restores the terminal and keeps the turn
  tui_stop
  tui_start || return 0
  tui_type "flood: 300001"; sleep 0.3; tui_keys Enter
  wait_for "working" 20 >/dev/null
  wait_for "flood line" 20 >/dev/null
  _flood_quit "Ctrl+D twice during a burst" C-d C-d
  local saved
  saved="$(find "$home" -type f -path '*/sessions/*.json' -exec grep -l 'flood: 300001' {} + 2>/dev/null)"
  if [ -n "$saved" ]; then _pass "quitting during a burst keeps the prompt"
  else _fail "quitting during a burst keeps the prompt" "no saved session names it"; fi
  if [ -n "$saved" ] && grep -q 'flood line' "$saved"; then _pass "quitting during a burst keeps the partial answer"
  else _fail "quitting during a burst keeps the partial answer" "${saved:-no saved session}"; fi

  # ---- the first Ctrl+C cancels; the backlog must not swallow the next two
  tui_stop
  tui_start || return 0
  tui_type "flood: 300000"; sleep 0.3; tui_keys Enter
  wait_for "working" 20 >/dev/null
  wait_for "flood line" 20 >/dev/null
  tui_keys C-c
  # A fast build may have settled by the first look, hence "cancelling" or "Cancelled".
  if wait_for "ancel" 5; then _pass "Ctrl+C cancels during the burst"
  else _fail "Ctrl+C cancels during the burst" "$(tui_capture)"; fi
  _flood_quit "Ctrl+C twice more during a burst" C-c C-c

  # ---- quitting while a tool ignores the stop still exits within the bound.
  # Quitting waits up to 2 s for the turn; 8 s leaves room for a loaded host and
  # still fails if that wait grows. This bounds it from above only: the partial
  # answer check after a burst is what fails if the wait goes away. Only the
  # prompt can be saved here.
  tui_stop
  tui_start || return 0
  tui_type "run: sleep 20"; sleep 0.3; tui_keys Enter
  if wait_for "Approve Bash" 15; then
    sleep 0.7; tui_keys y; sleep 1.5
    _flood_quit "Ctrl+D twice while a tool ignores the stop" C-d C-d
    if [ "$FLOOD_QUIT_TENTHS" -lt 80 ]; then
      _pass "quitting exits within 8 s while a turn does not stop"
    else
      _fail "quitting exits within 8 s while a turn does not stop" "took about $((FLOOD_QUIT_TENTHS / 10))s"
    fi
    saved="$(find "$home" -type f -path '*/sessions/*.json' -exec grep -l 'run: sleep 20' {} + 2>/dev/null)"
    if [ -n "$saved" ]; then _pass "a turn that did not stop in time keeps its prompt"
    else _fail "a turn that did not stop in time keeps its prompt" "no saved session names it"; fi
  else
    _fail "a Bash call asks for approval" "$(tui_capture)"
  fi
}
