#!/usr/bin/env bash
# shellcheck shell=bash
#
# Quitting during a turn, in Classic and in Next (#408). A local fake of the
# Messages API drives real turns through the real client and query loop.
# Quitting mid-turn with Ctrl+D twice must exit, give the terminal back, stop
# the turn and save its prompt and the output it delivered, and must not wait
# long while an approved command runs. Only Next reports on the turn after it
# exits. When the save fails, both print the same one-line warning after the
# terminal is restored (decision 6 on #496). (A unit test covers a turn that
# ignores the stop: the exit waits 2 s at most.) Classic runs with COVEN_CODE_UI unset;
# Next, which case 18 also covers, with COVEN_CODE_UI=next. Time limits are
# generous because this also runs on debug builds and loaded hosts.

register_case tc_classic_quit_save

# The saved session file that names `text`, if any.
_cqs_saved_with() { # home, text
  find "$1" -type f -path '*/sessions/*.json' -exec grep -lF -- "$2" {} + 2>/dev/null | head -n 1
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
      _cqs_command "$r" "$home" "$port" "$scratch/project"
      tui_stop
      _cqs_unsaved "$r" "$scratch/home-$r-unsaved" "$port" "$scratch/project"
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
  local r="$1" home="$2" saved pane
  _cqs_launch "$@" || return 0
  tui_type "flood: 2000 hold"; sleep 0.3; tui_keys Enter
  if ! wait_for "held line" 30; then
    _fail "$r: the held turn starts" "$(tui_capture)"; return 0
  fi
  tui_quit_and_check "$r: Ctrl+D twice during a turn" C-d C-d
  # The pane now shows the shell, with whatever the TUI printed as it exited:
  # Next's report on the turn and the resume command; nothing from Classic.
  pane="$(tui_capture)"
  if [ "$r" = next ]; then
    assert_contains "$pane" "Quit during a turn" "$r: the exit report follows the exit"
    assert_contains "$pane" "Pick it up with" "$r: the resume command follows the exit"
  else
    assert_absent "$pane" "Quit during a turn" "$r: no exit report follows the exit"
    assert_absent "$pane" "Pick it up with" "$r: no resume command follows the exit"
  fi
  saved="$(_cqs_saved_with "$home" "flood: 2000 hold")"
  if [ -n "$saved" ]; then _pass "$r: quitting during a turn keeps the prompt"
  else _fail "$r: quitting during a turn keeps the prompt" "no saved session names it"; fi
  if [ -n "$saved" ] && grep -q 'flood line' "$saved"; then
    _pass "$r: quitting during a turn keeps the answer so far"
  else
    _fail "$r: quitting during a turn keeps the answer so far" "${saved:-no saved session}"
  fi
}

# Quitting while an approved command runs exits promptly; 8 s leaves room for a
# loaded host. The command stops with the turn here, so this does not reach the
# 2 s bound. The prompt must be saved.
_cqs_command() { # renderer, home, port, project
  local r="$1" home="$2" saved
  _cqs_launch "$@" || return 0
  tui_type "run: sleep 20"; sleep 0.3; tui_keys Enter
  if ! wait_for "allow once" 15; then
    _fail "$r: a Bash call asks for approval" "$(tui_capture)"; return 0
  fi
  sleep 0.7; tui_keys y; sleep 1.5
  tui_quit_and_check "$r: Ctrl+D twice while a command runs" C-d C-d
  if [ "$TUI_QUIT_TENTHS" -lt 80 ]; then
    _pass "$r: quitting exits within 8 s while a command runs"
  else
    _fail "$r: quitting exits within 8 s while a command runs" "took about $((TUI_QUIT_TENTHS / 10))s"
  fi
  saved="$(_cqs_saved_with "$home" "run: sleep 20")"
  if [ -n "$saved" ]; then _pass "$r: quitting while a command runs keeps the prompt"
  else _fail "$r: quitting while a command runs keeps the prompt" "no saved session names it"; fi
}

# Quitting during a turn when the session cannot be saved: the sessions
# directory is read-only. Once the terminal is back, the pane shows one line
# saying the session was not saved, in both renderers, and Classic prints
# nothing else.
_cqs_unsaved() { # renderer, home, port, project
  local r="$1" home="$2" pane
  local warning="Session not saved; any earlier saved copy lacks this turn."
  if [ "$(id -u)" = 0 ]; then
    _skip "$r: a read-only sessions directory does not stop root"; return 0
  fi
  mkdir -p "$home/.coven-code/sessions"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$home/.coven-code/settings.json"
  chmod 500 "$home/.coven-code/sessions"
  if _cqs_launch "$@"; then
    tui_type "flood: 2000 hold"; sleep 0.3; tui_keys Enter
    if wait_for "held line" 30; then
      tui_quit_and_check "$r: Ctrl+D twice during a turn that cannot be saved" C-d C-d
      pane="$(tui_capture)"
      assert_eq "$(grep -cF -- "$warning" <<<"$pane")" "1" \
        "$r: one line after the exit says the session was not saved"
      assert_absent "$pane" "Pick it up with" "$r: no resume command follows a failed save"
      if [ "$r" = next ]; then
        assert_contains "$pane" "Quit during a turn" "$r: the exit report still follows the exit"
      else
        assert_absent "$pane" "Quit during a turn" "$r: no exit report follows the exit"
      fi
      assert_eq "$(find "$home/.coven-code/sessions" -type f | wc -l | tr -d ' ')" "0" \
        "$r: nothing was saved"
    else
      _fail "$r: the held turn starts" "$(tui_capture)"
    fi
  fi
  chmod 700 "$home/.coven-code/sessions"
}
