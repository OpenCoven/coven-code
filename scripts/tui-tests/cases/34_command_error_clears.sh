#!/usr/bin/env bash
# shellcheck shell=bash
#
# A slash command's error leaves the status row when a later slash command
# runs (#529), in Classic and in Next. A disposable COVEN_HOME holds a
# `coven.sock` that is a plain file, as a killed daemon can leave it, so a
# client is found and every request fails. `/coven sessions` then puts
# `Error: Coven daemon offline. ...` on the status row. `/coven status`
# succeeds: it prints `Coven daemon: offline` to the transcript, and the
# earlier error is gone from the status row. Classic shows the status row
# text only before its first turn, which is the case here. Classic runs with
# COVEN_CODE_UI unset and no provider; nothing listens anywhere.

register_case tc_command_error_clears

_cec_journey() { # renderer
  local r="$1" scratch
  scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/coven" "$scratch/project"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"
  : >"$scratch/coven/coven.sock"

  local saved_prefix="$TUI_LAUNCH_PREFIX" saved_boot="$TUI_BOOT_STRING" var ui=""
  TUI_LAUNCH_PREFIX="cd $(printf '%q' "$scratch/project") && env"
  for var in ANTHROPIC_API_KEY ANTHROPIC_BASE_URL OPENAI_API_KEY COVEN_CODE_PROVIDER \
    COVEN_CODE_API_BASE COVEN_CODE_HOME COVEN_CODE_TEST_HOME COVEN_CODE_CLAUDE_BIN \
    COVEN_PARENT COVEN_DAEMON_SOCKET COVEN_CODE_UI; do
    TUI_LAUNCH_PREFIX+=" -u $var"
  done
  if [ "$r" = next ]; then ui="COVEN_CODE_UI=next"; TUI_BOOT_STRING="preview"
  else TUI_BOOT_STRING="Coven v"; fi
  # shellcheck disable=SC2086 # an empty selector adds nothing
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$scratch/home" "COVEN_HOME=$scratch/coven" $ui)"

  if tui_start; then
    tui_type "/coven sessions"; sleep 0.3; tui_keys Enter
    if wait_for "Error: Coven daemon offline" 10; then
      _pass "$r: a failing command puts its error on the status row"
      tui_type "/coven status"; sleep 0.3; tui_keys Enter
      if wait_for "Coven daemon: offline" 10; then
        _pass "$r: the next command runs and prints its answer"
        sleep 0.5
        assert_absent "$(tui_capture)" "Error: Coven daemon offline" \
          "$r: after the next command the earlier error is gone from the status row"
      else
        _fail "$r: the next command runs and prints its answer" "$(tui_capture)"
      fi
    else
      _fail "$r: a failing command puts its error on the status row" "$(tui_capture)"
    fi
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

tc_command_error_clears() {
  describe "A command's error leaves the status row when a later command runs"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  _cec_journey classic
  _cec_journey next
}
