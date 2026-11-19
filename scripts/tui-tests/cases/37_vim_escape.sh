#!/usr/bin/env bash
# shellcheck shell=bash
#
# Escape takes vim mode out of Insert (#569), in Classic and in Next. `/vim`
# turns vim mode on in Insert, and `hello` is typed. Escape then reaches vim:
# `0` moves to the start of the draft instead of being typed, `i` inserts
# there, and a second Escape, after an insert that did not end at the end of
# the text, leaves Insert without a panic. `A` then appends, so the draft
# reads `say hello world`. Without the fix the keys are typed as text. Classic
# also shows the mode on its status row; Next draws no vim mode, so there the
# draft is the proof. Each Escape is followed by a pause, so it is not read
# together with the next key as Alt+key. No provider; nothing listens.

register_case tc_vim_escape

_vim_escape_journey() { # renderer
  local r="$1" scratch
  scratch="$(mktemp -d)"
  mkdir -p "$scratch/home/.coven-code" "$scratch/project"
  printf '%s\n' '{"hasCompletedOnboarding": true}' >"$scratch/home/.coven-code/settings.json"

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
  TUI_LAUNCH_PREFIX+=" $(printf '%q ' "HOME=$scratch/home" "COVEN_HOME=$scratch/home/.coven" $ui)"

  if tui_start; then
    tui_type "/vim"; sleep 0.3; tui_keys Enter
    sleep 1
    if [ "$r" = classic ]; then
      if wait_for "-- INSERT --" 10; then _pass "$r: /vim starts vim mode in Insert"
      else _fail "$r: /vim starts vim mode in Insert" "$(tui_capture)"; fi
    fi
    tui_type "hello"; sleep 0.3
    tui_keys Escape; sleep 0.5
    if [ "$r" = classic ]; then
      if wait_for "-- NORMAL --" 10; then _pass "$r: Escape leaves Insert"
      else _fail "$r: Escape leaves Insert" "$(tui_capture)"; fi
    fi
    tui_type "0"; sleep 0.2; tui_type "i"; sleep 0.2
    tui_type "say "; sleep 0.3
    tui_keys Escape; sleep 0.5
    tui_type "A"; sleep 0.2
    tui_type " world"
    if wait_for "say hello world" 10; then
      _pass "$r: after Escape, 0, i and A are vim commands and the draft reads 'say hello world'"
    else
      _fail "$r: after Escape, 0, i and A are vim commands and the draft reads 'say hello world'" \
        "$(tui_capture)"
    fi
    assert_absent "$(tui_capture)" "panicked" "$r: leaving an insert before the end does not panic"
  fi
  tui_stop

  TUI_LAUNCH_PREFIX="$saved_prefix"
  TUI_BOOT_STRING="$saved_boot"
  rm -rf "$scratch"
}

tc_vim_escape() {
  describe "Escape takes vim mode out of Insert"
  if ! have_tmux; then _skip "tmux not installed"; return 0; fi
  _vim_escape_journey classic
  _vim_escape_journey next
}
