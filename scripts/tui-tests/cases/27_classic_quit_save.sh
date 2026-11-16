#!/usr/bin/env bash
# shellcheck shell=bash
#
# Quitting during a turn, in Classic and in Next (#408). A local fake of the
# Messages API drives real turns through the real client and query loop.
# Quitting mid-turn with Ctrl+D twice must exit, give the terminal back, stop
# the turn and save its prompt and the output it delivered, and must not wait
# long for a tool that ignores the stop. Classic runs with COVEN_CODE_UI unset;
# Next, which case 18 also covers, with COVEN_CODE_UI=next. Time limits are
# generous because this also runs on debug builds and loaded hosts.

register_case tc_classic_quit_save

# Wall-clock time in tenths of a second.
_cqs_now_tenths() { python3 -c 'import time; print(int(time.time() * 10))'; }

# The saved session file that names `text`, if any.
_cqs_saved_with() { # home, text
  find "$1" -type f -path '*/sessions/*.json' -exec grep -lF -- "$2" {} + 2>/dev/null | head -n 1
}

# Press the quit keys, wait for the TUI to exit, then check it gave the terminal
# back. Sets CQS_QUIT_TENTHS to how long the exit took after the last key.
_cqs_quit() { # how, keys...
  local how="$1" key passes=0 running start
  shift
  running="$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')"
  for key in "$@"; do tui_keys "$key"; start="$(_cqs_now_tenths)"; sleep 0.25; done
  while [ "$passes" -lt 300 ] && \
    [ "$(_tmux display-message -p -t "$TUI_SESSION" '#{pane_current_command}')" = "$running" ]; do
    sleep 0.1; passes=$((passes + 1))
  done
  CQS_QUIT_TENTHS=$(($(_cqs_now_tenths) - start))
  if [ "$passes" -lt 300 ]; then _pass "$how exits (after about $((CQS_QUIT_TENTHS / 10))s)"
  else _fail "$how exits" "$(tui_capture)"; fi
  assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{alternate_on}')" "0" \
    "$how leaves the alternate screen"
  assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{cursor_flag}')" "1" \
    "$how restores the cursor"
  assert_eq "$(_tmux display-message -p -t "$TUI_SESSION" '#{mouse_any_flag}')" "0" \
    "$how turns mouse reporting off"
}

# Start the TUI for one renderer against the fake API, with its own home.
_cqs_launch() { # renderer, home, port, project
  local var ui=""
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$4") && env"
  for var in OPENAI_API_KEY COVEN_CODE_PROVIDER COVEN_CODE_API_BASE COVEN_CODE_HOME \
    COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  if [ "$1" = next ]; then ui="COVEN_CODE_UI=next"; TUI_BOOT_STRING="preview"
  else TUI_BOOT_STRING="Coven v"; fi
  # shellcheck disable=SC2086 # an empty selector adds nothing
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$2" "COVEN_HOME=$2/.coven" $ui \
    ANTHROPIC_API_KEY=fake-key "ANTHROPIC_BASE_URL=http://127.0.0.1:$3")"
  tui_start
}

tc_classic_quit_save() {
  describe "Quit during a turn in each renderer"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  if ! command -v python3 >/dev/null 2>&1; then _skip "python3 not installed"; return 0; fi

  local scratch; scratch="$(mktemp -d)"
  mkdir -p "$scratch/project" "$scratch/markers"
  python3 -B "${BASH_SOURCE[0]%/*}/../fake_anthropic.py" \
    "$scratch/port" "$scratch/requests.log" "$scratch/markers" 2>/dev/null &
  local fake_pid=$! waited=0 r home port
  while [ "$waited" -lt 50 ] && [ ! -s "$scratch/port" ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING"

  if [ -s "$scratch/port" ]; then
    port="$(cat "$scratch/port")"
    for r in classic next; do
      home="$scratch/home-$r"
      mkdir -p "$home/.coven-code"
      printf '%s\n' '{"hasCompletedOnboarding": true}' >"$home/.coven-code/settings.json"
      _cqs_held "$r" "$home" "$port" "$scratch/project"
      tui_stop
      _cqs_stuck "$r" "$home" "$port" "$scratch/project"
      tui_stop
    done
  else
    _fail "the fake API starts"
  fi
  kill "$fake_pid" 2>/dev/null
  wait "$fake_pid" 2>/dev/null
  TUI_LAUNCH_PREFIX="$saved_prefix" TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

# Quitting while text is still arriving keeps the prompt and the answer so far.
# The fake holds the stream open after its burst, so the turn is still running
# when the keys land.
_cqs_held() { # renderer, home, port, project
  local r="$1" home="$2" saved
  _cqs_launch "$@" || return 0
  tui_type "flood: 2000 hold"; sleep 0.3; tui_keys Enter
  if ! wait_for "held line" 30; then
    _fail "$r: the held turn starts" "$(tui_capture)"; return 0
  fi
  _cqs_quit "$r: Ctrl+D twice during a turn" C-d C-d
  saved="$(_cqs_saved_with "$home" "flood: 2000 hold")"
  if [ -n "$saved" ]; then _pass "$r: quitting during a turn keeps the prompt"
  else _fail "$r: quitting during a turn keeps the prompt" "no saved session names it"; fi
  if [ -n "$saved" ] && grep -q 'flood line' "$saved"; then
    _pass "$r: quitting during a turn keeps the answer so far"
  else
    _fail "$r: quitting during a turn keeps the answer so far" "${saved:-no saved session}"
  fi
}

# Quitting while a tool ignores the stop still exits within the bound. Quitting
# waits up to 2 s for the turn; 8 s leaves room for a loaded host and still
# fails if that wait grows. Only the prompt can be saved here.
_cqs_stuck() { # renderer, home, port, project
  local r="$1" home="$2" saved
  _cqs_launch "$@" || return 0
  tui_type "run: sleep 20"; sleep 0.3; tui_keys Enter
  if ! wait_for "allow once" 15; then
    _fail "$r: a Bash call asks for approval" "$(tui_capture)"; return 0
  fi
  sleep 0.7; tui_keys y; sleep 1.5
  _cqs_quit "$r: Ctrl+D twice while a tool ignores the stop" C-d C-d
  if [ "$CQS_QUIT_TENTHS" -lt 80 ]; then
    _pass "$r: quitting exits within 8 s while a turn does not stop"
  else
    _fail "$r: quitting exits within 8 s while a turn does not stop" "took about $((CQS_QUIT_TENTHS / 10))s"
  fi
  saved="$(_cqs_saved_with "$home" "run: sleep 20")"
  if [ -n "$saved" ]; then _pass "$r: a turn that did not stop in time keeps its prompt"
  else _fail "$r: a turn that did not stop in time keeps its prompt" "no saved session names it"; fi
}
