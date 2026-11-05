#!/usr/bin/env bash
# shellcheck shell=bash
#
# Several tools in one response (#411, #451). Next's rows say what each tool is
# doing rather than waiting for the whole response to end, and two edits to one
# file both land in either renderer. Quitting during a turn says what is not
# known (#412). A local fake of the Messages API drives the real client, query
# loop and tools.

register_case tc_tool_states

# The screen line about a tool. Rows read "Bash sleep 4" and "Read ...notes.txt",
# which the JSON in the prompt that named the tool never does.
_states_row() { # ERE
  tui_capture | grep -E "$1" | head -1
}

_states_wait_row() { # row ERE, state ERE, message
  local waited=0 row=""
  while [ "$waited" -lt 150 ]; do
    row="$(_states_row "$1")"
    if grep -qE -- "$2" <<<"$row"; then _pass "$3"; return 0; fi
    sleep 0.1; waited=$((waited + 1))
  done
  _fail "$3" "row: $row"$'\n'"$(tui_capture)"
}

_states_launch() { # ui, boot, scratch
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$3/project") && env"
  local var
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' \
    "HOME=$3/home" "COVEN_HOME=$3/home/.coven" "COVEN_CODE_UI=$1" \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$(cat "$3/port")")"
  TUI_BOOT_STRING="$2"
  tui_start
}

# Next: a quick read beside a slow command, then the same two rows afterwards.
_states_rows() { # scratch
  local notes="$1/project/notes.txt"
  tui_type "tools: [[\"Read\",{\"file_path\":\"$notes\"}],[\"Bash\",{\"command\":\"sleep 4\"}]]"
  tui_keys Enter
  if wait_for "Approve Bash" 15; then _pass "next: the slow command asks first"
  else _fail "next: the slow command asks first" "$(tui_capture)"; fi
  sleep 0.8; tui_keys y
  _states_wait_row "Bash sleep 4" 'running' "next: the command is running"
  _states_wait_row 'Read .*notes\.txt' 'done' "next: the read shows done while the command runs"
  assert_matches "$(_states_row "Bash sleep 4")" 'running' "next: the command still runs at that point"
  _states_wait_row "Bash sleep 4" 'done [0-9]+s' "next: the command ends showing how long it took"
}

# Either renderer: two edits to one file in one response, each approved.
_states_edits() { # ui, scratch
  local file="$2/project/edits.txt"
  printf 'first line\nsecond line\n' >"$file"
  tui_type "tools: [[\"Edit\",{\"file_path\":\"$file\",\"old_string\":\"first line\",\"new_string\":\"FIRST\"}],[\"Edit\",{\"file_path\":\"$file\",\"old_string\":\"second line\",\"new_string\":\"SECOND\"}]]"
  tui_keys Enter
  sleep 2.5; tui_keys y
  sleep 2.5; tui_keys y
  local waited=0
  while [ "$waited" -lt 150 ] && [ "$(tr '\n' '|' <"$file")" != 'FIRST|SECOND|' ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  assert_eq "$(tr '\n' '|' <"$file")" 'FIRST|SECOND|' "$1: both edits to one file land"
  assert_eq "$(grep -c 'changed while waiting' "$2/requests.log")" "0" \
    "$1: neither edit is refused as a stale review"
}

# Next: quitting during a turn says what is not known, and an idle quit says nothing.
_states_quit() { # scratch
  if _states_launch next preview "$1"; then
    tui_type "run: sleep 30"; tui_keys Enter
    if wait_for "Approve Bash" 15; then sleep 0.8; tui_keys y; fi
    _states_wait_row "Bash sleep 30" 'running' "next: a command is running before the quit"
    tui_keys C-d; sleep 0.25; tui_keys C-d
    if wait_for "No result from Bash sleep 30" 10; then
      _pass "next: quitting names the tool with no result"
    else
      _fail "next: quitting names the tool with no result" "$(tui_capture)"
    fi
    local said; said="$(tui_capture)"
    assert_contains "$said" "nothing carries it on" "next: quitting says nothing carries the turn on"
    assert_contains "$said" "is unknown" "next: quitting says the outcome is unknown"
    assert_contains "$said" "coven-code --resume" "next: quitting gives the way back"
  fi
  tui_stop
  if _states_launch next preview "$1"; then
    tui_keys C-d; sleep 0.25; tui_keys C-d; sleep 1.5
    assert_absent "$(tui_capture)" "Quit during a turn" "next: an idle quit says nothing about a turn"
  fi
  tui_stop
}

tc_tool_states() {
  describe "Several tools in one response"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/project" "$scratch/markers"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  printf 'notes\n' >"$scratch/project/notes.txt"

  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$scratch/port" "$scratch/requests.log" "$scratch/markers" 2>/dev/null &
  local fake_pid=$!
  local waited=0
  while [ "$waited" -lt 50 ] && [ ! -s "$scratch/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"
  if _states_launch next preview "$scratch"; then
    _states_rows "$scratch"
    sleep 1
    _states_edits next "$scratch"
    tui_keys C-c; sleep 0.3; tui_keys C-c
  fi
  tui_stop
  if _states_launch classic "Coven v" "$scratch"; then
    _states_edits classic "$scratch"
    tui_keys C-c; sleep 0.3; tui_keys C-c
  fi
  tui_stop

  _states_quit "$scratch"

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
  rm -rf "$scratch"
}
