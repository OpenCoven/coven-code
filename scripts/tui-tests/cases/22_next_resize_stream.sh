#!/usr/bin/env bash
# shellcheck shell=bash
#
# Resizing while a turn streams (#417). A burst of text deltas keeps the turn busy
# while the terminal changes size, including sizes too small to draw anything.
# Nothing may be lost, the draft stays as typed and the terminal comes back whole.

register_case tc_next_resize_stream

_resize_saved_count() { # home
  find "$1" -type f -path '*/sessions/*.json' -exec grep -o 'flood line' {} + 2>/dev/null \
    | wc -l | tr -d ' '
}

tc_next_resize_stream() {
  describe "Next: resizing while a turn streams"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/project" "$scratch/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"

  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$scratch/port" "$scratch/requests.log" "$scratch/markers" 2>/dev/null &
  local fake_pid=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -s "$scratch/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" var
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/project") && env"
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" COVEN_CODE_UI=next \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$(cat "$scratch/port")")"
  TUI_BOOT_STRING=preview

  if tui_start; then
    local count=150000 running sizes_while_working=0 size
    running="$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')"
    tui_type "flood: $count"; sleep 0.3; tui_keys Enter
    if wait_for "flood line" 20; then _pass "the turn is streaming"
    else _fail "the turn is streaming" "$(tui_capture)"; fi

    tui_type "kept draft"
    for size in "80 24" "40 10" "200 50" "20 5" "120 40" "1 1" "2 2" "160 30" "10 3" "80 24" "120 40"; do
      _tmux resize-window -t "$TUI_SESSION" -x "${size% *}" -y "${size#* }"
      sleep 0.25
      if tui_capture | grep -q "working"; then sizes_while_working=$((sizes_while_working + 1)); fi
    done
    # The smallest sizes have no room for the word, so not every look can see it.
    if [ "$sizes_while_working" -ge 3 ]; then
      _pass "sizes changed while the turn was running ($sizes_while_working seen working)"
    else
      _fail "sizes changed while the turn was running ($sizes_while_working seen working)" "$(tui_capture)"
    fi

    assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" "$running" \
      "the process survives every size"
    # tmux itself sometimes leaves the window at an earlier size when the host
    # is loaded. Reapply the last size so the checks below judge the app, and
    # say so if tmux never takes it.
    local applied tries=0
    applied="$(_tmux display-message -p -t "$TUI_SESSION" '#{window_width}x#{window_height}')"
    while [ "$applied" != "120x40" ] && [ "$tries" -lt 5 ]; do
      _tmux resize-window -t "$TUI_SESSION" -x 120 -y 40; sleep 0.25; tries=$((tries + 1))
      applied="$(_tmux display-message -p -t "$TUI_SESSION" '#{window_width}x#{window_height}')"
    done
    if [ "$tries" -gt 0 ]; then
      info "tmux needed $tries more resize(s) to reach 120x40 (now $applied)"
    fi
    assert_eq "$applied" "120x40" "tmux applied the final size"
    if wait_absent "working" 240; then _pass "the turn finishes"
    else _fail "the turn finishes" "$(tui_capture)"; fi
    local screen; screen="$(tui_capture)"
    assert_contains "$screen" "kept draft" "the draft is as typed"
    assert_contains "$screen" "flood line" "the answer is on screen"
    assert_absent "$screen" "panicked" "nothing panicked"

    # The last row of the frame is the composer at the size the terminal has now.
    local rows; rows="$(grep -c '' <<<"$screen")"
    assert_eq "$rows" "40" "the frame fills the final height"

    # Quitting saves the turn; the count proves no delta was dropped.
    tui_keys C-u; sleep 0.2; tui_keys C-c; sleep 0.3; tui_keys C-c
    waited=0
    while [ "$waited" -lt 100 ] && \
      [ "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" = "$running" ]; do
      sleep 0.1; waited=$((waited + 1))
    done
    assert_eq "$(_resize_saved_count "$scratch/home")" "$count" "every delta reached the saved turn"
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"; TUI_BOOT_STRING="$saved_boot"
  kill "$fake_pid" 2>/dev/null; wait "$fake_pid" 2>/dev/null
  rm -rf "$scratch"
}
